# Scheduler Guide — Tutorial & User Reference

本文档是 ROCode Scheduler 的完整使用指南，覆盖从基础概念到高级自定义拓扑的全部内容。

## 目录

- [概述](#概述)
- [核心概念：四个正交维度](#核心概念四个正交维度)
- [五个内置 Preset](#五个内置-preset)
- [Session Continuity 与 Hydration](#session-continuity-与-hydration)
- [JSON 配置基础](#json-配置基础)
- [Per-Stage 策略覆盖](#per-stage-策略覆盖)
- [Stage 自由组合](#stage-自由组合)
- [Agent Tree — 执行者组织](#agent-tree--执行者组织)
- [Skill Graph — 图执行策略](#skill-graph--图执行策略)
- [Skill Tree — 知识注入](#skill-tree--知识注入)
- [Stage 内执行优先级链](#stage-内执行优先级链)
- [高级拓扑：PSO 粒子群示例](#高级拓扑pso-粒子群示例)
- [配置参考](#配置参考)
- [选型指南](#选型指南)

---

## 概述

Scheduler 是 ROCode 的任务调度核心。它决定一个用户请求经过哪些阶段（stages）、
每个阶段用什么策略执行、由哪些 agent 协作完成。

Scheduler 不是一个固定的流水线——它是一个**可配置的调度框架**，通过 JSON/JSONC
文件定义行为。用户可以：

1. 直接使用 5 个内置 preset（零配置）
2. 通过 JSON 调整 preset 的 stage 策略
3. 通过 9 种 stage 的任意序列定义全新拓扑
4. 通过 agent tree 或 skill graph 定义 stage 内部的执行结构
5. 通过 skill tree 注入领域知识上下文

---

## 核心概念：四个正交维度

同一个 scheduler profile 可以组合以下四个维度，它们彼此独立：

| 维度 | 解决什么问题 | 配置字段 |
|------|------------|---------|
| **Skill List** | 加载什么能力 | `skillList: ["request-analysis", "plan", ...]` |
| **Agent Tree** | 由谁执行，怎么协作 | `agentTree: { agent: {...}, children: [...] }` |
| **Skill Graph** | 什么顺序、什么条件流转 | `skillGraph: { entryNodeId: "...", nodes: [...], edges: [...] }` |
| **Skill Tree** | 携带什么背景知识 | `skillTree: { contextMarkdown: "...", tokenBudget: 256, truncationStrategy: "head-tail" }` |

这四个维度不是"四选一"，而是可以自由叠加。例如一个 profile 可以同时定义
agent tree（执行者组织）和 skill tree（知识上下文），两者互不干扰。

> Agent Tree 和 Skill Graph 是互斥的执行策略——如果两者都配置了，
> Agent Tree 优先级更高（见 [Stage 内执行优先级链](#stage-内执行优先级链)）。

---

## 五个内置 Preset

每个 preset 不只是一组 stage 列表，它还包含完整的行为契约：执行工作流类型、
gate 策略、effect 协议、finalization 模式等几十个钩子函数。

### Sisyphus — 执行优先

```
request-analysis → route → execution-orchestration
```

| 属性 | 值 |
|------|---|
| 执行工作流 | `SinglePass` — 单次执行，不循环 |
| Stage 拓扑 | **可配置** — JSON `stages` 数组生效 |
| 路由模式 | Passthrough — 不强制约束 |
| 最大轮次 | 1 |
| 适用场景 | 明确的执行任务、bug 修复、功能实现 |

Sisyphus 的哲学：**分类一次，执行到底**。不做多轮规划，不做反复审查，
把精力集中在一次高质量的执行上。

### Prometheus — 规划优先

```
request-analysis → route → interview → plan → review → handoff
```

| 属性 | 值 |
|------|---|
| 执行工作流 | `Direct` — 直接执行（规划产出，非代码执行） |
| Stage 拓扑 | **锁死** — JSON `stages` 数组被忽略 |
| 路由模式 | Orchestrate（强制） — 不允许转为直接回复 |
| 最大轮次 | 1 |
| 适用场景 | 需求澄清、架构规划、实现方案设计 |

Prometheus 的哲学：**先问清楚，再规划，审查后交付方案**。它不执行代码，
只产出经过审查的实现计划。

> **重要**：Prometheus 是唯一一个 stage 拓扑锁死的 preset。你可以通过
> per-stage override 调整每个 stage 的策略（如 `loopBudget`、`toolPolicy`），
> 但不能增删或重排 stage。

### Atlas — 协调优先

```
request-analysis → execution-orchestration → synthesis
```

| 属性 | 值 |
|------|---|
| 执行工作流 | `CoordinationLoop` — 协调循环，带验证 |
| Stage 拓扑 | **可配置** — JSON `stages` 数组生效 |
| 路由模式 | Passthrough |
| 最大轮次 | 3 |
| 验证模式 | Required — 必须通过协调验证 gate |
| 适用场景 | 多工作流协调、并行任务分解与汇总 |

Atlas 的哲学：**分解、并行执行、验证、汇总**。它把任务拆成多个工作流，
协调并行推进，通过 gate 验证确保结果收敛后再合成最终输出。

### Hephaestus — 自治优先

```
request-analysis → execution-orchestration
```

| 属性 | 值 |
|------|---|
| 执行工作流 | `AutonomousLoop` — 自治循环，带验证 |
| Stage 拓扑 | **可配置** — JSON `stages` 数组生效 |
| 路由模式 | Passthrough |
| 最大轮次 | 3 |
| 验证模式 | Required — 必须通过自治验证 gate |
| 适用场景 | 深度自治执行、复杂代码变更、需要自我验证的任务 |

Hephaestus 的哲学：**深入执行，自我验证，失败时逐级升级**。它是最"放手"
的 preset，给 agent 最大的自治空间，但要求执行结果必须通过自我验证。

### Verifier — 候选比较优先

```
request-analysis → execution-orchestration
```

| 属性 | 值 |
|------|---|
| 执行工作流 | `AutonomousLoop` + `verify` workflow mode |
| Stage 拓扑 | **可配置** — JSON `stages` 数组生效 |
| 路由模式 | Passthrough |
| 最大轮次 | 3 |
| 候选选择 | LLM judge + metric fallback |
| 适用场景 | 需要保留多个候选并显式比较选优的实现任务 |

Verifier 的哲学：**不要把最后一次迭代误当成最好的一次**。它复用 Hephaestus
的执行内核，但在 workflow 层显式记录候选，再通过重复的 pairwise judging、
criterion 权重和指标回退选出最终结果。

Canonical verifier 配置使用 `useLogprobs=true`：每个候选对、criterion、
repetition 形成一个 score job，用 A-T score-token top-logprobs 计算 expected
reward，并通过 `score-job-matrix`、`pairwise-score-matrix`、
`round-robin-win-counts` 和 `selection-report` artifacts 暴露证据。provider
未返回可用 logprobs 时会显式记录 `logprob_status` 并回退到文本 score tag。

### Preset 对比总览

| | Sisyphus | Prometheus | Atlas | Hephaestus | Verifier |
|---|---------|-----------|-------|-----------|----------|
| Stage 数量 | 3 | 6 | 3 | 2 | 2 |
| Stage 可配置 | ✅ | ❌ 锁死 | ✅ | ✅ | ✅ |
| 执行工作流 | SinglePass | Direct | CoordinationLoop | AutonomousLoop | AutonomousLoop + verify mode |
| 最大轮次 | 1 | 1 | 3 | 3 | 3 |
| 验证 gate | 无 | 无 | 协调验证 | 自治验证 | 候选比较验证 |
| 子 agent 模式 | Sequential | Parallel | Parallel | Sequential | Sequential |
| 典型用途 | 执行 | 规划 | 协调 | 深度自治 | 多候选选优 |

---

## Session Continuity 与 Hydration

Scheduler stage 的输入由 stage composer 构建，不是把原始 `messages[]` 直通给模型。为了让 follow-up prompt 能可靠引用同一 session 的上一轮工作，ROCode 会构建 `SchedulerSessionContextPacket`：

- markdown projection：`Session Continuity Context`，包含 coverage、anchors、guidance、ledger、compaction summary 和 exact recent tail
- structured contract：`scheduler_session_context_packet`，包含 `version`、message anchors、memory anchors、limits 和 recall policy

当前 recall 面分成两类：

| Anchor | Tool | 说明 |
|--------|------|------|
| Source Anchors | `scheduler_context_hydrate({"message_ids":[...]})` | 只允许回查 packet 授权的同会话 message id / compaction summary |
| Memory Anchors | `scheduler_memory_hydrate({"record_ids":[...]})` | 只允许回查 auto-memory prefetch 授权的 memory record id，可选 `include_evidence` |

两个 hydration 工具都是只读工具，并且对只读 stage 可用。运行时会拒绝未出现在 anchors 中的 id；调用结果会沉淀到 stage metadata，记录 hydrated / rejected / missing ids，便于审计。

这套机制是所有公开 scheduler preset 的共享上下文契约。

---

## JSON 配置基础

Scheduler 通过 JSON/JSONC 文件配置。JSONC 支持注释和尾逗号。

### 最小配置

```jsonc
{
  "defaults": { "profile": "my-profile" },
  "profiles": {
    "my-profile": {
      "orchestrator": "sisyphus"
    }
  }
}
```

这会创建一个使用 Sisyphus 默认 stage 和默认策略的 profile。

### 完整配置结构

```jsonc
{
  "$schema": "https://rocode.dev/schemas/scheduler-profile.schema.json",
  "version": "2026-05-05",
  "defaults": {
    "profile": "my-default"       // 默认激活的 profile 名
  },
  "profiles": {
    "my-default": {
      "orchestrator": "sisyphus",   // 基于哪个 preset
      "description": "...",         // 人类可读描述
      "model": {                    // 可选：覆盖 scheduler 使用的模型
        "providerId": "zhipuai",
        "modelId": "glm-5.1"
      },
      "skillList": ["request-analysis", "execution-orchestration"],
      "stages": [...],              // stage 序列（见下文）
      "agentTree": {...},           // 执行者组织（见 Agent Tree 章节）
      "skillGraph": {...},          // 图执行策略（见 Skill Graph 章节）
      "skillTree": {                // 知识注入
        "contextMarkdown": "...",
        "tokenBudget": 256,
        "truncationStrategy": "head-tail"
      }
    },
    "another-profile": { ... }      // 同一文件可包含多个 profile
  }
}
```

### 多 Profile

一个配置文件可以包含多个命名 profile，通过 `defaults.profile` 指定默认激活哪个。
这允许在同一个文件中维护不同场景的配置：

```jsonc
{
  "defaults": { "profile": "fast" },
  "profiles": {
    "fast": { "orchestrator": "sisyphus", ... },
    "thorough": { "orchestrator": "atlas", ... },
    "plan-only": { "orchestrator": "prometheus", ... }
  }
}
```

### 引用方式

在 `rocode.json` / `rocode.jsonc` 中通过 `schedulerPath` 引用：

```jsonc
{
  "schedulerPath": "./scheduler/my-config.jsonc"
}
```

---

## Per-Stage 策略覆盖

`stages` 数组中的每个条目可以是简单字符串或带覆盖的对象，两种形式可以混用：

```jsonc
"stages": [
  "request-analysis",                    // 简单字符串：使用 preset 默认策略
  {                                      // 对象：自定义该 stage 的策略
    "kind": "execution-orchestration",
    "toolPolicy": "allow-all",
    "loopBudget": "step-limit:10",
    "childSession": true
  },
  "synthesis"                            // 简单字符串
]
```

### 覆盖字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `kind` | `string` | **必填**。stage 类型（如 `"plan"`、`"execution-orchestration"`） |
| `toolPolicy` | `"allow-all"` \| `"allow-read-only"` \| `"disable-all"` | 工具访问策略 |
| `loopBudget` | `"unbounded"` \| `"step-limit:N"` | 最大 LLM 循环迭代次数 |
| `sessionProjection` | `"hidden"` \| `"transcript"` | stage 输出是否可见 |
| `childSession` | `boolean` | 是否创建隔离的子会话 |
| `agentTree` | `AgentTreeNode` \| `string` | per-stage agent tree（覆盖 profile 级别） |
| `agents` | `string[]` | agent 名称过滤器 |
| `skillList` | `string[]` | 该 stage 可用的 skill 列表 |

### 三层覆盖链

```
per-stage JSON 覆盖  →  preset 函数默认  →  Sisyphus 硬编码默认
```

省略的字段会沿着这条链向下查找默认值。例如，如果你只覆盖了 `toolPolicy`，
其他字段（`loopBudget`、`sessionProjection` 等）仍然使用 preset 的默认值。

### toolPolicy 详解

| 值 | 含义 | 可用工具 |
|----|------|---------|
| `allow-all` | 所有工具可用 | read, write, glob, grep, bash, ... |
| `allow-read-only` | 只读工具 | read, glob, grep, ls, ast_grep_search |
| `disable-all` | 禁用所有工具 | 无 |

### loopBudget 详解

| 值 | 含义 |
|----|------|
| `unbounded` | 无步数限制（谨慎使用） |
| `step-limit:N` | 最多 N 步 LLM 迭代（如 `step-limit:10`） |

### sessionProjection 详解

| 值 | 含义 |
|----|------|
| `hidden` | stage 输出不写入 transcript，后续 stage 看不到 |
| `transcript` | stage 输出写入 transcript，后续 stage 可以看到 |

`transcript` 对于迭代式拓扑（如 PSO）至关重要——它是跨迭代上下文传递的机制。

---

## Stage 自由组合

### 9 种 Stage

Scheduler 提供 9 种 stage 类型，每种有明确的语义职责：

| Stage | JSON 名 | 职责 |
|-------|---------|------|
| **RequestAnalysis** | `request-analysis` | 解析用户请求意图，生成 request brief |
| **Route** | `route` | 意图分类，决定执行路径（可触发 preset 切换） |
| **Interview** | `interview` | 向用户提问以澄清需求（阻塞式） |
| **Plan** | `plan` | 生成实现计划 |
| **Delegation** | `delegation` | 将任务委派给 agent tree 或 skill graph |
| **Review** | `review` | 审查执行结果 |
| **ExecutionOrchestration** | `execution-orchestration` | 核心执行阶段，驱动 agent 完成任务 |
| **Synthesis** | `synthesis` | 汇总和格式化最终输出 |
| **Handoff** | `handoff` | 交付结果（通常用于规划类 preset） |

### 自由序列，不只是组合

`stages` 是一个 `Vec`，不是 `Set`。这意味着：

- **stage 可以重复出现**——同一种 stage 可以在序列中出现多次
- **顺序自由**——stage 按数组顺序依次执行
- **长度不限**——可以是 2 个 stage，也可以是 11 个

例如 PSO 拓扑的 stage 序列：

```jsonc
"stages": [
  "request-analysis",
  { "kind": "execution-orchestration", ... },  // 迭代 1
  "synthesis",
  { "kind": "execution-orchestration", ... },  // 迭代 2
  "synthesis",
  { "kind": "execution-orchestration", ... },  // 迭代 3
  "synthesis"
]
```

这里 `execution-orchestration` 出现了 3 次，`synthesis` 出现了 3 次。

### 同种 stage 重复时的 override 行为

`stage_overrides` 内部是 `HashMap<SchedulerStageKind, SchedulerStageOverride>`，
所以同种 stage 的多次出现**共享同一份 override 配置**。

这意味着：
- ✅ 每次迭代使用相同的 agent tree 和策略（对 PSO 等迭代模式正好合适）
- ❌ 无法给第 1 次和第 3 次的 `execution-orchestration` 设置不同的 `loopBudget`

如果需要区分不同迭代的配置，目前需要使用不同的 stage kind（如一次用
`execution-orchestration`，一次用 `delegation`）。

### 常见拓扑模式

**执行型**（Sisyphus 风格）：
```jsonc
["request-analysis", "route", "execution-orchestration"]
```

**规划型**（Prometheus 风格）：
```jsonc
["request-analysis", "route", "interview", "plan", "review", "handoff"]
```

**协调型**（Atlas 风格）：
```jsonc
["request-analysis", "execution-orchestration", "synthesis"]
```

**迭代收敛型**（PSO 风格）：
```jsonc
["request-analysis", "execution-orchestration", "synthesis",
 "execution-orchestration", "synthesis", "execution-orchestration", "synthesis"]
```

**规划 + 执行 + 审查**（混合型）：
```jsonc
["request-analysis", "interview", "plan", "execution-orchestration", "review", "synthesis"]
```

---

## Agent Tree — 执行者组织

Agent Tree 定义了 stage 内部的执行者层级结构：谁是主 agent，谁是子 agent，
子 agent 之间是并行还是串行。

### 基本结构

```jsonc
{
  "agent": {                          // 根节点 agent（必填）
    "name": "deep-worker",
    "systemPrompt": "...",            // 可选：自定义系统提示
    "model": {                        // 可选：per-agent 模型覆盖
      "providerId": "ethnopic",
      "modelId": "your-model-id"
    },
    "maxSteps": 10,                   // 可选：最大步数
    "temperature": 0.7,               // 可选：温度
    "allowedTools": ["read", "glob"]  // 可选：工具白名单
  },
  "prompt": "...",                    // 可选：角色提示
  "children": [                       // 可选：子 agent 列表
    {
      "agent": { "name": "code-explorer" },
      "prompt": "Explore the codebase.",
      "children": [...]               // 递归嵌套
    }
  ]
}
```

### 执行流程

Agent Tree 的执行遵循固定的三步流程（`agent_tree.rs`）：

```
1. Root agent 执行 → 产出初始 draft
2. Children 并行执行 → 每个 child 看到 root 的 draft + 原始任务 + 自己的 role prompt
3. Root agent 再次执行 → 聚合所有 child 的输出，产出最终结果
```

如果没有 children，Root agent 直接返回结果（退化为单 agent 执行）。

Children 默认**并行执行**（`ChildExecutionMode::Parallel`），也支持串行模式。

### 两个层级的 Agent Tree

Agent Tree 可以在两个层级配置：

**Profile 级别**——作为所有 stage 的默认 agent tree：

```jsonc
{
  "profiles": {
    "my-profile": {
      "agentTree": { "agent": { "name": "deep-worker" } }
    }
  }
}
```

**Per-stage 级别**——覆盖特定 stage 的 agent tree：

```jsonc
{
  "stages": [
    {
      "kind": "execution-orchestration",
      "agentTree": {
        "agent": { "name": "coordinator" },
        "children": [
          { "agent": { "name": "worker-a" }, "prompt": "Do A." },
          { "agent": { "name": "worker-b" }, "prompt": "Do B." }
        ]
      }
    }
  ]
}
```

Per-stage agent tree 优先级高于 profile 级别。

### 外部文件引用

Agent tree 可以是内联对象，也可以是指向外部 JSON/JSONC 文件的路径：

```jsonc
// 内联
"agentTree": { "agent": { "name": "deep-worker" }, "children": [...] }

// 文件路径（相对于配置文件所在目录）
"agentTree": "./trees/my-team.json"
```

外部文件的好处：
- **复用**：多个 profile 或 stage 引用同一个 tree 文件
- **可读性**：复杂的 tree 不会让 scheduler 配置变得臃肿
- **独立管理**：agent 团队组成和 stage 编排策略分离

外部文件在 `SchedulerConfig::load_from_file()` 时自动解析，支持 JSONC 格式。

### Agent Tree 设计原则

1. **Children 之间应该有认知差异**——如果所有 child 做同样的事，不如用单 agent
2. **Root agent 的 systemPrompt 决定聚合质量**——它需要知道如何综合不同视角
3. **Tree 深度不宜过深**——每层都有 root 执行 + children 执行 + 聚合的开销
4. **allowedTools 可以限制 child 的能力范围**——比如只给 read-only 工具

---

## Skill Graph — 图执行策略

Skill Graph 是 Agent Tree 之外的另一种 stage 内执行策略。它用有向图模型
定义 agent 之间的流转关系，支持条件分支。

### 基本结构

```jsonc
{
  "skillGraph": {
    "entryNodeId": "analyze",         // 入口节点 ID
    "maxHops": 20,                    // 最大跳转次数（防止无限循环）
    "nodes": [
      {
        "id": "analyze",
        "agent": { "name": "analyzer" },
        "prompt": "Analyze the problem."
      },
      {
        "id": "implement",
        "agent": { "name": "implementer" },
        "prompt": "Implement the solution."
      },
      {
        "id": "review",
        "agent": { "name": "reviewer" },
        "prompt": "Review the implementation."
      }
    ],
    "edges": [
      { "from": "analyze", "to": "implement", "condition": "always" },
      { "from": "implement", "to": "review", "condition": "always" },
      {
        "from": "review",
        "to": "implement",
        "condition": { "outputContains": "NEEDS_REVISION" }
      }
    ]
  }
}
```

### 执行流程

1. 从 `entryNodeId` 指定的节点开始执行
2. 节点的 agent 执行任务，产出输出
3. 评估该节点的所有出边条件
4. 跳转到第一个条件满足的目标节点
5. 重复直到没有匹配的出边，或达到 `maxHops` 上限

### 边条件类型

| 条件 | 含义 |
|------|------|
| `"always"` | 无条件跳转 |
| `{ "outputContains": "KEYWORD" }` | 节点输出包含指定关键词时跳转 |
| `{ "outputNotContains": "KEYWORD" }` | 节点输出不包含指定关键词时跳转 |

### Agent Tree vs Skill Graph

| | Agent Tree | Skill Graph |
|---|-----------|------------|
| 拓扑 | 树形（parent → children → aggregation） | 有向图（任意节点间跳转） |
| 并行 | Children 天然并行 | 节点串行执行 |
| 循环 | 不支持 | 通过边条件支持 |
| 聚合 | Root 自动聚合 children 输出 | 无内置聚合，靠节点 prompt 传递 |
| 适用场景 | 多视角并行探索 | 条件分支流程、审查-修改循环 |

两者互斥——如果同时配置了 agent tree 和 skill graph，agent tree 优先。

---

## Skill Tree — 知识注入

Skill Tree 不是执行策略，而是**上下文注入机制**。它给 scheduler 的所有 stage
携带背景知识。

```jsonc
{
  "skillTree": {
    "contextMarkdown": "This project uses a hexagonal architecture. All domain logic lives in the core module. Adapters are in the adapters/ directory. Never import from adapters into core.",
    "tokenBudget": 256,
    "truncationStrategy": "head-tail"
  }
}
```

`contextMarkdown` 的内容会被注入到 scheduler 的系统提示中，影响所有 stage
的 agent 行为。

`tokenBudget` 用近似 token 预算保护 skill tree 上下文，超预算时会按
`truncationStrategy` 执行截断，并插入明确的截断标记。支持：

- `head`
- `tail`
- `head-tail`（默认）

典型用途：
- 项目架构约束（"所有 API 必须经过 middleware 层"）
- 编码规范（"使用 immutable 模式，不要 mutation"）
- 领域知识（"这是一个支付系统，所有金额用 decimal 不用 float"）
- 调度策略提示（"优先使用并行探索，不要串行"）

当 profile plan 中启用了 `skillTree`，scheduler 的 `Current Plan` 调试快照会显式展示：

- `skill-tree-estimated-tokens`
- `skill-tree-budget`
- `skill-tree-truncation`
- `skill-tree-truncated`

---

## Stage 内执行优先级链

当一个 stage（如 `execution-orchestration`）需要执行任务时，scheduler 按以下
优先级选择执行策略（`execution_adapter.rs:120`）：

```
1. Per-stage agent tree    ← 最高优先级
2. Profile-level agent tree
3. Skill graph
4. Execution fallback      ← 最低优先级（直接工具执行）
```

```rust
let stage_tree = stage.and_then(|s| self.plan.stage_agent_tree(s));
if let Some(agent_tree) = stage_tree.or(self.plan.agent_tree.as_ref()) {
    self.execute_agent_tree(agent_tree, ...)
} else if let Some(skill_graph) = &self.plan.skill_graph {
    self.execute_skill_graph(skill_graph, ...)
} else if allow_execution_fallback {
    self.execute_execution_fallback_stage(...)
}
```

这意味着：
- 如果某个 stage 有自己的 agent tree，用它
- 否则用 profile 级别的 agent tree
- 如果都没有 agent tree，用 skill graph
- 如果连 skill graph 都没有，用 fallback（直接执行）

---

## 高级拓扑：PSO 粒子群示例

PSO（Particle Swarm Optimization）是一个用户自定义拓扑的完整示例，展示了
如何用现有的 stage 组合 + agent tree 实现迭代式多 agent 收敛。

### 核心思路

3 个认知粒子（Architect、Empiricist、Adversary）从正交视角独立探索任务，
通过多轮 synthesis 迭代收敛到全局最优解。

```
request-analysis
       │
       ▼
┌────────────── 迭代 1 ──────────────┐
│  execution-orchestration           │
│    coordinator                     │
│      ├── Alpha  (结构性思维)        │
│      ├── Beta   (证据驱动)          │
│      └── Gamma  (对抗性思维)        │
│  synthesis → 全局最优汇总           │
└────────────────────────────────────┘
       │  transcript 累积
       ▼
┌────────────── 迭代 2 ──────────────┐
│  粒子看到上轮 synthesis            │
│  反思 → 从各自角度深入探索          │
│  synthesis → 更新全局最优           │
└────────────────────────────────────┘
       │
       ▼
┌────────────── 迭代 3 ──────────────┐
│  最终收敛                          │
│  synthesis → 收敛结果              │
└────────────────────────────────────┘
```

### 为什么迭代有效

每轮迭代中，粒子看到的 context 包含：
- **原始任务**——不变的目标
- **自己上一轮的输出**——个体最优（PSO 的 pbest）
- **上一轮的 synthesis 结果**——全局最优（PSO 的 gbest）
- **自己的 systemPrompt**——认知惯性（PSO 的 w·v(t)）

这自然实现了 PSO 的速度更新公式，无需显式的状态管理。

### 适用场景

| 场景 | 为什么适合 PSO |
|------|--------------|
| 大型架构决策 | 多个正交评价维度（结构、证据、风险），单次探索必然遗漏 |
| 复杂 bug 根因分析 | 线索碎片化，需要交叉验证才能收敛 |
| 安全审计 | 架构审查 + 代码审查 + 攻击向量构造需要迭代深化 |
| 技术选型 / 迁移评估 | 多维度权衡，第一轮分析往往不够深入 |

### 不适用场景

| 场景 | 用什么替代 |
|------|----------|
| 明确的 bug 修复 | Hephaestus |
| 新增 CRUD endpoint | Sisyphus |
| 文档更新 | Sisyphus |
| 小范围重构（< 3 文件） | Hephaestus |

### 快速判断

> 如果你拿到任务会先想 10 分钟再动手，并且中途可能推翻最初的想法——用 PSO。
> 如果你拿到任务就知道该怎么做——用 Sisyphus 或 Hephaestus。

详细文档见 [`pso/README.md`](pso/README.md)。

Verifier 的详细算法、配置和 artifact 解读见 [`verifier/README.md`](verifier/README.md)。

---

## 配置参考

### 文件清单

```
docs/examples/scheduler/
├── SCHEDULER_GUIDE.md                 ← 本文档
├── README.md                          ← 示例文件索引
├── scheduler-profile.schema.json      ← JSON Schema
├── presets/
│   ├── sisyphus.example.jsonc         ← Sisyphus 示例（default + custom）
│   ├── prometheus.example.jsonc       ← Prometheus 示例（default + custom）
│   ├── atlas.example.jsonc            ← Atlas 示例（default + custom）
│   └── hephaestus.example.jsonc       ← Hephaestus 示例（default + custom）
├── verifier/
│   ├── README.md                      ← Verifier 算法与示例详细指南
│   ├── minimal.example.jsonc          ← Verifier 最小配置
│   ├── profile.example.jsonc          ← Verifier 完整 profile 示例
│   └── workflow.example.jsonc         ← 外置 workflow 示例
├── pso/
│   ├── README.md                      ← PSO 拓扑详细文档
│   ├── example.jsonc                  ← PSO 示例（3iter + 5iter）
│   └── trees/
│       └── pso-swarm.json             ← PSO 3 粒子 agent tree
├── autoresearch/
│   ├── README.md                      ← workflow 级 autoresearch 示例说明
│   └── book-authoring.example.jsonc   ← 长文写作 workflow 示例
└── trees/
    ├── coordinator-tree.json          ← 可复用 agent tree：coordinator
    ├── deep-worker-tree.jsonc         ← 可复用 agent tree：deep-worker
```

### AgentDescriptor 字段

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `name` | `string` | ✅ | agent 标识名 |
| `systemPrompt` | `string` | | 自定义系统提示 |
| `model` | `ModelRef` | | per-agent 模型覆盖（优先级最高） |
| `maxSteps` | `integer` | | 最大执行步数 |
| `temperature` | `number` | | 温度参数 |
| `allowedTools` | `string[]` | | 工具白名单 |

### ModelRef 字段

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `providerId` | `string` | ✅ | 提供商（如 `"zhipuai"`、`"alibaba-cn"`、`"kimi-for-coding"`） |
| `modelId` | `string` | ✅ | 模型 ID（如 `"glm-5.1"`、`"qwen3.6-plus"`、`"kimi-k2.5"`) |

### 模型优先级

```
per-agent model (agentTree agent.model)
  → profile-level model (profile.model)
    → session 当前模型（fallback）
```

---

## 选型指南

### 按任务类型选择

| 任务 | 推荐 Preset | 理由 |
|------|------------|------|
| 修 bug（已知根因） | Hephaestus | 深度自治，自我验证 |
| 修 bug（未知根因） | Atlas 或 PSO | 需要多视角探索 |
| 候选都能跑通，但需要更稳的最终选优 | Verifier | 保留多个候选并做重复加权比较 |
| 实现新功能（需求清晰） | Sisyphus | 一次执行到位 |
| 实现新功能（需求模糊） | Prometheus → Sisyphus | 先规划再执行 |
| 大型重构 | Atlas | 多工作流协调 |
| 架构决策 | PSO | 多维度迭代收敛 |
| 安全审计 | PSO 或 Atlas + review | 需要对抗性视角 |
| 代码审查 | Atlas + review stage | 协调 + 审查 |
| 文档编写 | Sisyphus | 简单直接 |

### 按复杂度选择

```
简单任务（< 3 文件，需求明确）
  → Sisyphus 或 Hephaestus

中等任务（3-10 文件，有一定设计决策）
  → Atlas

复杂任务（> 10 文件，多维度权衡）
  → PSO 或 Prometheus + Atlas

规划类任务（不执行代码，只出方案）
  → Prometheus
```

### 成本意识

| 拓扑 | 相对 token 消耗 | 说明 |
|------|----------------|------|
| Hephaestus | 1x | 最精简，2 个 stage |
| Sisyphus | 1.5x | 3 个 stage，含路由 |
| Atlas | 2-3x | 协调循环，最多 3 轮 |
| Prometheus | 2x | 6 个 stage，但不执行代码 |
| PSO-3iter | 6-8x | 3 轮 × 3 粒子 + synthesis |
| PSO-5iter | 10-12x | 5 轮 × 3 粒子 + synthesis |

PSO 的 token 消耗显著高于其他拓扑。只在高价值决策场景使用。
