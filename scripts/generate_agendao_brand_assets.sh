#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "${SCRIPT_DIR}/.." && pwd)

ICON_SRC="${REPO_ROOT}/icons/icon.svg"
LOGO_SRC="${REPO_ROOT}/icons/logo.svg"
WEB_PUBLIC_DIR="${REPO_ROOT}/apps/agendao-web/public"
WEB_BRAND_DIR="${WEB_PUBLIC_DIR}/brand"
ICONSET_DIR="${REPO_ROOT}/packaging/macos/AgenDao.iconset"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 1
  fi
}

require_file() {
  if [[ ! -f "$1" ]]; then
    echo "missing required source asset: $1" >&2
    exit 1
  fi
}

require_cmd inkscape
require_cmd convert
require_file "${ICON_SRC}"
require_file "${LOGO_SRC}"

mkdir -p "${WEB_BRAND_DIR}" "${ICONSET_DIR}" "${REPO_ROOT}/icons"

install -m 0644 "${LOGO_SRC}" "${WEB_BRAND_DIR}/agendao-logo.svg"
install -m 0644 "${ICON_SRC}" "${WEB_BRAND_DIR}/agendao-icon-mark.svg"

inkscape "${ICON_SRC}" --export-filename="${WEB_PUBLIC_DIR}/favicon-32.png" -w 32 -h 32 >/dev/null
inkscape "${ICON_SRC}" --export-filename="${WEB_PUBLIC_DIR}/apple-touch-icon.png" -w 180 -h 180 >/dev/null
inkscape "${ICON_SRC}" --export-filename="${REPO_ROOT}/icons/agendao.png" -w 1024 -h 1024 >/dev/null

for size in 16 32 64 128 256; do
  inkscape "${ICON_SRC}" --export-filename="${REPO_ROOT}/icons/.agendao-${size}.png" -w "${size}" -h "${size}" >/dev/null
done

convert \
  "${REPO_ROOT}/icons/.agendao-16.png" \
  "${REPO_ROOT}/icons/.agendao-32.png" \
  "${REPO_ROOT}/icons/.agendao-64.png" \
  "${REPO_ROOT}/icons/.agendao-128.png" \
  "${REPO_ROOT}/icons/.agendao-256.png" \
  "${REPO_ROOT}/icons/agendao.ico"

for size in 16 32 128 256 512; do
  inkscape "${ICON_SRC}" --export-filename="${ICONSET_DIR}/icon_${size}x${size}.png" -w "${size}" -h "${size}" >/dev/null
done

for size in 16 32 128 256 512; do
  doubled=$((size * 2))
  inkscape "${ICON_SRC}" --export-filename="${ICONSET_DIR}/icon_${size}x${size}@2x.png" -w "${doubled}" -h "${doubled}" >/dev/null
done

rm -f "${REPO_ROOT}"/icons/.agendao-*.png

echo "generated AgenDao brand assets:"
echo "  ${WEB_BRAND_DIR}/agendao-logo.svg"
echo "  ${WEB_BRAND_DIR}/agendao-icon-mark.svg"
echo "  ${WEB_PUBLIC_DIR}/favicon-32.png"
echo "  ${WEB_PUBLIC_DIR}/apple-touch-icon.png"
echo "  ${REPO_ROOT}/icons/agendao.png"
echo "  ${REPO_ROOT}/icons/agendao.ico"
echo "  ${ICONSET_DIR}/"
