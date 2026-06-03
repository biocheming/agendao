# Agent 系统完整参考

AgenDao 的 Agent 系统支持从单 agent 执行到多层 agent 树协调的完整谱系。Agent 通过 `AgentDescriptor` 定义身份与能力，通过 `AgentTreeNode` 组织为层级协作结构。

---

## 目录

- [核心数据结构](#核心数据结构)
- [AgentDescriptor -- agent 身份定义](#agentdescriptor----agent-身份定义)
- [AgentTreeNode -- 执行者组织节点](#agenttreenode----执行者组织节点)
- [AgentTreeOrchestrator -- 树执行引擎](#agenttreeorchestrator----树执行引擎)
- [在 Profile 中定义 Agent Tree](#在-profile-中定义-agent-tree)
- [动态 Agent Tree（LLM 输出解析）](#动态-agent-trellm-输出解析)
- [Agent 工具过滤与权限](#agent-工具过滤与权限)
- [Agent 模型覆盖](#agent-模型覆盖)
- [Agent 执行上下文](#agent-执行上下文)
- [与 Scheduler 的集成](#与-scheduler-的集成)
- [设计原则](#设计原则)

---

## 核心数据结构

```
AgentDescriptor      -- 定义单个 agent 的身份与能力
    |
    v
AgentTreeNode        -- 将 AgentDescriptor 组织为层级树结构
    |
    v
AgentTreeOrchestrator -- 执行 agent 树，管理 root -> children -> aggregation 流程
```

---

## AgentDescriptor -- agent 身份定义

`AgentDescriptor` 定义了单个 agent 的完整身份。它是在 scheduler profile JSON 和 Rust 代码中共用的核心类型。

### 结构定义

```rust
pub struct AgentDescriptor {
    pub name: String,                    // agent 标识名（必填）
    pub system_prompt: Option<String>,   // 自定义系统提示
    pub model: Option<ModelRef>,         // per-agent 模型覆盖
    pub max_steps: Option<u32>,          // 最大执行步数
    pub temperature: Option<f32>,        // 温度参数
    pub allowed_tools: Vec<String>,      // 工具白名单
}
```

### JSON 配置字段

| 字段 | JSON 键 | 类型 | 必填 | 说明 |
|------|---------|------|------|------|
| `name` | `name` | `string` | 是 | agent 标识名，在 agent tree 中用于路由和引用 |
| `system_prompt` | `systemPrompt` | `string` | | 自定义系统提示前缀 |
| `model` | `model` | `ModelRef` | | per-agent 模型覆盖（优先级最高） |
| `max_steps` | `maxSteps` | `integer` | | 最大执行步数限制 |
| `temperature` | `temperature` | `number` | | 采样温度参数（0.0-1.0） |
| `allowed_tools` | `allowedTools` | `string[]` | | 工具白名单（空数组 = 无限制） |

### 配置示例

```jsonc
{
  "name": "deep-worker",
  "systemPrompt": "You are a deep-work execution agent. Focus on complete, tested implementations.",
  "model": {
    "providerId": "zhipuai",
    "modelId": "glm-5.1"
  },
  "maxSteps": 12,
  "temperature": 0.3,
  "allowedTools": ["read", "glob", "grep", "bash", "write"]
}
```

### Agent 名称约定

常见的 agent 名称及其语义角色：

| 名称 | 典型角色 |
|------|---------|
| `deep-worker` | 通用深度执行 agent |
| `code-explorer` | 代码库探索和证据收集 |
| `docs-researcher` | 外部文档和依赖研究 |
| `architecture-advisor` | 架构约束检查 |
| `coordinator` | Atlas 协调 agent（只协调不写码） |
| `qa-reviewer` | 质量审查 agent |
| `security-auditor` | 安全审计 agent |
| `test-writer` | 测试编写 agent |
| `autonomous-executor` | Hephaestus 自治执行 agent |
| `oracle` | 高推理能力专家（昂贵） |

---

## AgentTreeNode -- 执行者组织节点

`AgentTreeNode` 是 agent 树的基本构建单元。它包含一个 agent 定义、可选的角色提示、以及子 agent 列表。

### 结构定义

```rust
pub struct AgentTreeNode {
    pub agent: AgentDescriptor,          // 本节点的 agent（必填）
    pub prompt: Option<String>,          // 角色/任务提示
    pub children: Vec<AgentTreeNode>,    // 子 agent 节点列表
}
```

### JSON 配置

```jsonc
{
  "agent": { "name": "coordinator" },
  "prompt": "Coordinate the parallel workstreams.",
  "children": [
    {
      "agent": { "name": "worker-a", "allowedTools": ["read", "glob", "bash"] },
      "prompt": "Own workstream A.",
      "children": []
    },
    {
      "agent": { "name": "worker-b", "allowedTools": ["read", "glob", "bash"] },
      "prompt": "Own workstream B."
    }
  ]
}
```

### 字段说明

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `agent` | `AgentDescriptor` | 是 | 本节点的 agent 定义 |
| `prompt` | `string` | | 角色/任务提示，附加到 agent 的系统提示中 |
| `children` | `AgentTreeNode[]` | | 子 agent 列表。为空则退化为单 agent 执行 |

树结构支持递归嵌套，理论上不限深度。实践中建议不超过 2-3 层。

---

## AgentTreeOrchestrator -- 树执行引擎

`AgentTreeOrchestrator` 负责执行 agent 树。它实现了 `Orchestrator` trait，被 scheduler 在 `execution-orchestration` stage 中调用。

### 执行流程

Agent Tree 的执行遵循固定的三步流程：

```
1. Root agent 执行    -> 产出初始 draft
2. Children 执行     -> 每个 child 看到 root draft + 原始任务 + 自己的 role prompt
3. Root 再次执行     -> 聚合所有 child 输出，产出最终结果
```

如果没有 children，root agent 直接返回结果（退化为单 agent 执行）。

### 任务组合规则

**Root 初始任务**（有 role prompt 时）：

```
Task:
{input}

Role:
{role_prompt}
```

**Child 任务**：

```
Parent Task:
{original_task}

Parent Draft:
{parent_output}

Delegated Child Agent:
{child_name}
```

**Root 聚合任务**：

```
Original Task:
{original_task}

Your Previous Draft:
{parent_draft}

Child Outputs:
- child_a: {output_a}
- child_b: {output_b}

Synthesize a single final answer.
```

### Child 执行模式

| 模式 | 说明 |
|------|------|
| `Parallel`（默认） | 所有 children 通过 `try_join_all` 并行执行 |
| `Sequential` | Children 按顺序依次执行 |

默认并行模式充分利用异步运行时，适合独立的探索或执行任务。

### 输出聚合

Root agent 的最终输出包含所有层级汇聚的统计数据：

- `content` -- 聚合后的最终内容
- `steps` -- 所有节点（root + children + aggregation）的总步数
- `tool_calls_count` -- 所有节点的总工具调用数
- `metadata` -- 合并后的元数据（包含子 agent 的 session 信息）
- `finish_reason` -- 聚合轮次的完成原因

### Stage 上下文传播

`AgentTreeOrchestrator` 支持 `set_stage_context(stage_name, stage_index)`，将当前 scheduler stage 信息传播到每个子 `SkillListOrchestrator`，使工具活动能正确归属到当前 stage。

---

## 在 Profile 中定义 Agent Tree

Agent Tree 可以在三个层级配置，按优先级从高到低：

### 1. Per-stage 级别

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

### 2. Profile 级别

```jsonc
{
  "profiles": {
    "my-profile": {
      "agentTree": {
        "agent": { "name": "deep-worker" },
        "children": [
          { "agent": { "name": "code-explorer" }, "prompt": "Explore code." }
        ]
      }
    }
  }
}
```

### 3. 外部文件引用

```jsonc
// 引用外部 JSON 文件
"agentTree": "./trees/coordinator-tree.json"

// 或 JSONC 文件（支持注释和尾逗号）
"agentTree": "./trees/deep-worker-tree.jsonc"
```

文件路径相对于配置文件所在目录解析。外部文件的好处：

- **复用**：多个 profile 或 stage 引用同一个 tree 文件
- **可读性**：复杂的 tree 不会让 scheduler 配置变得臃肿
- **独立管理**：agent 团队组成和 stage 编排策略分离

### 推荐的文件布局

```
project/
  agendao.jsonc                  -- schedulerPath -> ./scheduler.jsonc
  scheduler.jsonc               -- 主调度配置
  trees/
    coordinator-tree.json       -- 可复用的协调 agent 树
    deep-worker-tree.jsonc      -- 可复用的深度执行 agent 树
    pso-swarm.json              -- PSO 粒子群 agent 树
```

---

## 动态 Agent Tree（LLM 输出解析）

在运行时，协调 agent（如 Atlas）可以输出 `<parallel_plan>` XML 块来动态创建 agent 树，无需预先在配置中定义。

### 解析入口

`parse_dynamic_agent_tree(output: &str) -> Option<AgentTreeNode>`

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
      "allowed_tools": ["read", "bash"],
      "model": "alibaba-cn:qwen3.6-plus"
    }
  ]
}
</parallel_plan>
```

### DynamicAgentTreeDeclaration 结构

```rust
pub struct DynamicAgentTreeDeclaration {
    pub root_task: String,                      // 父协调任务描述
    pub children: Vec<DynamicChildAgent>,       // 并行子 agent 列表
    pub aggregation: Option<AggregationStrategy>, // 聚合策略
}
```

### DynamicChildAgent 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `name` | `string` | 子 agent 唯一标识（用于输出聚合） |
| `task` | `string` | 系统提示 / 任务描述 |
| `allowed_tools` | `string[]` | 工具白名单（默认空，即无限制） |
| `model` | `string` | 可选模型覆盖，`"provider:model"` 格式 |

### AggregationStrategy

| 策略 | 说明 |
|------|------|
| `synthesize`（默认） | 父 agent 综合 children 输出为统一结果 |
| `concatenate` | 逐字拼接 children 输出 |

### 验证规则

动态声明经过严格验证，以下情况会被拒绝（返回 `None`）：

- 没有 children（`children` 为空）
- 超过 5 个 children（`DYNAMIC_AGENT_TREE_MAX_CHILDREN = 5`）
- 存在重复的 child `name`
- child `name` 为空字符串

### 解析优先级

Scheduler 在 LLM 输出中按以下优先级查找声明：

1. `<parallel_plan>...</parallel_plan>` XML 块
2. JSON 代码块（`` ```json ... ``` ``）
3. 顶层 JSON 对象（包含 `children` 字段）

解析失败时优雅降级（返回 `None`），scheduler 继续使用标准委派路径。

### 转换结果

解析成功后，声明被转换为标准的 `AgentTreeNode`：

- `root_task` 变为 root agent 的 `system_prompt`
- root agent 的 `name` 固定为 `"coordinator"`
- 每个 child 的 `task` 变为其 `system_prompt`
- 如果 child 指定了 `model`（如 `"alibaba-cn:qwen3.6-plus"`），解析为 `ModelRef { provider_id, model_id }`

---

## Agent 工具过滤与权限

### allowedTools 白名单

`AgentDescriptor.allowedTools` 控制该 agent 可以使用的工具集合：

```jsonc
{
  "name": "reader-only",
  "allowedTools": ["read", "glob", "grep"]
}
```

| 配置 | 效果 |
|------|------|
| 空数组（默认） | 无限制，所有工具可用 |
| `["read", "glob", "grep"]` | 只读探索 |
| `["read", "glob", "grep", "bash"]` | 探索 + 命令执行 |
| `["read", "glob", "grep", "write", "bash"]` | 完整能力 |

### toolPolicy 与 allowedTools 的关系

Scheduler 的 `toolPolicy`（stage 级别）和 `allowedTools`（agent 级别）是两层过滤：

```
toolPolicy 定义可用的工具大类
    -> allowedTools 在大类内进一步限制具体工具
```

例如：`toolPolicy: "allow-all"` + `allowedTools: ["read"]` = 实际只能用 read 工具。

### 动态并行中的工具限制

`<parallel_plan>` 的 `allowed_tools` 必须是父 agent 可用工具的子集。这是在 `validate_declaration` 中检查的。

---

## Agent 模型覆盖

### ModelRef 结构

```rust
pub struct ModelRef {
    pub provider_id: String,   // 提供商标识
    pub model_id: String,      // 模型 ID
}
```

### JSON 配置

```jsonc
{
  "model": {
    "providerId": "zhipuai",
    "modelId": "glm-5.1"
  }
}
```

### 模型优先级链

```
per-agent model (agentTree agent.model)
  -> profile-level model (profile.model)
    -> session 当前模型（fallback）
```

### 动态声明中的模型覆盖

在 `<parallel_plan>` 中，模型使用紧凑的 `"provider:model"` 格式：

```jsonc
{
  "name": "fast-worker",
  "task": "Quick analysis",
  "model": "alibaba-cn:qwen3.6-plus"
}
```

支持 `:` 或 `/` 作为分隔符。

---

## Agent 执行上下文

### ExecutionContext

每次 agent 执行都有一个 `ExecutionContext`：

```rust
pub struct ExecutionContext {
    pub session_id: String,           // 会话 ID
    pub workdir: String,              // 工作目录
    pub agent_name: String,           // agent 名称
    pub metadata: HashMap<String, Value>,  // 扩展元数据
}
```

### OrchestratorContext

Agent 执行需要的完整上下文：

```rust
pub struct OrchestratorContext {
    pub agent_resolver: Arc<dyn AgentResolver>,     // agent 名称解析器
    pub model_resolver: Arc<dyn ModelResolver>,     // 模型调用接口
    pub tool_executor: Arc<dyn ToolExecutor>,       // 工具执行接口
    pub lifecycle_hook: Arc<dyn LifecycleHook>,     // 生命周期钩子
    pub cancel_token: Arc<dyn CancelToken>,         // 取消令牌
    pub exec_ctx: ExecutionContext,                 // 执行上下文
}
```

`cancel_token` 是 agent 执行的统一取消入口。当前运行时注册表已经支持给 agent task 单独挂接 token，但并不是所有调用点都已接成“每个 task 一枚独立 token”；因此用户侧精确取消仍以 `/tasks kill <ID>` 为准，而 `/abort` 更多针对当前会话的活动执行边界。

### AgentResolver

`AgentResolver` trait 负责将 agent 名称解析为 `AgentDescriptor`：

```rust
pub trait AgentResolver: Send + Sync {
    fn resolve(&self, name: &str) -> Option<AgentDescriptor>;
}
```

当 `AgentTreeNode` 中的 agent 只指定了 `name`（没有完整的 `AgentDescriptor`），执行引擎会通过 `AgentResolver` 查找完整的 agent 定义。

---

## 与 Scheduler 的集成

### 执行优先级

在 scheduler 的 `execution-orchestration` stage 中，agent tree 的选择遵循以下优先级（详见 [scheduler.md](scheduler.md)）：

```
1. Per-stage agent tree    <- 最高优先级
2. Profile-level agent tree
3. Skill graph
4. Execution fallback      <- 最低优先级（直接工具执行）
```

### 不同 Preset 的默认 Agent Tree

| Preset | 默认 Root Agent | 默认 Children |
|--------|----------------|---------------|
| Sisyphus | deep-worker | code-explorer + docs-researcher |
| Prometheus | deep-worker | code-explorer + docs-researcher + architecture-advisor |
| Atlas | deep-worker | code-explorer + docs-researcher + architecture-advisor |
| Hephaestus | deep-worker | 无（单 agent） |

### Atlas 的 Agent 角色分工

Atlas 模式下 agent 角色有明确分工：

- **coordinator**（root）-- 指挥，不写码。负责分解任务、委派、验证
- **code-explorer**（child）-- 代码库探索，收集实现证据
- **docs-researcher**（child）-- 外部文档和依赖 API 研究
- **architecture-advisor**（child）-- 检查并行工作流是否能收敛

Atlas 的协调章程要求 root agent 遵循 6 段式委派简报结构：TASK、EXPECTED OUTCOME、REQUIRED TOOLS、MUST DO、MUST NOT DO、CONTEXT。

---

## 设计原则

1. **Children 之间应该有认知差异** -- 如果所有 child 做同样的事，不如用单 agent
2. **Root agent 的 systemPrompt 决定聚合质量** -- 它需要知道如何综合不同视角
3. **Tree 深度不宜过深** -- 每层都有 root 执行 + children 执行 + 聚合的开销
4. **allowedTools 可以限制 child 的能力范围** -- 例如只给 read-only 工具做探索
5. **动态 agent tree 适合运行时决策** -- 不确定是否需要并行时，让 LLM 决定
6. **静态 agent tree 适合已知模式** -- 团队组成固定时，在配置中声明更清晰
7. **AgentResolver 解耦名称与定义** -- 配置中只写名称，运行时解析完整定义
