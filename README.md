<p>
  <img src="icons/rocode_logo.png" alt="ROCode" width="280" />
</p>

# RustingOpenCode (ROCode)

RustingOpenCode（简称 `ROCode`）是一个面向本地仓库工作的 Rust 编码代理系统。它提供统一的 CLI、TUI、HTTP Server 和 Web 界面，并把 session、scheduler、tool、provider、skill、runtime telemetry 这些能力收敛到同一套 authority 驱动的运行模型中。

## 当前版本

- 软件名：`RustingOpenCode` / `ROCode`
- 版本：`v2026.4.21`
- 可执行命令：`rocode`
- 当前公开 scheduler presets：`sisyphus` / `prometheus` / `atlas` / `hephaestus`

## 它现在能做什么

- 在本地仓库里运行编码代理，支持交互式 TUI、单次 `run`、HTTP Server、Web UI、ACP
- 维护会话树、会话分叉、会话导入导出，以及统一的 session telemetry / usage / events 读模型
- 统一管理模型目录、provider 连接、认证状态，以及 provider catalog 刷新
- 统一管理 workspace skill、remote skill hub、distribution / artifact cache / lifecycle / guard / timeline
- 把复杂回合中的可复用方法沉淀为 skill 候选，并把 skill 写入、反思、patch 提示接回运行时闭环
- 提供受 workspace 边界约束的 memory 系统，覆盖 evidence、validation、retrieval preview、conflict、consolidation 与 lesson/pattern/methodology promotion
- 接入 MCP、LSP、插件与 scheduler profile，并把它们暴露到同一套 runtime 中
- 以 workspace authority 为中心处理配置解析、sandbox `.rocode`、global config 与 shared / isolated workspace 模式

## 运行界面

- `rocode tui`
  - 默认终端界面，适合日常交互开发
- `rocode run`
  - 非交互单次执行，适合集成脚本与 CI
- `rocode serve`
  - 启动 HTTP Server
- `rocode web`
  - 启动 headless server 并打开 Web
- `rocode attach`
  - 连接到已运行的 server
- `rocode acp`
  - 启动 Agent Client Protocol server

## 快速开始

### 环境要求

- Rust stable
- Cargo
- Git

### 构建

```bash
cargo build -p rocode-cli
```

### 查看帮助

```bash
cargo run -p rocode-cli -- --help
```

### 启动方式

默认进入 TUI：

```bash
cargo run -p rocode-cli --
```

显式指定 TUI：

```bash
cargo run -p rocode-cli -- tui
```

单次运行：

```bash
cargo run -p rocode-cli -- run "请审查当前仓库里最危险的改动"
```

启动 HTTP Server：

```bash
cargo run -p rocode-cli -- serve --hostname 127.0.0.1 --port 3000
```

启动 Web：

```bash
cargo run -p rocode-cli -- web --hostname 127.0.0.1 --port 3000
```

显式指定 workspace 打开 Web：

```bash
cargo run -p rocode-cli -- web --dir /path/to/workspace
```

图标资产位于 `icons/`。当前已接入 Web favicon，并在 `windows-msvc` 目标编译时尝试把 `icons/rocode.ico` 嵌入 `rocode.exe`；Linux 桌面入口模板见 `packaging/linux/rocode.desktop`；macOS 图标与 `.app` bundle 链路见 `icons/rocode.icns`、`packaging/macos/ROCode.iconset` 和 `scripts/build_macos_app_bundle.sh`。

构建 macOS `.app` bundle：

```bash
./scripts/build_macos_app_bundle.sh release
```

## 当前 CLI 入口

当前顶层命令分组以 `crates/rocode-cli/src/cli.rs` 为准，主要包括：

- `tui`
- `attach`
- `run`
- `serve`
- `web`
- `acp`
- `models`
- `session`
- `skill`
- `stats`
- `db`
- `config`
- `auth`
- `agent`
- `debug`
- `mcp`
- `export`
- `import`
- `github`
- `pr`
- `upgrade`
- `uninstall`
- `generate`
- `version`
- `info`

最常用的帮助入口：

```bash
rocode tui --help
rocode run --help
rocode models --help
rocode session --help
rocode skill hub --help
rocode debug --help
```

## Workspace 与配置模型

ROCode 当前已经不是“只读一份全局配置”的工具。运行时会同时考虑 workspace authority、sandbox `.rocode`、global config 和缓存状态，但优先级是明确的：

- 当前工作区内的 `.rocode/` 是 workspace runtime 的正式本地 authority
- `rocode.jsonc` / `rocode.json` 与 `.rocode/rocode.jsonc` / `.json` 是项目侧配置入口
- `~/.config/rocode/rocode.jsonc` 是全局配置入口
- shared / isolated workspace mode 会影响当前 runtime 是否继承 global config

如果当前 workspace 处于 isolated 模式，global config 的修改不会自动变成当前 sandbox runtime。

## 模型与 Provider

模型目录与 provider catalog 已经支持显式刷新：

```bash
rocode models
rocode models --refresh
rocode models zhipu --refresh --verbose
```

常用认证命令：

```bash
rocode auth list
rocode auth login --help
rocode auth logout --help
```

## Skill Hub

当前 `skill hub` 已经是正式的一组 CLI / Server / TUI / Web 能力，不再是零散调试命令。它覆盖：

- managed skill provenance
- source index
- distribution records
- artifact cache
- artifact policy
- lifecycle records
- install / update / detach / remove
- sync plan / sync apply

常用入口：

```bash
rocode skill hub status
rocode skill hub managed
rocode skill hub index
rocode skill hub distributions
rocode skill hub artifact-cache
rocode skill hub policy
rocode skill hub lifecycle
```

写操作示例：

```bash
rocode skill hub install-plan --source-id <id> --source-kind registry --locator <locator> --skill-name <name>
rocode skill hub install-apply --session-id <session> --source-id <id> --source-kind registry --locator <locator> --skill-name <name>
rocode skill hub update-apply --session-id <session> --source-id <id> --source-kind registry --locator <locator> --skill-name <name>
rocode skill hub detach --session-id <session> --source-id <id> --source-kind registry --locator <locator> --skill-name <name>
rocode skill hub remove --session-id <session> --source-id <id> --source-kind registry --locator <locator> --skill-name <name>
```

## Memory 与 Skill 自进化

ROCode 当前已经不只是“能加载 skill”。运行时会把复杂回合中的经验提炼成可审视、可验证、可复用的结构化能力：

- 当一个回合表现出明显的“编辑 -> 验证”“错误恢复”“多工具协同”特征时，运行时可以追加 `skill_save_suggestion` 提示，提醒把它整理成可复用 skill，而不是把一次性过程直接固化。
- 已加载 skill 在会话中使用后，运行时会附带 skill reflection 语境，提示检查“实际执行步骤”是否已经偏离现有 skill，并在必要时通过 `skill_manage("patch", ...)` 修补。
- `skill_manage` 的写入结果会被纳入 memory observation。新建、修补、失败 guard、supporting file 变化等信号都会进入后续的记忆验证与归纳链。

Memory 不是无边界的历史堆积，而是受 workspace authority 约束的正式系统：

- 记录包含 `evidence_refs`、`trigger_conditions`、`boundaries`、`normalized_facts`，不是只存一句摘要。
- 记录先进入 candidate，再经过 validation、conflict 检查与状态裁决，只有 `validated` / `consolidated` 记录才会进入稳定检索面。
- 检索分为 frozen snapshot 和 turn-scoped retrieval preview。前者提供稳定背景，后者解释“为什么这条记忆会注入当前回合”。
- consolidation 会合并相近记录，并把重复 lesson 提升为 pattern，再把结构化 pattern 提升为 methodology candidate，从而把零散经验收束成更强的可复用方法。

## TUI / Web 当前约定

- TUI 是当前最完整的交互前端
- Web 首页 `/` 是唯一正式入口
- Web 左侧展示当前 workspace 范围内的 session tree
- Web settings 已暴露 workspace mode / workspace root / skill hub policy / governance timeline 等信息
- TUI 与 Web 都直接读取统一的 session / skill / telemetry 读模型，而不是各端自己推断状态
- TUI 当前已完成 reratui 迁移主线，进入以 hybrid app shell 为边界的稳定阶段
- Web 当前已切到更高密度的消息阅读节奏、可检索 model picker、批量 session 删除与更轻的 sidebar / composer 语法
- TUI 与 Web 都已具备 memory 可观测面，包括检索预览、validation report、conflicts、rule packs、rule hits 与 consolidation runs
- 无参数且非终端环境启动时，`rocode` 会默认走桌面 Web 启动路径，并优先解析/确认 workspace，再打开浏览器

## Release Notes

- 当前版本发布说明见 [CHANGELOG.md](/home/biocheming/tests/python/rust/rocode/CHANGELOG.md)

## 运行时观测

当前系统已经把 runtime telemetry 做成正式读模型。你可以通过 server / CLI / TUI / Web 查看：

- session telemetry
- stage summaries
- usage
- paginated events
- provenance timeline

调试入口主要在：

```bash
rocode debug --help
rocode debug skills --help
rocode debug docs --help
rocode stats --help
```

## MCP / LSP / 插件

MCP 常用入口：

```bash
rocode mcp list
rocode mcp add --help
rocode mcp connect <NAME>
rocode mcp disconnect <NAME>
rocode mcp auth list
```

Agent 与调试入口：

```bash
rocode agent list
rocode agent create --help
rocode debug agent <NAME>
```

## 仓库结构

- `crates/rocode-cli`
  - CLI 入口与命令编排
- `crates/rocode-tui`
  - 终端前端与交互状态机
- `crates/rocode-server`
  - HTTP / SSE / Web 前端与路由
- `crates/rocode-session`
  - session 领域模型与持久化
- `crates/rocode-agent`
  - agent 执行与封装
- `crates/rocode-orchestrator`
  - scheduler / orchestration authority
- `crates/rocode-tool`
  - 工具注册与 tool-facing adapter
- `crates/rocode-skill`
  - skill authority、hub、distribution、artifact、guard、lifecycle
- `crates/rocode-provider`
  - provider / model protocol 适配
- `crates/rocode-config`
  - 配置发现、解析、合并
- `crates/rocode-types`
  - 跨端共享读写模型

## 开发验证

常用：

```bash
cargo fmt --all
cargo check
```

前端 / 服务侧常用：

```bash
cargo check -p rocode-cli -p rocode-server -p rocode-tui
```

## 文档入口

- 用户使用指南：[USER_GUIDE.md](/home/biocheming/tests/python/rust/rocode/USER_GUIDE.md)
- 文档索引：[docs/README.md](/home/biocheming/tests/python/rust/rocode/docs/README.md)
- Scheduler 示例：[docs/examples/scheduler/README.md](/home/biocheming/tests/python/rust/rocode/docs/examples/scheduler/README.md)
- Context Docs：[docs/examples/context_docs/README.md](/home/biocheming/tests/python/rust/rocode/docs/examples/context_docs/README.md)
- 插件 / skill 示例：[docs/plugins_example/README.md](/home/biocheming/tests/python/rust/rocode/docs/plugins_example/README.md)
