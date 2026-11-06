#!/usr/bin/env bash
# Rewrite website download buttons to match GitHub Release assets for a tag.
# Usage: ./scripts/sync-website-downloads.sh v1.0.0
set -euo pipefail

TAG="${1:?Usage: $0 v1.0.0}"
REPO="${GITHUB_REPOSITORY:-Madan-bhat/tidy}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HTML="$ROOT/website/index.html"

if [[ ! -f "$HTML" ]]; then
  echo "Missing $HTML"
  exit 1
fi

if ! command -v gh >/dev/null 2>&1; then
  echo "gh CLI is required"
  exit 1
fi

echo "Fetching assets for $TAG on $REPO..."
mapfile -t ASSETS < <(gh release view "$TAG" --repo "$REPO" --json assets --jq '.assets[].name')

if [[ ${#ASSETS[@]} -eq 0 ]]; then
  echo "No assets found on release $TAG"
  exit 1
fi

printf 'Assets:\n'
printf '  - %s\n' "${ASSETS[@]}"

pick() {
  local pattern="$1"
  local match=""
  for name in "${ASSETS[@]}"; do
    if [[ "$name" == $pattern ]]; then
      match="$name"
      break
    fi
  done
  echo "$match"
}

DMG="$(pick '*.dmg')"
EXE="$(pick '*-setup.exe')"
[[ -z "$EXE" ]] && EXE="$(pick '*.exe')"
APPIMAGE="$(pick '*.AppImage')"

if [[ -z "$DMG" || -z "$EXE" || -z "$APPIMAGE" ]]; then
  echo "Could not find all installers (dmg=$DMG exe=$EXE appimage=$APPIMAGE)"
  exit 1
fi

BASE="https://github.com/${REPO}/releases/latest/download"

python3 - "$HTML" "$BASE" "$DMG" "$EXE" "$APPIMAGE" <<'PY'
import pathlib
import re
import sys

html_path, base, dmg, exe, appimage = sys.argv[1:]
text = pathlib.Path(html_path).read_text(encoding="utf-8")

def replace_card(html: str, os_key: str, filename: str, base: str) -> str:
    href = f"{base}/{filename}"
    # Replace href on the matching dl-card
    html = re.sub(
        rf'(<a\s+class="dl-card"\s+data-os="{os_key}"\s+href=")[^"]*(")',
        rf"\1{href}\2",
        html,
        count=1,
        flags=re.IGNORECASE,
    )
    # Replace the filename label inside that card (next .dl-file after data-os)
    pattern = rf'(data-os="{os_key}"[\s\S]*?<span class="dl-file">)[^<]*(</span>)'
    html = re.sub(pattern, rf"\1{filename}\2", html, count=1, flags=re.IGNORECASE)
    return html

text = replace_card(text, "mac", dmg, base)
text = replace_card(text, "win", exe, base)
text = replace_card(text, "linux", appimage, base)

pathlib.Path(html_path).write_text(text, encoding="utf-8")
print(f"Updated {html_path}")
print(f"  mac  → {base}/{dmg}")
print(f"  win  → {base}/{exe}")
print(f"  linux→ {base}/{appimage}")
PY
