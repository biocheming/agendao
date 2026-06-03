#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WEB_DIR="$ROOT_DIR/apps/agendao-web"
FIXTURE_PATH="$WEB_DIR/fixture-preview.html"
OUTPUT_DIR_DEFAULT="$WEB_DIR/visual-baseline"

LABEL="${1:-baseline}"
OUTPUT_DIR="${2:-$OUTPUT_DIR_DEFAULT}"

if [[ ! -f "$FIXTURE_PATH" ]]; then
  echo "ERROR: fixture preview not found: $FIXTURE_PATH" >&2
  exit 1
fi

if [[ -n "${CHROME_BIN:-}" ]]; then
  CHROME="$CHROME_BIN"
elif command -v google-chrome >/dev/null 2>&1; then
  CHROME="google-chrome"
elif command -v chromium >/dev/null 2>&1; then
  CHROME="chromium"
elif command -v chromium-browser >/dev/null 2>&1; then
  CHROME="chromium-browser"
else
  echo "ERROR: no Chrome/Chromium binary found. Set CHROME_BIN to override." >&2
  exit 1
fi

mkdir -p "$OUTPUT_DIR"

FIXTURE_URL="file://$FIXTURE_PATH"
DESKTOP_OUTPUT="$OUTPUT_DIR/${LABEL}-desktop.png"
MOBILE_OUTPUT="$OUTPUT_DIR/${LABEL}-mobile.png"

COMMON_FLAGS=(
  --headless=new
  --disable-gpu
  --hide-scrollbars
  --no-first-run
  --no-default-browser-check
  --run-all-compositor-stages-before-draw
)

echo "[1/2] Capturing desktop fixture preview..."
"$CHROME" \
  "${COMMON_FLAGS[@]}" \
  --force-device-scale-factor=2 \
  --window-size=1600,1800 \
  --screenshot="$DESKTOP_OUTPUT" \
  "$FIXTURE_URL"

echo "[2/2] Capturing mobile fixture preview..."
"$CHROME" \
  "${COMMON_FLAGS[@]}" \
  --force-device-scale-factor=2 \
  --window-size=860,1700 \
  --screenshot="$MOBILE_OUTPUT" \
  "$FIXTURE_URL"

echo "Captured visual fixture previews:"
echo "  desktop: $DESKTOP_OUTPUT"
echo "  mobile:  $MOBILE_OUTPUT"
echo ""
echo "Example compare flow:"
echo "  $0 baseline"
echo "  $0 candidate"
