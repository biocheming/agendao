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
2. session 已锁定的 Blueprint 保持不变。
3. 明确的简单、验证、并行或迭代研究任务选择对应内置模板。
4. 其余任务由 AI planner 在 catalog 和 policy 边界内选择模板或生成 Blueprint。

planner 的结果必须通过同一个 validator。planner 失败或 Blueprint 非法时请求失败，不会切换到
另一套执行路径。

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

## Agent、Skill 与 Verifier

- Agent 是 Blueprint 的叶节点配置，不拥有子图或调度器。
- Skill 是 agent node 上的 typed 引用，负责知识和方法上下文，不执行控制流。
- Verifier 是 catalog evaluator，由 `gate` 或 `loop` 调用。
- Autoresearch 是 `loop + evaluator + checkpoint` 模板，不是独立引擎。

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
