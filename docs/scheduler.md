# Scheduler 调度器完整参考

Scheduler 是 ROCode 的任务调度核心。它决定一个用户请求经过哪些阶段（stages）、每个阶段用什么策略执行、由哪些 agent 协作完成。

Scheduler 不是固定的流水线，而是一个可配置的调度框架，通过 JSON/JSONC 文件定义行为。用户可以：

1. 直接使用 5 个内置 preset（零配置）
2. 通过 JSON 调整 preset 的 stage 策略
3. 通过 9 种 stage 的任意序列定义全新拓扑
4. 通过 agent tree 或 skill graph 定义 stage 内部的执行结构
5. 通过 skill tree 注入领域知识上下文

---

## 目录

- [核心概念：四个正交维度](#核心概念四个正交维度)
- [五个内置 Preset](#五个内置-preset)
- [Session Continuity 与 Hydration](#session-continuity-与-hydration)
- [9 种 Stage 类型](#9-种-stage-类型)
- [JSON 配置基础](#json-配置基础)
- [Per-Stage 策略覆盖](#per-stage-策略覆盖)
- [Agent Tree -- 执行者组织](#agent-tree----执行者组织)
- [Skill Graph -- 图执行策略](#skill-graph----图执行策略)
- [Skill Tree -- 知识注入](#skill-tree----知识注入)
- [动态并行执行（parallel_plan）](#动态并行执行parallel_plan)
- [Stage 内执行优先级链](#stage-内执行优先级链)
- [协调循环与验证 Gate](#协调循环与验证-gate)
- [Stage 能力可观测性](#stage-能力可观测性)
- [Profile JSON Schema 参考](#profile-json-schema-参考)
- [选型指南](#选型指南)

---

## 核心概念：四个正交维度

同一个 scheduler profile 可以组合以下四个维度，它们彼此独立：

| 维度 | 解决什么问题 | 配置字段 |
|------|------------|---------|
| **Skill List** | 加载什么能力 | `skillList: ["request-analysis", "plan", ...]` |
| **Agent Tree** | 由谁执行，怎么协作 | `agentTree: { agent: {...}, children: [...] }` |
| **Skill Graph** | 什么顺序、什么条件流转 | `skillGraph: { entryNodeId: "...", nodes: [...], edges: [...] }` |
| **Skill Tree** | 携带什么背景知识 | `skillTree: { contextMarkdown: "...", tokenBudget: 256, truncationStrategy: "head-tail" }` |

这四个维度不是"四选一"，而是可以自由叠加。例如一个 profile 可以同时定义 agent tree（执行者组织）和 skill tree（知识上下文），两者互不干扰。

> Agent Tree 和 Skill Graph 是互斥的执行策略——如果两者都配置了，Agent Tree 优先级更高（见 [Stage 内执行优先级链](#stage-内执行优先级链)）。

---

## 五个内置 Preset

每个 preset 不只是一组 stage 列表，它还包含完整的行为契约：执行工作流类型、gate 策略、effect 协议、finalization 模式等。

### Sisyphus -- 执行优先

```
request-analysis -> route -> execution-orchestration
```

| 属性 | 值 |
|------|---|
| 执行工作流 | `SinglePass` -- 单次执行，不循环 |
| Stage 拓扑 | **可配置** -- JSON `stages` 数组生效 |
| 路由模式 | Passthrough -- 不强制约束 |
| 最大轮次 | 1 |
| 子 agent 模式 | Sequential |
| 适用场景 | 明确的执行任务、bug 修复、功能实现 |

Sisyphus 的哲学：**分类一次，执行到底**。不做多轮规划，不做反复审查，把精力集中在一次高质量的执行上。

默认 profile 示例：

```jsonc
{
  "orchestrator": "sisyphus",
  "skillList": ["request-analysis", "route", "execution-orchestration"],
  "stages": ["request-analysis", "route", "execution-orchestration"],
  "skillTree": {
    "contextMarkdown": "Route once, preserve request intent, and let the shared execution kernel own the loop until completion."
  },
  "agentTree": {
    "agent": { "name": "deep-worker" },
    "children": [
      { "agent": { "name": "code-explorer" }, "prompt": "Map implementation hotspots and collect repository evidence." },
      { "agent": { "name": "docs-researcher" }, "prompt": "Bring in external documentation when it changes risk or constraints." }
    ]
  }
}
```

### Prometheus -- 规划优先

```
request-analysis -> route -> interview -> plan -> review -> handoff
```

| 属性 | 值 |
|------|---|
| 执行工作流 | `Direct` -- 直接执行（规划产出，非代码执行） |
| Stage 拓扑 | **锁死** -- JSON `stages` 数组被忽略 |
| 路由模式 | Orchestrate（强制） -- 不允许转为直接回复 |
| 最大轮次 | 1 |
| 子 agent 模式 | Parallel |
| 适用场景 | 需求澄清、架构规划、实现方案设计 |

Prometheus 的哲学：**先问清楚，再规划，审查后交付方案**。它不执行代码，只产出经过审查的实现计划。

> **重要**：Prometheus 是唯一一个 stage 拓扑锁死的 preset。你可以通过 per-stage override 调整每个 stage 的策略（如 `loopBudget`、`toolPolicy`），但不能增删或重排 stage。

自定义 Prometheus 示例（per-stage override）：

```jsonc
{
  "orchestrator": "prometheus",
  "stages": [
    "request-analysis",
    "route",
    { "kind": "interview", "loopBudget": "step-limit:3" },
    {
      "kind": "plan",
      "toolPolicy": "allow-all",
      "agentTree": {
        "agent": { "name": "senior-architect", "maxSteps": 10 },
        "children": [
          { "agent": { "name": "code-explorer" }, "prompt": "Map the codebase before the plan is frozen." }
        ]
      }
    },
    {
      "kind": "review",
      "agentTree": {
        "agent": { "name": "security-auditor" },
        "children": [
          { "agent": { "name": "dependency-scanner" }, "prompt": "Scan for known CVEs." }
        ]
      }
    },
    "handoff"
  ]
}
```

### Atlas -- 协调优先

```
request-analysis -> execution-orchestration -> synthesis
```

| 属性 | 值 |
|------|---|
| 执行工作流 | `CoordinationLoop` -- 协调循环，带验证 |
| Stage 拓扑 | **可配置** -- JSON `stages` 数组生效 |
| 路由模式 | Passthrough |
| 最大轮次 | 3 |
| 验证模式 | Required -- 必须通过协调验证 gate |
| 子 agent 模式 | Parallel |
| 适用场景 | 多工作流协调、并行任务分解与汇总 |

Atlas 的哲学：**分解、并行执行、验证、汇总**。它把任务拆成多个工作流，协调并行推进，通过 gate 验证确保结果收敛后再合成最终输出。

Atlas 的协调章程要求：

- 你是 conductor（指挥），不是 musician（乐手）。只协调，不亲自写代码
- 每次委派只给一个有界任务
- 独立任务并行执行
- 所有委派必须经过 4 阶段 QA 验证（读取实际输出 -> 自动化检查 -> 动手 QA -> Gate 决策）
- 支持 `<parallel_plan>` 动态并行执行（见下文）

### Hephaestus -- 自治优先

```
request-analysis -> execution-orchestration
```

| 属性 | 值 |
|------|---|
| 执行工作流 | `AutonomousLoop` -- 自治循环，带验证 |
| Stage 拓扑 | **可配置** -- JSON `stages` 数组生效 |
| 路由模式 | Passthrough |
| 最大轮次 | 3 |
| 验证模式 | Required -- 必须通过自治验证 gate |
| 子 agent 模式 | Sequential |
| 适用场景 | 深度自治执行、复杂代码变更、需要自我验证的任务 |

Hephaestus 的哲学：**深入执行，自我验证，失败时逐级升级**。它是最"放手"的 preset，给 agent 最大的自治空间，但要求执行结果必须通过自我验证。

自定义 Hephaestus 示例：

```jsonc
{
  "orchestrator": "hephaestus",
  "stages": [
    "request-analysis",
    {
      "kind": "execution-orchestration",
      "loopBudget": "step-limit:15",
      "childSession": true,
      "agentTree": {
        "agent": {
          "name": "autonomous-executor",
          "maxSteps": 12,
          "model": { "providerId": "your-provider", "modelId": "your-model-id" }
        },
        "children": [
          {
            "agent": { "name": "test-writer", "allowedTools": ["read", "glob", "grep", "bash"] },
            "prompt": "Write tests for every change before the main executor marks it done."
          }
        ]
      }
    },
    "synthesis"
  ]
}
```

### Verifier -- 候选比较优先

```
request-analysis -> execution-orchestration
```

| 属性 | 值 |
|------|---|
| 执行工作流 | `AutonomousLoop` + `verify` workflow mode |
| Stage 拓扑 | **可配置** -- JSON `stages` 数组生效 |
| 路由模式 | Passthrough |
| 最大轮次 | 3 |
| 候选选择 | all-pairs LLM judge + metric fallback |
| 适用场景 | 同一任务生成多个候选，再通过显式 verifier 比较选优 |

Verifier 的哲学：**不要默认最后一次迭代就是最好的一次**。它复用 Hephaestus 的深度执行内核，但在 workflow 层记录多个候选，随后通过重复的成对比较、加权 criterion 聚合和指标回退选择最终赢家。`round-robin` 会对当前候选池做全组合比较；`tournament` 则按顺序维护 champion/challenger。

Canonical verifier 路径是 `useLogprobs=true` 的 score-job 算法：每个候选对、criterion、repetition 生成一个 score job，读取 A-T score-token top-logprobs 计算 expected reward，并按 `C*K` 均值决定 pair winner。logprob 不可用时会回退到文本 score tag，同时在 artifact 中记录 `logprob_status`。JSON pairwise judge 保留为兼容路径。score-job cache 与 pairwise cache 会按 objective/profile 持久化到 artifact root；summary 会暴露 pairwise comparison / judge call / cache hit / logprob usage 计数，并写出 `score-job-matrix`、`pairwise-score-matrix`、`round-robin-win-counts` 与 `selection-report` mode artifacts。

Verifier trace 是 ROCode-native trajectory projection：候选最终输出、execution steps/tool calls、execution metadata、verify/guard 命令证据、metric、workspace patch 摘要和结构化 mode artifact 摘要都会进入 judge 输入与稳定 trajectory fingerprint。为了控制 judge 成本，配置层限制 criteria 数、`repetitions`、`maxCandidates`、`granularity`，并校验估算的 pairwise judge job budget。

### Preset 对比总览

| | Sisyphus | Prometheus | Atlas | Hephaestus | Verifier |
|---|---------|-----------|-------|-----------|----------|
| Stage 数量 | 3 | 6 | 3 | 2 | 2 |
| Stage 可配置 | 是 | 否（锁死） | 是 | 是 | 是 |
| 执行工作流 | SinglePass | Direct | CoordinationLoop | AutonomousLoop | AutonomousLoop + verify mode |
| 最大轮次 | 1 | 1 | 3 | 3 | 3 |
| 验证 gate | 无 | 无 | 协调验证 | 自治验证 | 候选比较验证 |
| 子 agent 模式 | Sequential | Parallel | Parallel | Sequential | Sequential |
| 典型用途 | 执行 | 规划 | 协调 | 深度自治 | 多候选选优 |

---

## Session Continuity 与 Hydration

Scheduler 每次执行都会收到当前用户 prompt，但 follow-up 任务还需要知道同一 session 之前发生了什么。ROCode 不把全部历史直接塞进每个 stage，而是由 server 构建 shared `SessionContinuityPacket`，再从这个 packet 派生 markdown continuity projection 给模型读取。

也就是说：

- `scheduler_session_context_packet`：唯一结构化 contract / authority
- markdown continuity block：从 packet 渲染出的派生视图，不再是另一条独立真源

派生出的 markdown continuity block 会渲染 `Context Coverage`、`Source Anchors`、`Memory Anchors`、`Hydration Guidance`、`Working Ledger`、`Latest Compaction Summary` 和 `Exact Recent Tail`。它告诉模型当前快照覆盖了多少历史、遗漏了多少 older turns、哪些 message id / memory record id 是可授权回查的。

结构化 packet 带 `version` 门控。orchestrator 只接受当前版本的 packet；版本不匹配时，受控 hydration 的授权面为空，不做猜测降级。

当前有两个只读 hydration 工具：

| 工具 | 授权来源 | 数据源 | 用途 |
|------|----------|--------|------|
| `scheduler_context_hydrate` | `Source Anchors` 中的 `message_id` | 当前 session messages / compaction summary | 回查被截断或摘要化的同会话正文 |
| `scheduler_memory_hydrate` | `Memory Anchors` 中的 `record_id` | workspace-scoped runtime memory | 回查 auto-memory prefetch 授权的跨会话记忆详情，可选 evidence refs |

两个工具都进入 `READ_ONLY_STAGE_TOOLS`，所以 route、plan、review 等只读 stage 也可以在授权范围内回查上下文。每次调用都会在 stage metadata 中记录 hydrated / rejected / missing ids 和截断参数，供 Web、TUI、telemetry 审计。

这套机制适用于所有公开 scheduler preset；具体行为以本节和 scheduler 示例目录中的当前说明为准。

---

## 9 种 Stage 类型

Scheduler 提供 9 种 stage 类型，每种有明确的语义职责：

| Stage | JSON 名 | 职责 |
|-------|---------|------|
| RequestAnalysis | `request-analysis` | 解析用户请求意图，生成 request brief |
| Route | `route` | 意图分类，决定执行路径（可触发 preset 切换） |
| Interview | `interview` | 向用户提问以澄清需求（阻塞式） |
| Plan | `plan` | 生成实现计划 |
| Delegation | `delegation` | 将任务委派给 agent tree 或 skill graph |
| Review | `review` | 审查执行结果 |
| ExecutionOrchestration | `execution-orchestration` | 核心执行阶段，驱动 agent 完成任务 |
| Synthesis | `synthesis` | 汇总和格式化最终输出 |
| Handoff | `handoff` | 交付结果（通常用于规划类 preset） |

`stages` 是一个有序数组（`Vec`，不是 `Set`），因此：

- **stage 可以重复出现** -- 同一种 stage 可以在序列中出现多次
- **顺序自由** -- stage 按数组顺序依次执行
- **长度不限** -- 可以是 2 个 stage，也可以是 11 个

注意：同种 stage 的多次出现共享同一份 override 配置（`HashMap<SchedulerStageKind, SchedulerStageOverride>`）。如果需要区分不同迭代的配置，目前需要使用不同的 stage kind。

### 常见拓扑模式

**执行型**（Sisyphus 风格）：

```
request-analysis -> route -> execution-orchestration
```

**规划型**（Prometheus 风格）：

```
request-analysis -> route -> interview -> plan -> review -> handoff
```

**协调型**（Atlas 风格）：

```
request-analysis -> execution-orchestration -> synthesis
```

**迭代收敛型**：

```
request-analysis -> execution-orchestration -> synthesis
                 -> execution-orchestration -> synthesis
                 -> execution-orchestration -> synthesis
```

**混合型**：

```
request-analysis -> interview -> plan -> execution-orchestration -> review -> synthesis
```

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

### 完整配置结构

```jsonc
{
  "$schema": "https://rocode.dev/schemas/scheduler-profile.schema.json",
  "version": "2026-05-05",
  "defaults": {
    "profile": "my-default"
  },
  "profiles": {
    "my-default": {
      "orchestrator": "sisyphus",
      "description": "自定义描述",
      "model": {
        "providerId": "ethnopic",
        "modelId": "your-model-id"
      },
      "skillList": ["request-analysis", "execution-orchestration"],
      "stages": [...],
      "agentTree": {...},
      "skillGraph": {...},
      "skillTree": {
        "contextMarkdown": "...",
        "tokenBudget": 256,
        "truncationStrategy": "head-tail"
      },
      "workflow": {...}
    },
    "another-profile": { ... }
  }
}
```

### 多 Profile

一个配置文件可以包含多个命名 profile，通过 `defaults.profile` 指定默认激活哪个：

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
  "request-analysis",
  {
    "kind": "execution-orchestration",
    "toolPolicy": "allow-all",
    "loopBudget": "step-limit:10",
    "childSession": true,
    "agentTree": {
      "agent": { "name": "coordinator" },
      "children": [
        { "agent": { "name": "worker-a" }, "prompt": "Do A." },
        { "agent": { "name": "worker-b" }, "prompt": "Do B." }
      ]
    }
  },
  "synthesis"
]
```

### 覆盖字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `kind` | `string` | **必填**。stage 类型（如 `"plan"`、`"execution-orchestration"`） |
| `toolPolicy` | `"allow-all"` / `"allow-read-only"` / `"disable-all"` | 工具访问策略 |
| `loopBudget` | `"unbounded"` / `"step-limit:N"` | 最大 LLM 循环迭代次数 |
| `sessionProjection` | `"hidden"` / `"transcript"` | stage 输出是否可见 |
| `childSession` | `boolean` | 是否创建隔离的子会话 |
| `agentTree` | `AgentTreeNode` / `string` | per-stage agent tree（覆盖 profile 级别） |
| `agents` | `string[]` | agent 名称过滤器 |
| `skillList` | `string[]` | 该 stage 可用的 skill 列表 |

`childSession: true` 会让该 stage 在隔离子会话中运行；用户侧的 `/abort` 取消的是当前活动执行边界，而不是把一条文本消息塞进这个子会话。若要精确终止某个已登记的 agent task，应走 `/tasks kill <ID>` 或 `task_flow cancel`。

### 三层覆盖链

```
per-stage JSON 覆盖  ->  preset 函数默认  ->  硬编码默认
```

省略的字段会沿着这条链向下查找默认值。

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

`transcript` 对于迭代式拓扑至关重要——它是跨迭代上下文传递的机制。

---

## Agent Tree -- 执行者组织

Agent Tree 定义了 stage 内部的执行者层级结构。详见 [agents.md](agents.md) 获取完整的 agent 系统参考。

### 在 Scheduler 中的配置位置

Agent Tree 可以在两个层级配置：

**Profile 级别** -- 作为所有 stage 的默认 agent tree：

```jsonc
{
  "profiles": {
    "my-profile": {
      "agentTree": { "agent": { "name": "deep-worker" } }
    }
  }
}
```

**Per-stage 级别** -- 覆盖特定 stage 的 agent tree：

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
"agentTree": "./trees/coordinator-tree.json"
```

外部文件在加载时自动解析，支持 JSONC 格式（注释、尾逗号）。

---

## Skill Graph -- 图执行策略

Skill Graph 是 Agent Tree 之外的另一种 stage 内执行策略。它用有向图模型定义 agent 之间的流转关系，支持条件分支。

```jsonc
{
  "skillGraph": {
    "entryNodeId": "analyze",
    "maxHops": 20,
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
| 拓扑 | 树形（parent -> children -> aggregation） | 有向图（任意节点间跳转） |
| 并行 | Children 天然并行 | 节点串行执行 |
| 循环 | 不支持 | 通过边条件支持 |
| 聚合 | Root 自动聚合 children 输出 | 无内置聚合，靠节点 prompt 传递 |
| 适用场景 | 多视角并行探索 | 条件分支流程、审查-修改循环 |

两者互斥——如果同时配置了 agent tree 和 skill graph，agent tree 优先。

---

## Skill Tree -- 知识注入

Skill Tree 不是执行策略，而是**上下文注入机制**。它给 scheduler 的所有 stage 携带背景知识。

```jsonc
{
  "skillTree": {
    "contextMarkdown": "This project uses a hexagonal architecture. All domain logic lives in the core module. Adapters are in the adapters/ directory.",
    "tokenBudget": 256,
    "truncationStrategy": "head-tail"
  }
}
```

`contextMarkdown` 的内容会被注入到 scheduler 的系统提示中，影响所有 stage 的 agent 行为。

`tokenBudget` 用近似 token 预算保护 skill tree 上下文，超预算时会按 `truncationStrategy` 执行截断。支持 `head`、`tail`、`head-tail`（默认）三种策略。

典型用途：

- 项目架构约束（"所有 API 必须经过 middleware 层"）
- 编码规范（"使用 immutable 模式，不要 mutation"）
- 领域知识（"这是一个支付系统，所有金额用 decimal 不用 float"）
- 调度策略提示（"优先使用并行探索，不要串行"）

---

## 动态并行执行（parallel_plan）

在 Atlas 和 Hephaestus 的 `execution-orchestration` stage 中，协调 agent 可以在运行时输出 `<parallel_plan>` XML 块来动态创建并行 worker 阵列。

### 声明格式

```xml
<parallel_plan>
{
  "root_task": "Coordinate the migration cleanup",
  "children": [
    {
      "name": "worker-alpha",
      "task": "Update schema files",
      "allowed_tools": ["read", "write", "glob"]
    },
    {
      "name": "worker-beta",
      "task": "Verify migration paths",
      "allowed_tools": ["read", "bash"]
    }
  ]
}
</parallel_plan>
```

### 声明结构

| 字段 | 类型 | 说明 |
|------|------|------|
| `root_task` | `string` | 父协调任务描述 |
| `children` | `DynamicChildAgent[]` | 并行子 agent 列表 |
| `aggregation` | `"synthesize"` / `"concatenate"` | 聚合策略（默认 `synthesize`） |

### DynamicChildAgent 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `name` | `string` | 子 agent 唯一标识 |
| `task` | `string` | 子 agent 的系统提示 / 任务描述 |
| `allowed_tools` | `string[]` | 工具白名单（必须是父 agent 工具的子集） |
| `model` | `string` | 可选模型覆盖（`"provider:model"` 格式） |

### 约束

- 最多 **5** 个子 agent（`DYNAMIC_AGENT_TREE_MAX_CHILDREN`）
- 每个 child 必须有唯一的 `name`
- `allowed_tools` 必须是父 agent 可用工具的子集
- 任务之间必须真正独立：不共享文件写入、无顺序依赖

### 聚合策略

| 策略 | 说明 |
|------|------|
| `synthesize`（默认） | 父 agent 综合 children 输出为统一结果 |
| `concatenate` | 逐字拼接 children 输出 |

### 解析流程

Scheduler 在 LLM 输出中按以下优先级查找声明：

1. `<parallel_plan>...</parallel_plan>` XML 块
2. JSON 代码块（`` ```json ... ``` ``）
3. 顶层 JSON 对象（包含 `children` 字段）

解析成功后，声明被验证并转换为 `AgentTreeNode`，在下一轮协调中由 `AgentTreeOrchestrator` 执行。

---

## Stage 内执行优先级链

当一个 stage（如 `execution-orchestration`）需要执行任务时，scheduler 按以下优先级选择执行策略：

```
1. Per-stage agent tree    <- 最高优先级
2. Profile-level agent tree
3. Skill graph
4. Execution fallback      <- 最低优先级（直接工具执行）
```

这意味着：

- 如果某个 stage 有自己的 agent tree，用它
- 否则用 profile 级别的 agent tree
- 如果都没有 agent tree，用 skill graph
- 如果连 skill graph 都没有，用 fallback（直接执行）

---

## 协调循环与验证 Gate

Atlas 和 Hephaestus 支持多轮协调循环（最大轮次分别为 3）。每轮执行后通过 gate 决策判断是否继续。

### Gate 决策

协调 agent 输出一个 JSON 结构来决定下一步：

```json
{
  "status": "done|continue|blocked",
  "summary": "short summary",
  "next_input": "optional next round task",
  "final_response": "optional final response"
}
```

| 状态 | 含义 |
|------|------|
| `done` | 所有任务完成，有证据支持 |
| `continue` | 仍有未完成任务，需要下一轮 worker |
| `blocked` | 遇到具体阻碍，无法继续 |

### Atlas 的 4 阶段 QA

Atlas 在每次委派后执行强制性 4 阶段验证：

1. **Phase 1: 读取实际输出** -- 不信任 worker 摘要，直接读取每个变更文件
2. **Phase 2: 自动化检查** -- 诊断、测试、构建
3. **Phase 3: 动手 QA** -- 对用户可见的变更进行实际验证
4. **Phase 4: Gate 决策** -- 三个 YES/NO 问题全部通过才标记完成

> Atlas 的 QA gate 是**内部质量检查**，不是用户问卷。只在真正需要用户决策时才使用 `question` 工具。

---

## Stage 能力可观测性

Scheduler stage 运行时元数据区分能力池和运行时激活：

**可用能力池**（描述 stage 可访问的能力范围）：

- `available_skill_count`
- `available_agent_count`
- `available_category_count`

**运行时激活**（描述实际使用的）：

- `active_skills`
- `active_agents`
- `active_categories`

权限边界：

- Scheduler / orchestration 运行时拥有 `active_*` 的语义定义权
- TUI / CLI / Web 消费并渲染这些字段
- 适配层不得从完整能力池推断"已使用的能力"
- 通用工具活动、问题流、摘要和 stage 叙述本身不算能力激活

---

## Profile JSON Schema 参考

完整 JSON Schema 位于 `docs/examples/scheduler/scheduler-profile.schema.json`，Schema ID：`https://rocode.dev/schemas/scheduler-profile.schema.json`。

### 顶层结构

| 字段 | 类型 | 说明 |
|------|------|------|
| `$schema` | `string` | JSON Schema URI |
| `version` | `string` | 配置版本 |
| `defaults.profile` | `string` | 默认激活的 profile 名 |
| `profiles` | `object` | 命名 profile 映射 |

### Profile 字段

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `orchestrator` | `"sisyphus"` / `"prometheus"` / `"atlas"` / `"hephaestus"` / `"verifier"` | 是 | 基于 preset |
| `description` | `string` | | 人类可读描述 |
| `model` | `ModelRef` | | 覆盖 scheduler 使用的模型 |
| `skillList` | `string[]` | | 能力列表 |
| `stages` | `stageEntry[]` | | stage 序列 |
| `agentTree` | `AgentTreeNode` / `string` | | 执行者组织 |
| `skillGraph` | `SkillGraph` | | 图执行策略 |
| `skillTree` | `SkillTree` | | 知识注入 |
| `workflow` / `workflowPath` / `iterativeWorkflow` | `IterativeWorkflowConfig` / `string` | | 内联 workflow 或外部 workflow 文件 |

### ModelRef 字段

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `providerId` | `string` | 是 | 提供商 |
| `modelId` | `string` | 是 | 模型 ID |

### AgentDescriptor 字段

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `name` | `string` | 是 | agent 标识名 |
| `systemPrompt` | `string` | | 自定义系统提示 |
| `model` | `ModelRef` | | per-agent 模型覆盖（优先级最高） |
| `maxSteps` | `integer` | | 最大执行步数 |
| `temperature` | `number` | | 温度参数 |
| `allowedTools` | `string[]` | | 工具白名单 |

### 模型优先级

```
per-agent model (agentTree agent.model)
  -> profile-level model (profile.model)
    -> session 当前模型（fallback）
```

---

## 选型指南

### 按任务类型选择

| 任务 | 推荐 Preset | 理由 |
|------|------------|------|
| 修 bug（已知根因） | Hephaestus | 深度自治，自我验证 |
| 修 bug（未知根因） | Atlas | 需要多视角探索 |
| 候选方案很多，需要显式比较再选优 | Verifier | 保留多个候选并做重复加权 judging |
| 实现新功能（需求清晰） | Sisyphus | 一次执行到位 |
| 实现新功能（需求模糊） | Prometheus -> Sisyphus | 先规划再执行 |
| 大型重构 | Atlas | 多工作流协调 |
| 架构决策 | PSO 拓扑 | 多维度迭代收敛 |
| 安全审计 | Atlas + review stage | 协调 + 审查 |
| 文档编写 | Sisyphus | 简单直接 |

### 按复杂度选择

```
简单任务（< 3 文件，需求明确）
  -> Sisyphus 或 Hephaestus

中等任务（3-10 文件，有一定设计决策）
  -> Atlas

复杂任务（> 10 文件，多维度权衡）
  -> PSO 拓扑 或 Prometheus + Atlas

规划类任务（不执行代码，只出方案）
  -> Prometheus
```

### 成本意识

| 拓扑 | 相对 token 消耗 | 说明 |
|------|----------------|------|
| Hephaestus | 1x | 最精简，2 个 stage |
| Sisyphus | 1.5x | 3 个 stage，含路由 |
| Atlas | 2-3x | 协调循环，最多 3 轮 |
| Prometheus | 2x | 6 个 stage，但不执行代码 |
| PSO-3iter | 6-8x | 3 轮 x 3 粒子 + synthesis |
| PSO-5iter | 10-12x | 5 轮 x 3 粒子 + synthesis |
