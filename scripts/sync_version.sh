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
IFS='.' read -r YEAR MONTH DAY <<< "$VERSION"
DATE_ISO="$(printf "%04d-%02d-%02d" "$YEAR" "$MONTH" "$DAY")"

replace_doc_file() {
  local file="$1"
  perl -0pi -e '
    s/v\d{4}\.\d{1,2}\.\d{1,2}/$ENV{VERSION_TAG}/g;
    s/\d{4}-\d{2}-\d{2}/$ENV{DATE_ISO}/g;
    s/\d{4}\.\d{1,2}\.\d{1,2}/$ENV{VERSION}/g
      if $ARGV =~ /(docs\/index\.md|docs\/installation\.md)$/;
  ' "$file"
}

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

for file in \
  "$ROOT_DIR/README.md" \
  "$ROOT_DIR/USER_GUIDE.md" \
  "$ROOT_DIR/docs/README.md" \
  "$ROOT_DIR/docs/index.md" \
  "$ROOT_DIR/docs/installation.md" \
  "$ROOT_DIR/docs/examples/plugins_example/README.md" \
  "$ROOT_DIR/docs/examples/plugins_example/skill/SKILL.md"
do
  VERSION="$VERSION" VERSION_TAG="$VERSION_TAG" DATE_ISO="$DATE_ISO" replace_doc_file "$file"
done

VERSION="$VERSION" replace_package_json_version \
  "$ROOT_DIR/crates/rocode-server/web/package.json"

VERSION="$VERSION" replace_package_lock_root_versions \
  "$ROOT_DIR/crates/rocode-server/web/package-lock.json"

echo "Synced version $VERSION_TAG ($DATE_ISO)"
