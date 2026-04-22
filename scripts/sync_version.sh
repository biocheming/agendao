#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ROOT_CARGO="$ROOT_DIR/Cargo.toml"

VERSION="$(
  perl -0ne '
    if (/\[workspace\.package\]\n(?:.*\n)*?version = "(\d{4}\.\d{1,2}\.\d{1,2})"/s) {
      print "$1\n";
      exit 0;
    }
    exit 1;
  ' "$ROOT_CARGO"
)"

VERSION_TAG="v$VERSION"

replace_package_json_version() {
  local file="$1"
  perl -0pi -e '
    s/^(\s*"version"\s*:\s*")[^"]+(")/$1$ENV{VERSION}$2/m;
  ' "$file"
}

replace_package_lock_root_versions() {
  local file="$1"
  perl -0pi -e '
    s/^(\s*"version"\s*:\s*")[^"]+(")/$1$ENV{VERSION}$2/m;
    s/("packages"\s*:\s*\{\s*""\s*:\s*\{.*?\n\s*"version"\s*:\s*")[^"]+(")/$1$ENV{VERSION}$2/s;
  ' "$file"
}

replace_rocode_lock_versions() {
  local file="$1"
  perl -0pi -e '
    s/(\[\[package\]\]\nname = "rocode(?:-[^"]+)?\"\nversion = ")\d{4}\.\d{1,2}\.\d{1,2}(")/$1$ENV{VERSION}$2/g;
  ' "$file"
}

VERSION="$VERSION" replace_package_json_version \
  "$ROOT_DIR/crates/rocode-server/web/package.json"

VERSION="$VERSION" replace_package_lock_root_versions \
  "$ROOT_DIR/crates/rocode-server/web/package-lock.json"

for file in \
  "$ROOT_DIR/Cargo.lock" \
  "$ROOT_DIR/docs/examples/plugins_example/rust/Cargo.lock"
do
  VERSION="$VERSION" replace_rocode_lock_versions "$file"
done

echo "Synced owned package versions to $VERSION_TAG"
