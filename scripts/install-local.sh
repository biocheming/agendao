#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "${SCRIPT_DIR}/.." && pwd)
TARGET_DIR="${REPO_ROOT}/../target"
PROFILE=${1:-release}
PREFIX=${2:-"${HOME}/.local"}
WEB_DIST_DIR="${REPO_ROOT}/apps/agendao-web/dist"

if [[ "${PROFILE}" != "release" && "${PROFILE}" != "debug" ]]; then
  echo "usage: $0 [release|debug] [prefix]" >&2
  exit 2
fi

PROFILE_DIR="${PROFILE}"
BUILD_ARGS=(-p agendao)
if [[ "${PROFILE}" == "release" ]]; then
  BUILD_ARGS+=(--release)
fi

BIN_DIR="${PREFIX}/bin"
mkdir -p "${BIN_DIR}"

echo "[1/2] Building agendao (${PROFILE})..."
cargo build "${BUILD_ARGS[@]}"

echo "[2/2] Installing binaries into ${BIN_DIR}..."
install -m 755 "${TARGET_DIR}/${PROFILE_DIR}/agendao" "${BIN_DIR}/agendao"

if [[ -d "${WEB_DIST_DIR}" ]]; then
  SHARE_DIR="${PREFIX}/share/agendao"
  mkdir -p "${SHARE_DIR}"
  rm -rf "${SHARE_DIR}/web"
  cp -R "${WEB_DIST_DIR}" "${SHARE_DIR}/web"
else
  echo "Warning: ${WEB_DIST_DIR} not found; agendao web will need AGENDAO_WEB_DIST or a separate web build."
fi

echo "Installed:"
echo "  ${BIN_DIR}/agendao"
if [[ -d "${WEB_DIST_DIR}" ]]; then
  echo "  ${PREFIX}/share/agendao/web"
fi
