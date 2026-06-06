# Frontend × Transport × Event 能力矩阵（2026-05-28，阶段快照）

这份矩阵是 2026-05-28 时点的覆盖快照，用来说明当时各传输层和事件通道的实现差异。它不是当前版本的正式兼容性承诺。

今天如果要判断产品真相，应以：

- `docs/index.md`
- `docs/commands.md`
- `docs/agendao-统一前端事件契约-2026-06-05.md`

为主，再把这份表当成历史背景。

## Canonical Event Contract
`ServerEvent` in `events.rs` — sole canonical event contract.

## Shared DirectEventBridge
`direct_bridge.rs` 在当时是 TUI 和旧 CLI 交互面共用的 poll bridge。
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
