# Frontend × Transport × Event 能力矩阵（2026-05-28）

## Canonical Event Contract
`ServerEvent` in `events.rs` — sole canonical event contract.

## Shared DirectEventBridge
`direct_bridge.rs` — TUI+CLI shared poll bridge. Both frontends consume same event stream.
Unix: `subscribe_events` → `DirectEvent` JSON lines.

## Transport × Event Coverage Matrix

| Event | HTTP | Direct | Unix |
|-------|------|--------|------|
| session_status | ✅ | ✅ | ✅ |
| session_updated | ✅ | ✅ | ✅ |
| question | ✅ | ✅ | ✅ |
| permission | ✅ | ✅ | ✅ |
| output_block | ✅ | ⬜ | ⬜ |
| tool_lifecycle | ✅ | ✅ | ✅ |
| topology | ✅ | ✅ | ✅ |
| config | ✅ | ✅ | ✅ |
| control_input | ✅ | ⬜ | ⬜ |
| error | ✅ | ✅ | ✅ |
| diff | ✅ | ✗ | ✗ |
| usage | ✅ | ✗ | ✗ |
| session_tree | ✅ | ✗ | ✗ |

✅ = supported, ⬜ = partial (poll-text-only or enum exists but no full consumer), ✗ = not supported

## Notes
- Direct `output_block`: text parts only (poll-based), no tool/system/structured blocks
- `control_input`: `DirectEvent::ControlInputTransition` variant exists, emission + consumer partial
- `diff`/`session_tree`: `DirectEvent` variants exist but no poll-loop emission logic
- `usage`: no variant in `DirectEvent`, needs telemetry API
