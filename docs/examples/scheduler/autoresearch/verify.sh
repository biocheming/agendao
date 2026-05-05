#!/usr/bin/env bash

set -euo pipefail

file="${1:-manuscript/book-chapter.md}"

if [[ ! -f "$file" ]]; then
  echo "missing file: $file"
  echo "score=0"
  exit 1
fi

score=0
hard_fail=0

char_count="$(wc -m < "$file" | tr -d ' ')"
mermaid_blocks="$(grep -c '^```mermaid' "$file" || true)"

questions_block="$(
  awk '
    /^## Questions$/ { capture=1; next }
    /^## / && capture { exit }
    capture { print }
  ' "$file"
)"

question_marks="$(printf "%s\n" "$questions_block" | grep -Eo '[?？]' | wc -l | tr -d ' ')"
diagram_types=0
for kind in flowchart sequenceDiagram stateDiagram-v2 mindmap classDiagram journey gantt erDiagram; do
  if grep -q "$kind" "$file"; then
    diagram_types=$((diagram_types + 1))
  fi
done

required_headings=(
  "## Questions"
  "## Philosophical Decomposition and Abstraction"
  "## Technical Workflow"
  "## Imaginative Projection"
  "## Reflection"
  "## Summary"
)

if (( char_count >= 4500 )); then
  score=$((score + 20))
else
  hard_fail=1
fi

if (( question_marks >= 8 )); then
  score=$((score + 10))
else
  hard_fail=1
fi

if (( mermaid_blocks >= 2 )); then
  score=$((score + 10))
else
  hard_fail=1
fi

if (( diagram_types >= 2 )); then
  score=$((score + 10))
else
  hard_fail=1
fi

for heading in "${required_headings[@]}"; do
  if grep -Fxq "$heading" "$file"; then
    score=$((score + 10))
  else
    hard_fail=1
  fi
done

echo "chars=$char_count"
echo "question_marks=$question_marks"
echo "mermaid_blocks=$mermaid_blocks"
echo "diagram_types=$diagram_types"
echo "score=$score"

if (( hard_fail != 0 )); then
  exit 1
fi
