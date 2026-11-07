# Releasing Tidy

## Platforms

| Platform | Runner | Notes |
|---|---|---|
| macOS | `macos-latest` | DMG + `.app`. Notarization needs Apple secrets (below). |
| Windows | `windows-latest` | NSIS installer. Optional Authenticode via Tauri signing key. |
| Linux | `ubuntu-24.04` | AppImage + `.deb`. Needs glibc ≥ 2.38 (Ubuntu 24.04+, Fedora 39+, Debian 13). |

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

## OCR on release machines

Linux CI installs `tesseract-ocr` and `poppler-utils` so scanned PDFs / screenshots can be read during tests. End users who want OCR should install the same tools locally (`brew install tesseract poppler`, `choco install tesseract`, etc.). Matching still works without them.
