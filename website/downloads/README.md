# Installers

| Platform | File |
|---|---|
| macOS (Apple Silicon) | `Tidy_1.0.0_aarch64.dmg` |
| Windows x64 | `Tidy_1.0.0_x64-setup.exe` (via GitHub Actions) |
| Linux x86_64 | `Tidy_1.0.0_amd64.AppImage` (via GitHub Actions) |

Build macOS locally:

```bash
cd ../..
npm run tauri build
cp src-tauri/target/release/bundle/dmg/*.dmg website/downloads/
```

Build Windows / Linux: push a `v*` tag or run the **Release** workflow in `.github/workflows/release.yml` on GitHub, then copy artifacts into this folder.
