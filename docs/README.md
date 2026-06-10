# AgenDao Docs

文档基线：`v2026.6.10`（更新日期：`2026-06-10`）

这里不是“今天改了什么”的补丁板，而是 AgenDao 文档系统的正式入口。后续读 `agendao/docs` 时，先用这份文档判断每类文档的用途，再进入具体主题。

## 先看阅读规则

`agendao/docs` 里的内容分三类：

1. 稳定产品文档
   - 解释当前已经对用户、集成方、前端或 CLI/TUI 开放的能力。
2. 设计/实施参考
   - 解释为什么这样设计、哪些边界不能退化、哪些局部计划仍在进行中。
3. 示例与 schema
   - 提供合法样例、模板和教程，不直接代表主线完成度。

带日期或 `phase*` / `progress` / `report` 命名的文档，默认按阶段记录、实现复盘或测试归档理解。它们可以解释“当时为什么这么做”，但不能替代 `index.md`、`commands.md`、`configuration.md` 这类当前产品文档。

如果你只想知道“现在什么是真的”，先读：

- `documentation-status.md`
- `index.md`
- `commands.md`
- `tools.md`
- `context-caching.md`

## 当前产品面

如果你是来判断“AgenDao 现在已经具备哪些正式能力”，优先抓下面几条：

- **统一 authority 运行时**
  - CLI、TUI、Web、Server 共用同一套 session、provider、tool、scheduler、skill、memory、telemetry authority。
- **长回合上下文治理**
  - replay authority、prompt surface、context closure、cache diagnostics 和 compaction 边界都已进入正式实现，而不是停留在局部约定。
- **工具轨迹可解释**
  - tool repair、trajectory quality、tool-result governance、permission/steering/runtime state 都已有正式读面，能被三端消费。
- **方法沉淀与运行治理**
  - skill hub、memory validation/consolidation、scheduler continuity、proposal/review/gate 已形成完整产品面。
- **计划文档有边界**
  - `plans/` 里的文档默认是局部设计与实施参考；只有被总览文档吸收的内容，才算当前产品真相。

更细的判断见 `documentation-status.md`。

## 当前文档入口

- `README.md`
  - docs 门户、阅读顺序、当前文档分类
- `documentation-status.md`
  - 每份核心文档与计划文档的当前定位、状态和使用边界
- `installation.md`
  - 单一 `agendao` 分发入口的安装、升级、卸载，以及默认内嵌 Web 资源与可选外部覆盖说明
- `../CHANGELOG.md`
  - 发布记录，不替代产品总览文档
- `../USER_GUIDE.md`
  - 面向使用者的命令、scheduler、TUI 交互说明，以及 memory / skill 自进化使用心智
- `skills.md`
  - Skill lifecycle、skill reflection、`skill_manage` 写入与 memory linkage，以及 skill hub search / trust / stale index 发现链路
- `tools.md`
  - 工具层参考，包括 `skill_manage`、`task` / `task_flow` 的 canonical-first 约束，以及 memory 可观测面入口
- `configuration.md`
  - 配置分层、workspace 边界，以及 memory 受 workspace mode 约束的作用域说明
- `context-caching.md`
  - closeai-compatible / ethnopic-compatible 两类协议族下的上下文缓存策略、稳定提示面、replay continuity、输出投影与 cache diagnostic
- `examples/scheduler/README.md`
  - public scheduler presets、stage 默认值、当前行为说明
- `examples/scheduler/SCHEDULER_GUIDE.md`
  - Scheduler 完整使用指南（Tutorial & User Guide）
- `examples/context_docs/README.md`
  - `context_docs` schema、registry、index 示例
- `examples/plugins_example/README.md`
  - Skill / TS plugin / Rust 扩展示例

## Examples

- `examples/context_docs/`
  - Formal examples for `context_docs`
  - Includes minimal `agendao.json` / `agendao.jsonc` config samples
  - Includes `context-docs-registry` schema and example
  - Includes `context-docs-index` schema and example docs index
- `examples/scheduler/`
  - 按目录拆分的 scheduler 示例入口：公开 preset、verifier、PSO、autoresearch workflow、共享 trees
  - Includes generic scheduler JSON Schema and current public example profiles / workflow examples
- `plugins_example/`
  - Skill / TS plugin / Rust extension examples

## Plans

- `plans/`
  - 设计笔记与架构计划。默认只作为实施参考，不自动构成主线。
- `plans/message-replay-authority-refactor.md`
  - replay authority 收口复盘；当前状态为已完成的局部技术线
- `docs/plans/frontend-backend-decoupling-blueprint.md`
  - 前端/后端解耦蓝图；当前主要作为边界守护参考
- `docs/plans/tui-session-graph-sidebar.md`
  - TUI session graph sidebar 的设计 backlog
- `docs/plans/verifier-mode-preset-design.md`
  - Verifier preset 的算法与架构设计；主体已落地，文档现在偏复盘与边界说明
- `docs/plans/verifier-mode-implementation-checklist.md`
  - Verifier mode 已完成项与剩余 polish

更完整的计划状态，统一看 `documentation-status.md`。

## Context Docs Entry

The canonical entry for `context_docs` examples is:

- `docs/examples/context_docs/README.md`
- `docs/examples/context_docs/context-docs-registry.schema.json`
- `docs/examples/context_docs/context-docs-index.schema.json`
- `docs/examples/context_docs/context-docs-registry.example.json`
- `docs/examples/context_docs/react-router.docs-index.example.json`
- `docs/examples/context_docs/tokio.docs-index.example.json`

The canonical schema IDs are:

- `https://agendao.dev/schemas/context-docs-registry.schema.json`
- `https://agendao.dev/schemas/context-docs-index.schema.json`

Read-only validation entry:

```bash
agendao debug docs validate
agendao debug docs validate --registry ./docs/examples/context_docs/context-docs-registry.example.json
agendao debug docs validate --index ./docs/examples/context_docs/react-router.docs-index.example.json
```

## Scheduler Entry

The canonical scheduler example entry is:

- `docs/examples/scheduler/README.md`
- `docs/examples/scheduler/scheduler-profile.schema.json`
- `docs/examples/scheduler/presets/`
  - `sisyphus.example.jsonc`
  - `prometheus.example.jsonc`
  - `atlas.example.jsonc`
  - `hephaestus.example.jsonc`
- `docs/examples/scheduler/verifier/README.md`
  - Verifier preset 的完整上手指南：解决的问题、原算法、AgenDao 实现路线、artifacts、fallback、cache 和调优建议
- `docs/examples/scheduler/verifier/minimal.example.jsonc`
  - Verifier preset 的最小上手配置：保留通过验证的候选，并用一个 criterion 选优
- `docs/examples/scheduler/verifier/profile.example.jsonc`
- `docs/examples/scheduler/verifier/workflow.example.jsonc`
- `docs/examples/scheduler/pso/README.md`
- `docs/examples/scheduler/pso/example.jsonc`
- `docs/examples/scheduler/autoresearch/README.md`
- `docs/examples/scheduler/autoresearch/book-authoring.example.jsonc`

## Tool Config Entry

The canonical external tool config example entry is:

- `docs/examples/tools/README.md`
- `docs/examples/tools/agendao.jsonc.example`
- `docs/examples/tools/single-file/`
- `docs/examples/tools/split-imports/`
- `docs/examples/tools/directory-infer/`
- `docs/examples/tools/partial-backfill/`
- `docs/examples/tools/catalog-only/`

The public scheduler presets are:

- `sisyphus`
- `prometheus`
- `atlas`
- `hephaestus`
- `verifier`

The current schema IDs are:

- `https://agendao.dev/schemas/scheduler-profile.schema.json`

## Web Frontend Entry

当前默认 Web 前端源码目录是 `apps/agendao-web`（React 版本）：

- `/` 是正式 Web 入口
- `/web/*` 是正式静态资源前缀
- `agendao-server` 会把 `apps/agendao-web/dist` 内嵌进发布二进制
- `build.rs` 只会在 Web 源码缺失或变更时增量触发 `npm run build`
- `agendao web` 默认优先使用内嵌资源；仅在显式设置 `AGENDAO_WEB_DIST` 或使用 `AGENDAO_WEB_DEV_URL` 时走外部覆盖/开发路径
- 当前 Web 交互已包含可过滤 model picker、批量 session 删除确认和更高密度的消息阅读节奏

## Skill Hub CLI

远程 skill distribution / artifact cache / managed lifecycle 的正式 CLI 入口现在是：

```bash
agendao skill hub status
agendao skill hub managed
agendao skill hub usage
agendao skill hub negative-entropy
agendao skill hub semantic-conflicts
agendao skill hub index
agendao skill hub distributions
agendao skill hub artifact-cache
agendao skill hub policy
agendao skill hub lifecycle
agendao skill hub review-candidates-sync --session-id <session>
agendao skill hub semantic-conflict-review-sync --session-id <session>
agendao skill hub vitality-set --session-id <session> --skill-name <name> --state review-candidate --summary <text>
agendao skill hub install-plan --source-id <id> --source-kind registry --locator <locator> --skill-name <name>
agendao skill hub install-apply --session-id <session> --source-id <id> --source-kind registry --locator <locator> --skill-name <name>
agendao skill hub update-apply --session-id <session> --source-id <id> --source-kind registry --locator <locator> --skill-name <name>
agendao skill hub detach --session-id <session> --source-id <id> --source-kind registry --locator <locator> --skill-name <name>
agendao skill hub remove --session-id <session> --source-id <id> --source-kind registry --locator <locator> --skill-name <name>
```

所有读写命令都通过 `agendao-server` 的 `/skill/hub/*` 路由进入 authority，不在 CLI 侧直接执行副作用。

## Memory 与 Skill 自进化文档入口

如果你要理解 AgenDao 如何把会话经验沉淀为可复用能力，优先看：

- `../README.md`
  - 产品层能力总览，说明 memory 与 skill 自进化的正式定位
- `../USER_GUIDE.md`
  - 用户视角的使用方式、观察入口与风险边界
- `skills.md`
  - skill reflection、`skill_manage` 回写与 methodology linkage
- `tools.md`
  - `/memory` 与 `skill_manage` 这些运行时入口
- `configuration.md`
  - shared / isolated workspace mode 对 memory scope 的影响

## Skill Hub Policy

第三卷 phase 7 的 artifact policy 通过唯一配置真相 `skills.hub` 提供，authority 会把当前生效值暴露到 `/skill/hub/policy`，CLI/TUI/Web 都应读取这一正式读面，而不是各端自己解析配置文件。

`agendao.jsonc` 示例：

```jsonc
{
  "skills": {
    "hub": {
      "artifactCacheRetentionSeconds": 604800,
      "fetchTimeoutMs": 30000,
      "maxDownloadBytes": 8388608,
      "maxExtractBytes": 8388608
    }
  }
}
```
