# Agent
Agent 是 Scheduler Blueprint 中的叶节点身份。它描述模型偏好、system policy、允许的工具、
最大步数和权限；它不描述 children、阶段、重试图、workflow 或 checkpoint。

## 运行模型

所有 agent node 都由 `agendao-orchestrator::agent_loop::AgentLoop` 执行：

1. 从 catalog 解析 agent，并计算该节点允许的 tool/skill 交集。
2. 按稳定前缀、半稳定能力面和动态尾部组装模型请求。
3. 调用 provider；若返回 tool calls，则通过宿主 ToolRunner 执行。
4. 将结构化结果加入本节点 conversation，并受 `max_steps` 和全局预算约束。
5. 输出有界 node result，交回 `SchedulerEngine` 决定下一节点。

Agent 无权自行创建另一套运行循环。需要并行、验证或迭代时，由 Blueprint 使用 `parallel`、
`gate` 或 `loop` 节点表达。

## Agent Registry

registry 合并内置 agent 与当前配置中的 agent 定义。可见 agent 被投影到 SchedulerCatalog，
其中包含：

- `id` 和稳定 system policy；
- 可用 skills 与 tools；
- 模型能力，如 tool calls、reasoning、attachments 和 structured output。

Blueprint 引用不存在的 agent，或请求 agent 未暴露的 skill/tool/model capability，会在执行前
被 validator 拒绝。

当前可见内置身份收敛为 `build`、`plan`、`general`、`deep-worker`、`explore`、
`architecture-advisor`、`docs-researcher` 和 `media-reader`。`compaction`、`title`、`summary`
是隐藏的系统身份。它们共享同一个 AgentLoop；增加身份只增加数据和 policy，不增加执行器。

## 配置

项目或全局配置中的 `agent` map 用于覆盖或新增 AgentInfo。常用字段包括：

| 字段 | 含义 |
|---|---|
| `description` | catalog/UI 中的用途说明 |
| `mode` | `primary`、`subagent` 或 `all` |
| `model` / `variant` | 模型与模型变体 |
| `prompt` | agent 的稳定 system policy |
| `temperature` / `top_p` / `max_tokens` | 模型请求参数 |
| `steps` / `max_steps` | 单个 leaf loop 的步数上限 |
| `tools` | agent 可见工具开关映射 |
| `permission` | 该 agent 的权限规则 |
| `hidden` | 是否从可选择 catalog 隐藏 |

Agent 的 `max_steps` 不能扩大 Blueprint 或 PolicyEnvelope 的硬预算。

## Skill 组合

skill 组合属于 agent node：

```json
{
  "kind": "agent",
  "agent": "build",
  "skills": ["code-review", "rust-testing"],
  "tools": ["read", "grep", "bash"],
  "max_steps": 12,
  "next": "done"
}
```

AI planner 可以根据任务和 catalog 自主选择 skill 组合；用户也可以在显式 Blueprint 中指定。
内置模板会根据 goal、任务形状、每个 Agent 的工具面和 Skill 前置条件分别选择，单个 Agent 最多
三个；不会把一份全局 Skill 集合复制给所有 Agent。skill 的 frontmatter 只描述工具可用性等自身
前置条件，不再通过字符串阶段控制可见性。

## 临时 Agent

AI planner 可以在 `create-blueprint` 决策中同时声明最多四个 session-scoped Agent Profile。
每个临时 Agent 只能声明新 kebab-case ID、现有 `base_agent` 和任务专用 system policy。运行时从
base Agent 原样继承 tools、skills、model capabilities、权限和模型路由；临时 policy 只能追加，
不能覆盖 base policy。profile 随 session Blueprint lock 保存，不写入 JSONC，也不会注册到全局
Agent Registry。用户显式持久化 Agent 仍使用唯一的 `agent` 配置入口。

## 用户选择

用户可以用 CLI/TUI/Web 的 agent 选择覆盖模板 primary leaf，也可以在
`SchedulerChoice::Blueprint` 中精确指定每个节点的 agent。仅指定 Agent 时等价于选择 `direct`
模板并覆盖其 primary leaf；同时指定模板时仍覆盖该模板的 primary leaf；显式 Blueprint 内的节点
身份以 Blueprint 自身为准。所有三种形式最终都进入同一个 SchedulerEngine，不存在 direct prompt
runtime。运行期间，已验证 Blueprint 连同真实来源 `user / heuristic / planner` 和临时 Agent manifest
一起保存在 session lock 中。

## 内置 Agent 怎么用

内置 Agent 不需要写进 `~/.agendao/agendao.json`：

```bash
agendao tui
agendao run "修复并测试这个问题" --agent build
```

API 请求可以写 `{ "agent": "build" }`。只指定 Agent 等价于 `direct` Scheduler 的 primary leaf；
显式 Blueprint 则在每个 agent node 中声明 `agent`，不受顶层 Agent 覆盖。

| Agent | 适合 | 默认边界 |
|---|---|---|
| `build` | 默认实现、修复、测试 | 主工具面，最多 100 step |
| `general` | 通用任务 | 主工具面，最多 20 step |
| `plan` | 分析和规划 | 不提供编辑工具，最多 50 step |
| `deep-worker` | 多步实现和验证 | 主工具面，最多 100 step |
| `explore` | 代码搜索和理解 | 只读工具，最多 30 step |
| `architecture-advisor` | 架构分析和审查 | 只读工具，最多 24 step |
| `docs-researcher` | 文档、GitHub、外部证据 | 研究工具面，最多 30 step |
| `media-reader` | 已路由的 PDF、截图和图表 | 只读媒体内容，最多 12 step |

实际运行还同时受 node `max_steps`、`runtimeBudget.scheduler_max_agent_steps`、模型/工具调用数、
token 和活跃 wall-time 约束。

## 上下文边界

- system policy 和稳定 tool definitions 放在可缓存前缀。
- skill 正文按节点需要加载，并由 content fingerprint 标识。
- 并行分支不复制其他分支的 conversation。
- 父节点只接收有界 handoff/result，不接收完整 child history。
- compaction 与 continuation boundary 变化会进入 cache diagnostic。

这种边界既减少 token 和内存开销，也避免 agent 身份变化无谓破坏 provider prefix cache。
