#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "${SCRIPT_DIR}/.." && pwd)
PROFILE=${1:-release}

if [[ "${PROFILE}" != "release" && "${PROFILE}" != "debug" ]]; then
  echo "usage: $0 [release|debug]" >&2
  exit 2
fi

APP_NAME="AgenDao"
CLI_BINARY_NAME="agendao"
BUNDLE_NAME="${APP_NAME}.app"
TARGET_DIR="${REPO_ROOT}/../target"
WEB_DIST_DIR="${REPO_ROOT}/apps/agendao-web/dist"
PROFILE_DIR="${PROFILE}"
if [[ "${PROFILE}" == "debug" ]]; then
  PROFILE_DIR="debug"
fi

CLI_BINARY_PATH="${TARGET_DIR}/${PROFILE_DIR}/${CLI_BINARY_NAME}"
DIST_DIR="${REPO_ROOT}/dist/macos"
APP_DIR="${DIST_DIR}/${BUNDLE_NAME}"
CONTENTS_DIR="${APP_DIR}/Contents"
MACOS_DIR="${CONTENTS_DIR}/MacOS"
RESOURCES_DIR="${CONTENTS_DIR}/Resources"
ICONSET_DIR="${REPO_ROOT}/packaging/macos/AgenDao.iconset"
ICNS_PATH="${REPO_ROOT}/icons/agendao.icns"
PLIST_TEMPLATE="${REPO_ROOT}/packaging/macos/Info.plist.template"
PLIST_PATH="${CONTENTS_DIR}/Info.plist"
PKGINFO_PATH="${CONTENTS_DIR}/PkgInfo"
SOURCE_ICON="${REPO_ROOT}/icons/agendao.png"

if [[ ! -f "${SOURCE_ICON}" ]]; then
  echo "missing source icon: ${SOURCE_ICON}" >&2
  exit 1
fi

if [[ ! -f "${PLIST_TEMPLATE}" ]]; then
  echo "missing Info.plist template: ${PLIST_TEMPLATE}" >&2
  exit 1
fi

render_icon_png() {
  local size=$1
  local output=$2

  if command -v sips >/dev/null 2>&1; then
    sips -z "${size}" "${size}" "${SOURCE_ICON}" --out "${output}" >/dev/null
    return
  fi

  if command -v convert >/dev/null 2>&1; then
    convert "${SOURCE_ICON}" -background none -gravity center -resize "${size}x${size}" -extent "${size}x${size}" "${output}"
    return
  fi

  echo "need either sips or convert to render macOS iconset assets" >&2
  exit 1
}

generate_iconset() {
  mkdir -p "${ICONSET_DIR}"
  render_icon_png 16 "${ICONSET_DIR}/icon_16x16.png"
  render_icon_png 32 "${ICONSET_DIR}/icon_16x16@2x.png"
  render_icon_png 32 "${ICONSET_DIR}/icon_32x32.png"
  render_icon_png 64 "${ICONSET_DIR}/icon_32x32@2x.png"
  render_icon_png 128 "${ICONSET_DIR}/icon_128x128.png"
  render_icon_png 256 "${ICONSET_DIR}/icon_128x128@2x.png"
  render_icon_png 256 "${ICONSET_DIR}/icon_256x256.png"
  render_icon_png 512 "${ICONSET_DIR}/icon_256x256@2x.png"
  render_icon_png 512 "${ICONSET_DIR}/icon_512x512.png"
  render_icon_png 1024 "${ICONSET_DIR}/icon_512x512@2x.png"
}

echo "[1/4] Building ${CLI_BINARY_NAME} (${PROFILE})..."
if [[ "${PROFILE}" == "release" ]]; then
  cargo build -p agendao --release
else
  cargo build -p agendao
fi

for path in "${CLI_BINARY_PATH}"; do
  if [[ ! -x "${path}" ]]; then
    echo "expected binary not found: ${path}" >&2
    exit 1
  fi
done

generate_iconset

if [[ "$(uname -s)" == "Darwin" ]] && command -v iconutil >/dev/null 2>&1; then
  echo "[2/4] Regenerating agendao.icns from iconset..."
  iconutil -c icns "${ICONSET_DIR}" -o "${ICNS_PATH}"
fi

if [[ ! -f "${ICNS_PATH}" ]]; then
  echo "missing icns asset: ${ICNS_PATH}" >&2
  exit 1
fi

VERSION=$(awk '
  /^\[workspace\.package\]/ { in_section=1; next }
  /^\[/ && in_section { exit }
  in_section && /^version = / {
    gsub(/version = "/, "", $0)
    gsub(/"/, "", $0)
    print
    exit
  }
' "${REPO_ROOT}/Cargo.toml")
if [[ -z "${VERSION}" ]]; then
  VERSION="unknown"
fi

echo "[3/4] Assembling ${BUNDLE_NAME}..."
rm -rf "${APP_DIR}"
mkdir -p "${MACOS_DIR}" "${RESOURCES_DIR}"
cp "${CLI_BINARY_PATH}" "${MACOS_DIR}/${APP_NAME}"
if [[ -d "${WEB_DIST_DIR}" ]]; then
  cp -R "${WEB_DIST_DIR}" "${RESOURCES_DIR}/web"
else
  echo "Warning: ${WEB_DIST_DIR} not found; bundle will not contain AgenDao Web assets."
fi
cp "${ICNS_PATH}" "${RESOURCES_DIR}/agendao.icns"
printf 'APPLAGND' > "${PKGINFO_PATH}"
sed "s/__AGENDAO_VERSION__/${VERSION}/g" "${PLIST_TEMPLATE}" > "${PLIST_PATH}"

echo "[4/4] Bundle ready:"
echo "  ${APP_DIR}"
