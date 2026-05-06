# Moments

Moments is a native desktop image viewer for very large local photo libraries. It is built with Rust, Tauri, and Angular so filesystem scanning, indexing, and thumbnail work can run natively while the interface stays fast and focused.

The first implementation slice supports adding multiple folders, keeping an in-memory library root list, and scanning selected folders for supported image files.

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

## Planning

- [docs/PLAN.md](docs/PLAN.md)
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)