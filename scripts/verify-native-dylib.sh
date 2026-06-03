#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PLUGIN_MANIFEST="$ROOT_DIR/examples/native-dylib-plugin/Cargo.toml"
PLUGIN_TARGET_DIR="$ROOT_DIR/examples/native-dylib-plugin/target/release"
VERIFY_EXAMPLE="verify_native_dylib"

echo "[1/4] Building native dylib plugin..."
cargo build --manifest-path "$PLUGIN_MANIFEST" --release >/dev/null

echo "[2/4] Locating built dynamic library..."
PLUGIN_LIB="$(find "$PLUGIN_TARGET_DIR" -maxdepth 1 -type f \( -name "*.so" -o -name "*.dylib" -o -name "*.dll" \) | head -n 1)"
if [[ -z "${PLUGIN_LIB:-}" ]]; then
  echo "ERROR: no dylib artifact found under $PLUGIN_TARGET_DIR" >&2
  exit 1
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
CONFIG_PATH="$TMP_DIR/agendao.json"

cat >"$CONFIG_PATH" <<EOF
{
  "plugin": {
    "native-demo": {
      "type": "dylib",
      "path": "$PLUGIN_LIB"
    }
  }
}
EOF

echo "[3/4] Running native plugin verifier..."
cargo run -p agendao-plugin --example "$VERIFY_EXAMPLE" -- "$CONFIG_PATH" >/dev/null

echo "[4/4] Success"
echo "Native dylib plugin verified:"
echo "  config: $CONFIG_PATH"
echo "  library: $PLUGIN_LIB"
