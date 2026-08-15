# Scheduler-native Harness 重建计划

状态：已完成（Phase 1-7 与原子切换门禁全部通过）

日期：2026-08-14

范围：`agendao-orchestrator`、所有生产入口、agent/scheduler 选择、skill 组合、autoresearch、
verifier、上下文缓存，以及切换后触发的全仓死代码/依赖/资源审计

性质：破坏性架构重建，不承担旧接口兼容

实施记录（截至 2026-08-14）：

- Phase 1：Blueprint/Catalog/Policy/Validator 与稳定 fingerprint 已完成。
- Phase 2：唯一 AgentLoop、唯一 SchedulerEngine、五种节点、全局预算、deadline、取消、
  scoped result path 和结构化事件已完成。
- Phase 3：stable/semi-stable/dynamic prompt surface 已接入 AgentLoop；分层 cache diagnostic
  已完成。
- Phase 4：五个纯数据模板、用户显式选择、session lock、auto selector 和 typed AI planner
  已完成；所有输出经过同一 Validator，失败不 fallback。
- Phase 5：真实 verifier、checkpoint、artifact/workspace capability 已在核心之外完成；blocking I/O、
  路径/文件数/字节/deadline/磁盘余量限制和确定性 cleanup 已通过测试。
- Phase 6：HTTP、Unix、CLI、TUI、Web 已一次性切到唯一 SchedulerEngine 和 typed execution
  projection；旧 authority、旧配置、旧文档和第二套 AgentLoop 已物理删除。
- Phase 7：public API、依赖、allocation、上下文缓存和资源生命周期已收缩；workspace tests、
  clippy、machete、udeps、Web 门禁、旧符号搜索和 diff hygiene 全部通过。
- 新实现已从临时隔离 `native/` 命名空间提升到 `agendao-orchestrator` crate 根；`native/` 已物理
  删除，不存在 re-export、路径 alias 或 `v2` 命名。canonical session/server 入口均已接入。
- Direct authority 和 server orchestration adapter 已删除；HTTP 与 Unix 共用 canonical
  server/session authority，不得重新引入第三条本地直连 authority。
- 旧 orchestrator `runtime/`、scheduler profile/stages/presets、agent tree、skill graph/list/tree、
  workflow/verifier authority 已物理删除；`/start-work`、`task`、`task_flow`、`media_inspect` 等绕过
  Blueprint 的独立编排入口也已删除。
- provider 已收缩为 OpenAI Responses、OpenAI Chat Completions、Anthropic Messages 三种协议；
  不再保留其他 provider protocol 的兼容壳。
- workspace/checkpoint/artifact I/O 位于 server 宿主；核心不包含同步 Git、文件系统遍历或 shell
  authority。
- typed execution event 已成为唯一运行投影；不存在“新路径可用但旧路径仍保留”的中间状态。

## 1. 决策摘要

AgenDao 的 harness 以 **Scheduler** 作为唯一编排抽象，并在新模块中从零实现唯一的
`AgentLoop` 作为 leaf agent 执行循环。现有 `runtime::run_loop` 只作为行为与测试场景参考，
不得成为新模块依赖。

Agent、skill、verifier、autoresearch 不再各自发展成另一套 orchestrator：

- Agent 是 scheduler graph 中的叶节点配置，不拥有另一套编排内核。
- Skill 是可解析、可组合的能力与上下文引用，不是执行引擎。
- Verifier 是 gate/evaluator 能力，不是独立 scheduler 实现。
- Autoresearch 是有界 loop、checkpoint 和 verifier 的组合模板，不是独立执行内核。
- 内置 agents/schedulers 是数据化模板，不包含重复的 Rust 控制流。
- 用户声明、内置模板、AI 临时生成都产生同一种 `SchedulerBlueprint`。
- JSONC 只是 Blueprint 的一种持久化格式，不是 scheduler 的唯一入口。

默认模式是 `auto`：harness 根据任务、可用 agent、skill、tool、模型能力和治理预算，
选择内置模板或生成新的 Blueprint。用户显式指定 scheduler/Blueprint 时，用户选择优先，
harness 不得暗中改换拓扑。

## 2. 不可违反的重建规则

以下规则是本计划的硬约束，任何实施 PR 都不得以“先兼容、以后清理”为理由违反。

1. 不实现 legacy compiler，不把旧 SchedulerProfile、AgentTree、SkillGraph 或旧 JSONC
   转译成新 Blueprint。
2. 不实现 adapter、shim、bridge、facade 或兼容 trait 来让新内核继续调用旧实现。
3. 不保留 deprecated re-export、类型别名、旧字段别名或旧 API fallback。
4. 不允许新旧执行路径在生产中双跑，也不允许失败后回退旧内核。
5. 不引入 `v1/v2` 长期分支和运行时 feature switch。
6. 旧代码只允许用于阅读行为和提取测试场景，不能进入新模块依赖图。
7. 新模块在隔离命名空间中完整实现；切换时在一个原子变更中改完调用方、内置配置、
   文档和测试，并同时删除旧模块。
8. 外部旧 JSONC 视为破坏性升级：配置入口只按新 schema 解析，解析失败即报告新 schema
   要求；不得为了给旧格式定制错误而实现旧 schema detector，也不提供转换器或静默降级。
9. 新实现不得复制旧模块代码后继续演化；SchedulerEngine、AgentLoop、消息状态和事件契约
   都以新领域模型重新实现。可以依赖 provider/session/types 等 crate 的稳定公共契约，但不能
   import `agendao-orchestrator` 的任何旧 runtime、scheduler、tree、graph 或 workflow 模块。
10. 本计划不接受“临时兼容”。禁止项覆盖源代码、配置、CLI/UI、数据库 metadata、测试、fixture、
    文档、feature flag、可选依赖、build script 和生成代码；不能把旧路径藏在非默认 feature 或
    测试 helper 中。
11. 旧代码只可用于提取外部可观察行为、错误场景和测试输入。不得复制旧控制流，不得让新测试
    import 旧类型，也不得以 replay fixture 的形式保留旧 schema 作为可加载输入。
12. 一次性切换是架构验收条件而非排期偏好：调用方、配置、文档和测试迁移与旧模块物理删除必须
    属于同一个可合并变更集。评审不得批准“先合入 adapter/双路径，另开 issue 清理”。

允许隔离开发阶段由旧模块服务产品。进入 Phase 6 后，未发布工作树可以暂时处于调用方迁移的
不可发布中间态，但最终可合并变更集的生产调用图中不得出现两套 scheduler authority；任何中间
commit 都不能单独发布或拆出合并。

## 3. 目标与非目标

### 3.1 目标

- 一个 Scheduler IR、一个 validator、一个 engine、一个全新 leaf loop。Engine 直接解释
  `ValidatedBlueprint`，不设置 compiler 层。
- AI 能选择内置模板，也能根据任务即时生成合法的新 scheduler。
- AI 能选择和组合 skills，形成有界 graph，而不要求用户先写 JSONC。
- 用户可显式选择、编辑、保存、检查或拒绝 AI 生成的 Blueprint。
- 所有自治行为都受统一预算、权限、可观测性和上下文边界治理。
- 结构足够小，增加新模板主要增加数据，不增加新的 orchestrator 实现。
- 保持 prompt prefix 稳定，避免动态 scheduler 信息破坏 provider context cache。
- 自由扩展只通过 Blueprint、Catalog 注册项和宿主 capability 契约进入；不允许通过新增
  orchestrator trait 实现、专用执行器或任意脚本控制流扩展调度语义。

### 3.2 非目标

- 不保留旧 scheduler schema 或旧 API 的运行时兼容。
- 不让模型生成 Rust、脚本或任意控制流来扩展 scheduler。
- 不让 Blueprint 绕过 tool permission、session authority 或 workspace boundary。
- 不把完整 child history、graph 运行史或 verifier 日志塞回 parent prompt。
- 不以“支持所有可能工作流”为理由扩展 IR；只保留可验证的最小控制原语。

## 4. 统一领域模型

### 4.1 四个概念的唯一含义

| 概念 | 唯一职责 | 禁止承担的职责 |
|---|---|---|
| Agent | 模型、system policy、允许的 tools、leaf loop policy | graph、重试拓扑、checkpoint、阶段调度 |
| Skill | 可发现的知识、方法、约束或能力描述 | 自己执行 agent loop |
| Scheduler | 组织节点、边、预算、gate、loop 和输出 | provider 协议、session 持久化、文件复制 |
| Capability | verifier、checkpoint、artifact 等宿主服务 | 自己决定任务拓扑 |

现有 `SkillListOrchestrator` 应重建为 Blueprint 的 `Agent` 节点；现有
`SkillGraphOrchestrator` 和 `AgentTreeOrchestrator` 不再作为独立执行器存在。

### 4.2 SchedulerBlueprint

Blueprint 是可序列化、可校验、与来源无关的声明。建议最小结构：

```rust
pub struct SchedulerBlueprint {
    pub schema: BlueprintSchemaVersion,
    pub name: BlueprintName,
    pub entry: NodeId,
    pub nodes: BTreeMap<NodeId, NodeSpec>,
    pub limits: ExecutionLimits,
    pub output: OutputContract,
}

pub enum NodeSpec {
    Agent(AgentNode),
    Parallel(ParallelNode),
    Gate(GateNode),
    Loop(LoopNode),
    End(EndNode),
}
```

顺序由节点的有向边表达，不再增加 `Sequence` 类型。条件分支只由 `Gate` 表达；重复只由
`Loop` 表达。这样 validator 不需要理解多套等价控制结构。

### 4.3 节点语义

#### Agent

- 引用一个 `AgentProfile`，可附加本节点的 skill refs。
- 声明 tool policy、模型约束和 leaf `LoopPolicy`。
- 唯一执行方式是调用新模块自己的 `AgentLoop`。
- agent profile 不包含 child、next stage、retry graph 或 scheduler 名称。

#### Parallel

- 引用固定的 branch entry nodes 和一个 join node。
- 必须声明 `max_parallelism`，并受全局更小上限约束。
- 只传显式 handoff packet；不得复制 parent 全历史。
- 一个 branch 失败时的 fail-fast/collect 策略必须显式声明。

#### Gate

- 调用一个 typed evaluator，产生 `pass/fail/indeterminate`。
- Verifier 是 evaluator 的一种实现。
- Gate 只决定下一条边，不直接执行恢复、文件回滚或模型循环。

#### Loop

- 声明 body、gate、最大迭代数和退出边。
- `max_iterations` 必填且必须大于零。
- 可请求 checkpoint capability，但 checkpoint 的具体 I/O 由宿主服务执行。
- Autoresearch 是这种节点的内置 Blueprint 模板，不是特殊 runtime。

#### End

- 选择最终结果、摘要或 artifact reference。
- 不重新调用模型，不隐式执行 synthesis；需要 synthesis 时显式放一个 Agent 节点。

## 5. Catalog：AI 与用户共享的能力视图

AI 不能凭 prompt 猜测系统拥有什么。`SchedulerCatalog` 是生成和校验 Blueprint 的唯一能力清单：

```text
SchedulerCatalog
├── agents: id + model/tool/skill constraints
├── skills: id + summary + input/output capability tags
├── tools: id + effect class + permission class
├── evaluators: verifier/metric/policy gates
├── workspace capabilities: checkpoint/artifact/read-only
├── model capabilities: context/tool/reasoning/attachment
└── policy envelope: hard budgets and denied capabilities
```

Catalog 必须：

- 使用稳定排序和稳定 ID。
- 带 revision/fingerprint；任何成员变化都会产生新 fingerprint。
- 对 AI 暴露摘要而不是把每个 skill 全文一次性塞入选择 prompt。
- 在 Blueprint 选定 skill 后才按需 hydration 详细内容。
- 由真实 registry 生成，不能由 scheduler prompt 手写第二份清单。

## 6. 自治选择与生成

### 6.1 选择优先级

```text
用户显式 Blueprint / scheduler 名称
        ↓ 无显式选择
会话已锁定且仍适用于当前任务的 Blueprint
        ↓ 无锁定方案
Auto Selector
        ├── 选择内置模板
        ├── 生成新的 BlueprintDraft
        └── 对简单请求选择 single-agent Blueprint
```

- 显式用户选择必须被记录为 `selection_source=user`，AI 不得覆盖。
- `auto` 是默认选择，不等于总是构造复杂 graph。
- 简单问答和单工具任务应落到 single-agent Blueprint，避免多一次昂贵规划调用。
- 复杂度、风险、跨领域 skill 需求或验证要求达到阈值时，才调用 AI Planner。
- 选择结果按 session/task scope 固定；运行中不得每轮重新选择造成 prompt 和 cache 抖动。

### 6.2 AI Planner 的输出

AI Planner 接收：

- 用户原始目标与硬约束。
- Catalog 摘要和 fingerprint。
- system/admin/user policy 合并后的预算上限。
- 当前 workspace 的稳定摘要，不接收完整会话和 tool 日志。

它只能返回两种 typed 结果：

```text
UseTemplate { template_id, parameter_bindings }
CreateBlueprint { blueprint }
```

模型不得直接写 JSONC 文件。生成结果是内存中的 `BlueprintDraft`；通过 validator 后成为
`ValidatedBlueprint`。只有用户明确要求保存时，才由配置层把它序列化为 JSONC。

Skill 组合采用两阶段解析，避免 Planner prompt 随 skill 数量线性膨胀：

1. Planner 只读取稳定排序的 skill ID、短摘要、capability tags 和 catalog fingerprint，选择
   候选 skill 与 graph 拓扑。
2. 宿主仅 hydration 被选中的 skill 正文和依赖，重新解析约束并对完整 Blueprint 再做一次
   validator 校验；hydration 或复验失败即终止规划，不改选 direct，也不回退旧路径。

AI 自动生成与用户手写 Blueprint 没有权限差异。两者必须走相同的 catalog resolution、
canonicalization、validator、policy 收紧和审计事件；`selection_source` 只记录来源，不参与
授权。

### 6.3 运行中调整

第一版不允许任意 self-modifying graph。确有运行中发现新任务的需求时，只允许在节点边界提交
typed `PlanPatch`：

- 只能增加尚未执行的节点或收紧预算。
- 不能改写已执行节点和历史。
- 不能提高权限、并发、token、成本或迭代上限。
- 必须重新通过完整 validator。
- patch 次数有硬上限，并进入审计事件。

若这套需求尚未被真实场景证明，第一版不实现 `PlanPatch`，避免预先制造复杂度。

## 7. 治理模型

治理不通过增加专用 scheduler 实现完成，而通过统一 `PolicyEnvelope` 完成。

### 7.1 硬预算

所有 Blueprint 必须同时具备：

- `max_model_calls`
- `max_tool_calls`
- `max_total_tokens` 或 provider 可用的成本上限
- `max_wall_time`
- `max_parallelism`
- `max_graph_nodes`
- `max_graph_depth`
- 每个 Loop 的 `max_iterations`
- 每个 Agent 的有限 `max_steps`

禁止 `Unbounded`。AI 或用户 Blueprint 只能收紧上级 policy，不能扩大它。

### 7.2 Validator

Validator 是纯函数，不做 I/O。它至少拒绝：

- 缺失或不可达的 entry/end。
- 未解析的 agent、skill、tool、evaluator ID。
- 普通边形成的环；循环只能存在于 typed `Loop`。
- 无迭代上限、无退出边或无 gate 的 Loop。
- 无并发上限的 Parallel。
- 超过 policy envelope 的预算。
- 请求不存在的模型能力或 permission class。
- effectful workflow 缺少 checkpoint/approval policy。
- 节点输出与后继输入 contract 不兼容。

### 7.3 权限和副作用

- Blueprint 只能声明期望权限，最终授权仍由 permission authority 决定。
- AI Planner 不能把 read-only task 提升为 workspace mutation。
- checkpoint、Git、文件复制、命令执行都通过 capability trait 注入。
- `agendao-orchestrator` 核心不得直接依赖 `std::fs`、`walkdir` 或
  `std::process::Command`。
- autoresearch 的 workspace capability 在阻塞线程池执行，并有文件数、字节数、路径和时间上限。

## 8. 上下文缓存设计

上下文缓存是 Blueprint 和 prompt authority 的设计约束，不是 provider 层补丁。

### 8.1 三段提示面

每个 Agent 节点的请求严格组织为：

1. **Stable Zone**
   - harness/system policy
   - resolved AgentProfile
   - canonical tool schemas
   - Blueprint 中与该节点相关的静态 policy
   - 已选 skill 的稳定摘要
2. **Semi-Stable Zone**
   - session/workspace summary
   - parent handoff packet
   - artifact anchors
   - scheduler progress summary
3. **Dynamic Zone**
   - 当前 node input
   - 最近必要 history tail
   - tool results、retrieval slice、临时 permission 状态

运行状态、轮次计数和 telemetry 不得插入 Stable Zone。

### 8.2 指纹层级

使用分层 fingerprint，而不是每轮 hash 整个 prompt：

```text
catalog_fingerprint
blueprint_fingerprint
agent_surface_fingerprint
tool_surface_fingerprint
skill_bundle_fingerprint
continuation_fingerprint
```

- Blueprint canonicalization 使用稳定 node 排序、稳定 JSON key 排序和显式 schema version。
- AI 生成的语义相同 Blueprint 必须 canonicalize 为相同 fingerprint。
- tool definitions 在一次 run 内固定并缓存；不能每 step 重建或重排。
- skill 全文不进入 Planner prompt；只在目标 Agent 节点首次使用时 hydration。
- Blueprint fingerprint 只参与调度审计和 cache affinity，不把完整 graph 文本重复塞入每个 leaf prompt。

### 8.3 Graph 的上下文隔离

- 每个 branch 拥有 owner-local history。
- parent -> child 只传 `HandoffPacket { goal, constraints, inputs, artifact_refs }`。
- child -> parent 只返回 `NodeResult { summary, output_ref, usage }`。
- child 的内部 transcript 只计入 workflow cumulative usage，不进入 parent live prefix。
- verifier 读取候选 output/artifact，不继承候选 agent 的完整消息历史。
- Loop 下一轮默认只带上轮摘要、metric 和必要 artifact ref，不累积所有轮次正文。

### 8.4 缓存失效规则

以下变化允许失效 Stable Zone：model/protocol、AgentProfile、tool schema、选中 skill bundle、
system policy 或 cache-control 能力变化。节点进度、stage 名称、iteration number、usage、UI label
变化不得导致 Stable Zone 失效。

每次调用记录 cache diagnostic，至少能区分：

- blueprint changed
- agent surface changed
- tool surface changed
- skill bundle changed
- continuation boundary changed
- dynamic tail only

### 8.5 选择阶段自身的缓存

- Catalog 摘要按 fingerprint 缓存，registry 不变时不重新序列化。
- 相同 task class、policy envelope 和 catalog revision 可复用选择结果，但不得仅凭文本相似度跨
  workspace 复用带副作用的 Blueprint。
- 用户显式选择直接跳过 AI Planner。
- 会话固定 Blueprint 后，后续 continuation 不重复调用 Planner。

### 8.6 Provider 协议级缓存契约

三种保留的 provider 协议共享同一个 canonical conversation authority，但 continuation 方式不同，
不得为了统一接口而牺牲上下文或缓存命中：

- **OpenAI Responses**：首次请求发送完整 canonical seed；后续 continuation 优先使用
  `previous_response_id`，同时在本地保留完整 conversation state 用于恢复、审计和 provider
  capability 变化时重新构造请求。`previous_response_id` 失效必须返回明确错误，不得静默改走
  Chat Completions 或丢失历史重试。
- **OpenAI Chat Completions**：每次请求按相同顺序重放 canonical messages；system/tool schema/
  selected skill 的稳定字节必须位于历史前缀，tool call 与 tool result 必须成对且顺序不变。
- **Anthropic Messages**：每次请求重放 canonical history，并按协议生成稳定 system/cache-control
  边界；tool-use/tool-result 配对和 reasoning continuation 不得被 scheduler node 切换打断。

无论协议如何，tool continuation 都不能只携带“当前首轮输入”或最近一条消息。完整 seed 的
本地所有权属于 AgentLoop；transport 只能选择等价的线性 replay 或 provider continuation token，
不能自行摘要、截断或重排。协议切换、模型切换和 cache-control 能力变化属于显式 cache bust，
必须进入 diagnostic。

### 8.7 缓存数据所有权与构造次数

缓存优化首先消除重复构造，而不是在 provider 外再堆一层不透明缓存：

- `RunContext` 在一次 scheduler run 开始时冻结 blueprint、catalog、policy、agent/tool/skill
  surfaces；节点只持有共享只读引用，不逐 step clone 或重新序列化整份结构。
- canonical tool schema 每个 tool-surface fingerprint 每个 run 最多构造一次；稳定 system/agent/
  skill prefix 每个 agent-surface fingerprint 每个 run 最多构造一次。
- Catalog 摘要按 catalog fingerprint 进程内有界复用；hydrated skill bundle 按内容 fingerprint
  有界复用，但 workspace-local、permission-sensitive 内容必须进入 key，禁止跨权限域命中。
- Blueprint canonical bytes 与 fingerprint 在 validation 时一次产生并随 `ValidatedBlueprint`
  携带；Engine 不得在每个节点重新 canonicalize。
- 完整 canonical conversation 归 `AgentLoop` 所有；provider transport 不维护第二份可分叉的
  消息 authority。Responses continuation token 只是传输状态，不是历史的唯一副本。
- branch、verifier 和 loop 只通过 typed packet/result 过边界。摘要必须是显式节点产物或明确的
  context policy 结果，禁止 transport 或缓存层为省 token 静默截断、改写和重排历史。
- 所有进程内缓存必须有条目数/字节上限、失效策略和可观测命中率；禁止无界 `HashMap`、永久
  保存完整 transcript，或把大 artifact 正文复制进缓存 value。

### 8.8 Cache key 与失效矩阵

| 变化 | Stable Zone | Semi-Stable Zone | Dynamic Zone | Planner |
|---|---|---|---|---|
| provider protocol/model | 失效 | 保留可重投影数据 | continuation 失效 | 通常不重跑 |
| agent/system policy | 失效 | 不变 | 不变 | policy 影响拓扑时重跑 |
| tool schema/permission surface | 失效 | 不变 | permission result 失效 | catalog revision 变化时重跑 |
| selected skill content | 失效对应 agent | 不变 | 不变 | skill catalog 变化时重跑 |
| workspace/session summary | 不失效 | 失效 | 不变 | 仅 task suitability 变化时重跑 |
| node input/tool result | 不失效 | 不失效 | 失效 | 不重跑 |
| telemetry/UI/stage label | 不失效 | 不失效 | 不失效 | 不重跑 |

key 必须包含实际影响请求字节或授权语义的字段，不能包含时间戳、运行计数、UI label 等噪声。
任何新增 key 字段都必须说明它改变了哪段请求或哪项权限；否则不得加入。

## 9. 目标模块结构

新模块已按隔离契约完成实现并提升到 `crates/agendao-orchestrator/src/` 正式根模块。正式结构不
保留 `native`、`v2` 或兼容命名，也不能 import 已删除的旧 scheduler/agent-tree/workflow 模块。

```text
agendao-orchestrator/src/
├── blueprint/
│   ├── types.rs
│   ├── canonical.rs
│   └── validate.rs
├── agent_loop/
│   ├── loop_impl.rs
│   ├── conversation.rs
│   └── provider.rs
├── catalog.rs
├── policy.rs
├── selector.rs
├── engine.rs
├── context.rs
├── events.rs
├── model_request.rs
├── model_resolution.rs
├── output_projection.rs
└── templates.rs
```

约束：

- `templates.rs` 只能构造 Blueprint 数据，不得包含执行循环。
- `engine.rs` 只解释 `ValidatedBlueprint`，不识别 sisyphus/prometheus 等品牌名。
- `agent_loop/` 不 import 旧 `runtime/`；其消息状态、tool continuation、取消和超时从新契约实现。
- selector 使用的 planner backend 只生成 Draft，不执行节点。
- Draft 只经过 canonicalization、catalog resolution 和 validator 成为 `ValidatedBlueprint`；不得在
  两者之间增加旧 schema compiler、profile converter 或隐藏的默认拓扑注入。
- `context.rs` 只构造 typed handoff/result 和 cache surface，不持久化 session。
- blocking workspace 实现放到核心 crate 之外，由 server/session 注入 capability。
- 所有模块默认私有；crate root 只导出宿主真正需要的少量契约。

## 10. 内置能力如何表达

| 当前概念 | 新结构 | 是否保留专用执行器 |
|---|---|---|
| 普通 agent | single Agent node Blueprint | 否 |
| SkillListOrchestrator | Agent node + skill refs | 否 |
| AgentTreeOrchestrator | Parallel nodes + join Agent | 否 |
| SkillGraphOrchestrator | Blueprint nodes/edges + skill refs | 否 |
| SchedulerProfileOrchestrator | SchedulerEngine | 仅此一个 engine |
| Sisyphus | direct/execution Blueprint template | 否 |
| Prometheus | plan/review/handoff Blueprint template | 否 |
| Atlas | parallel + gate + synthesis template | 否 |
| Hephaestus | bounded Loop + execution + gate template | 否 |
| VerifierOrchestrator | Gate evaluator + verify template | 否 |
| Autoresearch WorkflowController | Loop + checkpoint capability + Gate | 否 |

品牌名可以保留为用户友好的模板 ID，但不能出现在执行引擎分支判断中。

## 11. 实施阶段

每个阶段完成后都必须保持新模块自洽。禁止为了阶段性接线增加兼容层。

### Phase 0：冻结新契约和行为场景

- 写定 Blueprint schema、节点语义、PolicyEnvelope 和错误 taxonomy。
- 从旧实现提取场景，不复用旧实现测试 helper：single agent、parallel、gate、bounded loop、
  cancellation、tool error、context checkpoint、cache fingerprint。
- 建立预算与 cache 指标基线。
- 冻结期间不再给旧 scheduler 增加功能。

验收：schema 文档和场景测试清单通过评审；所有控制结构都能映射到五种 NodeSpec。

### Phase 1：独立实现 Blueprint、Catalog、Policy、Validator

- 在 `native/` 中从零实现领域类型和纯 validator。
- 建立 canonical serialization 和稳定 fingerprint golden tests。
- 实现真实 registry 到 Catalog 的直接构建；不得经过旧 SchedulerProfile 类型。
- 所有预算默认有限。

验收：fuzz/property tests 能拒绝无界 loop、非法 graph、权限提升和不稳定 canonicalization。

### Phase 2：独立实现 AgentLoop 和 SchedulerEngine

- 从 provider/session 的稳定公共契约出发，从零实现新 AgentLoop。
- 完整实现 assistant/tool history、reasoning continuation、usage、取消、超时和有限 step policy。
- Engine 解释 ValidatedBlueprint。
- Agent 节点只调用新 AgentLoop。
- 实现 Parallel、Gate、Loop、End；全局预算跨节点累计。
- 实现 cancellation、并发 semaphore、deadline 和结构化事件。
- capability 使用 fake/in-memory 实现完成完整测试。

验收：新 AgentLoop/Engine 的测试不 import 任何旧 runtime、scheduler、tree、graph、workflow 类型。

### Phase 3：上下文与缓存闭环

- 实现 stable/semi-stable/dynamic prompt surface。
- 实现 HandoffPacket、NodeResult、artifact reference 和 branch history 隔离。
- 接入共享 replay authority、reasoning continuation 和 runtime context checkpoint。
- 建立 fingerprint/cache diagnostic golden tests。
- 为 Responses、Chat Completions、Anthropic 分别建立多轮 tool continuation golden tests；验证
  transport 形式可以不同，但 canonical history、stable prefix 和节点输入语义一致。

验收：同 Blueprint 连续多轮时 stable prefix 字节级不变；branch transcript 不泄漏到 parent；
tool-call/tool-result 顺序完整。

### Phase 4：实现内置模板和 AI Planner

- 用纯 Blueprint 构造器重写 direct、plan、coordinate、verify、autoresearch 模板。
- AI Planner 使用 structured output 生成 `UseTemplate` 或 `CreateBlueprint`。
- Planner 输出必须经过同一个 validator；失败时返回明确错误，不回退旧 scheduler。
- 实现用户显式选择优先、auto 选择、会话固定和可检查 effective Blueprint。
- 提供“保存当前 Blueprint”为新 schema JSONC 的显式用户操作。

验收：同一执行引擎运行全部内置模板和 AI 新生成 graph；模板代码不包含执行循环。

### Phase 5：实现真实 capabilities

- 在 orchestrator 核心之外实现 verifier、artifact、checkpoint/workspace capability。
- 所有 blocking I/O 使用受限 blocking executor。
- 增加路径、文件数、总字节、命令超时、磁盘余量和 cleanup 策略。
- 在新 autoresearch Blueprint 上进行破坏性场景测试。

验收：核心 crate 不含文件系统遍历和 Git 进程；失败 checkpoint 不留下失控 worktree/artifact。

实施结果：完成。capability 宿主使用受限 `spawn_blocking`，checkpoint 与 artifact 均有路径、文件数、
总字节、deadline 和磁盘余量门禁；显式 cleanup 与 Drop 都转移并清空 checkpoint registry，失败写入
删除 partial 文件。

### Phase 6：原子切换并删除旧系统

这是一个不可拆成“先接新、后删旧”的生产迁移步骤。允许此前只合入不在生产调用图中的隔离
实现，但最终切换必须在同一个可合并变更集内完成；主分支和任何可发布构建都不得出现已启用
新入口但仍保留旧入口的中间状态。同一变更必须完成：

1. 将 HTTP、Unix Socket、CLI/TUI/Web 的 scheduler 入口切到同一新 authority；Direct authority
   已删除，不得恢复。
2. 将 bundled scheduler 配置和默认选择改为新 Blueprint schema。
3. 更新公开 API、事件、文档和配置 schema。
4. 删除旧 `core.rs`、`prompt_execution.rs`、server orchestration adapter。
5. 删除旧 `runtime/` 及其 loop、bridge、policy、normalizer 和事件类型。
6. 删除旧 agent_tree、skill_graph、skill_list、SchedulerProfileOrchestrator 及 presets 控制流。
7. 删除旧 iterative workflow runtime、verifier 专用 orchestrator 和对应测试。
8. 删除不再使用的依赖、feature、re-export 和文档示例。
9. 确认已提升的正式模块路径不残留 `native`、路径 alias、兼容 re-export 或 `v2` 名称。
10. 删除旧配置读取、环境变量、CLI flag、数据库 metadata 字段和 UI 选项；不得留下“读取但
    忽略”的伪兼容。
11. 删除 `AgentExecutor`/`SessionPrompt` 的 persistent subsession、动态 agent build、`@agent`/
    `Subtask` prompt part、recovery subtask 和 ToolContext callback 链；agent 只能作为 Blueprint
    叶节点被 SchedulerEngine 调用。
12. 用 `ExecutionEvent` 直接建立唯一 typed 运行投影；删除无生产者的 `scheduler_stage_*`
    message metadata、`SchedulerStageBlock`、recovery/stage fixture 和 UI/TUI 解析链。禁止通过旧
    metadata adapter 投影新事件。
13. CI 在切换变更中同时执行新端到端测试与旧符号禁止清单；任一旧执行 authority、旧 schema
    或兼容关键词重新进入生产依赖图即失败。

验收：全仓搜索不存在旧类型、旧字段、旧 schema、deprecated 标记、compat/legacy adapter 或
旧入口；生产调用图只有一个 SchedulerEngine 和一个新 AgentLoop。

实施结果：完成。所有生产入口共用 server/session scheduler authority；旧 runtime、preset/profile、
tree/graph/workflow/verifier、persistent subsession、stage metadata 投影和旧配置/fixture 已在同一工作树
物理删除，未保留 converter、adapter、deprecated re-export 或 fallback。

### Phase 7：收缩与性能验收

- 删除仅为旧结构服务的 metadata、prompt 拼接和测试 fixture。
- 检查 public API、依赖、编译时间、二进制体积和 runtime allocation。
- 对 single-agent、parallel、autoresearch 三类任务做 token/cache/磁盘/延迟对比。
- 更新 `docs/architecture.md`、`docs/scheduler.md`、`docs/context-caching.md` 为新 authority。
- 使用 `cargo machete`、`cargo udeps` 和全仓符号/依赖图搜索审计删除后残留；逐项判断 warning，
  不以全局 `allow(dead_code)`、`allow(unused)` 或保留未使用 feature 掩盖问题。
- 所有 Cargo 命令显式使用 `CARGO_TARGET_DIR=../target`；不得使用 `/tmp` 作为 target 目录。

验收：除下面架构门禁外，workspace test、clippy `-D warnings`、udeps、machete 全部通过；若工具
存在可证明的误报，必须在计划验收记录中逐条说明依赖边和保留理由，不能用宽泛 ignore 清单。

实施结果：完成。最终检查没有 machete/udeps 误报，也没有通过 allow 清单压制结果；具体证据见
第 14.1 节。

## 12. 删除清单

### 12.1 新模块完成前可直接删除

这些项目没有生产消费者，不需要等待原子切换：

- `execute_prompt_simple`
- 只有测试调用的旧 `execute_prompt_streaming_with_session`
- 永远失败的 `OrchestrationCore::execute_tool` 及专用 request/result 类型
- 只在自身测试使用的 `ServerStateOrchestrationAdapter`（当前工作树已删除）
- 无生产构造方的 client Direct authority（当前工作树已删除）

删除仍应作为独立清理提交，不得借此创建替代兼容入口。

### 12.2 必须在原子切换中删除

- `OrchestrationCore` 旧 prompt 执行链
- 旧 `runtime::run_loop` 及其配套 runtime 模块
- `SkillListOrchestrator`
- `AgentTreeOrchestrator`
- `SkillGraphOrchestrator`
- `SchedulerProfileOrchestrator`
- Rust 内置 preset 专用控制流
- `WorkflowController` 及同步 snapshot engine
- Verifier 专用 orchestrator/runtime 分支
- 旧 scheduler JSONC schema 和旧文档示例

## 13. 测试策略

### 13.1 领域测试

- Blueprint canonicalization/fingerprint golden tests
- graph reachability、cycle、loop、parallel、budget property tests
- Catalog resolution 和权限收紧测试
- AI 生成非法 ID、无界 loop、权限提升的拒绝测试

### 13.2 Engine 测试

- single agent、parallel join、gate 三态、loop 收敛/耗尽
- cancellation 在 model/tool/node/branch/checkpoint 边界生效
- 全局 model/tool/token/deadline 预算准确跨节点累计
- branch fail-fast/collect 行为确定
- tool call history 和 usage 只累计一次

### 13.3 Context/cache 测试

- canonical tools 和 skills 顺序稳定
- 动态 node progress 不改变 Stable Zone
- Blueprint 语义变化准确触发 fingerprint 变化
- parent/child/verifier/loop 历史隔离
- reasoning continuation 和 replay ordering 不退化
- cache read/miss/write 与 workflow usage 分账准确
- Responses 的 `previous_response_id` continuation 不重复注入动态 graph 文本
- Chat Completions 与 Anthropic 每轮保留完整 canonical seed，tool continuation 不丢历史
- planner 只接收 catalog 摘要，未选 skill 正文不进入 prompt 或 fingerprint
- 同一 run 内每个 tool-surface fingerprint 只构造一次 canonical tool schema
- 同一 run 内每个 agent-surface fingerprint 只构造一次 Stable Zone
- cache key 不受 iteration、telemetry、UI label 或时间戳影响
- 所有进程内 cache 在条目数或字节上限达到后确定性淘汰，不保留完整 transcript/artifact 正文

### 13.4 端到端场景

- 用户显式 single-agent，Planner 不运行
- auto 选择内置模板
- auto 生成新的 skill composition graph
- verifier gate 拒绝候选并有界退出
- autoresearch 改进、回滚、磁盘不足、命令超时和取消
- 三种 provider 协议下 prompt shape、tool continuation、cache diagnostic 一致

## 14. 量化验收标准

架构完成必须同时满足：

- 生产中恰好一个 SchedulerEngine。
- 生产中恰好一个新 AgentLoop；旧 `runtime::run_loop` 全仓引用为零并被删除。
- `Orchestrator` 多实现模式被删除，而不是重命名保留。
- 没有 `Unbounded` budget。
- 核心 orchestrator 不直接执行文件系统遍历、Git 或 shell。
- 内置模板增加新变体时不修改 Engine。
- AI Blueprint 与用户 Blueprint 使用同一 schema、validator 和 engine。
- 所有旧 scheduler 类型、旧字段和旧 re-export 全仓引用为零。
- 配置入口只接受新 Blueprint schema；旧 scheduler JSONC、profile 或 agent 配置无法通过新
  schema 校验。不存在旧格式 detector、converter、adapter、fallback 或“读取后忽略”。
- 新 orchestrator 非测试 Rust 代码目标不超过 10,000 行；超过时必须按职责证明必要性，
  不能以模板数量为理由。
- 核心公开 API 控制在宿主必需的 Blueprint、Catalog、Policy、Engine、Event 和 capability
  契约内，不通配 re-export 内部模块。
- single-agent 请求不因 scheduler 重建增加额外 model call。
- 固定 Blueprint 的连续请求保持 stable prefix；仅 dynamic tail 变化时不报告高等级 cache bust。
- 同一 run 内 canonical tool surface、Stable Zone 和 Blueprint canonical bytes 对每个对应 fingerprint
  各构造一次；节点数和 agent step 数增加不能使这些稳定结构被重复序列化。
- AI Planner 对用户显式选择和可确定的 single-agent 任务调用次数为零；session 锁定且 catalog/
  policy 仍有效时，continuation 的 Planner 调用次数为零。
- cache telemetry 至少报告 stable-prefix fingerprint、失效原因以及 provider 返回的 read/write/miss
  token；不能用估算命中掩盖协议实际统计。
- Responses continuation、Chat Completions replay、Anthropic replay 都通过完整 seed/tool pairing
  测试；任何协议都不能用丢历史换取更少 token。
- Parallel 和 autoresearch 在预算耗尽、取消或磁盘压力下确定性结束并清理资源。

### 14.1 最终验收证据（2026-08-14）

| 门禁 | 结果 |
|---|---|
| 唯一 authority | `SchedulerEngine`、leaf `AgentLoop`、`ExecutionEvent` 各只有一个定义；旧 authority 精确符号搜索零命中 |
| 代码规模 | `agendao-orchestrator/src` 共 6,734 行：生产代码 5,004 行，独立 `scheduler_tests.rs` 1,730 行，低于 10,000 行门禁 |
| public API | crate root 无 wildcard re-export；11 个公开领域模块均有 workspace 外部消费者（1-20 个文件），其内部子模块保持私有。将这些契约平铺重导出只会扩大 root surface，因此未做无收益的路径改写 |
| 模板与 planner | 五个内置模板经过同一 Validator；显式选择、session lock、simple heuristic 的 planner 调用均为 0；非法 AI Blueprint 明确失败且不 fallback |
| model/tool/token 预算 | single-agent 为 2 model/1 tool/8 tokens；parallel 为 2 model/0 tool/10 tokens；bounded autoresearch 为 2 model/0 tool/10 tokens、2 checkpoint、1 restore，均有精确断言 |
| 上下文缓存 | 同一 run 的 Stable Zone 使用同一个 `Arc<[u8]>` 分配；dynamic progress 只产生 `DynamicTailOnly`；planner catalog 与 hydrated skill 使用共享的条目数+字节数双上限确定性 LRU |
| 资源治理 | single/parallel 不请求磁盘 capability；autoresearch 的真实宿主测试覆盖 checkpoint/artifact 字节上限、磁盘余量、路径穿越、commit/rollback cleanup 和 dropped caller，失败不留残骸 |
| provider | 生产协议仅 OpenAI Responses、OpenAI Chat Completions、Anthropic Messages；其他品牌仅存在于拒绝已删除协议的测试输入 |
| Rust 测试 | `cargo test --workspace` 与全部 doctest 通过；关键包包括 orchestrator 63、provider 229 unit + 34 integration、command 172、server 285、TUI 456 项 |
| Rust 静态门禁 | `cargo check --workspace --all-targets`、clippy `-D warnings`、`cargo machete`、`RUSTC_BOOTSTRAP=1 cargo udeps --workspace --all-targets` 全部通过；udeps 报告所有依赖均已使用 |
| Web 门禁 | 22 个测试文件、138 项测试通过；typecheck、ESLint/Oxlint、live transcript 与 session insights governance 通过 |
| Web canonical transcript | 无 `live_identity` 的 typed live output fail closed；raw `id/tool_call_id` 路由、streaming delta 降级路径、structured-envelope 重复清洗及旧 summary 降级来源已物理删除 |
| 运行时延迟 | 同一已编译 debug test target：single 0.23 s/80,332 KiB，parallel 0.22 s/80,456 KiB，bounded autoresearch 0.21 s/80,464 KiB；时间含 Cargo 启动，测试体均低于计时分辨率 |
| release 构建 | 当前源重建 701.28 s、峰值 3,980,256 KiB；`agendao` 为 74,950,504 bytes（72 MiB）；no-op 重建 1.46 s、112,208 KiB |
| 物理残留 | 旧 authority、旧字段、provider 旧拼写、Web fallback、dead/unused allow 五组精确搜索零命中；`git diff --check` 通过 |
| 构建目录 | 所有 Cargo 门禁均显式使用 `CARGO_TARGET_DIR=../target`，未使用 `/tmp` target |

## 15. 提交与评审纪律

- 每个 Phase 可有多个开发提交，但不得把未完成的新模块接入生产。
- Phase 6 原子切换前必须有完整新系统测试，不能靠线上双跑验证。
- Phase 6 的入口切换、调用方迁移、配置/文档更新和旧模块删除必须作为一个合并单元评审；
  不接受拆成多个可独立部署 PR 的安排。
- 任何提出 compatibility、legacy、fallback、deprecated、temporary adapter 的实现必须被拒绝。
- 任何新增 NodeSpec 都必须证明现有五种原语无法表达真实需求。
- 任何新增内置 scheduler 都必须是 Blueprint 数据或模板构造器，不得增加 Engine 分支。
- 任何影响 Stable Zone 的字段都必须同时提交 cache fingerprint 和诊断测试。
- 删除旧系统是 Phase 6 的完成条件，不允许登记为“后续清理”。

## 16. 原子变更集完成记录

原定八项收口步骤已全部完成：第二套 leaf loop 与 subsession runtime 已删除；外层 subtask callback
契约已删除；`ExecutionEvent` 直接驱动唯一投影；配置、API、CLI/TUI/Web 和文档已切到
`SchedulerBlueprint`/`SchedulerChoice`；旧 profile/tree/workflow schema 和 fixture 已物理删除；
public API、features 与依赖已收缩；第 14 节全部门禁已通过。

本变更集没有兼容尾项或“以后清理”清单。后续新增 scheduler 行为必须继续通过 Blueprint、Catalog、
Policy、Validator 和同一个 SchedulerEngine 扩展，不得恢复已删除的 adapter、旧 schema、双路径或
fallback。
