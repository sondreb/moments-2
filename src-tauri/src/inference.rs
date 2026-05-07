use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
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

pub fn classify_image(
    model_path: &Path,
    image_path: &Path,
    cache_dir: &Path,
) -> Result<Vec<String>, String> {
    let input_image = load_or_create_inference_image(
        image_path,
        cache_dir,
        "classification",
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

    let mut ranked = scores
        .iter()
        .enumerate()
        .map(|(index, score)| (index, *score))
        .collect::<Vec<_>>();
    ranked.sort_by(|first, second| second.1.total_cmp(&first.1));

    let mut tags = image_descriptor_tags(&input_image.image);
    tags.push("onnx-classified".to_string());
    tags.extend(
        ranked.into_iter().take(3).map(|(index, score)| {
            format!("imagenet-{index}-{:.0}%", soft_confidence(score) * 100.0)
        }),
    );
    Ok(tags)
}

pub fn detect_faces(
    model_path: &Path,
    image_path: &Path,
    cache_dir: &Path,
    media_id: &str,
) -> Result<Vec<FaceCandidate>, String> {
    let input_image = load_or_create_inference_image(image_path, cache_dir, "faces", FACE_SIZE)?;
    let input = face_tensor(&input_image.image)?;
    let mut session = load_session(model_path)?;
    let outputs = session
        .run(inputs![input])
        .map_err(|error| format!("face detection inference failed: {error}"))?;

    let mut detections = Vec::new();
    for (_, output) in outputs.iter() {
        let Ok((shape, values)) = output.try_extract_tensor::<f32>() else {
            continue;
        };
        detections.extend(face_detections_from_output(&shape.to_vec(), values));
    }

    Ok(face_candidates_from_detections(
        media_id,
        detections,
        input_image.geometry,
    ))
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

fn load_or_create_inference_image(
    source_path: &Path,
    cache_dir: &Path,
    bucket: &str,
    size: u32,
) -> Result<InferenceImage, String> {
    let cached_path = inference_image_path(source_path, cache_dir, bucket, size)?;
    if cached_path.exists() {
        let image = load_rgb_image(&cached_path)?;
        let source_dimensions = image::image_dimensions(source_path).map_err(|error| {
            format!(
                "failed to inspect image dimensions '{}': {error}",
                source_path.display()
            )
        })?;
        return Ok(InferenceImage {
            image,
            geometry: SquareGeometry::from_source(source_dimensions.0, source_dimensions.1, size),
        });
    }

    let source = load_rgb_image(source_path)?;
    let geometry = SquareGeometry::from_source(source.width(), source.height(), size);
    let thumbnail = square_thumbnail(&source, size);
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
    let resized = image
        .resize_exact(FACE_SIZE, FACE_SIZE, FilterType::Triangle)
        .to_rgb8();
    let mut input = Vec::with_capacity((3 * FACE_SIZE * FACE_SIZE) as usize);

    for channel in 0..3 {
        for pixel in resized.pixels() {
            input.push(pixel[channel] as f32);
        }
    }

    Tensor::from_array((
        [1_usize, 3, FACE_SIZE as usize, FACE_SIZE as usize],
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

fn face_detections_from_output(shape: &[i64], values: &[f32]) -> Vec<FaceDetection> {
    if values.is_empty() || shape.is_empty() {
        return Vec::new();
    }

    if let Some(width) = shape.last().copied().filter(|width| *width >= 15) {
        return values
            .chunks(width as usize)
            .filter_map(face_detection_from_row)
            .collect();
    }

    if values.len() >= 15 && values.len() % 15 == 0 {
        return values
            .chunks(15)
            .filter_map(face_detection_from_row)
            .collect();
    }

    if shape.len() >= 3 {
        let attributes = shape[shape.len() - 2] as usize;
        let candidates = shape[shape.len() - 1] as usize;
        if attributes >= 15 && values.len() >= attributes * candidates {
            return (0..candidates)
                .filter_map(|candidate| {
                    let row = (0..attributes)
                        .map(|attribute| values[attribute * candidates + candidate])
                        .collect::<Vec<_>>();
                    face_detection_from_row(&row)
                })
                .collect();
        }
    }

    Vec::new()
}

fn face_detection_from_row(row: &[f32]) -> Option<FaceDetection> {
    if row.len() < 15 {
        return None;
    }

    let confidence = row[14];

    if !confidence.is_finite() || confidence < FACE_CONFIDENCE_THRESHOLD || row.len() < 4 {
        return None;
    }

    let x = row[0];
    let y = row[1];
    let width = row[2].abs();
    let height = row[3].abs();
    let values_are_finite = [x, y, width, height].iter().all(|value| value.is_finite());
    let aspect_ratio = width.abs() / height.abs().max(1.0);
    let face_area = width.abs() * height.abs();
    let canvas_area = (FACE_SIZE * FACE_SIZE) as f32;
    if !values_are_finite
        || width <= 6.0
        || height <= 6.0
        || face_area < 144.0
        || face_area > canvas_area * 0.72
        || !(0.45..=1.9).contains(&aspect_ratio)
        || x < -(FACE_SIZE as f32 * 0.1)
        || y < -(FACE_SIZE as f32 * 0.1)
        || x > FACE_SIZE as f32 * 1.1
        || y > FACE_SIZE as f32 * 1.1
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
    let mut tags = vec!["photo".to_string()];
    tags.push(
        if width >= height {
            "landscape"
        } else {
            "portrait"
        }
        .to_string(),
    );

    let rgb = image.thumbnail(64, 64).to_rgb8();
    let luminance = rgb
        .pixels()
        .map(|pixel| 0.2126 * pixel[0] as f32 + 0.7152 * pixel[1] as f32 + 0.0722 * pixel[2] as f32)
        .sum::<f32>()
        / rgb.pixels().len().max(1) as f32;

    tags.push(
        if luminance >= 150.0 {
            "bright"
        } else {
            "low-light"
        }
        .to_string(),
    );
    tags
}

fn soft_confidence(score: f32) -> f32 {
    if score.is_finite() {
        (1.0 / (1.0 + (-score).exp())).clamp(0.0, 1.0)
    } else {
        0.0
    }
}
