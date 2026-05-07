use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use rusqlite::{params, Connection};
use tauri::{AppHandle, Manager};

use crate::models::{
    AiModelInfo, CacheClearResult, DatabaseStats, FaceCandidate, ModelDeleteResult,
    ModelInstallResult,
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
    media: &[(String, String, String, String)],
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

    for (id, name, path, media_type) in media {
        transaction
            .execute(
                "insert into media_items (id, root_id, name, path, media_type) values (?1, ?2, ?3, ?4, ?5)",
                params![id, root_id, name, path, media_type],
            )
            .map_err(|error| format!("failed to record media in database: {error}"))?;
    }

    transaction
        .commit()
        .map_err(|error| format!("failed to commit media database transaction: {error}"))?;
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
                media_type text not null
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
