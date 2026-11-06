# Tidy

Minimalist cross-platform file organizer for macOS and Windows.

Tidy watches a folder you choose (usually Downloads), waits until downloads finish, then moves files into a destination folder as:

```text
Destination/{Subject}/{Type}/filename
```

Subject classification is local and conservative (filename keywords, then light text extraction for PDF/TXT/DOCX). Uncertain files go to `General`.

## Requirements

- Node.js 20+
- Rust stable (via [rustup](https://rustup.rs))
- macOS: Xcode Command Line Tools
- Windows: MSVC Build Tools (for packaging)

## Develop

```bash
cd ~/Development/projects/tidy
npm install
npm run tauri dev
```

## Build installers

```bash
# macOS DMG + copy into website/downloads
npm run release:mac

# Marketing / download site
npm run website
# → http://localhost:4173
```

Artifacts:

- macOS: [`website/downloads/Tidy_1.0.0_aarch64.dmg`](website/downloads/Tidy_1.0.0_aarch64.dmg)
- Windows `.exe` / Linux AppImage: build via GitHub Actions ([`.github/workflows/release.yml`](.github/workflows/release.yml)). Tauri cannot cross-compile those from macOS on this machine.

Landing page: [`website/`](website/)

## V1 screens

1. **Home** — Start/Pause, confirm-before-move, revert previous changes
2. **Import** — syllabus / course list → subject rules
3. **Rules** — file types + editable subjects / presets
4. **Settings** — folders + launch behavior

No account, API key, or cloud service is required.

## Safety

- Never overwrites existing files (appends ` (2)`, ` (3)`, …)
- Never deletes files automatically
- Skips incomplete downloads (`.crdownload`, `.part`, size stability checks)
- Recent moves can be undone
