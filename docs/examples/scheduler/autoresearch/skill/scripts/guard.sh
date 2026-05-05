#!/usr/bin/env bash

set -euo pipefail

file="${1:-manuscript/book-chapter.md}"

if [[ ! -f "$file" ]]; then
  echo "missing file: $file"
  exit 1
fi

placeholder_hits="$(grep -Eic 'TODO|TBD|lorem ipsum|\[insert|\[placeholder' "$file" || true)"
fence_count="$(grep -c '^```' "$file" || true)"

if (( placeholder_hits > 0 )); then
  echo "guard failed: placeholder content remains"
  exit 1
fi

if (( fence_count % 2 != 0 )); then
  echo "guard failed: unbalanced code fences"
  exit 1
fi

if ! grep -Fxq "## Summary" "$file"; then
  echo "guard failed: missing summary section"
  exit 1
fi

echo "guard=ok"
