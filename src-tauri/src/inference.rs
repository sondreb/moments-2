use std::{
    collections::{HashMap, HashSet},
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::OnceLock,
    time::UNIX_EPOCH,
};

use image::{imageops, imageops::FilterType, DynamicImage, GenericImageView, RgbImage};
use ort::{inputs, session::Session, value::Tensor};

use crate::models::FaceCandidate;

const CLASSIFICATION_SIZE: u32 = 224;
const FACE_SIZE: u32 = 640;
const FACE_CONFIDENCE_THRESHOLD: f32 = 0.82;
const FACE_NMS_IOU_THRESHOLD: f32 = 0.35;
const CLASSIFICATION_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const CLASSIFICATION_STD: [f32; 3] = [0.229, 0.224, 0.225];
const CLASSIFICATION_MIN_PROBABILITY: f32 = 0.08;
const CLASSIFICATION_TAG_LIMIT: usize = 4;
const IMAGENET_CLASSES: &str = include_str!("imagenet_classes.txt");

pub fn classify_image(
    model_path: &Path,
    image_path: &Path,
    cache_dir: &Path,
) -> Result<Vec<String>, String> {
    let source = load_rgb_image(image_path)?;
    let input_image = load_or_create_classification_image(
        image_path,
        &source,
        cache_dir,
        CLASSIFICATION_SIZE,
    )?;
    let input = classification_tensor(&input_image.image)?;
    let mut session = load_session(model_path)?;
    let outputs = session
        .run(inputs![input])
        .map_err(|error| format!("image classification inference failed: {error}"))?;
    let (_, scores) = outputs[0]
        .try_extract_tensor::<f32>()
        .map_err(|error| format!("image classification output was not a float tensor: {error}"))?;

    let probabilities = softmax_probabilities(scores);
    let mut ranked = probabilities
        .iter()
        .enumerate()
        .map(|(index, score)| (index, *score))
        .collect::<Vec<_>>();
    ranked.sort_by(|first, second| second.1.total_cmp(&first.1));

    let mut tags = image_descriptor_tags(&source);
    tags.extend(classification_label_tags(&ranked));
    Ok(deduplicate_tags(tags))
}

pub fn detect_faces(
    model_path: &Path,
    image_path: &Path,
    _cache_dir: &Path,
    media_id: &str,
) -> Result<Vec<FaceCandidate>, String> {
    let input_image = load_face_inference_image(image_path)?;
    eprintln!(
        "[face-detect] media={media_id} image={} model={} input={}x{} content={}x{} offset=({:.1},{:.1})",
        image_path.display(),
        model_path.display(),
        input_image.image.width(),
        input_image.image.height(),
        input_image.geometry.content_width,
        input_image.geometry.content_height,
        input_image.geometry.content_x,
        input_image.geometry.content_y,
    );
    let input = face_tensor(&input_image.image)?;
    let mut session = load_session(model_path)?;
    let outputs = session
        .run(inputs![input])
        .map_err(|error| format!("face detection inference failed: {error}"))?;

    let mut named_outputs = HashMap::new();
    let mut output_logs = Vec::new();
    for (name, output) in outputs.iter() {
        let Ok((shape, values)) = output.try_extract_tensor::<f32>() else {
            output_logs.push(format!("{name}: non-f32 tensor skipped"));
            continue;
        };
        output_logs.push(format!(
            "{name}: shape={:?} values={}",
            shape,
            values.len(),
        ));
        named_outputs.insert(
            name.to_string(),
            FaceOutputTensor {
                shape: shape.to_vec(),
                values: values.to_vec(),
            },
        );
    }

    let (detections, decode_logs) = decode_yunet_outputs(
        &named_outputs,
        input_image.image.width() as usize,
        input_image.image.height() as usize,
        &input_image.geometry,
    );
    output_logs.extend(decode_logs);

    eprintln!("[face-detect] media={media_id} outputs: {}", output_logs.join(" | "));

    let faces = face_candidates_from_detections(
        media_id,
        detections,
        input_image.geometry,
    );
    let confidences = faces
        .iter()
        .take(5)
        .map(|face| format!("{:.4}", face.confidence))
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!(
        "[face-detect] media={media_id} finalFaces={} topConfidences=[{}] threshold={} nmsIou={}",
        faces.len(),
        confidences,
        FACE_CONFIDENCE_THRESHOLD,
        FACE_NMS_IOU_THRESHOLD,
    );

    Ok(faces)
}

fn load_session(model_path: &Path) -> Result<Session, String> {
    Session::builder()
        .map_err(|error| format!("failed to create ONNX session: {error}"))?
        .commit_from_file(model_path)
        .map_err(|error| {
            format!(
                "failed to load ONNX model '{}': {error}",
                model_path.display()
            )
        })
}

fn load_rgb_image(path: &Path) -> Result<DynamicImage, String> {
    image::open(path)
        .map_err(|error| format!("failed to decode image '{}': {error}", path.display()))
}

fn load_or_create_classification_image(
    source_path: &Path,
    source: &DynamicImage,
    cache_dir: &Path,
    size: u32,
) -> Result<InferenceImage, String> {
    let cached_path = inference_image_path(source_path, cache_dir, "classification-v2", size)?;
    if cached_path.exists() {
        return Ok(InferenceImage {
            image: load_rgb_image(&cached_path)?,
            geometry: SquareGeometry::from_source(source.width(), source.height(), size),
        });
    }

    let geometry = SquareGeometry::from_source(source.width(), source.height(), size);
    let thumbnail = classification_thumbnail(source, size);
    if let Some(parent) = cached_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create inference cache: {error}"))?;
    }
    thumbnail.save(&cached_path).map_err(|error| {
        format!(
            "failed to save inference thumbnail '{}': {error}",
            cached_path.display()
        )
    })?;

    Ok(InferenceImage {
        image: DynamicImage::ImageRgb8(thumbnail),
        geometry,
    })
}

fn load_face_inference_image(source_path: &Path) -> Result<InferenceImage, String> {
    let source = load_rgb_image(source_path)?;
    let geometry = SquareGeometry::from_source(source.width(), source.height(), FACE_SIZE);
    let canvas = square_thumbnail(&source, FACE_SIZE);

    Ok(InferenceImage {
        image: DynamicImage::ImageRgb8(canvas),
        geometry,
    })
}

fn inference_image_path(
    source_path: &Path,
    cache_dir: &Path,
    bucket: &str,
    size: u32,
) -> Result<PathBuf, String> {
    let metadata = fs::metadata(source_path).map_err(|error| {
        format!(
            "failed to inspect image '{}': {error}",
            source_path.display()
        )
    })?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0);

    let mut hasher = DefaultHasher::new();
    source_path.to_string_lossy().hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    modified.hash(&mut hasher);

    Ok(cache_dir
        .join(bucket)
        .join(format!("{size}px"))
        .join(format!("{:016x}.png", hasher.finish())))
}

fn square_thumbnail(image: &DynamicImage, size: u32) -> RgbImage {
    let resized = image.thumbnail(size, size).to_rgb8();
    let mut canvas = RgbImage::new(size, size);
    let x = (size.saturating_sub(resized.width())) / 2;
    let y = (size.saturating_sub(resized.height())) / 2;
    imageops::replace(&mut canvas, &resized, x.into(), y.into());
    canvas
}

fn classification_thumbnail(image: &DynamicImage, size: u32) -> RgbImage {
    image
        .resize_to_fill(size, size, FilterType::Triangle)
        .to_rgb8()
}

fn classification_tensor(image: &DynamicImage) -> Result<Tensor<f32>, String> {
    let resized = image
        .resize_exact(
            CLASSIFICATION_SIZE,
            CLASSIFICATION_SIZE,
            FilterType::Triangle,
        )
        .to_rgb8();
    let mut input = Vec::with_capacity((3 * CLASSIFICATION_SIZE * CLASSIFICATION_SIZE) as usize);

    for channel in 0..3 {
        for pixel in resized.pixels() {
            let value = pixel[channel] as f32 / 255.0;
            input.push((value - CLASSIFICATION_MEAN[channel]) / CLASSIFICATION_STD[channel]);
        }
    }

    Tensor::from_array((
        [
            1_usize,
            3,
            CLASSIFICATION_SIZE as usize,
            CLASSIFICATION_SIZE as usize,
        ],
        input.into_boxed_slice(),
    ))
    .map_err(|error| format!("failed to build classification tensor: {error}"))
}

fn face_tensor(image: &DynamicImage) -> Result<Tensor<f32>, String> {
    let rgb = image.to_rgb8();
    let width = rgb.width() as usize;
    let height = rgb.height() as usize;
    let mut input = Vec::with_capacity(3 * width * height);

    for source_index in [2_usize, 1, 0] {
        for pixel in rgb.pixels() {
            input.push(pixel[source_index] as f32);
        }
    }

    Tensor::from_array((
        [1_usize, 3, height, width],
        input.into_boxed_slice(),
    ))
    .map_err(|error| format!("failed to build face detection tensor: {error}"))
}

fn face_candidates_from_detections(
    media_id: &str,
    mut detections: Vec<FaceDetection>,
    geometry: SquareGeometry,
) -> Vec<FaceCandidate> {
    detections.sort_by(|first, second| second.confidence.total_cmp(&first.confidence));

    let mut selected: Vec<FaceDetection> = Vec::new();
    for detection in detections {
        if selected
            .iter()
            .all(|candidate| detection.iou(candidate) < FACE_NMS_IOU_THRESHOLD)
        {
            selected.push(detection);
        }
    }

    selected
        .into_iter()
        .enumerate()
        .filter_map(|(index, detection)| {
            let bounds = detection.normalized_bounds(geometry)?;
            Some(FaceCandidate {
                id: format!("{media_id}-face-{}", index + 1),
                media_id: media_id.to_string(),
                name: None,
                confidence: detection.confidence.clamp(0.0, 1.0),
                x: bounds.x,
                y: bounds.y,
                width: bounds.width,
                height: bounds.height,
            })
        })
        .collect()
}

fn decode_yunet_outputs(
    outputs: &HashMap<String, FaceOutputTensor>,
    input_width: usize,
    input_height: usize,
    geometry: &SquareGeometry,
) -> (Vec<FaceDetection>, Vec<String>) {
    let mut detections = Vec::new();
    let mut logs = Vec::new();

    for stride in [8_u32, 16, 32] {
        let cls_name = format!("cls_{stride}");
        let obj_name = format!("obj_{stride}");
        let bbox_name = format!("bbox_{stride}");
        let kps_name = format!("kps_{stride}");

        let Some(cls) = outputs.get(&cls_name) else {
            logs.push(format!("stride={stride}: missing {cls_name}"));
            continue;
        };
        let Some(obj) = outputs.get(&obj_name) else {
            logs.push(format!("stride={stride}: missing {obj_name}"));
            continue;
        };
        let Some(bbox) = outputs.get(&bbox_name) else {
            logs.push(format!("stride={stride}: missing {bbox_name}"));
            continue;
        };
        let Some(kps) = outputs.get(&kps_name) else {
            logs.push(format!("stride={stride}: missing {kps_name}"));
            continue;
        };

        let cols = input_width.div_ceil(stride as usize);
        let rows = input_height.div_ceil(stride as usize);
        let candidate_count = rows * cols;

        let Some(cls_scores) = extract_scores(cls, candidate_count) else {
            logs.push(format!("stride={stride}: could not decode cls tensor {:?}", cls.shape));
            continue;
        };
        let Some(obj_scores) = extract_scores(obj, candidate_count) else {
            logs.push(format!("stride={stride}: could not decode obj tensor {:?}", obj.shape));
            continue;
        };
        let Some(bbox_rows) = extract_rows(bbox, candidate_count, 4) else {
            logs.push(format!("stride={stride}: could not decode bbox tensor {:?}", bbox.shape));
            continue;
        };
        let Some(_kps_rows) = extract_rows(kps, candidate_count, 10) else {
            logs.push(format!("stride={stride}: could not decode kps tensor {:?}", kps.shape));
            continue;
        };

        let before = detections.len();
        let mut best_confidence = 0.0_f32;
        for index in 0..candidate_count {
            let cls_score = cls_scores[index].clamp(0.0, 1.0);
            let obj_score = obj_scores[index].clamp(0.0, 1.0);
            let score = (cls_score * obj_score).sqrt();
            best_confidence = best_confidence.max(score);
            if score < FACE_CONFIDENCE_THRESHOLD {
                continue;
            }

            let row = &bbox_rows[index * 4..index * 4 + 4];
            let col = index % cols;
            let r = index / cols;
            let stride_f = stride as f32;
            let cx = (col as f32 + row[0]) * stride_f;
            let cy = (r as f32 + row[1]) * stride_f;
            let width = row[2].exp() * stride_f;
            let height = row[3].exp() * stride_f;
            let x = cx - width / 2.0;
            let y = cy - height / 2.0;

            if let Some(detection) = face_detection_from_box(
                x,
                y,
                width,
                height,
                score,
                geometry,
                input_width as f32,
                input_height as f32,
            ) {
                detections.push(detection);
            }
        }

        logs.push(format!(
            "stride={stride}: candidateRows={} acceptedRows={} bestConfidence={:.4}",
            candidate_count,
            detections.len() - before,
            best_confidence,
        ));
    }

    (detections, logs)
}

fn extract_scores(tensor: &FaceOutputTensor, candidate_count: usize) -> Option<Vec<f32>> {
    if tensor.values.len() == candidate_count {
        return Some(tensor.values.clone());
    }
    if tensor.values.len() >= candidate_count && tensor.shape.last().copied() == Some(candidate_count as i64) {
        return Some(tensor.values[..candidate_count].to_vec());
    }
    if tensor.values.len() >= candidate_count && tensor.shape.contains(&1) {
        return Some(tensor.values[..candidate_count].to_vec());
    }
    None
}

fn extract_rows(tensor: &FaceOutputTensor, candidate_count: usize, attributes: usize) -> Option<Vec<f32>> {
    let required = candidate_count * attributes;
    if tensor.values.len() < required {
        return None;
    }

    if tensor.shape.last().copied() == Some(attributes as i64) {
        return Some(tensor.values[..required].to_vec());
    }

    if tensor.shape.len() >= 4 && tensor.shape[1] == attributes as i64 {
        let mut rows = vec![0.0; required];
        for attribute in 0..attributes {
            for candidate in 0..candidate_count {
                rows[candidate * attributes + attribute] = tensor.values[attribute * candidate_count + candidate];
            }
        }
        return Some(rows);
    }

    if tensor.shape.len() >= 3
        && tensor.shape[tensor.shape.len() - 2] == attributes as i64
        && tensor.shape[tensor.shape.len() - 1] == candidate_count as i64
    {
        let mut rows = vec![0.0; required];
        for attribute in 0..attributes {
            for candidate in 0..candidate_count {
                rows[candidate * attributes + attribute] = tensor.values[attribute * candidate_count + candidate];
            }
        }
        return Some(rows);
    }

    Some(tensor.values[..required].to_vec())
}

fn face_detection_from_box(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    confidence: f32,
    _geometry: &SquareGeometry,
    input_width: f32,
    input_height: f32,
) -> Option<FaceDetection> {
    if !confidence.is_finite() || confidence < FACE_CONFIDENCE_THRESHOLD {
        return None;
    }

    let values_are_finite = [x, y, width, height].iter().all(|value| value.is_finite());
    let aspect_ratio = width.abs() / height.abs().max(1.0);
    let face_area = width.abs() * height.abs();
    let canvas_area = input_width * input_height;
    if !values_are_finite
        || width <= 6.0
        || height <= 6.0
        || face_area < 144.0
        || face_area > canvas_area * 0.72
        || !(0.45..=1.9).contains(&aspect_ratio)
        || x < -(input_width * 0.1)
        || y < -(input_height * 0.1)
        || x > input_width * 1.1
        || y > input_height * 1.1
    {
        return None;
    }

    Some(FaceDetection {
        x,
        y,
        width,
        height,
        confidence,
    })
}

#[derive(Clone)]
struct FaceDetection {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    confidence: f32,
}

#[derive(Clone, Copy)]
struct SquareGeometry {
    content_x: f32,
    content_y: f32,
    content_width: f32,
    content_height: f32,
}

#[derive(Clone, Copy)]
struct FaceBounds {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

struct FaceOutputTensor {
    shape: Vec<i64>,
    values: Vec<f32>,
}

struct InferenceImage {
    image: DynamicImage,
    geometry: SquareGeometry,
}

impl SquareGeometry {
    fn from_source(source_width: u32, source_height: u32, canvas_size: u32) -> Self {
        let scale = (canvas_size as f32 / source_width.max(1) as f32)
            .min(canvas_size as f32 / source_height.max(1) as f32);
        let content_width = source_width as f32 * scale;
        let content_height = source_height as f32 * scale;
        Self {
            content_x: (canvas_size as f32 - content_width) / 2.0,
            content_y: (canvas_size as f32 - content_height) / 2.0,
            content_width,
            content_height,
        }
    }
}

impl FaceDetection {
    fn normalized_bounds(&self, geometry: SquareGeometry) -> Option<FaceBounds> {
        let left = self.x.max(geometry.content_x);
        let top = self.y.max(geometry.content_y);
        let right = (self.x + self.width).min(geometry.content_x + geometry.content_width);
        let bottom = (self.y + self.height).min(geometry.content_y + geometry.content_height);
        let width = right - left;
        let height = bottom - top;
        if width <= 1.0
            || height <= 1.0
            || geometry.content_width <= 0.0
            || geometry.content_height <= 0.0
        {
            return None;
        }

        Some(FaceBounds {
            x: ((left - geometry.content_x) / geometry.content_width).clamp(0.0, 1.0),
            y: ((top - geometry.content_y) / geometry.content_height).clamp(0.0, 1.0),
            width: (width / geometry.content_width).clamp(0.0, 1.0),
            height: (height / geometry.content_height).clamp(0.0, 1.0),
        })
    }

    fn iou(&self, other: &Self) -> f32 {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = (self.x + self.width).min(other.x + other.width);
        let bottom = (self.y + self.height).min(other.y + other.height);
        let intersection = (right - left).max(0.0) * (bottom - top).max(0.0);
        let union = self.width * self.height + other.width * other.height - intersection;
        if union <= 0.0 {
            0.0
        } else {
            intersection / union
        }
    }
}

fn image_descriptor_tags(image: &DynamicImage) -> Vec<String> {
    let (width, height) = image.dimensions();
    vec![
        if width >= height {
            "landscape"
        } else {
            "portrait"
        }
        .to_string(),
    ]
}

fn softmax_probabilities(scores: &[f32]) -> Vec<f32> {
    let max_score = scores
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .max_by(|first, second| first.total_cmp(second))
        .unwrap_or(0.0);
    let exps = scores
        .iter()
        .map(|score| if score.is_finite() { (score - max_score).exp() } else { 0.0 })
        .collect::<Vec<_>>();
    let total = exps.iter().sum::<f32>();
    if total <= f32::EPSILON {
        return vec![0.0; scores.len()];
    }

    exps.into_iter().map(|value| value / total).collect()
}

fn imagenet_classes() -> &'static Vec<&'static str> {
    static CLASSES: OnceLock<Vec<&'static str>> = OnceLock::new();
    CLASSES.get_or_init(|| {
        IMAGENET_CLASSES
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect()
    })
}

fn classification_label_tags(ranked: &[(usize, f32)]) -> Vec<String> {
    let classes = imagenet_classes();
    let mut tags = Vec::new();
    let mut seen = HashSet::new();

    for (index, probability) in ranked.iter().copied().take(8) {
        if probability < CLASSIFICATION_MIN_PROBABILITY {
            continue;
        }

        let Some(label) = classes.get(index) else {
            continue;
        };
        for tag in normalized_classification_tags(label) {
            if seen.insert(tag.clone()) {
                tags.push(tag);
                if tags.len() >= CLASSIFICATION_TAG_LIMIT {
                    return tags;
                }
            }
        }
    }

    if tags.is_empty() {
        if let Some((index, _)) = ranked.first() {
            if let Some(label) = classes.get(*index) {
                return normalized_classification_tags(label);
            }
        }
    }

    tags
}

fn normalized_classification_tags(label: &str) -> Vec<String> {
    let normalized = label.trim().to_lowercase();
    if normalized.is_empty() {
        return Vec::new();
    }

    let mut tags = Vec::new();
    if let Some(generic) = generic_label_tag(&normalized) {
        tags.push(generic.to_string());
    }

    let specific = normalize_tag(&normalized);
    if !specific.is_empty() && !tags.iter().any(|tag| tag == &specific) {
        tags.push(specific);
    }

    tags
}

fn generic_label_tag(label: &str) -> Option<&'static str> {
    const DOG_TERMS: &[&str] = &[
        "dog", "hound", "terrier", "retriever", "shepherd", "spaniel", "poodle",
        "pinscher", "chihuahua", "mastiff", "husky", "malamute", "beagle", "corgi",
        "dalmatian", "boxer", "doberman", "pug", "rottweiler", "schnauzer",
    ];
    const CAT_TERMS: &[&str] = &[
        "cat", "kitten", "tabby", "siamese", "persian", "egyptian", "lynx", "cougar",
        "leopard", "jaguar", "lion", "tiger", "cheetah",
    ];
    const PERSON_TERMS: &[&str] = &["groom", "ballplayer", "scuba diver"];

    if DOG_TERMS.iter().any(|term| label.contains(term)) {
        return Some("dog");
    }
    if CAT_TERMS.iter().any(|term| label.contains(term)) {
        return Some("cat");
    }
    if PERSON_TERMS.iter().any(|term| label.contains(term)) {
        return Some("person");
    }

    None
}

fn normalize_tag(value: &str) -> String {
    let mut normalized = String::new();
    let mut previous_dash = false;

    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            normalized.push(character.to_ascii_lowercase());
            previous_dash = false;
        } else if !previous_dash {
            normalized.push('-');
            previous_dash = true;
        }
    }

    normalized.trim_matches('-').to_string()
}

fn deduplicate_tags(tags: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for tag in tags {
        if seen.insert(tag.clone()) {
            deduped.push(tag);
        }
    }
    deduped
}
