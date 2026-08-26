# AgenDao Docs

文档基线：`v2026.8.24`（更新日期：`2026-08-24`）

这里不是“今天改了什么”的补丁板，而是 AgenDao 文档系统的正式入口。后续读 `agendao/docs` 时，先用这份文档判断每类文档的用途，再进入具体主题。

## 先看阅读规则

`agendao/docs` 里的内容分两类：

1. 产品文档
   - 解释当前已经对用户、集成方开放的能力。
2. 示例与 schema
   - 提供合法样例、模板和教程。

如果你只想知道“现在什么是真的”，先读：

- `documentation-status.md`
- `index.md`
- `commands.md`
- `tools.md`
- `context-caching.md`
- `sandbox.md`

## 当前产品面

如果你是来判断“AgenDao 现在已经具备哪些正式能力”，优先抓下面几条：

- **统一 authority 运行时**
  - CLI、TUI、Web、Server 共用同一套 session、provider、tool、scheduler、skill、memory、telemetry authority。
- **sandbox 执行边界**
  - 模型可达执行收口到唯一 `SandboxExecutionBoundary`；permission 与 sandbox 分离，Linux `bwrap` 完整、macOS `seatbelt` 完整、Windows 模型层 + fail-closed。见 `sandbox.md`。
- **长回合上下文治理**
  - replay authority、prompt surface、context closure、cache diagnostics 和 compaction 边界都已进入正式实现，而不是停留在局部约定。
- **工具轨迹可解释**
  - tool repair、trajectory quality、tool-result governance、permission/steering/runtime state 都已有正式读面，能被三端消费。
- **方法沉淀与运行治理**
  - skill hub、memory validation/consolidation、统一 Blueprint、proposal/review/gate 已形成完整产品面。

## 当前文档入口

- `README.md`
  - docs 门户、阅读顺序、当前文档分类
- `documentation-status.md`
  - 各文档的用途与当前状态
- `installation.md`
  - 单一 `agendao` 分发入口的安装、升级、卸载，以及默认内嵌 Web 资源与可选外部覆盖说明
- `../CHANGELOG.md`
  - 发布记录，不替代产品总览文档
- `../USER_GUIDE.md`
  - 面向使用者的命令、scheduler、TUI 交互说明，以及 memory / skill 自进化使用心智
- `skills.md`
  - Skill lifecycle、skill reflection、`skill_manage` 写入与 memory linkage，以及 skill hub search / trust / stale index 发现链路
- `tools.md`
  - 工具层参考，包括 `skill_manage`、结构化工具和 memory 可观测面入口
- `configuration.md`
  - 当前 schema 的配置入口、Provider 写法、Permission 规则与 session 权限模式
- `task-ledger.md`
  - `/goal` 的自然语言入口，以及服务端 TaskLedger 的目标、Next、证据、中断和恢复语义
- `context-caching.md`
  - openai-compatible / anthropic-compatible 两类协议族下的上下文缓存策略、稳定提示面、replay continuity、输出投影与 cache diagnostic
- `sandbox.md`
  - sandbox 权威、permission 与 sandbox 的分离、各平台 backend 真实能力与 fail-closed、host-management path 的诚实边界
- `examples/scheduler/README.md`
  - 当前 inline `SchedulerBlueprint` 示例与请求入口
- `examples/context_docs/README.md`
  - `context_docs` schema、registry、index 示例
- `examples/plugins_example/README.md`
  - Skill / TS plugin / Rust 扩展示例

## Examples

- `examples/configuration/`
  - 当前 schema 对齐的最小 Provider、Permission、Ollama 与 context docs 配置
- `examples/context_docs/`
  - Formal examples for `context_docs`
  - Includes minimal `agendao.json` / `agendao.jsonc` config samples
  - Includes `context-docs-registry` schema and example
  - Includes `context-docs-index` schema and example docs index
- `examples/scheduler/`
  - 当前 `SchedulerChoice::Blueprint` 示例；不包含旧 profile 或转换输入
- `plugins_example/`
  - Skill / TS plugin / Rust extension examples



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
- `docs/examples/scheduler/blueprint.example.json`

## Tool Config Entry

The canonical external tool config example entry is:

- `docs/examples/tools/README.md`
- `docs/examples/tools/agendao.jsonc.example`
- `docs/examples/tools/single-file/`
- `docs/examples/tools/split-imports/`
- `docs/examples/tools/directory-infer/`
- `docs/examples/tools/partial-backfill/`
- `docs/examples/tools/catalog-only/`

The built-in scheduler templates are:

- `direct`
- `plan`
- `coordinate`
- `verify`
- `autoresearch`

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
