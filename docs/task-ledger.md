# TaskLedger 与 `/goal`

TaskLedger 是 Session 内由服务端维护的长期任务状态。普通用户不需要编辑 JSON、维护 revision，
也不需要填写 actor 或时间戳。最常用的入口只有一条命令：

```text
/goal 完成 G-PSO 项目审查，运行构建和测试，并输出带证据的报告
```

这条命令会在当前 Session 中创建 TaskLedger；如果当前 Session 已有 Ledger，则原子地切换到新
Goal。随后请求仍走正常的 `auto` Scheduler 选择，因此 AgenDao 会根据任务形状选择
`direct`、`plan`、`coordinate`、`verify` 或 `autoresearch`，而不是由 `/goal` 固定某一个 Agent
或 Scheduler。

`/goal` 只证明“用户声明了这个目标”。它不证明目标已经完成，也不证明某项验收条件已满足；完成
状态仍必须来自当前 Goal generation 的检查点、确定性检查和未决问题收口。

目标描述是必填项；单独输入 `/goal` 会返回用法提示，不会创建空 Ledger。

## 运行期间记录什么

启用后，服务端在同一个 Session 权威状态中维护：

- `Goal`：当前目标和可选验收条件；
- `Core`：最多两个当前生效的核心约束；
- `Next`：中断或恢复时应继续的下一步；
- `Open`：尚未解决的问题以及解决它所需的证据；
- `Verified`：Evaluator、确定性检查或用户确认产生的检查点；
- `Status`：`active`、`awaiting_user`、`blocked`、`interrupted` 或 `completed`。

permission/question 等待、用户中断、Scheduler evaluator、工具批次和最终交付检查都会回写同一份
Ledger。模型只能读取服务端注入的紧凑投影，不能伪造 `model/evaluator/system` 来源的检查点。

## 替换目标

再次输入 `/goal <新目标>` 会：

- 增加 Goal generation，使旧检查点不能证明新目标；
- 保留历史检查点以便审计；
- 保留 Session 级核心约束；
- 清除上一目标的 Open、等待状态和 blocker；
- 把 `Next` 设置为新目标并重新进入 `active`。

如果当前仍有未解决的 permission/question，服务端不会偷偷丢弃它们；应先回答或中断当前交互，
再切换目标。活动执行期间提交的 `/goal` 会和其他 follow-up 一样排队，在当前回合结束后生效。

`/goal` 不会取消 Scheduler 的有界预算。若某一回合达到 step、token 或活跃 wall-time 上限，已完成
步骤和 Ledger 都会保留；在同一 Session 输入 `continue` 即可从保存的 `Next` 开启下一回合。

## 查看

- Web：创建 Ledger 后，Session 页面显示 Task State 卡片。
- TUI：按 `Ctrl+T` 打开 Task state。
- API/集成：`GET /session/{id}/task-ledger`。底层 PATCH JSON 是 SDK 和自动化接口，不是普通用户入口。

TaskLedger 不会创建 `task-ledger.txt`、`WORKSPACE.md` 或其他工作区镜像；Session metadata 是唯一
权威存储，fork 会携带已提交的快照。
