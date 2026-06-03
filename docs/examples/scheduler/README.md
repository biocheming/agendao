# Scheduler Examples

这个目录现在按“示例类型”分组，不再把所有 scheduler 相关文件平铺在一层。

如果你第一次进来，按这个顺序看：

1. [`SCHEDULER_GUIDE.md`](SCHEDULER_GUIDE.md)
   - 完整使用指南，解释 preset、stage、agent tree、skill tree、hydration 和自定义配置
2. [`presets/`](presets/)
   - 公开内置 preset 的正式示例：`sisyphus`、`prometheus`、`atlas`、`hephaestus`
3. [`verifier/`](verifier/)
   - verifier preset 的最小配置、完整配置和外置 workflow 示例
4. [`pso/`](pso/)
   - PSO 这种用户自定义 topology 的说明、配置和 swarm tree
5. [`autoresearch/`](autoresearch/)
   - workflow 级 autoresearch 示例，不属于公开 preset，但常和 scheduler profile 一起使用
6. [`trees/`](trees/)
   - 可被多个 profile 复用的共享 agent tree 示例

## 目录结构

```text
docs/examples/scheduler/
├── README.md
├── SCHEDULER_GUIDE.md
├── scheduler-profile.schema.json
├── presets/
│   ├── README.md
│   ├── sisyphus.example.jsonc
│   ├── prometheus.example.jsonc
│   ├── atlas.example.jsonc
│   └── hephaestus.example.jsonc
├── verifier/
│   ├── README.md
│   ├── minimal.example.jsonc
│   ├── profile.example.jsonc
│   └── workflow.example.jsonc
├── pso/
│   ├── README.md
│   ├── example.jsonc
│   └── trees/
│       └── pso-swarm.json
├── autoresearch/
│   ├── README.md
│   ├── book-authoring.example.jsonc
│   ├── verify.sh
│   ├── guard.sh
│   └── skill/
│       ├── SKILL.md
│       └── scripts/
│           ├── verify.sh
│           └── guard.sh
└── trees/
    ├── coordinator-tree.json
    └── deep-worker-tree.jsonc
```

## 这几个目录分别代表什么

### `presets/`

放的是 AgenDao 公开内置 preset 的正式外部配置示例。

- `sisyphus`：执行优先
- `prometheus`：规划优先
- `atlas`：协调优先
- `hephaestus`：自治执行优先

这些例子用于说明“公开 preset 如何通过外部 scheduler profile 文件显式配置”。

### `verifier/`

放的是 verifier 这一类“内置 preset + workflow mode”组合的完整材料。

- `minimal.example.jsonc`
  - 最小可用配置
- `profile.example.jsonc`
  - 带内联 workflow 和外置 workflowPath 的完整配置
- `workflow.example.jsonc`
  - 外置 workflow 文件
- `README.md`
  - verifier 算法、candidate trajectory、artifact 和调优说明

### `pso/`

放的是 PSO 这种用户自定义 topology 示例。它不是公开 preset，而是基于现有 stage 语义拼出的高级结构。

- `example.jsonc`
  - 3 轮 / 5 轮两种 PSO 收敛配置
- `trees/pso-swarm.json`
  - 3 粒子 swarm tree
- `README.md`
  - 适用场景、运行机制和自定义建议

### `autoresearch/`

放的是 workflow 级 autoresearch 示例。它不是 scheduler preset，但会嵌在 scheduler profile 的 `workflow` 字段里，因此放在这里更容易和 verifier / PSO 对照理解。

- `book-authoring.example.jsonc`
  - 一个长文写作 workflow
- `verify.sh` / `guard.sh`
  - 对应的验证与 guard
- `skill/`
  - machine-parsed skill 示例

### `trees/`

这是共享 agent tree 示例区，服务于多个 scheduler 配置，不绑定某一个 preset。

## 示例路径约定

- profile 里的 `agentTree` 路径，总是相对当前配置文件解析
- `workflowPath` 也是相对当前配置文件解析
- 因此：
  - `presets/sisyphus.example.jsonc` 引共享 tree 时写 `../trees/...`
  - `verifier/profile.example.jsonc` 引 workflow 时写 `./workflow.example.jsonc`
  - `pso/example.jsonc` 引 swarm tree 时写 `./trees/pso-swarm.json`

## 当前公开 preset

- `sisyphus`
- `prometheus`
- `atlas`
- `hephaestus`
- `verifier`

它们共享同一个 schema：

- `https://agendao.dev/schemas/scheduler-profile.schema.json`

## 一个最直接的入口

如果你只是想先跑一个公开 preset 示例，可以从：

```jsonc
{
  "schedulerPath": "./docs/examples/scheduler/presets/sisyphus.example.jsonc"
}
```

开始。等你需要 verifier、PSO 或 autoresearch，再进入对应子目录。
