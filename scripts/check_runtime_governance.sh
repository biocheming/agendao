#!/bin/bash
set -e

echo "=== Runtime Governance Checks ==="
echo ""

FAILED=0

# Gate A: Prevent LoopEvent leakage into provider/runtime/pipeline
echo "1. Checking Gate A: LoopEvent isolation..."
if rg -t rust "LoopEvent" crates/agendao-provider/src/runtime/pipeline/ 2>/dev/null; then
    echo "❌ FAIL: LoopEvent found in provider/runtime/pipeline (Gate A violation)"
    echo "   LoopEvent must only exist in agendao-orchestrator/src/runtime/"
    FAILED=1
else
    echo "✅ PASS: No LoopEvent in provider/runtime/pipeline"
fi
echo ""

# Gate B: Prevent direct StreamEvent interpretation outside normalizer
echo "2. Checking Gate B: StreamEvent interpretation centralization..."
VIOLATIONS=$(rg -t rust "match.*StreamEvent::" \
    --glob '!**/normalizer.rs' \
    --glob '!**/*_test.rs' \
    --glob '!**/tests/**' \
    crates/agendao-orchestrator/src/ \
    crates/agendao-agent/src/ \
    crates/agendao-session/src/ \
    crates/agendao-cli/src/ \
    crates/agendao-server/src/ \
    2>/dev/null || true)

if [ -n "$VIOLATIONS" ]; then
    echo "❌ FAIL: Direct StreamEvent pattern matching found outside normalizer:"
    echo "$VIOLATIONS"
    echo "   StreamEvent interpretation must only happen in runtime/normalizer.rs"
    FAILED=1
else
    echo "✅ PASS: No direct StreamEvent interpretation outside normalizer"
fi
echo ""

if [ $FAILED -eq 1 ]; then
    echo "=== Governance checks FAILED ==="
    exit 1
else
    echo "=== All governance checks PASSED ==="
    exit 0
fi
