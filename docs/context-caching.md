# ROCode 上下文缓存优化

ROCode 的上下文缓存优化核心原则是：维护一个长期稳定、可复现、可解释的 prompt prefix，而不是在每轮请求里临时拼接更多缓存字段。

这份文档现在不只解释“怎么命中缓存”，也解释 ROCode 如何界定上下文所有权。缓存优化不能靠偷偷塞更多历史；它依赖的是对 prompt surface、child handoff、compaction boundary 和 request-view checkpoint 的统一治理。

## 协议族边界

ROCode 只把缓存策略按内部协议族分派：

| 协议族 | ROCode 名称 | 缓存策略 |
|--------|-------------|----------|
| closeai-compatible | `closeai` | 自动前缀缓存；ROCode 负责稳定 prefix，并在能力明确时附加 `prompt_cache_key` |
| Ethnopic-compatible | `ethnopic` | 显式 cache breakpoint；ROCode 负责规划稳定边界并写入 `cache_control`。底层 wire path 仍可能是 `/messages`。 |

Provider 具体是谁不是主轴。厂商差异只作为 typed capability / usage parser override 后置处理，不能反过来驱动核心 prompt 结构。

## 三本账：请求、会话、工作流

ROCode 现在把上下文与用量拆成三本账，而不是继续把它们折叠成一个“Current”数字：

- `request_context_tokens`
  - 下一次真正发给 provider 的 request-view 估算大小
- `live_context_tokens`
  - 当前 session 自己拥有的 live prefix / live pressure
- `workflow_cumulative_tokens`
  - 整个 workflow 累计消耗，包含 child session / subsession / attached subtree 的花费

这三本账对应三个不同的问题：

- 这一轮还发不发得出去
- 当前 session 的 prefix 还能不能保持稳定
- 这条 workflow 到现在一共花了多少

因此：

- `Turn` 反映最近一次已完成调用的真实输入/输出
- `Current` 应结合 request/live 账本理解，而不是拿它去替代累计消耗
- `Session` / `workflow cumulative` 是成本账，不是下一轮 prompt 的真实大小

CLI、TUI、Web 现在都能从 `context_closure_contract` 读到这三本账的解释，而不是各端自己猜 usage 含义。

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

## Child / Fork 边界

上下文稳定还依赖 child handoff 语义。ROCode 当前收口后的规则是：

- parent 只向 child 传显式 packet，而不是把 parent 的完整运行史隐式倒给 child
- child 回来时，parent 只吸收 result / summary，不回收 child 内部全部历史
- attached subtree 的 token 成本进入 `workflow_cumulative_tokens`，但不应泄漏进 parent 的 live prefix
- 显式 full-history fork 是单独语义：它允许导入历史，但会冻结 fork policy，并把 imported history 视为只读来源

这套规则的目标不是节省一点点 token，而是保证缓存语义始终围绕 owner-local prompt surface，而不是围绕整棵 workflow 的混合历史。

## 输出投影

缓存命中不仅取决于本轮请求，也取决于上一轮输出如何进入下一轮上下文。ROCode 对输出采用 cache-aware projection：

- 用户原始指令、约束、偏好和验收标准保持保真。
- 大型 assistant final、tool output、scheduler stage detail 优先压缩成摘要、metadata 和 artifact reference。
- tool call / tool result 协议轮次不被破坏，避免模型下一轮无法恢复工具语义。
- scheduler 的可见交付和模型上下文投影分离，避免“给用户看的长报告”直接污染下一轮 prompt prefix。

这套策略的目标不是丢信息，而是让模型默认看到稳定摘要，并在需要时通过 anchor / artifact / hydration 按需回查细节。

### Reasoning continuation

模型返回的 thinking / reasoning 不是普通 assistant 文本，也不是可随意摘要的可见输出；它是协议级 continuation state。只要下一轮请求仍处于同一协议族和 thinking mode，ROCode 必须把它作为 typed reasoning part 保留到唯一提示面权威，再由 provider 序列化为对应 wire schema，例如 closeai-compatible 的 `reasoning_content` 或 Ethnopic-compatible thinking block。

因此：

- agent、scheduler、subtask、projection 层不得把 reasoning 拼进普通 assistant text。
- artifact / output projection 可以压缩可见报告和长 tool output，但不得压缩或丢弃协议必需的 reasoning continuation。
- 如果跨 provider、跨协议族或切换 thinking 字段导致 continuation 不可兼容，应形成新的 continuation boundary，并记录 cache / prompt-surface 诊断，而不是伪装成同一条 hidden reasoning 链。

## Message Replay Authority

上下文缓存和 replay authority 现在需要一起理解，而不是分开理解。

当前 replay authority 的关键约束是：

- assistant 历史 replay 走共享 authority，而不是 session / provider / orchestrator 各自拼装。
- canonical replay ordering 固定为：
  - `reasoning -> text -> tool_use -> tool_result -> file`
- text-only assistant turn 允许保持 `Content::Text`。
- raw tool-call replay shape 优先于 normalized shape。
- downgraded tool-summary 必须留在普通上下文，不得重新回到 `Role::Tool`。

这和缓存直接相关，因为：

- reasoning continuation 丢失会破坏同一 continuation boundary。
- tool-result role 漂移会让下一轮 request shape 不再稳定。
- projection 如果把 reasoning/tool-use assistant turn 误投影成纯 summary，也会导致 prefix 与 replay 语义同时退化。

因此，今天讨论 cache hit / miss，不能只看 `cache_control` 或 `prompt_cache_key`；还必须看 replay authority 是否保持了协议连续性。

## Tool Trajectory Quality

ROCode 现在不只记录 cache 和 usage，也会记录工具轨迹质量。

`tool_trajectory_quality` 的作用不是替代执行结果，而是解释：

- 这次工具轨迹到底有多干净
- 有没有大量 repair / sanitizer / provider diagnostics
- 当前成功是“干净成功”还是“修出来的成功”

CLI、TUI、Web 都会读这个 summary。典型信号包括：

- `score`
- `band`
- `repaired_tool_call_count / total_tool_calls`
- `error_tool_call_count`
- `sanitizer_event_count`
- `strict_would_fail_count`

它和 cache / context closure 的关系是：

- 工具轨迹越脏，下一轮 prompt surface 越容易失稳。
- 轨迹质量越差，缓存命中率和工具连续性解释就越不能只看单轮 usage。
- repair telemetry 与 trajectory quality 提供的是“这次为什么还能跑下去”的解释层。

## 可观测性

CLI、TUI 和 Web 都会显示缓存相关 usage：

- `Cache R/W`：cache read / cache write，常见于显式 breakpoint 语义。
- `Cache H/M`：cache hit / miss，常见于自动前缀缓存或 provider-native usage 语义。
- `Cache read`、`Cache miss`、`Cache write`：在 telemetry / insights 中保留独立字段。

如果 provider 没有返回某个字段，对应值可能为 `0`。例如 cache write 为 `0` 不一定表示缓存未工作；对只暴露 cached tokens 或 hit/miss 的协议族，写入成本可能没有单独字段。

ROCode 还会为每轮请求记录 cache fingerprint，并在检测到明显前缀退化时通过前端显示 cache diagnostic，例如：

```text
Cache high change · tool surface changed
Cache medium change · prefix changed before the stable boundary
```

诊断分级：

- **high change**：model、system、tools、cache control、cache-key-sensitive params 改变。
- **medium change**：message prefix、prompt cache affinity、continuation 状态改变。
- **low change**：冷启动、provider fallback、请求过短、能力未启用。

这些 cache 诊断现在优先从 `context_closure_contract` 读取词汇，而不是让消息级 cache 文案、telemetry 文案和前端 sidebar 各自生成一套表述。

## Context Closure Contract

为了把“为什么没命中缓存”说清楚，ROCode 现在用一份统一的 `context_closure_contract` 来汇总四类事实：

- `prefix_stability`
  - prefix 是否在 API view 上保持稳定，是否发生了 stable boundary 之前的变化
- `compaction_boundary`
  - 这轮是否记录了 compaction boundary、是否尝试过 compact、是否已经进入 block
- `cache_explainability`
  - 当前 cache 问题是否已经能用 cache evidence / surface evidence / boundary evidence 解释
- `child_history_isolation`
  - child/subsession 的累计成本是否只留在 workflow 账本，而没有泄漏进 owner-local live prefix

这意味着：

- cache bust 不再只是某条 message metadata 里的零散标签
- CLI / TUI / Web 会尽量复用同一套 closure 词汇来表达 prefix change、boundary recorded、cache explained、isolation leaked
- artifact/export 读面也可以通过 diagnostics sidecar 复用同一份解释语义

## 运行中 Checkpoint

过去只在 pre-run / post-run 两头检查上下文压力，很容易在一次长链执行里错过真正的溢出点。现在 run loop 自己持有 request view，并在 step checkpoint 上重新评估：

- 当前 request-view 估算 token
- 当前模型的 exact / best-effort limit
- 是否还有 compact/rewrite 尝试额度
- 是否应该继续、compact request view，或者直接 block 下一次 model call

默认策略已经下沉到 runtime loop / runtime policy；外层 hook 更适合做 override 和 observability，而不是重新担任 owner。

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
- `crates/rocode-orchestrator/src/runtime/policy.rs`
- `crates/rocode-orchestrator/src/runtime/loop_impl.rs`
- `crates/rocode-cli/src/run/session_projection.rs`
- `crates/rocode-tui/src/components/sidebar.rs`
- `apps/rocode-web/src/lib/cacheDiagnostics.ts`
