# ROCode

ROCode (RustingOpenCode) 是一个用 Rust 编写的高性能 AI 编码编排器。它将终端原生交互、多 Agent 协调、可扩展技能系统和多模型 Provider 整合为一个统一的开发工作流引擎。

> **版本:** 2026.4.26 · **许可证:** MIT · **作者:** Biocheming

---

## ROCode 能做什么

你用自然语言向 ROCode 描述任务。它规划、读写文件、运行命令、搜索代码库，并迭代执行 -- 所有步骤实时可见。

```bash
rocode run "add input validation to the signup form"
```

ROCode 读取你的代码库，跨多个文件实现变更，运行测试，并报告结果。

---

## 核心能力

### 编排内核

ROCode 由唯一的执行内核驱动所有 LLM 循环。调度器以 preset 形式提供不同的编排策略：

| Preset | 定位 | 默认阶段 |
|--------|------|---------|
| `sisyphus` | 委托优先、单循环执行 | request-analysis, route, execution-orchestration |
| `prometheus` | 规划优先、分步交付 | request-analysis, route, interview, plan, review, handoff |
| `atlas` | 协调/委派/验证 | request-analysis, execution-orchestration, synthesis |
| `hephaestus` | 自主深度执行 | request-analysis, execution-orchestration |

### 四个正交维度

同一任务可以组合多个维度，而非"四选一"：

- **Skill List** -- 能力选择：加载什么工具/技能
- **Agent Tree** -- 执行者组织：由谁执行（可嵌套、可引用外部文件）
- **Skill Graph** -- 流程控制：什么顺序和条件
- **Skill Tree** -- 知识继承：携带什么上下文（层级 Markdown 知识树）

### Skill Hub

远程 skill 分发、artifact 缓存和托管生命周期管理：

```bash
rocode skill hub status
rocode skill hub distributions
rocode skill hub install-plan --source-id <id> --source-kind registry --locator <loc> --skill-name <name>
```

所有读写命令经由 `rocode-server` 的 `/skill/hub/*` 路由进入 authority，不在 CLI 侧直接执行副作用。

### Memory 与 Skill 自进化

ROCode 当前已经把“会话经验 -> 可复用能力”的链路做成正式能力，而不是零散提示：

- 复杂回合会触发 skillworthy 检测，并在合适时给出 skill save suggestion，提醒把经验整理成具备 trigger、validation 与 boundary 的可复用 skill。
- 已有 skill 在运行后可以进入 skill reflection 视图，对照实际 tool call 检查是否需要 `patch`，避免 skill 内容和真实方法长期漂移。
- `skill_manage` 的创建、补丁、文件写入与 guard 结果会进入 memory observation，形成 lesson、pattern、methodology candidate 等后续材料。
- memory 检索只面向经过 validation / consolidation 的正式记录，并提供 retrieval preview 来解释注入原因，而不是把未经裁决的草稿直接塞回 prompt。
- TUI、Web 与 HTTP Server 都提供 memory 的 list、detail、validation、conflicts、rule hits、consolidation runs 等可观测面。

### 多 Provider 支持

通过 `models.dev` 获取完整模型目录，支持阿里云百炼、智谱 BigModel、Moonshot Kimi、DeepSeek、OpenRouter、Google、Azure、AWS Bedrock、Ollama 等 20+ Provider。参见 [认证](auth)。

### MCP 集成

Model Context Protocol 服务器管理 -- 本地（stdio）和远程（HTTP/SSE + OAuth）：

```bash
rocode mcp add my-server --command ./bin/my-server
rocode mcp list
rocode mcp connect my-server
```

### TUI 终端界面

基于 reratui reactive 渲染主线与 ratatui 兼容层的终端 UI，支持实时流式输出、语法高亮、diff 查看、权限对话、斜杠命令自动补全、会话浏览和更细粒度的消息渲染。

### Web 界面

内置 React 前端，通过 `rocode web` 启动；当前版本已补齐更高密度的消息阅读节奏、可过滤 model picker、批量 session 删除与更统一的 workspace / session / activity 视觉体系。

### HTTP Server

`rocode serve` 启动独立 API 服务，可被其他客户端（TUI、Web、自定义工具）通过 HTTP 连接。

### ACP Server

`rocode acp` 启动 Agent Client Protocol 服务器，用于 IDE 集成等场景。

### 插件系统

当前运行时会自动引导三类插件入口：`npm`、本地 `file` TypeScript 插件、以及 `dylib` 原生插件。配置模型里保留了 `pip` / `cargo` 字段，但它们目前不是自动加载路径。

### 上下文文档

`context_docs` 机制允许为特定库/框架注入精确的文档索引，通过 registry 和 index 文件管理。

---

## 快速开始

**1. 构建安装**

```bash
git clone <repo-url> && cd rocode
./scripts/install-local.sh release ~/.local
```

参见 [安装指南](installation) 了解完整安装方式。

**2. 设置 API 密钥**

```bash
export ZHIPUAI_API_KEY=zhipu-...
# 或
export ALIBABA_CN_API_KEY=dashscope-...
```

参见 [认证](auth) 了解所有 Provider 的配置方式。

**3. 启动 TUI 交互会话**

```bash
rocode
```

或发送单次任务后退出：

```bash
rocode run "explain the auth module"
```

---

## 运行模式对比

| 模式 | 命令 | 适用场景 |
|------|------|---------|
| TUI 交互 | `rocode` 或 `rocode tui` | 日常编码 |
| 单次执行 | `rocode run "task"` | 快速一次性任务 |
| JSON 输出 | `rocode run --format json "task"` | 脚本集成、CI |
| HTTP 服务 | `rocode serve` | 多客户端接入 |
| Web 界面 | `rocode web` | 浏览器使用 |
| ACP 服务 | `rocode acp` | IDE 集成 |
| 远程连接 | `rocode attach <url>` | 连接到已运行的 ROCode 实例 |

---

## 架构概览

ROCode 遵循严格的分层架构，每层有明确的职责边界：

```
  Adapters        展示、交互、流转发。可只读查询领域服务；副作用操作必须经由编排层。
  Orchestration   拓扑与调度。执行内核、事件归一化、工具调度抽象在此层。
  Session         会话状态、消息持久化、上下文管理。
  Domain Services 配置、权限、工具、Provider、插件 -- 各自领域的唯一权威。
  Infrastructure  IO 抽象（存储、LSP、PTY、格式化、VCS），无业务语义。
```

### 宪法原则

1. **唯一执行内核** -- 所有 LLM 循环由唯一内核驱动，适配层不得自建循环。
2. **唯一配置真相** -- 配置加载一次，变更通过唯一写入点。
3. **唯一权限裁决** -- 权限判定只在一个地方发生。
4. **唯一工具调度** -- 工具执行通过统一调度抽象。
5. **唯一状态所有权** -- 每个状态域有且仅有一个所有者。
6. **唯一插件契约** -- 插件通过单一协议与宿主通信。
7. **生命周期对称性** -- 注册即承诺注销，创建即承诺销毁。
8. **可观测性权利** -- 每个活跃执行体必须在权威注册表中可观测。
9. **副作用路径唯一** -- 产生副作用的操作必须经由编排层中转。

---

## CLI 命令索引

### 主命令

| 命令 | 说明 |
|------|------|
| `rocode` | 启动 TUI 交互会话（默认子命令） |
| `rocode tui` | 启动 TUI 会话（显式） |
| `rocode run "msg"` | 执行单次任务 |
| `rocode serve` | 启动 HTTP API 服务器 |
| `rocode web` | 启动服务器并打开 Web 界面 |
| `rocode acp` | 启动 ACP 服务器 |
| `rocode attach <url>` | 连接到已运行的远程实例 |
| `rocode models` | 列出可用模型 |
| `rocode config` | 显示当前配置 |
| `rocode version` | 显示版本号 |
| `rocode info` | 显示构建和环境信息 |

### 管理命令

| 命令 | 说明 |
|------|------|
| `rocode session list` | 列出会话 |
| `rocode session show <id>` | 查看会话详情 |
| `rocode session delete <id>` | 删除会话 |
| `rocode auth list` | 列出认证 Provider |
| `rocode auth login [provider]` | 登录 Provider |
| `rocode auth logout [provider]` | 登出 Provider |
| `rocode agent list` | 列出可用 Agent |
| `rocode agent create <name>` | 创建 Agent 定义 |
| `rocode skill hub status` | 查看 Skill Hub 状态 |
| `rocode mcp list` | 列出 MCP 服务器 |
| `rocode mcp add <name>` | 添加 MCP 服务器 |
| `rocode stats` | 显示 Token 使用统计 |
| `rocode export [session]` | 导出会话数据 |
| `rocode import <file>` | 导入会话数据 |
| `rocode upgrade` | 升级到最新版本 |
| `rocode uninstall` | 卸载 |

### 调试命令

| 命令 | 说明 |
|------|------|
| `rocode debug paths` | 显示重要本地路径 |
| `rocode debug config` | 显示解析后的配置 JSON |
| `rocode debug skill` | 列出所有可用技能 |
| `rocode debug docs validate` | 验证上下文文档 registry/index |
| `rocode debug agent <name>` | 显示 Agent 配置详情 |

### TUI 内斜杠命令

在 TUI 交互界面中输入 `/` 查看所有命令。常用命令：

| 命令 | 说明 |
|------|------|
| `/help` | 显示帮助 |
| `/abort` | 取消当前活动执行边界 |
| `/new` | 开始新会话 |
| `/models` | 列出可用模型 |
| `/model <id>` | 切换模型 |
| `/agents` | 列出可用 Agent |
| `/agent <name>` | 切换 Agent |
| `/presets` | 列出调度器预设 |
| `/preset <name>` | 切换调度器预设 |
| `/compact` | 压缩对话历史 |
| `/status` | 显示会话状态 |
| `/copy` | 复制最近一条助手回复 |

`/abort` 通过独立控制请求命中 server 的取消路由，不会作为普通用户消息插入当前 prompt。若目标是某个已登记的 agent task，应使用 `/tasks kill <ID>` 或 `task_flow cancel`。

---

## 文档索引

- [安装指南](installation) -- 系统要求、构建安装、环境配置
- [认证](auth) -- API 密钥、OAuth、Provider 注册表、模型目录
- [配置参考](configuration) -- `rocode.jsonc` 完整配置参考
- [Scheduler 指南](examples/scheduler/SCHEDULER_GUIDE) -- Scheduler 完整使用教程
- [Scheduler 示例](examples/scheduler/README) -- Preset 配置示例和 stage 覆盖
- [上下文文档](examples/context_docs/README) -- `context_docs` schema 和示例
- [插件示例](examples/plugins_example/README) -- Skill / 插件扩展示例
