#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DMG_DIR="$ROOT/src-tauri/target/release/bundle/dmg"
mkdir -p "$ROOT/website/downloads" "$ROOT/releases"
cp -f "$DMG_DIR"/*.dmg "$ROOT/website/downloads/"
cp -f "$DMG_DIR"/*.dmg "$ROOT/releases/"
echo "Copied:"
ls -lah "$ROOT/website/downloads"/*.dmg
