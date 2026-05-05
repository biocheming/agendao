# Built-in Scheduler Presets

这个目录只放公开内置 preset 的正式示例配置。

- `sisyphus.example.jsonc`
  - 执行优先，单轮 route 后进入共享执行内核
- `prometheus.example.jsonc`
  - 规划优先，固定 interview / plan / review / handoff 拓扑
- `atlas.example.jsonc`
  - 协调优先，适合分解、并行推进和综合
- `hephaestus.example.jsonc`
  - 自治执行优先，强调深入执行与自我验证

这些示例共享顶层 schema：

- `../scheduler-profile.schema.json`

共享 agent tree 示例位于：

- `../trees/coordinator-tree.json`
- `../trees/deep-worker-tree.jsonc`
