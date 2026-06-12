# Frontend Event Topology Audit

当前提交基线：`717abc3`

这份文档只审一件事：`ServerEvent -> FrontendEvent -> transport -> TUI applier`
是否满足 AgenDao 的单 authority、阴阳闭环、五行流转。

## 五行总图

```text
Server runtime / routes
  -> event_bus : ServerEvent
  -> FrontendProjector
  -> frontend_bus : FrontendEvent

Transport A: HTTP SSE
  /event?session=&tier=
  -> stream_frontend_events()
  -> CustomEvent::FrontendEvent

Transport B: Unix socket event stream
  subscribe_events(session_id, tier)
  -> frontend_bus subscriber
  -> line-delimited FrontendEvent JSON
  -> CustomEvent::FrontendEvent

Transport C: Direct local
  spawn_direct_event_bus()
  -> frontend_bus subscriber
  -> local session filter
  -> CustomEvent::FrontendEvent

All transports
  -> App::apply_frontend_event()
```

## 9 个 FrontendEvent 变体总表

| Variant | Producer authority | HTTP SSE | Unix socket | Direct | TUI consumer | 备注 |
| --- | --- | --- | --- | --- | --- | --- |
| `SessionRuntimeReplaced` | `frontend_projection::project_runtime_replaced()` | `stream_frontend_events()` session+tier 过滤 | 订阅后 session+tier 过滤 | 全局 bus + 本地 session filter | `apply_session_runtime_snapshot()` | runtime 唯一 authority |
| `SessionProjectionReplaced` | `project_projection_replaced()` + `projection_authority` | 同上 | 同上 | 同上 | `apply_session_projection_snapshot()` | attached sessions 在 TUI 本地由 stages 推导 |
| `QuestionUpsert` | `project_question_upsert()` | 同上 | 同上 | 同上 | `upsert_question_request()` | 已基本纯事件驱动 |
| `QuestionRemoved` | `ServerEvent::QuestionResolved` 映射 | 同上 | 同上 | 同上 | `clear_question_tracking()` | 已基本纯事件驱动 |
| `PermissionUpsert` | `project_permission_upsert()` | 同上 | 同上 | 同上 | `enqueue_permission_request()` | 已基本纯事件驱动 |
| `PermissionRemoved` | `ServerEvent::PermissionResolved` 映射 | 同上 | 同上 | 同上 | `clear_permission_request()` | 已基本纯事件驱动 |
| `ToolCallUpsert` | `ServerEvent::ToolCallLifecycle` 映射 | 同上 | 同上 | 同上 | `queue_session_telemetry_refresh()` | 仍非纯事件驱动 |
| `DiffReplaced` | `ServerEvent::DiffUpdated` passthrough | 同上 | 同上 | 同上 | `session_diff.insert()` | 已是 replace authority |
| `OutputBlockAppended` | `ServerEvent::OutputBlock` passthrough | 同上；HTTP 侧不再额外 coalesce frontend event | direct-style coalesce 后发送 | direct-style coalesce 后发送 | `apply_output_block_change()` | live_identity 是 transcript authority 锚点 |

## 三条 transport 对照

### 1. HTTP SSE

- 入口：`routes/event_stream.rs::event_stream()`
- 源：`frontend_bus.subscribe()`
- 过滤：
  - session filter：`frontend_raw_matches_filter()`
  - capability filter：`frontend_event_passes_subscription_caps()`
- tier authority：`ResolvedFrontendSubscription::from_wire_tier()`
- 输出：原始 `FrontendEvent` JSON

结论：

- 金律正确：走 canonical `FrontendEvent`
- 水律较弱：缺少 frontend bus telemetry 对等面

### 2. Unix socket

- 入口：`unix_socket.rs::subscribe_events`
- 当前状态：
  - 显式携带 `tier`
  - 服务端从 canonical `frontend_bus` 订阅
  - 可选 session 预过滤；TUI 默认走全局订阅 + 本地 session filter
  - capability 过滤与 HTTP SSE 同 authority

结论：

- 第一层分叉（缺 tier）已收口
- 第二层分叉（单 session 专线模型）也已向 Direct 同构收敛

### 3. Direct local

- 入口：`spawn_direct_event_bus()`
- 源：`frontend_bus.subscribe()`
- 过滤：TUI 本地 `session_filter`
- 特点：
  - 避免切 session 时 unsubscribe/subscribe 竞态
  - 对 `OutputBlockAppended` 做本地 full-snapshot coalesce

结论：

- 木面最强，火面也通
- 但 direct 特有 coalesce 语义必须与其他 transport 保持同构

## 当前高风险分叉点

### A. Unix socket 未完全加入 subscription tier authority

表现：

- HTTP SSE 已有 `tier=tui|web|cli`
- Unix socket 原先无 tier 参数

风险：

- 同一 `FrontendEvent` 在不同 transport 上可见性不同
- CLI/Web/TUI 若未来复用 Unix transport，会直接破坏金律

### B. ToolCallUpsert 仍是“事件通知 + telemetry 补查”

位置：`crates/agendao-tui/src/app/sync.rs`

旧表现：

- 收到 `ToolCallUpsert` 后并不直接更新本地工具 authority
- 仍然 `queue_session_telemetry_refresh()`

当前状态：

- `ToolCallUpsert` 优先直接增量更新 `session_runtime.active_tools`
- 只有拿不到 runtime authority 时才 fallback 到 telemetry refresh

剩余风险：

- 高频 tool lifecycle 仍有水面回补压力
- 不是完整的单 authority 事件闭环

### C. frontend_bus 缺少独立 telemetry summary

表现：

- `event_bus` 有 send/error/receiver telemetry
- `frontend_bus` 无对等观测

风险：

- projector 放大量、frontend transport lag、frontend-only backlog 不可见

## 收口顺序

1. 先统一 transport subscription authority
   - HTTP SSE / Unix socket / Direct 都显式归到同一 `tier` 语义
2. 再收 `ToolCallUpsert`
   - 做无锁或低锁本地 authority
3. 最后补 `frontend_bus` telemetry
   - 把 projector 下游放大量显式化

## 审计结论

当前主干已经从“多 transport 多语义”收束到“单 `FrontendEvent` 契约 + 单 TUI applier”。
剩余工作不在大方向，而在 transport 细缝：

- Unix socket 的订阅 authority 还要继续归一
- ToolCallUpsert 还没完成真正的事件驱动闭环
- frontend_bus 可观测性仍弱于 event_bus

这三项收完，前端事件链才算真正进入“土稳金成，水足木生”的状态。
