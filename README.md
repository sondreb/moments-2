# Moments

Moments is a native desktop viewer for very large local photo and video libraries. It is built with Rust, Tauri, and Angular so filesystem scanning, indexing, and thumbnail work can run natively while the interface stays fast and focused.

The first implementation slice supports adding multiple folders, keeping an in-memory library root list, scanning selected folders for supported media files, and rendering real local photo/video thumbnails in the gallery.

## Development

Install dependencies:

```bash
npm install
```

Run the desktop app in development mode:

```bash
npm run tauri dev
```

Build the Angular frontend only:

```bash
npm run build
```

Increase the patch version across the desktop app metadata:

```bash
npm run version:patch
```

## Releases

Pushing to `main` triggers the GitHub Actions desktop release workflow. It builds the Tauri app for macOS, Linux, and Windows x64/ARM64, uploads signed updater artifacts, and creates or updates a GitHub draft release using the current app version.

Release builds require these GitHub repository settings:

- `TAURI_SIGNING_PRIVATE_KEY` secret
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secret, if your signing key uses one
- `MOMENTS_UPDATER_PUBLIC_KEY` repository variable

Published releases expose in-app update checks, download progress, and install support in the desktop Settings view.

## Planning

- [docs/PLAN.md](docs/PLAN.md)
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)