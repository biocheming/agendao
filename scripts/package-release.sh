#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "${SCRIPT_DIR}/.." && pwd)
TARGET_DIR="${REPO_ROOT}/../target"
DIST_DIR="${REPO_ROOT}/dist/release"
PROFILE=${1:-release}
WEB_DIST_DIR="${REPO_ROOT}/apps/rocode-web/dist"

if [[ "${PROFILE}" != "release" && "${PROFILE}" != "debug" ]]; then
  echo "usage: $0 [release|debug]" >&2
  exit 2
fi

PROFILE_DIR="${PROFILE}"
BUILD_ARGS=(-p rocode)
if [[ "${PROFILE}" == "release" ]]; then
  BUILD_ARGS+=(--release)
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

HOST_TRIPLE=$(rustc -vV | awk '/^host: / { print $2; exit }')
ARTIFACT_NAME="rocode-${VERSION}-${HOST_TRIPLE}"
STAGE_DIR="${DIST_DIR}/${ARTIFACT_NAME}"
BIN_DIR="${STAGE_DIR}/bin"
ARCHIVE_PATH="${DIST_DIR}/${ARTIFACT_NAME}.tar.gz"

echo "[1/3] Building rocode (${PROFILE})..."
cargo build "${BUILD_ARGS[@]}"

echo "[2/3] Assembling release layout at ${STAGE_DIR}..."
rm -rf "${STAGE_DIR}"
mkdir -p "${BIN_DIR}"
install -m 755 "${TARGET_DIR}/${PROFILE_DIR}/rocode" "${BIN_DIR}/rocode"
if [[ -d "${WEB_DIST_DIR}" ]]; then
  mkdir -p "${STAGE_DIR}/share/rocode"
  cp -R "${WEB_DIST_DIR}" "${STAGE_DIR}/share/rocode/web"
else
  echo "Warning: ${WEB_DIST_DIR} not found; release bundle will not contain ROCode Web assets."
fi
cp "${REPO_ROOT}/README.md" "${STAGE_DIR}/README.md"
cp "${REPO_ROOT}/docs/installation.md" "${STAGE_DIR}/INSTALL.md"

echo "[3/3] Creating ${ARCHIVE_PATH}..."
mkdir -p "${DIST_DIR}"
tar -C "${DIST_DIR}" -czf "${ARCHIVE_PATH}" "${ARTIFACT_NAME}"

echo "Release bundle ready:"
echo "  ${STAGE_DIR}"
echo "  ${ARCHIVE_PATH}"
