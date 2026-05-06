# Moments Plan

Moments is a high-performance desktop image viewer for people with very large local photo collections. The app should make hundreds of thousands of photos feel immediate by indexing once, doing heavy work in Rust, and keeping the Angular UI focused on paged and virtualized views.

## Product Goals

- Add and manage multiple local folder roots.
- Index photos without copying or owning the original files.
- Scroll large libraries smoothly with virtualized rendering.
- Generate and cache thumbnails in the background.
- Show metadata and EXIF details in an inspector.
- Support safe batch rename workflows with preview and undo.

## Initial Scope

- Tauri 2 desktop shell with Angular frontend.
- Folder picker for adding library roots.
- Rust commands for adding, listing, and scanning roots.
- Basic photo count scan for common image extensions.
- App shell with sidebar, gallery workspace, and inspector.

## Later Scope

- SQLite-backed persistent library index.
- Incremental scanner with cancellation and progress events.
- Background thumbnail generation and cache cleanup.
- EXIF extraction and searchable metadata.
- File watching for added, changed, renamed, and deleted files.
- Virtualized gallery backed by paged Rust queries.
- Rename preview, apply, and undo history.

## Non-Goals For Now

- Cloud sync.
- Destructive file editing without preview.
- Image editing tools.
- Importing photos into an app-owned library folder.

## Implementation Phases

1. Foundation: scaffold Tauri and Angular, establish app layout, and add folder root commands.
2. Index: add SQLite, schema migrations, persisted roots, paged photo queries, and scan jobs.
3. Thumbnails: generate deterministic WebP thumbnails, cache them on disk, and expose thumbnail URLs to Angular.
4. Gallery: replace placeholder tiles with a virtualized grid that requests visible photo pages.
5. Metadata: extract EXIF in background jobs and render selected photo details in the inspector.
6. Live Library: add filesystem watchers and incremental reindexing.
7. Rename: add template preview, validation, batch apply, and undo log.

## Acceptance Criteria

- The app can add more than one folder root.
- Scans do not freeze the Angular renderer.
- Large result sets are paged across the Tauri boundary.
- Thumbnail files are cached outside SQLite.
- Batch rename operations always produce a preview before writing to disk.