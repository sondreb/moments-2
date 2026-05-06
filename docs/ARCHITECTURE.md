# Moments Architecture

Moments uses Rust for native library work and Angular for the desktop interface. Tauri is the boundary between them.

## System Shape

```text
Angular UI
  -> Tauri commands and events
Rust backend
  -> library roots, scanner, index, thumbnail cache, metadata workers
Local disk
  -> original photos and videos, SQLite index, generated thumbnails
```

The app should avoid loading the whole library into memory or sending huge arrays across IPC. Angular requests visible slices; Rust owns indexing, query planning, and file operations.

## Rust Backend

Current modules:

- `commands`: Tauri command handlers.
- `library`: in-memory folder roots and initial recursive scanner.
- `models`: serializable data contracts shared with Angular.

Planned modules:

- `index`: SQLite connection, migrations, repositories, and paged queries.
- `thumbnail`: deterministic cache names, generation queue, and cleanup.
- `metadata`: EXIF extraction and normalized metadata records.
- `jobs`: background queues, cancellation, progress events, and resume state.
- `watcher`: filesystem updates for each library root.

## Tauri Commands

Implemented:

- `add_library_root(path)`
- `list_library_roots()`
- `scan_library_root(root_id)`
- `library_overview()`

Planned:

- `remove_library_root(root_id)`
- `get_photos(query, offset, limit)`
- `get_folder_children(root_id, parent_path)`
- `request_thumbnail(photo_id)`
- `get_photo_metadata(photo_id)`
- `preview_rename(request)`
- `apply_rename(plan_id)`

## Angular Frontend

Current shape:

- Sidebar for library roots.
- Toolbar for the selected root.
- Gallery workspace with indexed media totals and real local photo/video thumbnails.
- Inspector for selected root details.

Planned structure:

```text
src/app/
  components/
  services/
  store/
  views/
    gallery/
    folder-tree/
    inspector/
```

Angular Signals should remain the default state primitive until the app has enough cross-view complexity to justify NgRx.

## Storage Model

SQLite should store library roots, folders, media files, metadata, tags, thumbnail status, and rename history. Thumbnail image bytes should live on disk in an app data cache folder.

Initial tables:

- `library_roots`
- `folders`
- `media_files`
- `photo_metadata`
- `thumbnail_jobs`
- `tags`
- `photo_tags`
- `rename_batches`
- `rename_operations`

## Performance Rules

- Index file records before metadata and thumbnails.
- Track media type so photos and videos can share gallery infrastructure without losing type-specific behavior.
- Use background workers for expensive filesystem and image operations.
- Page all gallery queries.
- Keep thumbnail cache names deterministic.
- Do not block the UI thread while scanning.
- Treat unsupported and corrupt files as normal library conditions.

## Multi-Folder Library

Each folder root is an independent source. A single app library can contain many roots, and future SQLite records should keep `root_id` on folder and media rows so the app can rescan or remove one root without touching the others.