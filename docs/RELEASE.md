# Releasing Tidy

## Platforms

| Platform | Runner | Notes |
|---|---|---|
| macOS | `macos-latest` | DMG + `.app`. Notarization needs Apple secrets (below). |
| Windows | `windows-latest` | NSIS installer. Optional Authenticode via Tauri signing key. |
| Linux | `ubuntu-24.04` | AppImage + `.deb`. Needs glibc ≥ 2.38 (Ubuntu 24.04+, Fedora 39+, Debian 13). |

## What ships in the installer

- The Tidy UI binary
- `tidy-embed-worker` (hybrid MiniLM ranking sidecar)
- OCR models under `resources/ocr/` (`text-detection.onnx`, `text-recognition.onnx`) — downloaded by `scripts/fetch-ocr-models.mjs` during `beforeBuildCommand`

End users do **not** need to install tesseract, poppler, or any OCR toolkit. Screenshots and many scanned PDFs are read with the bundled engine. On macOS, Quick Look may also rasterise a PDF page when needed (built into the OS).

## Optional code signing

Signing is **not** required for a draft/internal release. Without secrets the workflow still builds unsigned installers.

### macOS (Developer ID + notarization)

Add these repository secrets, then re-run the Release workflow:

- `APPLE_CERTIFICATE` — base64-encoded `.p12`
- `APPLE_CERTIFICATE_PASSWORD`
- `APPLE_SIGNING_IDENTITY` — e.g. `Developer ID Application: …`
- `APPLE_ID`, `APPLE_PASSWORD` (app-specific), `APPLE_TEAM_ID`

### Windows

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

See [Tauri signing docs](https://v2.tauri.app/distribute/sign/).

## CI notes

Linux CI may still install `tesseract-ocr` / `poppler-utils` as a fallback for odd PDFs; they are not required for the shipped app to do OCR.
