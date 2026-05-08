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

Pushing to `main` triggers the GitHub Actions desktop release workflow. It prepares release metadata from `package.json`, builds the desktop artifacts in parallel, uploads them as workflow artifacts, and only then creates or updates a GitHub draft release for the current app version.

Release builds require these GitHub repository settings:

- `TAURI_SIGNING_PRIVATE_KEY` secret
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secret, if your updater private key uses one
- `MOMENTS_UPDATER_PUBLIC_KEY` repository variable

Published releases expose in-app update checks, download progress, and install support in the desktop Settings view.

### Tauri updater signing key

Desktop updater signing does not use an Android keystore. Tauri signs update metadata with an Ed25519 keypair.

Generate that keypair locally:

```bash
npx tauri signer generate -w ~/.tauri/moments.key
```

That command prints the public key and writes the private key file. Store them like this:

- `TAURI_SIGNING_PRIVATE_KEY`: contents of the generated private key file
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: the password you entered when creating the key, if any
- `MOMENTS_UPDATER_PUBLIC_KEY`: the public key printed by the signer command

### Android keystore secrets

If you later add Android release builds, those use a Java keystore instead of the Tauri updater keypair. Generate one with:

```bash
keytool -genkeypair -v \
	-keystore moments-upload.jks \
	-alias moments \
	-keyalg RSA \
	-keysize 2048 \
	-validity 10000
```

Then encode it for GitHub Actions:

```bash
base64 -w 0 moments-upload.jks
```

- `KEYSTORE_BASE64`: base64 output of the `.jks` or `.keystore` file
- `KEYSTORE_PASSWORD`: keystore password
- `KEY_ALIAS`: alias passed to `keytool`, for example `moments`
- `KEY_PASSWORD`: key password for that alias

## Planning

- [docs/PLAN.md](docs/PLAN.md)
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)