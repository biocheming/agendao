# ROCode 工具参考

本文档是 ROCode 所有内置工具的完整参考。工具是模型与外部世界交互的机制 -- 读写文件、执行命令、搜索代码、管理任务等。

---

## 目录

1. [工具系统概览](#工具系统概览)
2. [文件操作工具](#文件操作工具)
3. [搜索工具](#搜索工具)
4. [代码结构工具](#代码结构工具)
5. [Shell 执行工具](#shell-执行工具)
6. [Web 工具](#web-工具)
7. [Git 工具](#git-工具)
8. [技能管理工具](#技能管理工具)
9. [任务管理工具](#任务管理工具)
10. [Todo 工具](#todo-工具)
11. [计划模式工具](#计划模式工具)
12. [交互工具](#交互工具)
13. [文档查询工具](#文档查询工具)
14. [GitHub 研究工具](#github-研究工具)
15. [LSP 工具](#lsp-工具)
16. [媒体工具](#媒体工具)
17. [批量执行工具](#批量执行工具)
18. [插件工具](#插件工具)

---

## 工具系统概览

每个工具实现统一的 `Tool` trait：

- **标识** -- 工具 ID（name）、描述
- **输入模式** -- JSON Schema 定义参数
- **执行** -- `execute()` 异步方法执行操作
- **权限** -- 执行前通过权限检查

工具在会话启动时加载。模型接收工具描述和参数模式，根据需要选择调用。每次工具调用经过权限解析后才执行。

---

## 文件操作工具

### read

读取本地文件系统上的文件或目录内容。对目录返回目录列表，对文件返回带行号的内容。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `file_path` | string | 是 | 绝对路径或项目相对路径 |
| `offset` | integer | 否 | 起始行号（1-indexed） |
| `limit` | integer | 否 | 最大行数（默认 2000） |

特性：
- 支持读取图片和 PDF，以附件形式返回
- 目录条目每行一个，子目录带 `/` 后缀
- 超过 2000 字符的行会被截断
- 默认最多返回 2000 行

### write

将内容写入文件。文件不存在时创建（含中间目录），存在时覆盖。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `file_path` | string | 是 | 绝对路径或项目相对路径 |
| `content` | string | 是 | 要写入的内容 |

### edit

在文件中执行精确字符串替换。支持多种匹配策略。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `file_path` | string | 是 | 文件路径 |
| `old_string` | string | 是 | 要替换的文本 |
| `new_string` | string | 是 | 替换为的文本（必须与 old_string 不同） |
| `replace_all` | boolean | 否 | 替换所有匹配（默认 false） |

注意：空白和缩进必须完全匹配。

### multiedit

跨多个文件执行原子性字符串替换操作。同一文件的多次编辑按顺序执行。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `edits` | array | 是 | 编辑数组，每项包含 `file_path` 和 `edits`（编辑操作数组） |

每个编辑操作包含：

| 字段 | 类型 | 说明 |
|------|------|------|
| `old_string` | string | 要替换的文本 |
| `new_string` | string | 替换为的文本 |
| `replace_all` | boolean | 替换所有匹配（默认 false） |

### apply_patch

应用统一 diff 格式补丁。接受标准 `diff -u` / `git diff` 格式。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `patchText` (别名 `patch_text`) | string | 是 | 统一 diff 补丁文本 |

支持的补丁操作：Add（新增文件）、Update（修改文件）、Delete（删除文件）、Move（移动/重命名文件）。

### ls

列出目录内容。自动忽略常见非项目目录（node_modules, .git, target 等）。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `path` | string | 否 | 目录路径（默认当前目录） |
| `ignore` | string[] | 否 | 额外忽略模式 |

默认忽略的目录：`node_modules/`, `__pycache__/`, `.git/`, `dist/`, `build/`, `target/`, `vendor/`, `.venv/` 等。

---

## 搜索工具

### grep

使用正则表达式搜索文件内容。结果按文件修改时间排序（最新优先）。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `pattern` | string | 是 | 正则表达式模式 |
| `path` | string | 否 | 搜索目录或文件 |
| `glob` | string | 否 | 文件 glob 筛选（如 `*.rs`） |
| `output_mode` | string | 否 | 输出模式: `content`, `files_with_matches`, `count` |
| `-i` | boolean | 否 | 不区分大小写 |
| `head_limit` | integer | 否 | 限制输出行数（默认 250） |

### glob

使用 glob 模式查找文件。按修改时间排序返回匹配的文件路径。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `pattern` | string | 是 | Glob 模式（如 `**/*.rs`, `src/**/*.ts`） |
| `path` | string | 否 | 搜索起始目录 |

### codesearch

通过 Exa MCP 进行语义代码搜索。返回语义相关的代码片段。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `query` | string | 是 | 搜索查询 |
| `tokensNum` (别名 `tokens_num`) | u32 | 否 | 返回的 token 数量（默认 5000） |

---

## 代码结构工具

### ast_grep_search

使用 ast-grep 引擎进行结构化代码搜索。Phase 1 支持 Rust 语法。当纯文本 grep 精度不够时使用。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `pattern` | string | 是 | AST 模式 |
| `language` | string | 是 | 编程语言 |
| `path` | string | 否 | 搜索路径 |
| `glob` | string | 否 | 文件 glob 筛选 |
| `maxResults` | integer | 否 | 最大结果数 |
| `contextLines` | integer | 否 | 上下文行数 |

支持的语言：Rust, TypeScript, JavaScript, Python, Go, Java, C, C++, 等。

### ast_grep_replace

使用 ast-grep 引擎进行结构化代码替换。执行 AST 感知的重写而非纯文本替换。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `pattern` | string | 是 | AST 模式 |
| `replacement` | string | 是 | 替换模式 |
| `language` | string | 是 | 编程语言 |
| `path` | string | 否 | 搜索路径 |
| `glob` | string | 否 | 文件 glob 筛选 |
| `maxReplacements` | integer | 否 | 最大替换数（默认 50） |
| `apply` | boolean | 否 | 是否写入磁盘（默认 false = 仅预览） |

安全模型：
- 默认仅预览，不修改文件
- 设置 `apply=true` 写入磁盘
- 结果超过 `maxReplacements` 时拒绝 `apply=true`

---

## Shell 执行工具

### bash

在 bash 子进程中执行命令。支持超时、后台运行和进程树管理。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `command` | string | 是 | Shell 命令 |
| `description` | string | 否 | 命令描述（用于显示） |
| `timeout` | integer | 否 | 超时毫秒（默认 120000，最大 600000） |
| `run_in_background` | boolean | 否 | 后台运行 |

特性：
- 默认超时 2 分钟，最大 10 分钟
- 输出超过 50KB 时截断
- 后台运行时通过通知返回结果
- 会话内工作目录持久化

### shell_session

持久交互式 Shell 会话。基于 PTY 的长生命周期终端会话。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `operation` | enum | 是 | 操作: `start`, `write`, `read`, `status`, `terminate` |
| `sessionId` | string | 否 | 会话 ID |
| `command` | string | 否 | 启动命令 |
| `args` | string[] | 否 | 命令参数 |
| `cwd` | string | 否 | 工作目录 |
| `env` | map | 否 | 环境变量 |

操作说明：

| 操作 | 说明 |
|------|------|
| `start` | 创建 PTY 支持的 shell 会话 |
| `write` | 发送行输入到会话 |
| `read` | 读取缓冲的输出 |
| `status` | 检查会话状态 |
| `terminate` | 终止会话 |

---

## Web 工具

### webfetch

从 URL 获取内容。支持多种格式。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `url` | string | 是 | 要获取的 URL |
| `format` | enum | 否 | 返回格式: `text`, `markdown`（默认）, `html` |
| `timeout` | integer | 否 | 超时秒数 |

### websearch

通过 Exa MCP 进行 Web 搜索。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `query` | string | 是 | 搜索查询 |
| `searchType` | string | 否 | 搜索类型（默认 `auto`） |
| `numResults` | integer | 否 | 结果数量（默认 8） |

### browser_session

结构化浏览器式 HTTP 会话，支持 Cookie 和页面状态管理。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `operation` | enum | 是 | 操作: `start`, `visit`, `read`, `status`, `terminate` |
| `sessionId` | string | 否 | 会话 ID |
| `baseUrl` | string | 否 | 基础 URL |
| `url` | string | 否 | 访问 URL |
| `path` | string | 否 | 相对路径 |
| `headers` | map | 否 | 自定义请求头 |
| `userAgent` | string | 否 | User-Agent |
| `format` | string | 否 | 输出格式（默认 `markdown`） |
| `timeout` | integer | 否 | 超时秒数 |

注意：这不是 JS 浏览器，不支持 JavaScript 渲染。

---

## Git 工具

### repo_history

本地 git 仓库的结构化历史查询。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `operation` | enum | 是 | 操作（见下表） |
| `path` | string | 否 | 文件/目录路径 |
| `commit` (别名 `sha`) | string | 否 | 提交 SHA |
| `lineStart` | integer | 否 | 起始行号 |
| `lineEnd` | integer | 否 | 结束行号 |
| `limit` | integer | 否 | 返回数量限制（默认 20，最大 100） |

操作列表：

| 操作 | 说明 |
|------|------|
| `status` | 工作区状态 |
| `head` | 当前 HEAD 信息 |
| `log` | 提交日志 |
| `show_commit` | 查看提交详情 |
| `diff_uncommitted` | 未提交的变更 |
| `blame` | 行级 blame |

---

## 技能管理工具

### skills_list

列出可用技能及其名称和描述。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `category` | string | 否 | 按类别筛选 |

### skill_view

加载技能的完整 SKILL.md 内容或其支持文件。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `name` | string | 是 | 技能名称 |
| `file_path` | string | 否 | 技能根目录下的相对文件路径 |

### skill

加载并执行技能（兼容性别名，建议使用 `skill_view`）。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `skill_name` | string | 是 | 技能名称 |
| `arguments` | object | 否 | 传给技能的参数 |
| `prompt` | string | 否 | 额外提示词 |

### skill_manage

创建、编辑、删除工作区本地技能（`.rocode/skills/` 目录下）。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `action` | enum | 是 | 操作: `create`, `patch`, `edit`, `write_file`, `remove_file`, `delete` |
| `name` | string | 否 | 技能名称 |
| `new_name` | string | 否 | 新名称（重命名） |
| `description` | string | 否 | 技能描述 |
| `body` | string | 否 | SKILL.md 内容 |
| `content` | string | 否 | 文件内容 |
| `category` | string | 否 | 分类 |
| `directory_name` | string | 否 | 目录名 |
| `file_path` | string | 否 | 文件路径 |

`skill_manage` 也是 skill 自进化闭环中的正式写入点：

- 复杂回合后的 skill 保存建议，最终应通过它落到 workspace authority
- 已使用 skill 的 reflection / patch 提示，也以它作为正式修补入口
- 写入结果会被 runtime 送入 memory observation，后续可在 validation、conflicts、consolidation 与 methodology promotion 中看到影响

### skill_hub

检查托管技能治理状态、刷新来源索引、创建/应用 Hub 同步计划。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `action` | enum | 是 | 操作（见下表） |
| `source_id` | string | 否 | 来源标识符 |
| `source_kind` | enum | 否 | 来源类型 |
| `locator` | string | 否 | 来源定位符 |
| `revision` | string | 否 | 版本 |
| `skill_name` | string | 否 | 技能名称 |

操作列表：

| 操作 | 说明 |
|------|------|
| `managed` | 显示托管技能来源记录 |
| `index` | 显示缓存来源索引 |
| `distribution_list` | 显示远程分布记录 |
| `artifact_cache` | 显示工件缓存 |
| `lifecycle` | 显示生命周期记录 |
| `index_refresh` | 刷新来源索引 |
| `sync_plan` | 创建同步计划 |
| `sync_apply` | 应用同步计划 |
| `install_plan` | 规划安装 |
| `install_apply` | 应用安装 |
| `update_plan` | 规划更新 |
| `update_apply` | 应用更新 |
| `detach` | 分离托管技能 |
| `remove` | 移除托管技能 |
| `audit` | 审计记录 |
| `guard_run` | 守卫扫描 |

### memory surfaces

ROCode 当前把 memory 做成正式可观测能力，而不只是内部实验数据。常用入口包括：

- 交互命令：`/memory`、`/memory preview <query>`、`/memory show <id>`、`/memory validation <id>`、`/memory conflicts <id>`、`/memory rules`、`/memory hits ...`、`/memory runs`、`/memory consolidate ...`
- HTTP 路由：`/memory/list`、`/memory/search`、`/memory/retrieval-preview`、`/memory/{id}`、`/memory/{id}/validation-report`、`/memory/{id}/conflicts`、`/memory/rule-packs`、`/memory/rule-hits`、`/memory/consolidation/runs`、`/memory/consolidate`

这些入口覆盖的不是单一“搜索记忆”，而是完整治理面：

- record list / detail
- retrieval preview
- validation report
- conflict view
- rule packs / rule hits
- consolidation runs 与即时 consolidation

---

## 任务管理工具

### task

创建和管理后台代理任务。支持子代理分发和后台执行。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `description` | string | 否 | 任务描述 |
| `prompt` | string | 否 | 任务提示词 |
| `subagentType` | string | 否 | 子代理类型 |
| `category` | string | 否 | 任务分类 |
| `taskId` | string | 否 | 任务 ID |
| `command` | string | 否 | 命令 |
| `loadSkills` | string[] | 否 | 加载的技能列表 |
| `runInBackground` | boolean | 否 | 后台运行（默认 false） |
| `agentPrompt` | string | 否 | 自定义代理系统提示 |
| `agentTools` | string[] | 否 | 自定义代理工具列表 |

### task_flow

任务生命周期编排门面。提供稳定的请求级接口。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `operation` | enum | 是 | 操作: `create`, `resume`, `get`, `list`, `cancel` |
| `taskId` | string | 否 | 任务 ID |
| `prompt` | string | 否 | 任务提示 |
| `subagentType` | string | 否 | 子代理类型 |
| `todos` | array | 否 | Todo 项数组 |

`task_flow cancel` 用于按 `taskId` 精确取消已登记的 agent task。它和 `/abort` 的边界不同: `/abort` 取消当前会话的活动执行边界，`task_flow cancel` / `/tasks kill` 则针对具体任务记录。

---

## Todo 工具

### todoread

读取当前会话的 Todo 列表。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `session_id` | string | 否 | 会话 ID（默认当前会话） |

### todowrite

写入/更新 Todo 列表。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `todos` | array | 是 | Todo 项数组 |
| `session_id` | string | 否 | 会话 ID |

每项 Todo 包含：

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | string | 项 ID |
| `content` | string | 内容 |
| `status` | string | 状态: `pending`, `in_progress`, `completed` |
| `priority` | string | 优先级: `low`, `medium`, `high` |

---

## 计划模式工具

### plan_enter (PlanEnterTool)

进入计划模式。在计划模式下，所有写入和执行工具被阻止。代理只能读取文件、搜索和推理。

无参数。

### plan_exit (PlanExitTool)

退出计划模式，恢复到进入前的权限模式。

无参数。

---

## 交互工具

### question

在执行过程中向用户提出澄清性问题。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `questions` | array | 是 | 问题数组 |

每个问题包含：

| 字段 | 类型 | 说明 |
|------|------|------|
| `question` | string | 问题文本 |
| `header` | string | 可选标题 |
| `options` | array | 选项列表（`label` + 可选 `description`） |
| `multiple` | boolean | 是否允许多选 |

---

## 文档查询工具

### context_docs

文档感知的官方文档查找工具。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `operation` | enum | 是 | 操作（见下表） |
| `library` | string | 否 | 库名称 |
| `library_id` | string | 否 | 库 ID |
| `query` | string | 否 | 查询文本 |
| `limit` | integer | 否 | 结果限制（默认 5，最大 20） |
| `page` | integer | 否 | 页码 |

操作列表：

| 操作 | 说明 |
|------|------|
| `resolve_library` | 解析库名称 |
| `query_docs` | 查询文档 |
| `get_page` | 获取具体页面 |

---

## GitHub 研究工具

### github_research

GitHub 仓库结构化研究工具。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `operation` | enum | 是 | 操作（见下表） |
| `query` | string | 否 | 搜索查询 |
| `owner` | string | 否 | 仓库所有者 |
| `repo` | string | 否 | 仓库名称 |
| `number` | integer | 否 | Issue/PR 编号 |
| `path` | string | 否 | 文件路径 |
| `sha` | string | 否 | 提交 SHA |
| `limit` | integer | 否 | 结果限制 |

操作列表：

| 操作 | 说明 |
|------|------|
| `search_code` | 搜索代码 |
| `search_issues` | 搜索 Issue |
| `search_prs` | 搜索 PR |
| `view_issue` | 查看 Issue |
| `view_pr` | 查看 PR |
| `view_pr_files` | 查看 PR 文件变更 |
| `get_head_sha` | 获取 HEAD SHA |
| `build_permalink` | 构建永久链接 |
| `read_file` | 读取文件 |
| `clone_repo` | 克隆仓库 |
| `list_releases` | 列出发布 |
| `list_tags` | 列出标签 |
| `git_log` | Git 日志 |
| `git_blame` | Git blame |

---

## LSP 工具

### lsp

Language Server Protocol 操作，用于代码导航和分析。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `operation` | enum | 是 | 操作（见下表） |
| `filePath` | string | 是 | 文件路径 |
| `line` | integer | 否 | 行号（0-based） |
| `character` | integer | 否 | 列号（0-based） |
| `query` | string | 否 | 搜索查询 |
| `newName` | string | 否 | 新名称（重命名） |

操作列表：

| 操作 | 说明 |
|------|------|
| `goToDefinition` | 跳转到定义 |
| `findReferences` | 查找引用 |
| `hover` | 悬停信息 |
| `documentSymbol` | 文档符号 |
| `workspaceSymbol` | 工作区符号 |
| `goToImplementation` | 跳转到实现 |
| `typeDefinition` | 类型定义 |
| `rename` | 重命名 |
| `diagnostics` | 诊断 |
| `prepareCallHierarchy` | 调用层级 |
| `incomingCalls` | 入调用 |
| `outgoingCalls` | 出调用 |

---

## 媒体工具

### media_inspect

检查本地媒体文件。通过 `media-reader` 代理处理。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `filePath` | string | 是 | 本地文件路径 |
| `question` | string | 否 | 关于文件的问题（默认: 描述相关内容） |

---

## 批量执行工具

### batch

并行执行多个工具调用。每批最多 25 个。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `toolCalls` | array | 是 | 工具调用数组 |

每个调用包含：

| 字段 | 类型 | 说明 |
|------|------|------|
| `tool` | string | 工具名称 |
| `parameters` | object | 工具参数 |

注意：`batch` 工具本身不能被嵌套调用。

---

## 插件工具

### PluginTool

由插件注册的自定义工具。工具 ID、描述和参数由插件定义。

每个 `PluginTool` 持有 `PluginLoader` 引用，支持空闲关机后的透明恢复。

详见 [plugins.md](./plugins.md)。
