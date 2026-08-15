# Hooks
Hooks 用于在明确的宿主边界观察或变换数据。它们不是 scheduler 扩展点，不得创建图、推进节点、
维护 agent 状态或绕过 Tool/Permission authority。

## 当前脚本事件

TypeScript/subprocess plugin 可注册以下名称：

| 名称 | 时机 | 可变换内容 |
|---|---|---|
| `chat.headers` | provider 请求前 | 请求 headers |
| `chat.params` | provider 请求前 | 模型参数 |
| `chat.message` | assistant message 完成后 | message 观察/处理 |
| `tool.definition` | tool surface 构建时 | tool definition |
| `tool.execute.before` | 工具执行前 | 工具输入 |
| `tool.execute.after` | 工具执行后 | 工具输出 |
| `permission.ask` | 请求权限时 | 权限请求 |
| `command.execute.before` | 自定义命令执行前 | command 输入 |
| `shell.env` | shell 启动前 | 环境变量 |
| `experimental.chat.system.transform` | system prompt 构建时 | system 内容 |
| `experimental.chat.messages.transform` | provider message 构建时 | message 列表 |
| `experimental.session.compacting` | compaction 边界 | compaction 输入/输出 |
| `experimental.telemetry.snapshot.updated` | telemetry snapshot 更新后 | 只读通知 |
| `experimental.text.complete` | 文本完成边界 | 完成文本 |

只有实际生产路径触发的事件才构成公开能力。Scheduler 的 typed `ExecutionEvent` 通过 server
projection 公开，不转换为 plugin stage hooks。

## 执行策略

- `ConfigLoaded` 和 `ShellEnv` 可缓存；缓存键包含事件和输入摘要。
- session end、compaction、telemetry、error 和 file change 一类通知可异步触发。
- 影响模型或工具请求的 transform 必须同步完成，错误返回给调用边界。
- 每个 hook 的输入输出都使用结构化 JSON；不要依赖日志文本或 UI block 格式。

## Native Plugin

Rust native plugin 通过 `PluginHookRef` 声明 HookEvent，再向 PluginSystem 注册 handler。handler
接收 `HookContext`：

```rust
pub struct HookContext {
    pub event: HookEvent,
    pub data: HashMap<String, serde_json::Value>,
    pub session_id: Option<String>,
    pub timestamp: DateTime<Utc>,
}
```

handler 返回 `HookOutput` 或 `HookError`。只在需要进程内 Rust 能力时使用 native plugin；普通
扩展优先使用已支持的脚本事件。

## 编排边界

以下行为不属于 hook：

- 选择内置模板或生成 Blueprint；
- 增加 agent、parallel、gate、loop 或 end 节点；
- 在 verifier 失败时选择下一条边；
- 管理 scheduler budget、deadline、cancel 或 checkpoint；
- 从 UI 文本重建 execution state。

这些行为全部由 `AutoSelector`、Validator、`SchedulerEngine`、Catalog/Policy 和 typed projection
承担。需要改变拓扑时提交 Blueprint；需要增加知识时注册 Skill；需要外部副作用时注册 Tool 或
Capability。

## 缓存纪律

`chat.system`、message 和 tool definition transform 会影响 provider cache。hook 输出必须对同一
输入保持确定性；时间戳、随机 ID、运行进度等动态值只能进入动态尾部。修改稳定 prompt surface
后，cache fingerprint 和诊断必须能反映变化。
