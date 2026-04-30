# ROCode 上下文缓存优化

ROCode 的上下文缓存优化核心原则是：维护一个长期稳定、可复现、可解释的 prompt prefix，而不是在每轮请求里临时拼接更多缓存字段。

## 协议族边界

ROCode 只把缓存策略按内部协议族分派：

| 协议族 | ROCode 名称 | 缓存策略 |
|--------|-------------|----------|
| closeai-compatible | `closeai` | 自动前缀缓存；ROCode 负责稳定 prefix，并在能力明确时附加 `prompt_cache_key` |
| ethnopic-compatible / messages | `ethnopic` / `messages` | 显式 cache breakpoint；ROCode 负责规划稳定边界并写入 `cache_control` |

Provider 具体是谁不是主轴。厂商差异只作为 typed capability / usage parser override 后置处理，不能反过来驱动核心 prompt 结构。

## 稳定提示面

ROCode 会尽量把请求组织成三段：

1. **Stable Zone**
   - provider / model invariant system prompt
   - ROCode developer policy
   - canonicalized tool schemas
   - 稳定的项目摘要与会话摘要
2. **Semi-Stable Zone**
   - artifact summary
   - scheduler state summary
   - memory / source anchors
3. **Dynamic Zone**
   - 当前用户输入
   - 当前 stage brief
   - 临时权限状态
   - 本轮 tool output / retrieval slice

`Session Continuity Context` 的 Coverage、Source Anchors、Memory Anchors 属于 Semi-Stable；Exact Recent Tail 正文和本轮 hydration 结果属于 Dynamic。这样可以保留可追溯性，同时减少因为尾部正文变化导致的前缀失效。

## 输出投影

缓存命中不仅取决于本轮请求，也取决于上一轮输出如何进入下一轮上下文。ROCode 对输出采用 cache-aware projection：

- 用户原始指令、约束、偏好和验收标准保持保真。
- 大型 assistant final、tool output、scheduler stage detail 优先压缩成摘要、metadata 和 artifact reference。
- tool call / tool result 协议轮次不被破坏，避免模型下一轮无法恢复工具语义。
- scheduler 的可见交付和模型上下文投影分离，避免“给用户看的长报告”直接污染下一轮 prompt prefix。

这套策略的目标不是丢信息，而是让模型默认看到稳定摘要，并在需要时通过 anchor / artifact / hydration 按需回查细节。

### Reasoning continuation

模型返回的 thinking / reasoning 不是普通 assistant 文本，也不是可随意摘要的可见输出；它是协议级 continuation state。只要下一轮请求仍处于同一协议族和 thinking mode，ROCode 必须把它作为 typed reasoning part 保留到唯一提示面权威，再由 provider 序列化为对应 wire schema，例如 closeai-compatible 的 `reasoning_content` 或 ethnopic/messages 的 thinking block。

因此：

- agent、scheduler、subtask、projection 层不得把 reasoning 拼进普通 assistant text。
- artifact / output projection 可以压缩可见报告和长 tool output，但不得压缩或丢弃协议必需的 reasoning continuation。
- 如果跨 provider、跨协议族或切换 thinking 字段导致 continuation 不可兼容，应形成新的 continuation boundary，并记录 cache / prompt-surface 诊断，而不是伪装成同一条 hidden reasoning 链。

## 可观测性

CLI、TUI 和 Web 都会显示缓存相关 usage：

- `Cache R/W`：cache read / cache write，常见于显式 breakpoint 语义。
- `Cache H/M`：cache hit / miss，常见于自动前缀缓存或 provider-native usage 语义。
- `Cache read`、`Cache miss`、`Cache write`：在 telemetry / insights 中保留独立字段。

如果 provider 没有返回某个字段，对应值可能为 `0`。例如 cache write 为 `0` 不一定表示缓存未工作；对只暴露 cached tokens 或 hit/miss 的协议族，写入成本可能没有单独字段。

ROCode 还会为每轮请求记录 cache fingerprint，并在检测到明显前缀退化时通过前端显示 cache diagnostic，例如：

```text
Cache hard bust · toolsHash changed: tool schema or order changed
Cache likely bust · messagePrefixHash changed: message prefix changed before the stable boundary
```

诊断分级：

- **hard bust**：model、system、tools、cache control、cache-key-sensitive params 改变。
- **likely bust**：message prefix、prompt cache affinity、continuation 状态改变。
- **soft degradation**：冷启动、provider fallback、请求过短、能力未启用。

## 开发约束

上下文缓存优化受 ROCode 宪法第十条“唯一提示面权威”约束：

- 不能让 system builder、tool builder、scheduler、provider transform 多处独立修改最终 prompt surface。
- tools 必须 canonicalize：built-in 在前，外部 / dynamic 工具后置，schema key 稳定排序。
- closeai-compatible 只在 capability 明确支持时注入 `prompt_cache_key`。
- ethnopic-compatible 的 `cache_control` 由 planner 统一决策，避免多个层各自贴 breakpoint。
- 输出投影必须保留用户 intent 原文；大附件和长日志可以 reference 化，但决策性文本不能随意摘要替代。

相关代码入口：

- `crates/rocode-provider/src/cache.rs`
- `crates/rocode-provider/src/transform/normalize.rs`
- `crates/rocode-session/src/prompt/message_building.rs`
- `crates/rocode-session/src/prompt/loop_lifecycle.rs`
- `crates/rocode-cli/src/run/session_projection.rs`
- `crates/rocode-tui/src/components/sidebar.rs`
- `apps/rocode-web/src/lib/cacheDiagnostics.ts`
