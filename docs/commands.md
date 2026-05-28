# ROCode CLI 命令参考

本文档是 ROCode 所有 CLI 子命令和交互式斜杠命令的完整参考。命令行通过 `rocode <subcommand>` 调用；交互式命令在 REPL 提示符下输入 `/command` 触发。

---

## 目录

1. [命令系统概览](#命令系统概览)
2. [rocode tui -- 交互式 TUI 会话](#rocode-tui----交互式-tui-会话)
3. [rocode run -- 非交互式执行](#rocode-run----非交互式执行)
4. [rocode attach -- 附加到远程服务器](#rocode-attach----附加到远程服务器)
5. [rocode serve -- HTTP 服务器](#rocode-serve----http-服务器)
6. [rocode web -- Web 界面](#rocode-web----web-界面)
7. [rocode acp -- ACP 服务器](#rocode-acp----acp-服务器)
8. [rocode models -- 模型列表](#rocode-models----模型列表)
9. [rocode session -- 会话管理](#rocode-session----会话管理)
10. [rocode memory -- Memory 权威工件](#rocode-memory----memory-权威工件)
11. [rocode provider -- Provider 权威工件](#rocode-provider----provider-权威工件)
12. [rocode skill -- 技能目录管理](#rocode-skill----技能目录管理)
13. [rocode stats -- 用量统计](#rocode-stats----用量统计)
14. [rocode db -- 数据库工具](#rocode-db----数据库工具)
15. [rocode config -- 配置与 validation](#rocode-config----配置与-validation)
16. [rocode auth -- 凭证管理](#rocode-auth----凭证管理)
17. [rocode agent -- 智能体管理](#rocode-agent----智能体管理)
18. [rocode debug -- 调试工具](#rocode-debug----调试工具)
19. [rocode mcp -- MCP 服务器管理](#rocode-mcp----mcp-服务器管理)
20. [rocode export / import -- 会话导入导出](#rocode-export--import----会话导入导出)
21. [rocode github -- GitHub 智能体](#rocode-github----github-智能体)
22. [rocode pr -- PR 检出](#rocode-pr----pr-检出)
23. [rocode upgrade -- 升级](#rocode-upgrade----升级)
24. [rocode uninstall -- 卸载](#rocode-uninstall----卸载)
25. [rocode generate -- OpenAPI 生成](#rocode-generate----openapi-生成)
26. [rocode version / info -- 版本信息](#rocode-version--info----版本信息)
27. [交互式斜杠命令](#交互式斜杠命令)

---

## 命令系统概览

ROCode 的命令分两层：

- **CLI 子命令**：通过 `rocode <subcommand>` 调用，例如 `rocode tui`、`rocode run`。
- **交互式斜杠命令**：在 TUI 或 CLI REPL 内输入 `/command` 触发。

全局入口：

```
rocode [subcommand] [options]
```

不带子命令时，默认进入 `tui` 模式。

默认传输策略：

- `rocode tui` 默认 Direct（in-process）
- `rocode run` / `rocode cli` 默认 Direct（in-process）
- `--socket` 显式覆盖为 Unix socket
- `--attach-url` / `--attach` 显式覆盖为 HTTP
- `rocode web` 保持 HTTP-first

---

## rocode tui -- 交互式 TUI 会话

启动交互式终端用户界面 (TUI) 会话。这是 ROCode 的主要使用模式。

### 用法

```
rocode tui [PROJECT] [选项]
```

### 参数

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `PROJECT` | 路径 | 当前目录 | 项目目录 |
| `-m, --model` | string | 配置默认 | 指定模型（格式: `provider/model`） |
| `-c, --continue` | flag | false | 恢复上次会话 |
| `-s, --session` | string | -- | 指定会话 ID |
| `--fork` | flag | false | 从已有会话分叉后再进入 TUI（需要 `-c` 或 `-s`） |
| `--prompt` | string | -- | 初始提示词 |
| `--agent` | string | -- | 指定智能体名称 |
| `--attach-url` | string | -- | 显式改走 HTTP 并附加到给定服务地址 |
| `--socket` | flag | false | 显式改走标准本地 Unix socket |
| `--port` | u16 | 0 | HTTP 服务端口（0 = 自动） |
| `--hostname` | string | 127.0.0.1 | 绑定地址 |
| `--mdns` | flag | false | 启用 mDNS 服务发现 |
| `--mdns-domain` | string | rocode.local | mDNS 域名 |
| `--cors` | string[] | [] | CORS 允许源列表 |
| `--local` | flag | false | 强制 Direct；当前只是显式声明默认行为 |

### 示例

```bash
# 在当前目录启动 TUI
rocode tui

# 显式改走 Unix socket
rocode tui --socket

# 显式改走 HTTP
rocode tui --attach-url http://127.0.0.1:3000

# 指定模型和项目
rocode tui ./my-project -m zhipuai/glm-5.1

# 恢复上次会话
rocode tui -c

# 分叉一个已有会话
rocode tui -s abc123 --fork
```

---

## rocode run -- 非交互式执行

向 ROCode 发送单条消息或命令，以非交互方式运行。不传消息时进入交互式 CLI REPL。

### 用法

```
rocode run [MESSAGE...] [选项]
rocode run --command <command> [选项]
```

### 参数

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `MESSAGE` | string[] | -- | 消息文本（可含空格） |
| `--command` | string | -- | 执行斜杠命令 |
| `-c, --continue` | flag | false | 恢复上次会话 |
| `-s, --session` | string | -- | 指定会话 ID |
| `--fork` | flag | false | 从已有会话分叉 |
| `--share` | flag | false | 共享会话 |
| `-m, --model` | string | -- | 指定模型 |
| `--agent` | string | -- | 指定智能体（与 `--scheduler-profile` 互斥） |
| `--scheduler-profile` | string | -- | 指定调度器配置（与 `--agent` 互斥） |
| `-f, --file` | path[] | [] | 附加文件 |
| `--format` | enum | default | 输出格式: `default` 或 `json` |
| `--title` | string | -- | 会话标题 |
| `--attach` | string | -- | 显式改走 HTTP 并附加到指定 URL |
| `--socket` | flag | false | 显式改走标准本地 Unix socket |
| `--dir` | path | -- | 工作目录 |
| `--port` | u16 | -- | 自动拉起本地服务器时使用的端口 |
| `--variant` | string | -- | 模型变体 |
| `--thinking` | flag | false | 显示思考过程 |
| `--interactive-mode` | enum | rich | CLI 交互模式: `rich` 或 `compact` |
| `--local` | flag | false | 强制 Direct；当前只是显式声明默认行为 |

### 示例

```bash
# 发送单条消息
rocode run "解释这段代码的作用"

# 显式走 Unix socket
rocode run --socket "继续当前任务"

# 显式走 HTTP
rocode run --attach http://127.0.0.1:3000 "继续当前任务"

# 使用特定模型
rocode run -m alibaba-cn/qwen3.6-plus "写一个排序算法"

# 恢复上次会话并继续
rocode run -c "继续上次的任务"

# 以 JSON 格式输出
rocode run --format json "列出 TODO"

# 执行斜杠命令
rocode run --command /status
```

---

## rocode attach -- 附加到远程服务器

将 TUI 客户端附加到一个正在运行的 ROCode 服务。默认按给定 URL 走 HTTP；如果额外提供 `--socket`，则显式要求走标准本地 Unix socket，并仅把 URL 作为同一服务的基准地址。

### 用法

```
rocode attach <URL> [选项]
```

### 参数

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `URL` | string | 必填 | 服务器 URL |
| `--dir` | path | -- | 工作目录 |
| `-s, --session` | string | -- | 会话 ID |
| `-p, --password` | string | -- | 连接密码 |
| `--socket` | flag | false | 显式要求改走标准本地 Unix socket |

### 示例

```bash
rocode attach http://192.168.1.100:3000
rocode attach http://localhost:3000 -s abc123
```

---

## rocode serve -- HTTP 服务器

启动后台 HTTP 服务器，接收 API 请求处理会话。

### 用法

```
rocode serve [选项]
```

### 参数

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `--port` | u16 | 0 | 端口（0 = 自动） |
| `--hostname` | string | 127.0.0.1 | 绑定地址 |
| `--mdns` | flag | false | 启用 mDNS |
| `--mdns-domain` | string | rocode.local | mDNS 域名 |
| `--cors` | string[] | [] | CORS 允许源 |
| `--socket` | flag | false | 同时监听标准本地 Unix socket |

---

## rocode web -- Web 界面

启动后台服务器并打开 Web 浏览器界面。

当前默认行为是：

- `rocode web` / `rocode serve` 优先使用编译进 `rocode` 二进制的内嵌 Web 资源
- 如果显式设置 `ROCODE_WEB_DIST`，server 会改为使用该外部 `dist/` 目录作为运行时覆盖
- 已安装布局中的 `share/rocode/web`、macOS bundle 中的 `Contents/Resources/web` 仍可作为兼容性外部资源来源

从源码构建时，`rocode-server` 的 `build.rs` 会自动检查 `apps/rocode-web/dist` 是否缺失或过期；只有 Web 源码变更时才会增量触发一次 `npm run build`。

开发态也支持独立 Web dev server：

- 设置 `ROCODE_WEB_DEV_URL=http://127.0.0.1:5173` 后，`rocode web` 会只拉起后端，并把浏览器打开到该 dev server。
- launcher 会自动把后端地址追加为 `?api_base_url=http://127.0.0.1:3000`，前端的 HTTP / SSE / WebSocket / 文件下载请求都会改为走这个显式后端地址。
- 对于本机 `localhost` / `127.0.0.1` 开发地址通常不需要额外 CORS 配置；若使用其他 origin，launcher 会把该 origin 一并加入后端白名单。

### 用法

```
rocode web [选项]
```

参数与 `rocode serve` 相同。

---

## rocode acp -- ACP 服务器

启动 Agent Client Protocol (ACP) 服务器，用于外部客户端集成。

### 用法

```
rocode acp [选项]
```

### 参数

除 `serve` 通用参数外：

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `--cwd` | path | `.` | 工作目录 |

---

## rocode models -- 模型列表

列出所有可用的 AI 模型。

### 用法

```
rocode models [PROVIDER] [选项]
```

### 参数

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `PROVIDER` | string | -- | 按提供商筛选 |
| `--refresh` | flag | false | 强制刷新 `models.dev` 目录并更新本地缓存 |
| `--verbose` | flag | false | 显示详细信息 |

### 示例

```bash
rocode models
rocode models zhipuai --verbose
rocode models --refresh
```

说明：

- `rocode models --refresh` 会主动请求 `https://models.dev/api.json` 并更新本地 provider/model catalog
- 在交互式会话里，对应的斜杠命令是 `/models refresh`

---

## rocode session -- 会话管理

管理会话的创建、列表、查看和删除。

### 子命令

#### session list

```
rocode session list [选项]
```

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `-n, --max-count` | i64 | -- | 最大返回数量 |
| `--format` | enum | table | 输出格式: `table` 或 `json` |
| `--project` | string | -- | 按项目筛选 |

#### session show

```
rocode session show <SESSION_ID>
```

说明：

- 输出的是 authority-backed session info。
- 如果该会话已有 persisted telemetry，返回中会包含 usage、stage summaries、repair summary、repair query snapshot、tool trajectory quality 等结构化读面。

#### session delete

```
rocode session delete <SESSION_ID>
```

#### session provision-external-adapter

```
rocode session provision-external-adapter --adapter-id <ID> --actor-id <ID> [选项]
```

| 参数 | 类型 | 说明 |
|------|------|------|
| `--adapter-id` | string | 外部 adapter 标识 |
| `--actor-id` | string | 外部调用方标识 |
| `--workspace-id` | string | 外部工作区标识 |
| `--route-policy-id` | string | 路由策略标识 |
| `--scheduler-profile` | string | 调度器 profile |
| `--directory` | path | 工作目录 |
| `--project-id` | string | 项目标识 |
| `--title` | string | 会话标题 |
| `--format` | enum | 输出格式: `text` 或 `json` |

说明：

- 这是 external adapter 的正式 owner-local session provision 入口。
- 集成方不应该自己猜测 session id，而应先通过这个命令或对应 HTTP 路由申请会话。

---

## rocode memory -- Memory 权威工件

导出或导入 memory authority 持久化工件。

### 子命令

| 子命令 | 说明 |
|--------|------|
| `export` | 导出 memory authority 记录为 JSON |
| `import` | 从 JSON 导入 memory authority 记录 |

### 用法

```bash
rocode memory export --output memory.json
rocode memory import ./memory.json
```

---

## rocode provider -- Provider 权威工件

导出或导入 provider authority 持久化工件。

### 子命令

| 子命令 | 说明 |
|--------|------|
| `export` | 导出 provider authority 工件为 JSON |
| `import` | 从 JSON 导入 provider authority 工件 |

### 用法

```bash
rocode provider export --output provider.json
rocode provider import ./provider.json
```

---

## rocode skill -- 技能目录管理

管理技能目录和远程 Hub 操作。

### 子命令

```
rocode skill hub <action> [选项]
```

#### Hub 子命令

| 子命令 | 说明 |
|--------|------|
| `status` | 显示分布、缓存和生命周期状态总览 |
| `managed` | 显示托管技能来源记录 |
| `index` | 显示缓存技能来源索引 |
| `distributions` | 显示已解析的远程分布记录 |
| `artifact-cache` | 显示工件缓存条目 |
| `policy` | 显示当前技能 Hub 工件策略 |
| `lifecycle` | 显示托管生命周期记录 |
| `index-refresh` | 刷新一个来源的索引缓存 |
| `sync-plan` | 创建一个来源的 Hub 同步计划 |
| `sync-apply` | 应用一个来源的 Hub 同步计划 |
| `install-plan` | 规划一个远程分布安装 |
| `install-apply` | 应用一个远程分布安装 |
| `update-plan` | 规划一个托管技能更新 |
| `update-apply` | 应用一个托管技能更新 |
| `detach` | 从来源分离托管技能（保留工作区文件） |
| `remove` | 移除托管技能（仅在干净状态时删除工作区副本） |

#### Proposal 子命令

| 子命令 | 说明 |
|--------|------|
| `list` | 列出 skill evolution proposals |
| `show` | 查看 proposal 详情 |
| `approve` | 批准 proposal（不直接改写 SKILL.md） |
| `reject` | 拒绝 proposal |

#### 公共参数

| 参数 | 说明 |
|------|------|
| `--source-id` | 来源标识符 |
| `--source-kind` | 来源类型: `bundled`, `local-path`, `git`, `archive`, `registry` |
| `--locator` | 来源定位符 |
| `--revision` | 可选版本 |
| `--skill-name` | 技能名称（安装/更新/删除操作需要） |
| `--session-id` | 会话 ID（apply 操作需要） |
| `--format` | 输出格式: `text`（默认）或 `json` |

---

## rocode stats -- 用量统计

显示令牌使用和成本统计。

### 用法

```
rocode stats [选项]
```

| 参数 | 类型 | 说明 |
|------|------|------|
| `--days` | i64 | 统计天数 |
| `--tools` | usize | 显示的工具数量 |
| `--models` | usize | 显示的模型数量 |
| `--project` | string | 按项目筛选 |

---

## rocode db -- 数据库工具

访问本地 SQLite 数据库。

### 用法

```
rocode db [QUERY] [选项]
rocode db path
```

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `QUERY` | string | -- | SQL 查询 |
| `--format` | enum | tsv | 输出格式: `json` 或 `tsv` |

---

## rocode config -- 配置与 validation

显示当前已解析配置，或读取 authority-backed validation snapshot。

### 用法

```bash
rocode config
rocode config validation
rocode config validation --format json
```

### 子命令

| 子命令 | 说明 |
|--------|------|
| `validation` | 显示配置 validation 快照 |

说明：

- `rocode config` 侧重“当前解析结果”。
- `rocode config validation` 侧重 provider / external adapter / scheduler skill tree 等 owner-local validation 结果。

---

## rocode auth -- 凭证管理

管理 AI 提供商认证凭证。

### 子命令

| 子命令 | 说明 |
|--------|------|
| `list` (别名 `ls`) | 列出支持的认证提供商和当前环境状态 |
| `login [PROVIDER_OR_URL]` | 设置当前进程的凭证（非持久化） |
| `logout [PROVIDER]` | 清除当前进程的凭证 |

#### login 参数

| 参数 | 说明 |
|------|------|
| `PROVIDER_OR_URL` | 提供商名称或 URL |
| `--token` | 直接传入 API token |

### 示例

```bash
rocode auth list
rocode auth login zhipuai --token zhipu-xxx
rocode auth logout zhipuai
```

---

## rocode agent -- 智能体管理

管理智能体定义。

### 子命令

| 子命令 | 说明 |
|--------|------|
| `list` | 列出可用智能体 |
| `create` | 创建智能体 Markdown 文件 |

#### create 参数

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `NAME` | string | 必填 | 智能体名称 |
| `--description` | string | 必填 | 智能体描述 |
| `--mode` | enum | all | 文件模式: `all`, `primary`, `subagent` |
| `--path` | path | -- | 输出路径 |
| `--tools` | string | -- | 允许的工具列表 |
| `-m, --model` | string | -- | 默认模型 |

---

## rocode debug -- 调试工具

调试和故障排查工具集。

### 子命令

| 子命令 | 说明 |
|--------|------|
| `paths` | 显示重要的本地路径 |
| `config` | 显示已解析的 JSON 配置 |
| `skill` | 列出所有可用技能 |
| `skills` | 技能目录调试子命令 |
| `scrap` | 列出所有已知项目 |
| `wait` | 无限等待（用于调试） |
| `snapshot` | 快照调试工具 |
| `file` | 文件系统调试工具 |
| `rg` | Ripgrep 调试工具 |
| `lsp` | LSP 调试工具 |
| `docs` | 上下文文档调试工具 |
| `repair` | tool repair / sanitizer / repair query 调试工具 |
| `agent` | 显示智能体配置详情 |

#### debug agent

```
rocode debug agent <NAME> [--tool <tool>] [--params <params>]
```

#### debug file 子命令

| 子命令 | 说明 |
|--------|------|
| `search <QUERY>` | 搜索文件 |
| `read <PATH>` | 以 JSON 读取文件内容 |
| `status` | 显示文件状态 |
| `list <PATH>` | 列出目录内容 |
| `tree [DIR]` | 显示目录树 |

#### debug rg 子命令

| 子命令 | 说明 |
|--------|------|
| `tree [--limit N]` | 使用 ripgrep 显示文件树 |
| `files [--query Q] [--glob G] [--limit N]` | 使用 ripgrep 列出文件 |
| `search <PATTERN> [--glob G...] [--limit N]` | 使用 ripgrep 搜索内容 |

#### debug lsp 子命令

| 子命令 | 说明 |
|--------|------|
| `diagnostics <FILE>` | 获取文件诊断 |
| `symbols <QUERY>` | 搜索工作区符号 |
| `document-symbols <URI>` | 获取文档符号 |

#### debug repair 子命令

| 子命令 | 说明 |
|--------|------|
| `summary <SESSION_ID>` | 显示一个会话的 repair summary |
| `query` | 查询单会话或全局 repair events |

`query` 支持的主要过滤参数：

| 参数 | 说明 |
|------|------|
| `--session-id` | 限定一个会话 |
| `--provider-id` | 按 provider 过滤 |
| `--model-id` | 按 model 过滤 |
| `--tool-name` | 按工具过滤 |
| `--repair-kind` | 按 repair kind 过滤 |
| `--layer` | 按层过滤（如 sanitizer / execution） |
| `--strict-only` | 仅看 strict would fail 相关项 |
| `--include-samples` | 返回样本 |
| `--limit` | 限制结果数 |

#### debug snapshot 子命令

| 子命令 | 说明 |
|--------|------|
| `track` | 跟踪当前快照状态 |
| `patch <HASH>` | 显示快照哈希的补丁 |
| `diff <HASH>` | 显示快照哈希的差异 |

#### debug skills 子命令

| 子命令 | 说明 |
|--------|------|
| `list` | 列出已解析的技能目录 |
| `view <NAME>` | 显示一个技能的原始详情 |
| `managed` | 显示托管技能来源记录 |
| `index` | 显示缓存技能来源索引 |
| `distributions` | 显示远程分布记录 |
| `artifact-cache` | 显示工件缓存条目 |
| `lifecycle` | 显示托管生命周期记录 |
| `index-refresh` | 刷新来源索引缓存 |
| `audit` | 显示最近的技能治理审计事件 |
| `timeline` | 显示统一治理时间线 |
| `guard` | 运行技能守卫扫描 |
| `sync-plan / sync-apply` | Hub 同步 |
| `install-plan / install-apply` | 远程安装 |
| `update-plan / update-apply` | 托管更新 |
| `detach / remove` | 分离/移除 |

---

## rocode mcp -- MCP 服务器管理

管理 Model Context Protocol 服务器。详见 [mcp.md](./mcp.md)。

### 用法

```
rocode mcp [选项] <action> [参数]
```

| 全局参数 | 默认值 | 说明 |
|----------|--------|------|
| `--server` | `http://127.0.0.1:3000` | 服务器地址 |

### 子命令

| 子命令 | 说明 |
|--------|------|
| `list` (别名 `ls`) | 列出 MCP 服务器和状态 |
| `add <NAME>` | 添加 MCP 服务器 |
| `connect <NAME>` | 连接 MCP 服务器 |
| `disconnect <NAME>` | 断开 MCP 服务器 |
| `auth` | MCP OAuth 操作 |
| `logout [NAME]` | 移除 MCP OAuth 凭证 |
| `debug <NAME>` | 调试 OAuth 连接 |

#### mcp add 参数

| 参数 | 说明 |
|------|------|
| `<NAME>` | 服务器名称 |
| `--url` | 远程 URL（与 `--command` 二选一） |
| `--command` | 本地命令（与 `--url` 二选一） |
| `--arg` | 命令参数（可多次指定） |
| `--enabled` | 是否启用（默认 true） |
| `--timeout` | 超时（毫秒） |

### 示例

```bash
# 列出所有 MCP 服务器
rocode mcp list

# 添加远程 MCP 服务器
rocode mcp add my-server --url https://mcp.example.com/sse

# 添加本地 MCP 服务器
rocode mcp add filesystem --command npx --arg -y --arg @modelcontextprotocol/server-filesystem

# 连接/断开
rocode mcp connect my-server
rocode mcp disconnect my-server

# OAuth 认证
rocode mcp auth my-server --authenticate
```

---

## rocode export / import -- 会话导入导出

### export

将会话数据导出为 JSON。

```
rocode export [SESSION_ID] [-o, --output <PATH>]
```

### import

从 JSON 文件或共享 URL 导入会话数据。

```
rocode import <FILE_OR_URL>
```

---

## rocode github -- GitHub 智能体

管理 GitHub 智能体集成。

### 子命令

| 子命令 | 说明 |
|--------|------|
| `status` | 检查 GitHub CLI 安装和认证状态 |
| `install` | 在当前仓库安装 GitHub 智能体 |
| `run` | 运行 GitHub 智能体（CI 模式） |

#### github run 参数

| 参数 | 说明 |
|------|------|
| `--event` | GitHub 事件类型 |
| `--token` | GitHub token |

---

## rocode pr -- PR 检出

拉取并检出 GitHub PR 分支，然后启动 ROCode。

```
rocode pr <NUMBER>
```

---

## rocode upgrade -- 升级

升级 ROCode 到最新或指定版本。

```
rocode upgrade [TARGET] [-m, --method <METHOD>]
```

`rocode upgrade` 适合由安装器或包管理器维护的安装方式。对于源码 / 本地单文件安装，请重新安装完整的 `rocode` 分发物，而不是手工替换旧二进制：

```bash
./scripts/install-local.sh release ~/.local
```

---

## rocode uninstall -- 卸载

卸载 ROCode 及相关文件。

```
rocode uninstall [选项]
```

| 参数 | 说明 |
|------|------|
| `-c, --keep-config` | 保留配置文件 |
| `-d, --keep-data` | 保留数据文件 |
| `--dry-run` | 只显示将要执行的操作 |
| `-f, --force` | 强制卸载 |

当 `rocode` 以本地单文件布局安装时，卸载会删除 `rocode` 以及对应的 Web 资源目录。

---

## rocode generate -- OpenAPI 生成

生成 OpenAPI 规范 JSON 文件。

```
rocode generate
```

---

## rocode version / info -- 版本信息

| 命令 | 说明 |
|------|------|
| `rocode version` | 显示版本号 |
| `rocode info` | 显示构建和环境信息（编译器、目标平台、profile） |

---

## 交互式斜杠命令

在 TUI 或 CLI REPL 中，以下斜杠命令可用：

### 会话管理

| 命令 | 别名 | 说明 |
|------|------|------|
| `/help` | `help`, `/commands` | 显示帮助 |
| `/exit` | `exit`, `/quit`, `/q` | 退出会话 |
| `/new` | -- | 创建新会话 |
| `/clear` | `clear` | 清屏 |
| `/compact` | -- | 压缩上下文以释放令牌空间 |
| `/copy` | -- | 复制当前会话 |
| `/session` | `/sessions`, `/resume`, `/continue` | 列出/恢复会话 |
| `/parent` | `/back` | 返回父会话 |

### 附着会话管理

| 命令 | 说明 |
|------|------|
| `/attached` | 列出附着会话 |
| `/attached list` | 列出附着会话 |
| `/attached focus <ID>` | 聚焦到附着会话 |
| `/attached next` | 聚焦下一个附着会话 |
| `/attached prev` | 聚焦上一个附着会话 |
| `/attached back` / `/attached root` | 返回根会话 |

### 模型与提供商

| 命令 | 说明 |
|------|------|
| `/model` | 列出可用模型 |
| `/model <ref>` | 切换模型（格式: `provider/model`） |
| `/models` | 列出可用模型 |
| `/providers` | 列出提供商 |
| `/provider <name>` | 连接到提供商 |
| `/connect <name>` | 连接到提供商 |
| `/preset` | 列出调度器预设 |
| `/preset <name>` | 选择调度器预设 |

### 智能体与任务

| 命令 | 说明 |
|------|------|
| `/agent` | 列出可用智能体 |
| `/agent <name>` | 切换智能体 |
| `/tasks` | 列出智能体任务 |
| `/tasks show <ID>` | 显示任务详情 |
| `/tasks kill <ID>` | 终止任务（别名: `/tasks cancel`） |

### 恢复与调试

| 命令 | 说明 |
|------|------|
| `/abort` | 终止当前活动执行边界 |
| `/recover` | 显示恢复操作列表 |
| `/recover <key\|number>` | 执行恢复操作 |
| `/status` | 显示会话状态（别名: `/stats`） |
| `/runtime` | 显示运行时信息 |
| `/usage` | 显示令牌用量 |
| `/events` | 显示事件浏览器 |
| `/events <query>` | 按条件过滤事件 |
| `/inspect` | 显示阶段事件日志（别名: `/stage`, `/stages`） |
| `/inspect <stage_id>` | 显示特定阶段 |

`/abort` 通过独立控制请求命中 server 的取消路由，不会把 `/abort` 文本作为普通消息插进当前运行中的任务。若目标是某个已登记的 agent task，请使用 `/tasks kill <ID>`。

事件浏览器查询语法：

```
/events stage=stg_1 exec=exe_2 type=session.updated limit=10 page=2
/events next          -- 下一页
/events prev          -- 上一页
/events first         -- 第一页
/events clear         -- 清除过滤器
/events page 3        -- 跳转页
```

### 界面控制

| 命令 | 说明 |
|------|------|
| `/sidebar` | 切换侧边栏显示/隐藏 |
| `/active` | 切换活动面板显示/隐藏 |
| `/up` / `/pageup` | 向上滚动 |
| `/down` / `/pagedown` | 向下滚动 |
| `/bottom` / `/end` | 滚动到底部 |
| `/theme` | 列出/选择主题 |

### CLI 专用

| 命令 | 说明 |
|------|------|
| `/share` | 共享当前会话 |
| `/unshare` | 取消共享 |
| `/palette` | 打开命令面板 |
| `/rename <name>` | 重命名会话 |
