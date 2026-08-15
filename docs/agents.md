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

当前内置身份包括主执行、规划、探索、深度工作、架构、文档研究、代码探索、媒体读取和评审等
职责。它们共享同一个 AgentLoop；增加身份主要增加数据和 policy，不增加执行器。

## 配置

项目或全局配置中的 `agent` map 用于覆盖或新增 AgentInfo。常用字段包括：

| 字段 | 含义 |
|---|---|
| `description` | catalog/UI 中的用途说明 |
| `mode` | `primary`、`subagent` 或 `all` |
| `model` / `modelPreference` | 模型选择约束 |
| `systemPrompt` | agent 的稳定 system policy |
| `temperature` / `topP` / `maxTokens` | 模型请求参数 |
| `maxSteps` | 单个 leaf loop 的步数上限 |
| `allowedTools` | agent 可见工具集合 |
| `permission` | 该 agent 的权限规则 |
| `hidden` | 是否从可选择 catalog 隐藏 |

Agent 的 `maxSteps` 不能扩大 Blueprint 或 PolicyEnvelope 的硬预算。

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
skill 的 frontmatter 只描述工具可用性等自身前置条件，不再通过字符串阶段控制可见性。

## 用户选择

用户可以用现有 CLI/TUI agent 选择覆盖默认 leaf 身份，也可以在 `SchedulerChoice::Blueprint` 中
精确指定每个节点的 agent。显式 scheduler 选择优先于 auto selector，运行期间由 session lock
保持同一份已验证 Blueprint。

## 上下文边界

- system policy 和稳定 tool definitions 放在可缓存前缀。
- skill 正文按节点需要加载，并由 content fingerprint 标识。
- 并行分支不复制其他分支的 conversation。
- 父节点只接收有界 handoff/result，不接收完整 child history。
- compaction 与 continuation boundary 变化会进入 cache diagnostic。

这种边界既减少 token 和内存开销，也避免 agent 身份变化无谓破坏 provider prefix cache。
