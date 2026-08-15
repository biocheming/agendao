# Scheduler
AgenDao 只有一个编排抽象：`SchedulerBlueprint`。无论拓扑来自内置模板、用户请求，还是
AI planner 临时生成，最终都经过同一个 catalog、policy、validator 和 `SchedulerEngine`。

## 选择方式

session 创建和 prompt 请求都接受 `scheduler` 字段。未指定时等价于 `auto`。

```json
{ "scheduler": { "kind": "auto" } }
```

也可以显式选择内置模板：

```json
{ "scheduler": { "kind": "template", "template": "verify" } }
```

可用模板是 `direct`、`plan`、`coordinate`、`verify` 和 `autoresearch`。模板只是生成
Blueprint 的纯数据函数，不拥有独立 runtime。

单独提交 `agent` 字段时，运行时会选择 `direct` 模板，并把该 Agent 作为 primary leaf；同时提交
模板 Scheduler 时，该 Agent 覆盖模板的 primary leaf。显式 Blueprint 已经逐节点声明 Agent，因而
不读取这个 leaf override。无论哪种入口，请求都只进入 SchedulerEngine，不存在单 Agent 旁路。

用户也可以直接提交 Blueprint：

```json
{
  "scheduler": {
    "kind": "blueprint",
    "blueprint": {
      "schema": "v1",
      "name": "review-change",
      "entry": "review",
      "nodes": {
        "review": {
          "kind": "agent",
          "agent": "build",
          "skills": ["code-review"],
          "tools": ["read", "grep"],
          "required_model_capabilities": ["tool-calls"],
          "max_steps": 12,
          "next": "done"
        },
        "done": { "kind": "end", "result": "last-node" }
      },
      "limits": {
        "max_model_calls": 16,
        "max_tool_calls": 48,
        "max_total_tokens": 131072,
        "max_wall_time_ms": 900000,
        "max_parallelism": 2,
        "max_graph_nodes": 16,
        "max_graph_depth": 8,
        "max_loop_iterations": 4,
        "max_agent_steps": 12
      },
      "output": {
        "format": "markdown",
        "include_usage": true,
        "include_artifact_refs": true
      }
    }
  }
}
```

完整示例见 [examples/scheduler/blueprint.example.json](examples/scheduler/blueprint.example.json)。

## Auto 语义

`auto` 按以下顺序决定 Blueprint：

1. 用户显式选择优先。
2. `user` 来源的 session Blueprint lock 无条件优先；`heuristic` 或 `planner` 来源只有在拓扑仍满足
   当前任务形状时才复用。
3. 明确任务按“迭代研究、验证、并行、简单”的优先级选择对应内置模板。
4. 其余任务由 AI planner 在 catalog 和 policy 边界内选择模板或生成 Blueprint。

planner 的结果必须通过同一个 validator。planner 失败或 Blueprint 非法时请求失败，不会切换到
另一套执行路径。

planner 创建 Blueprint 时必须同时返回 `agents` 数组。数组可以为空；非空项只能从 catalog 中的
base Agent 派生受限临时身份。生成身份不能扩大 base Agent 的工具、Skill、模型能力、权限或模型
路由，不能覆盖已有 ID，也不能脱离 Blueprint 成为未使用配置。

`GET /session/{id}/blueprint` 返回 Blueprint、fingerprint、真实 selection source 和完整
generated-agent manifest。`PUT` 经过同一个 validator 后保存用户 Blueprint；reject 只适用于 AI
Planner 结果，并把 fingerprint 写入拒绝集合。Web 的 Session Insights 提供加载、JSON 编辑、保存、
重载和拒绝入口，不要求用户只能编辑磁盘 JSONC。

## Blueprint 节点

| 节点 | 作用 | 关键约束 |
|---|---|---|
| `agent` | 运行唯一的 leaf `AgentLoop` | agent、skill、tool 必须存在于 catalog；`max_steps` 有界 |
| `parallel` | 并行执行固定分支并汇合 | 至少两个唯一分支；并发受全局上限约束 |
| `gate` | 运行 typed evaluator 后选择边 | evaluator 必须存在；三种结果都有明确目标 |
| `loop` | 有界重复子图 | 必须声明 evaluator 和正整数迭代上限 |
| `end` | 从最后节点、指定节点或 artifact 取结果 | artifact source 必须引用 artifact-store capability |

普通图不允许环；重复只能由 `loop` 表达。所有节点必须从 entry 可达，并且图中必须存在
`end` 节点。

## Catalog 与 Policy

运行时 catalog 是 Agent、Skill、Tool、Evaluator 和 Capability 的唯一可引用集合。Blueprint
只能缩小 agent 已拥有的 skill/tool surface，不能凭字符串创建能力。

PolicyEnvelope 对工具、副作用、capability 和资源预算设置硬上限。validator 在执行前同时检查：

- catalog 引用与 agent 能力；
- tool/capability 权限和副作用等级；
- 图连通性、普通环、并行 join 和 loop 结构；
- graph depth、node count、model/tool call、token、wall time 和并发预算。

生产 PolicyEnvelope 由当前配置构造：Catalog 工具与顶层 `permission` 取交集，`deny` 工具不会
进入 allowlist，`ask` 工具保留交互式审批语义；没有审批通道的 workspace capability 只在对应
effect 被明确允许时开放。执行和 workspace 数值上限来自 `runtimeBudget`，请求只能进一步缩小
这些上限。

## Agent、Skill 与 Verifier

- Agent 是 Blueprint 的叶节点配置，不拥有子图或调度器。
- Skill 是 agent node 上的 typed 引用，负责知识和方法上下文，不执行控制流；模板按 Agent 的工具面
  和任务语义分别选择 Skill，Planner 也能看到 Skill 的工具/toolset 前置条件。
- Verifier 是 catalog evaluator，由 `gate` 或 `loop` 调用。
- Autoresearch 是 `loop + evaluator + checkpoint` 模板，不是独立引擎。

Agent node 的 `max_steps` 必须小于等于 Agent 自身 `maxSteps`、Blueprint
`limits.max_agent_steps` 和 PolicyEnvelope 硬上限三者的最小值；AgentLoop 按该节点值停止。

## 事件与投影

`SchedulerEngine` 发出 typed `ExecutionEvent`：run start/end/failure、node start/end、loop
iteration 和 evaluation。server 将这一事件流投影为唯一的 `SchedulerRun` / `SchedulerNode`
状态，CLI、TUI 和 Web 读取同一投影，不解析文本卡片或 message metadata 来重建运行状态。

## 上下文缓存

每个 agent 请求按稳定性组装：catalog/system policy 是稳定前缀，Blueprint/agent/tool/skill
surface 是半稳定区，当前 handoff 与用户输入是动态尾部。catalog、Blueprint、agent、tool、skill
bundle 和 continuation boundary 分别拥有 fingerprint；诊断能区分冷启动、具体 surface 变化和
仅动态尾部变化。

不要把完整 Blueprint、兄弟分支历史或运行日志反复写入动态 prompt。并行分支只传显式 handoff，
loop 每次迭代只保留有界结果和 checkpoint 引用。

## 取消与失败

`/abort` 或对应 server 控制请求取消当前运行。budget、deadline、权限拒绝、provider/tool 错误和
validator 错误都沿 typed 事件路径结束运行。系统不执行旧引擎回退，也不接受旧配置格式。
