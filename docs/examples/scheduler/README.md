# Scheduler Blueprint Example
此目录只包含当前 `SchedulerBlueprint` schema。Blueprint 通过 session 创建或 prompt 请求的
`scheduler` 字段内联提交，不由 `agendao.jsonc` 路径加载。

- `blueprint.example.json`：单 agent node 到 end node 的完整当前 schema。
- [Scheduler 文档](../../scheduler.md)：auto、template、Blueprint、validator 和缓存语义。

所有显式 Blueprint 都会先与运行时 SchedulerCatalog 和 PolicyEnvelope 一起校验。因此示例中的
agent、skill 和 tool ID 必须在实际 workspace catalog 中存在；示例不是绕过 catalog 的注册文件。
