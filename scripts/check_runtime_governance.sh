#!/bin/bash
set -e

echo "=== Runtime Governance Checks ==="
echo ""

FAILED=0

# Gate A: ExecutionEvent belongs to orchestrator and its server projection.
echo "1. Checking Gate A: ExecutionEvent isolation..."
if rg -t rust "ExecutionEvent" crates/agendao-provider/src/ crates/agendao-session/src/ 2>/dev/null; then
    echo "FAIL: ExecutionEvent leaked into provider or session"
    FAILED=1
else
    echo "PASS: ExecutionEvent is isolated from provider and session"
fi
echo ""

# Gate B: The scheduler leaf loop has one StreamEvent interpreter.
echo "2. Checking Gate B: StreamEvent interpretation centralization..."
VIOLATIONS=$(rg -t rust "match.*StreamEvent::" \
    --glob '!**/agent_loop/provider.rs' \
    --glob '!**/*_test.rs' \
    --glob '!**/tests/**' \
    crates/agendao-orchestrator/src/ \
    2>/dev/null || true)

if [ -n "$VIOLATIONS" ]; then
    echo "FAIL: Direct StreamEvent matching found outside agent_loop/provider.rs:"
    echo "$VIOLATIONS"
    FAILED=1
else
    echo "PASS: StreamEvent interpretation is centralized"
fi
echo ""

if [ $FAILED -eq 1 ]; then
    echo "=== Governance checks FAILED ==="
    exit 1
else
    echo "=== All governance checks PASSED ==="
    exit 0
fi
