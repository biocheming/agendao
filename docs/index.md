# AgenDao

AgenDao（RockyCode）是一个用 Rust 编写的本地编码智能体运行时。它把终端原生交互、多 Agent 协调、技能系统、多 Provider、持久化 session、memory 和 telemetry 组织成一个统一的开发工作流引擎。

> **版本:** 2026.5.17 · **许可证:** MIT · **作者:** Biocheming

---

## 当前产品面

如果你不是来看历史，而是想知道“这套系统当前的正式能力边界”，先抓四条：

- **统一 authority**
  - `agendao` 是产品壳，`agendao-cli` / `agendao-tui` / `agendao-web` 是前端层，`agendao-server` 是后端 authority；副作用和读模型都应回到唯一 owner。
- **长回合稳定性**
  - replay authority、tool repair、tool-result governance、permission/steering、prompt surface、context closure 和 cache diagnostics 已经进入正式运行面。
- **连续性与沉淀**
  - scheduler continuity、memory validation/consolidation、skill hub lifecycle、proposal/review/gate 共同处理“怎样把一次工作持续做下去”。
- **三端共享读面**
  - CLI、TUI、Web 读取同一套 telemetry、runtime state、effective policy、provider descriptor、session/activity 读模型，而不是各自拼装解释。

更细的文档状态判断，见 [documentation-status.md](documentation-status.md)。

---

## 先看三条长期能力

如果你第一次了解 AgenDao，最值得先看的是这三条线。它们不热闹，但决定了系统能不能在长周期工作里保持稳定。

### 记忆系统

AgenDao 的 memory 不是把历史都收进去。它更强调：

- candidate 先进入系统，再经过 validation / conflict / consolidation
- 检索前先给 retrieval preview，解释为什么注入
- 正式 memory record 和临时会话材料分开，不让草稿直接污染长期上下文

所以这套 memory 更像“裁决什么值得留下”，而不是“尽量多存”。

### Skill 系统

AgenDao 的 skill 也不只是安装和调用：

- skill hub 维护 usage ledger、negative entropy、semantic conflict、composition relationship
- inspection 可见和 runtime 可用分开处理，运行时还有独立 runtime gate
- proposal inbox 让新方法先进入审阅，而不是直接覆盖现有 skill

所以它更像一个会治理的方法目录，而不是一个不断变大的仓库。

### 上下文缓存优化

AgenDao 对缓存命中率的优化，不是简单多塞几个 cache 字段。它更关心：

- 稳定前缀和动态尾部怎么分开
- compact、handoff、output projection 会不会破坏 prefix
- 命中下降时，系统能不能解释到底是哪一层变了

这也是为什么 AgenDao 会长期维护 prompt surface、context closure、cache evidence 和三本 token 账。

---

## AgenDao 能做什么

你用自然语言向 AgenDao 描述任务。它规划、读写文件、运行命令、搜索代码库，并迭代执行 -- 所有步骤实时可见。

```bash
agendao run "add input validation to the signup form"
```

AgenDao 读取你的代码库，跨多个文件实现变更，运行测试，并报告结果。

---

## 核心能力

### 编排内核

AgenDao 由唯一的执行内核驱动所有 LLM 循环。调度器以 preset 形式提供不同的编排策略：

| Preset | 定位 | 默认阶段 |
|--------|------|---------|
| `sisyphus` | 委托优先、单循环执行 | request-analysis, execution-orchestration |
| `prometheus` | 规划优先、分步交付 | request-analysis, interview, plan, review, handoff |
| `atlas` | 协调/委派/验证 | request-analysis, execution-orchestration, synthesis |
| `hephaestus` | 自主深度执行 | request-analysis, execution-orchestration |
| `verifier` | 多候选比较选优 | request-analysis, execution-orchestration |

Scheduler continuity 是调度器的一等输入，而不是简单把历史消息拼回 prompt。当前运行时会为每个 scheduler turn 构建 `Session Continuity Context`：

- `Context Coverage` 说明 exact recent tail、omitted turns、memory anchors 和 recall policy
- `Source Anchors` 授权 `scheduler_context_hydrate` 回查同会话消息与 compaction summary
- `Memory Anchors` 授权 `scheduler_memory_hydrate` 回查持久化 memory detail
- 每次 hydration 都会写入 stage metadata，记录 hydrated / rejected / missing ids，供前端和 telemetry 审计

### 上下文缓存优化

AgenDao 将缓存优化收敛到稳定提示面，而不是按厂商分叉 prompt 结构。closeai-compatible 协议族侧重稳定 prefix 与可选 prompt cache affinity；Ethnopic-compatible 协议族侧重统一 cache breakpoint 规划。scheduler continuity、artifact summary、memory anchors 与当前动态输入会被分区放置，避免本轮 tail 正文或临时 tool output 破坏可复用前缀。

CLI、TUI 和 Web 会显示 cache read/write、hit/miss 以及 cache evidence / context closure diagnostics。详见 [上下文缓存优化](context-caching)。

如果你要看可直接拿来用的 scheduler 示例，当前目录已经按类型拆开：

- `examples/scheduler/presets/`：公开内置 preset 示例
- `examples/scheduler/verifier/`：verifier 配置与外置 workflow
- `examples/scheduler/pso/`：PSO 自定义 topology
- `examples/scheduler/autoresearch/`：workflow 级 autoresearch 示例

### 四个正交维度

同一任务可以组合多个维度，而非"四选一"：

- **Skill List** -- 能力选择：加载什么工具/技能
- **Agent Tree** -- 执行者组织：由谁执行（可嵌套、可引用外部文件）
- **Skill Graph** -- 流程控制：什么顺序和条件
- **Skill Tree** -- 知识继承：携带什么上下文（层级 Markdown 知识树）

### Skill Hub

远程 skill 分发、artifact 缓存和托管生命周期管理：

```bash
agendao skill hub status
agendao skill hub usage
agendao skill hub negative-entropy
agendao skill hub distributions
agendao skill hub install-plan --source-id <id> --source-kind registry --locator <loc> --skill-name <name>
```

所有读写命令经由 `agendao-server` 的 `/skill/hub/*` 路由进入 authority，不在 CLI 侧直接执行副作用。

这一层现在不只做“装 skill”。它还维护 usage ledger、negative entropy、semantic conflict、composition relationship、proposal inbox 和 runtime gate。

现在它还多了一层正式 discoverability：`skill hub search` 可以按 indexed source 搜索在线 skill，并返回 source、entry、trust、maintenance、staleness 与 refresh 建议，方便 agent 直接串联 search → install。

### Skill 运行与治理

AgenDao 不把 skill 看成静态目录。它更关心一个 skill 在真实运行里是不是还有效、是不是和别的 skill 重叠、是不是已经该退出主要运行面：

- 复杂回合会触发 skillworthy 检测，并在合适时给出 skill save suggestion，提醒把经验整理成具备 trigger、validation 与 boundary 的可复用 skill。
- 已有 skill 在运行后可以进入 skill reflection 视图，对照实际 tool call 检查是否需要 `patch`，避免 skill 内容和真实方法长期漂移。
- `skill_manage` 的创建、补丁、文件写入与 guard 结果会进入 memory observation，形成 lesson、pattern、methodology candidate 等后续材料。

### Memory 系统

AgenDao 的 memory 更接近经过筛选的长期材料，而不是临时对话的堆积：

- memory 检索只面向经过 validation / consolidation 的正式记录，并提供 retrieval preview 来解释注入原因，而不是把未经裁决的草稿直接塞回 prompt。
- TUI、Web 与 HTTP Server 都提供 memory 的 list、detail、validation、conflicts、rule hits、consolidation runs 等可观测面。

### 多 Provider 支持

通过 `models.dev` 获取完整模型目录，支持阿里云百炼、智谱 BigModel、Moonshot Kimi、DeepSeek、OpenRouter、Google、AWS Bedrock、Ollama 等多种 Provider 与认证插件。参见 [认证](auth)。

provider 的当前生效配置不再需要前端各自猜测。AgenDao 现在提供只读 provider descriptor 与 config validation 解释面，用来说明 typed provider profile、transport、认证与模型覆盖到底如何落地。

### MCP 集成

Model Context Protocol 服务器管理 -- 本地（stdio）和远程（HTTP/SSE + OAuth）：

```bash
agendao mcp add my-server --command ./bin/my-server
agendao mcp list
agendao mcp connect my-server
```

### TUI 终端界面

基于 reratui reactive 渲染主线与 ratatui 兼容层的终端 UI，支持实时流式输出、语法高亮、diff 查看、权限对话、斜杠命令自动补全、会话浏览和更细粒度的消息渲染。
在 provider 输出结束后，TUI 现在会通过 authoritative `session.updated` 重新同步 session，确保最后一条 assistant message 能从流式半成品回到最终持久化版本。

### Web 界面

内置 React 前端，通过 `agendao web` 启动；当前版本已补齐更高密度的消息阅读节奏、可过滤 model picker、批量 session 删除与更统一的 workspace / session / activity 视觉体系。

Web 现在也直接消费 context closure、effective policy、provider descriptor、external adapter provisioning 和 skill proposal 这些正式读面，而不是再从大响应里侧取零散字段。
Web 的最终 assistant message 同步链路也已收口：persisted history 与 live streaming block 现在会按统一的 message / reasoning block id 合并，避免 stale live snapshot 覆盖已落库的最终文本。

### HTTP Server

`agendao serve` 启动独立 API 服务，可被其他客户端（TUI、Web、自定义工具）通过 HTTP 连接。

### ACP Server

`agendao acp` 启动 Agent Client Protocol 服务器，用于 IDE 集成等场景。

### 插件系统

当前运行时会自动引导三类插件入口：`npm`、本地 `file` TypeScript 插件、以及 `dylib` 原生插件。配置模型里保留了 `pip` / `cargo` 字段，但它们目前不是自动加载路径。

### 上下文文档

`context_docs` 机制允许为特定库/框架注入精确的文档索引，通过 registry 和 index 文件管理。

---

## 快速开始

**1. 构建安装**

```bash
git clone <repo-url> && cd agendao
npm --prefix apps/agendao-web install
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
agendao
```

或发送单次任务后退出：

```bash
agendao run "explain the auth module"
```

---

## 运行模式对比

| 模式 | 命令 | 适用场景 |
|------|------|---------|
| TUI 交互 | `agendao` 或 `agendao tui` | 日常编码 |
| 单次执行 | `agendao run "task"` | 快速一次性任务 |
| JSON 输出 | `agendao run --format json "task"` | 脚本集成、CI |
| HTTP 服务 | `agendao serve` | 多客户端接入 |
| Web 界面 | `agendao web` | 浏览器使用 |
| ACP 服务 | `agendao acp` | IDE 集成 |
| 远程连接 | `agendao attach <url>` | 连接到已运行的 AgenDao 实例 |

---

## 架构概览

AgenDao 遵循严格的分层架构，每层有明确的职责边界：

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
| `agendao` | 启动 TUI 交互会话（默认子命令） |
| `agendao tui` | 启动 TUI 会话（显式） |
| `agendao run "msg"` | 执行单次任务 |
| `agendao serve` | 启动 HTTP API 服务器 |
| `agendao web` | 启动服务器并打开 Web 界面 |
| `agendao acp` | 启动 ACP 服务器 |
| `agendao attach <url>` | 连接到已运行的远程实例 |
| `agendao models` | 列出可用模型 |
| `agendao config` | 显示当前配置 |
| `agendao version` | 显示版本号 |
| `agendao info` | 显示构建和环境信息 |

### 管理命令

| 命令 | 说明 |
|------|------|
| `agendao session list` | 列出会话 |
| `agendao session show <id>` | 查看会话详情 |
| `agendao session delete <id>` | 删除会话 |
| `agendao auth list` | 列出认证 Provider |
| `agendao auth login [provider]` | 登录 Provider |
| `agendao auth logout [provider]` | 登出 Provider |
| `agendao agent list` | 列出可用 Agent |
| `agendao agent create <name>` | 创建 Agent 定义 |
| `agendao skill hub status` | 查看 Skill Hub 状态 |
| `agendao mcp list` | 列出 MCP 服务器 |
| `agendao mcp add <name>` | 添加 MCP 服务器 |
| `agendao stats` | 显示 Token 使用统计 |
| `agendao export [session]` | 导出会话数据 |
| `agendao import <file>` | 导入会话数据 |
| `agendao upgrade` | 升级到最新版本 |
| `agendao uninstall` | 卸载 |

### 调试命令

| 命令 | 说明 |
|------|------|
| `agendao debug paths` | 显示重要本地路径 |
| `agendao debug config` | 显示解析后的配置 JSON |
| `agendao debug skill` | 列出所有可用技能 |
| `agendao debug docs validate` | 验证上下文文档 registry/index |
| `agendao debug agent <name>` | 显示 Agent 配置详情 |

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
- [配置参考](configuration) -- `agendao.jsonc` 完整配置参考
- [Scheduler 指南](examples/scheduler/SCHEDULER_GUIDE) -- Scheduler 完整使用教程
- [Scheduler 示例](examples/scheduler/README) -- 按 presets / verifier / pso / autoresearch 分组的调度示例入口
- [上下文文档](examples/context_docs/README) -- `context_docs` schema 和示例
- [插件示例](examples/plugins_example/README) -- Skill / 插件扩展示例
