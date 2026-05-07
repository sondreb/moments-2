use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use rusqlite::{params, Connection};
use tauri::{AppHandle, Manager};

use crate::models::{
    AiModelInfo, CacheClearResult, DatabaseStats, FaceCandidate, LibraryRoot, LibraryRootStatus,
    MediaItem, MediaMetadata, MediaType, ModelDeleteResult, ModelInstallResult,
};

struct ModelDefinition {
    id: &'static str,
    name: &'static str,
    task: &'static str,
    accelerator: &'static str,
    description: &'static str,
    file_name: &'static str,
    download_url: &'static str,
}

const FACE_MODEL_ID: &str = "face-detection-yunet-cpu";
const FACE_GPU_MODEL_ID: &str = "face-detection-yunet-gpu";
const CLASSIFICATION_MODEL_ID: &str = "image-classification-mobilenet-cpu";
const CLASSIFICATION_GPU_MODEL_ID: &str = "image-classification-mobilenet-gpu";
const FACE_MODEL_URL: &str = "https://github.com/opencv/opencv_zoo/raw/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx";
const CLASSIFICATION_MODEL_URL: &str = "https://github.com/onnx/models/raw/main/validated/vision/classification/mobilenet/model/mobilenetv2-12.onnx";
const SAMPLES_DIR_NAME: &str = "Samples";

const MODEL_DEFINITIONS: &[ModelDefinition] = &[
    ModelDefinition {
        id: FACE_MODEL_ID,
        name: "YuNet face detector",
        task: "Face scanning",
        accelerator: "CPU",
        description: "Compact ONNX face detection model for local CPU inference.",
        file_name: "face_detection_yunet_2023mar.cpu.onnx",
        download_url: FACE_MODEL_URL,
    },
    ModelDefinition {
        id: FACE_GPU_MODEL_ID,
        name: "YuNet face detector",
        task: "Face scanning",
        accelerator: "GPU",
        description:
            "Same ONNX face model prepared for GPU execution providers such as DirectML or CUDA.",
        file_name: "face_detection_yunet_2023mar.gpu.onnx",
        download_url: FACE_MODEL_URL,
    },
    ModelDefinition {
        id: CLASSIFICATION_MODEL_ID,
        name: "MobileNetV2 classifier",
        task: "Image classification",
        accelerator: "CPU",
        description: "General image classification ONNX model for local CPU tagging workflows.",
        file_name: "mobilenetv2-12.cpu.onnx",
        download_url: CLASSIFICATION_MODEL_URL,
    },
    ModelDefinition {
        id: CLASSIFICATION_GPU_MODEL_ID,
        name: "MobileNetV2 classifier",
        task: "Image classification",
        accelerator: "GPU",
        description:
            "General image classification ONNX model prepared for GPU execution providers.",
        file_name: "mobilenetv2-12.gpu.onnx",
        download_url: CLASSIFICATION_MODEL_URL,
    },
];

pub fn available_models(app: &AppHandle) -> Result<Vec<AiModelInfo>, String> {
    MODEL_DEFINITIONS
        .iter()
        .map(|definition| model_info(app, definition))
        .collect()
}

pub fn install_model(app: &AppHandle, model_id: &str) -> Result<ModelInstallResult, String> {
    let definition = model_definition(model_id)?;

    let model_path = models_dir(app)?.join(definition.file_name);
    if model_path.exists() {
        return Ok(ModelInstallResult {
            model: model_info(app, definition)?,
            message: "Model is already installed.".to_string(),
        });
    }

    fs::create_dir_all(
        model_path
            .parent()
            .ok_or("model directory is unavailable")?,
    )
    .map_err(|error| format!("failed to create model directory: {error}"))?;

    let temp_path = model_path.with_extension("onnx.download");
    let response = ureq::get(definition.download_url)
        .call()
        .map_err(|error| format!("failed to download model: {error}"))?;
    let mut reader = response.into_reader();
    let mut file = fs::File::create(&temp_path)
        .map_err(|error| format!("failed to create model file: {error}"))?;
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("failed while downloading model: {error}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|error| format!("failed to write model file: {error}"))?;
    }

    fs::rename(&temp_path, &model_path)
        .map_err(|error| format!("failed to finalize model download: {error}"))?;

    Ok(ModelInstallResult {
        model: model_info(app, definition)?,
        message: format!(
            "{} {} model installed.",
            definition.task, definition.accelerator
        ),
    })
}

pub fn delete_model(app: &AppHandle, model_id: &str) -> Result<ModelDeleteResult, String> {
    let definition = model_definition(model_id)?;
    let model_path = models_dir(app)?.join(definition.file_name);
    let removed_bytes = fs::metadata(&model_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);

    if model_path.exists() {
        fs::remove_file(&model_path).map_err(|error| format!("failed to delete model: {error}"))?;
    }

    Ok(ModelDeleteResult {
        model: model_info(app, definition)?,
        removed_bytes,
        message: format!(
            "{} {} model deleted.",
            definition.task, definition.accelerator
        ),
    })
}

pub fn clear_cache(app: &AppHandle) -> Result<CacheClearResult, String> {
    let cache_dir = cache_dir(app)?;
    let mut result = CacheClearResult {
        removed_files: 0,
        removed_bytes: 0,
    };

    if !cache_dir.exists() {
        return Ok(result);
    }

    clear_directory(&cache_dir, &mut result)?;
    Ok(result)
}

pub fn database_stats(app: &AppHandle) -> Result<DatabaseStats, String> {
    let db_path = database_path(app)?;
    let connection = open_database(app)?;
    let size_bytes = fs::metadata(&db_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);

    Ok(DatabaseStats {
        path: db_path.to_string_lossy().to_string(),
        size_bytes,
        root_count: count_rows(&connection, "library_roots")?,
        media_count: count_rows(&connection, "media_items")?,
        metadata_count: count_rows(&connection, "media_metadata")?,
        favorite_count: count_where(&connection, "media_metadata", "favorite = 1")?,
        tag_count: count_rows(&connection, "media_tags")?,
        face_count: count_rows(&connection, "face_candidates")?,
    })
}

pub fn load_library_snapshot(
    app: &AppHandle,
) -> Result<
    (
        Vec<LibraryRoot>,
        Vec<MediaItem>,
        Vec<MediaMetadata>,
        Vec<FaceCandidate>,
    ),
    String,
> {
    let connection = open_database(app)?;
    Ok((
        load_roots(&connection)?,
        load_media_items(&connection)?,
        load_metadata(&connection)?,
        load_faces(&connection)?,
    ))
}

pub fn record_root(app: &AppHandle, id: &str, name: &str, path: &str) -> Result<(), String> {
    let connection = open_database(app)?;
    connection
        .execute(
            "insert into library_roots (id, name, path) values (?1, ?2, ?3)
             on conflict(id) do update set name = excluded.name, path = excluded.path",
            params![id, name, path],
        )
        .map_err(|error| format!("failed to record root in database: {error}"))?;
    Ok(())
}

pub fn record_media(
    app: &AppHandle,
    root_id: &str,
    media: &[(String, String, String, String, Option<String>)],
) -> Result<(), String> {
    let mut connection = open_database(app)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("failed to start media database transaction: {error}"))?;
    transaction
        .execute(
            "delete from media_items where root_id = ?1",
            params![root_id],
        )
        .map_err(|error| format!("failed to update media database: {error}"))?;

    for (id, name, path, media_type, content_hash) in media {
        transaction
            .execute(
                "insert into media_items (id, root_id, name, path, media_type, content_hash) values (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, root_id, name, path, media_type, content_hash],
            )
            .map_err(|error| format!("failed to record media in database: {error}"))?;
    }

    transaction
        .commit()
        .map_err(|error| format!("failed to commit media database transaction: {error}"))?;
    Ok(())
}

pub fn remove_root(app: &AppHandle, root_id: &str) -> Result<(), String> {
    let mut connection = open_database(app)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("failed to start root removal transaction: {error}"))?;
    delete_media_rows(&transaction, root_id, None)?;
    transaction
        .execute("delete from library_roots where id = ?1", params![root_id])
        .map_err(|error| format!("failed to delete root from database: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("failed to commit root removal transaction: {error}"))?;
    Ok(())
}

pub fn delete_media_items(app: &AppHandle, media_ids: &[String]) -> Result<(), String> {
    if media_ids.is_empty() {
        return Ok(());
    }

    let mut connection = open_database(app)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("failed to start media deletion transaction: {error}"))?;
    for media_id in media_ids {
        transaction
            .execute(
                "delete from face_candidates where media_id = ?1",
                params![media_id],
            )
            .map_err(|error| format!("failed to delete face candidates: {error}"))?;
        transaction
            .execute(
                "delete from media_tags where media_id = ?1",
                params![media_id],
            )
            .map_err(|error| format!("failed to delete media tags: {error}"))?;
        transaction
            .execute(
                "delete from media_metadata where media_id = ?1",
                params![media_id],
            )
            .map_err(|error| format!("failed to delete media metadata: {error}"))?;
        transaction
            .execute("delete from media_items where id = ?1", params![media_id])
            .map_err(|error| format!("failed to delete media item: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("failed to commit media deletion transaction: {error}"))?;
    Ok(())
}

pub fn record_metadata(
    app: &AppHandle,
    media_id: &str,
    favorite: bool,
    tags: &[String],
) -> Result<(), String> {
    let mut connection = open_database(app)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("failed to start metadata database transaction: {error}"))?;
    transaction
        .execute(
            "insert into media_metadata (media_id, favorite) values (?1, ?2)
             on conflict(media_id) do update set favorite = excluded.favorite",
            params![media_id, favorite as i32],
        )
        .map_err(|error| format!("failed to record metadata in database: {error}"))?;
    transaction
        .execute(
            "delete from media_tags where media_id = ?1",
            params![media_id],
        )
        .map_err(|error| format!("failed to update tags in database: {error}"))?;

    for tag in tags {
        transaction
            .execute(
                "insert into media_tags (media_id, tag) values (?1, ?2)",
                params![media_id, tag],
            )
            .map_err(|error| format!("failed to record tag in database: {error}"))?;
    }

    transaction
        .commit()
        .map_err(|error| format!("failed to commit metadata database transaction: {error}"))?;
    Ok(())
}

pub fn replace_faces_for_media(
    app: &AppHandle,
    media_id: &str,
    faces: &[FaceCandidate],
) -> Result<(), String> {
    let mut connection = open_database(app)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("failed to start face database transaction: {error}"))?;
    transaction
        .execute(
            "delete from face_candidates where media_id = ?1",
            params![media_id],
        )
        .map_err(|error| format!("failed to replace face candidates: {error}"))?;

    for face in faces {
        transaction
            .execute(
                "insert into face_candidates (id, media_id, name, confidence, x, y, width, height) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 on conflict(id) do update set media_id = excluded.media_id, name = excluded.name, confidence = excluded.confidence, x = excluded.x, y = excluded.y, width = excluded.width, height = excluded.height",
                params![&face.id, &face.media_id, &face.name, face.confidence, face.x, face.y, face.width, face.height],
            )
            .map_err(|error| format!("failed to record face candidate: {error}"))?;
    }

    transaction
        .commit()
        .map_err(|error| format!("failed to commit face database transaction: {error}"))?;
    Ok(())
}

pub fn installed_model(app: &AppHandle, task: &str) -> Result<Option<AiModelInfo>, String> {
    Ok(available_models(app)?
        .into_iter()
        .find(|model| model.installed && model.task.eq_ignore_ascii_case(task)))
}

pub fn model_path(app: &AppHandle, model: &AiModelInfo) -> Result<PathBuf, String> {
    Ok(models_dir(app)?.join(&model.file_name))
}

pub fn inference_cache_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(cache_dir(app)?.join("inference"))
}

pub fn ensure_samples_directory(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = samples_dir(app)?;
    fs::create_dir_all(&directory)
        .map_err(|error| format!("failed to create samples directory: {error}"))?;

    if let Some(source_dir) = samples_source_dir(app)? {
        sync_directory_contents(&source_dir, &directory)?;
    }

    Ok(directory)
}

fn model_definition(model_id: &str) -> Result<&'static ModelDefinition, String> {
    MODEL_DEFINITIONS
        .iter()
        .find(|definition| definition.id == model_id)
        .ok_or_else(|| format!("unknown model '{model_id}'"))
}

fn model_info(app: &AppHandle, definition: &ModelDefinition) -> Result<AiModelInfo, String> {
    let model_path = models_dir(app)?.join(definition.file_name);
    let size_bytes = fs::metadata(&model_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);

    Ok(AiModelInfo {
        id: definition.id.to_string(),
        name: definition.name.to_string(),
        task: definition.task.to_string(),
        accelerator: definition.accelerator.to_string(),
        description: definition.description.to_string(),
        file_name: definition.file_name.to_string(),
        download_url: definition.download_url.to_string(),
        installed: model_path.exists(),
        size_bytes,
    })
}

fn open_database(app: &AppHandle) -> Result<Connection, String> {
    let db_path = database_path(app)?;
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create database directory: {error}"))?;
    }
    let connection =
        Connection::open(&db_path).map_err(|error| format!("failed to open database: {error}"))?;
    initialize_database(&connection)?;
    Ok(connection)
}

fn initialize_database(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "create table if not exists library_roots (
                id text primary key,
                name text not null,
                path text not null
            );
            create table if not exists media_items (
                id text primary key,
                root_id text not null,
                name text not null,
                path text not null,
                media_type text not null,
                content_hash text
            );
            create table if not exists media_metadata (
                media_id text primary key,
                favorite integer not null default 0
            );
            create table if not exists media_tags (
                media_id text not null,
                tag text not null
            );
            create table if not exists face_candidates (
                id text primary key,
                media_id text not null,
                name text,
                confidence real not null default 0,
                x real not null default 0,
                y real not null default 0,
                width real not null default 0,
                height real not null default 0
            );",
        )
        .map_err(|error| format!("failed to initialize database: {error}"))?;
    ensure_column(connection, "media_items", "content_hash", "text")?;
    ensure_column(
        connection,
        "face_candidates",
        "x",
        "real not null default 0",
    )?;
    ensure_column(
        connection,
        "face_candidates",
        "y",
        "real not null default 0",
    )?;
    ensure_column(
        connection,
        "face_candidates",
        "width",
        "real not null default 0",
    )?;
    ensure_column(
        connection,
        "face_candidates",
        "height",
        "real not null default 0",
    )?;
    Ok(())
}

fn load_roots(connection: &Connection) -> Result<Vec<LibraryRoot>, String> {
    let mut statement = connection
        .prepare(
            "select id, name, path from library_roots order by name collate nocase, path collate nocase",
        )
        .map_err(|error| format!("failed to load library roots: {error}"))?;
    let roots = statement
        .query_map([], |row| {
            Ok(LibraryRoot {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                status: LibraryRootStatus::Ready,
                photo_count: 0,
                video_count: 0,
                media_count: 0,
            })
        })
        .map_err(|error| format!("failed to load library roots: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to load library roots: {error}"))?;
    Ok(roots)
}

fn load_media_items(connection: &Connection) -> Result<Vec<MediaItem>, String> {
    let mut statement = connection
        .prepare(
            "select id, root_id, name, path, media_type, content_hash from media_items order by path collate nocase",
        )
        .map_err(|error| format!("failed to load media items: {error}"))?;
    let media_items = statement
        .query_map([], |row| {
            let media_type = row.get::<_, String>(4)?;
            Ok(MediaItem {
                id: row.get(0)?,
                root_id: row.get(1)?,
                name: row.get(2)?,
                path: row.get(3)?,
                media_type: media_type_from_database(&media_type),
                content_hash: row.get(5)?,
            })
        })
        .map_err(|error| format!("failed to load media items: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to load media items: {error}"))?;
    Ok(media_items)
}

fn load_metadata(connection: &Connection) -> Result<Vec<MediaMetadata>, String> {
    let mut tags_statement = connection
        .prepare("select media_id, tag from media_tags order by media_id, tag collate nocase")
        .map_err(|error| format!("failed to load media tags: {error}"))?;
    let mut tags_by_media = std::collections::BTreeMap::<String, Vec<String>>::new();
    for tag in tags_statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("failed to load media tags: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to load media tags: {error}"))?
    {
        tags_by_media.entry(tag.0).or_default().push(tag.1);
    }

    let mut statement = connection
        .prepare("select media_id, favorite from media_metadata order by media_id")
        .map_err(|error| format!("failed to load media metadata: {error}"))?;
    let metadata = statement
        .query_map([], |row| {
            let media_id = row.get::<_, String>(0)?;
            Ok(MediaMetadata {
                tags: tags_by_media.remove(&media_id).unwrap_or_default(),
                media_id,
                favorite: row.get::<_, i64>(1)? != 0,
                face_ids: Vec::new(),
            })
        })
        .map_err(|error| format!("failed to load media metadata: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to load media metadata: {error}"))?;
    Ok(metadata)
}

fn load_faces(connection: &Connection) -> Result<Vec<FaceCandidate>, String> {
    let mut statement = connection
        .prepare(
            "select id, media_id, name, confidence, x, y, width, height from face_candidates order by media_id, id",
        )
        .map_err(|error| format!("failed to load face candidates: {error}"))?;
    let faces = statement
        .query_map([], |row| {
            Ok(FaceCandidate {
                id: row.get(0)?,
                media_id: row.get(1)?,
                name: row.get(2)?,
                confidence: row.get(3)?,
                x: row.get(4)?,
                y: row.get(5)?,
                width: row.get(6)?,
                height: row.get(7)?,
            })
        })
        .map_err(|error| format!("failed to load face candidates: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to load face candidates: {error}"))?;
    Ok(faces)
}

fn delete_media_rows(
    connection: &Connection,
    root_id: &str,
    media_ids: Option<&[String]>,
) -> Result<(), String> {
    let target_ids = if let Some(media_ids) = media_ids {
        media_ids.to_vec()
    } else {
        let mut statement = connection
            .prepare("select id from media_items where root_id = ?1")
            .map_err(|error| format!("failed to load media IDs for deletion: {error}"))?;
        let media_ids = statement
            .query_map(params![root_id], |row| row.get::<_, String>(0))
            .map_err(|error| format!("failed to load media IDs for deletion: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("failed to load media IDs for deletion: {error}"))?;
        media_ids
    };

    for media_id in target_ids {
        connection
            .execute(
                "delete from face_candidates where media_id = ?1",
                params![&media_id],
            )
            .map_err(|error| format!("failed to delete face candidates: {error}"))?;
        connection
            .execute(
                "delete from media_tags where media_id = ?1",
                params![&media_id],
            )
            .map_err(|error| format!("failed to delete media tags: {error}"))?;
        connection
            .execute(
                "delete from media_metadata where media_id = ?1",
                params![&media_id],
            )
            .map_err(|error| format!("failed to delete media metadata: {error}"))?;
        connection
            .execute("delete from media_items where id = ?1", params![&media_id])
            .map_err(|error| format!("failed to delete media item: {error}"))?;
    }

    Ok(())
}

fn media_type_from_database(value: &str) -> MediaType {
    if value.eq_ignore_ascii_case("video") {
        MediaType::Video
    } else {
        MediaType::Photo
    }
}

fn ensure_column(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), String> {
    let mut statement = connection
        .prepare(&format!("pragma table_info({table})"))
        .map_err(|error| format!("failed to inspect {table}: {error}"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("failed to inspect {table}: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to inspect {table}: {error}"))?;

    if !columns.iter().any(|candidate| candidate == column) {
        connection
            .execute(
                &format!("alter table {table} add column {column} {definition}"),
                [],
            )
            .map_err(|error| format!("failed to migrate {table}.{column}: {error}"))?;
    }

    Ok(())
}

fn count_rows(connection: &Connection, table: &str) -> Result<u64, String> {
    let sql = format!("select count(*) from {table}");
    connection
        .query_row(&sql, [], |row| row.get::<_, u64>(0))
        .map_err(|error| format!("failed to count {table}: {error}"))
}

fn count_where(connection: &Connection, table: &str, predicate: &str) -> Result<u64, String> {
    let sql = format!("select count(*) from {table} where {predicate}");
    connection
        .query_row(&sql, [], |row| row.get::<_, u64>(0))
        .map_err(|error| format!("failed to count {table}: {error}"))
}

fn clear_directory(path: &Path, result: &mut CacheClearResult) -> Result<(), String> {
    for entry in fs::read_dir(path).map_err(|error| format!("failed to read cache: {error}"))? {
        let entry = entry.map_err(|error| format!("failed to inspect cache entry: {error}"))?;
        let path = entry.path();
        let metadata = entry
            .metadata()
            .map_err(|error| format!("failed to inspect cache entry: {error}"))?;

        if metadata.is_dir() {
            clear_directory(&path, result)?;
            fs::remove_dir(&path)
                .map_err(|error| format!("failed to remove cache folder: {error}"))?;
        } else {
            result.removed_files += 1;
            result.removed_bytes += metadata.len();
            fs::remove_file(&path)
                .map_err(|error| format!("failed to remove cache file: {error}"))?;
        }
    }
    Ok(())
}

fn database_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join("moments.sqlite3"))
}

fn samples_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join(SAMPLES_DIR_NAME))
}

fn samples_source_dir(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    let repo_samples = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("samples");
    if repo_samples.exists() {
        return Ok(Some(repo_samples));
    }

    let resource_samples = app
        .path()
        .resource_dir()
        .map_err(|error| format!("failed to resolve resource directory: {error}"))?
        .join("samples");
    if resource_samples.exists() {
        return Ok(Some(resource_samples));
    }

    Ok(None)
}

fn models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join("models"))
}

fn cache_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join("cache"))
}

fn app_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data directory: {error}"))
}

fn sync_directory_contents(source: &Path, destination: &Path) -> Result<(), String> {
    for entry in fs::read_dir(source)
        .map_err(|error| format!("failed to read samples source '{}': {error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to inspect sample entry: {error}"))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = entry
            .metadata()
            .map_err(|error| format!("failed to inspect sample entry: {error}"))?;

        if metadata.is_dir() {
            fs::create_dir_all(&destination_path).map_err(|error| {
                format!(
                    "failed to create samples subdirectory '{}': {error}",
                    destination_path.display()
                )
            })?;
            sync_directory_contents(&source_path, &destination_path)?;
        } else {
            let should_copy = match fs::metadata(&destination_path) {
                Ok(destination_metadata) => {
                    destination_metadata.len() != metadata.len()
                        || metadata.modified().ok() > destination_metadata.modified().ok()
                }
                Err(_) => true,
            };

            if should_copy {
                fs::copy(&source_path, &destination_path).map_err(|error| {
                    format!(
                        "failed to copy sample '{}' to '{}': {error}",
                        source_path.display(),
                        destination_path.display()
                    )
                })?;
            }
        }
    }

    Ok(())
}
