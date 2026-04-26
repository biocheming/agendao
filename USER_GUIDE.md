# USER GUIDE - RustingOpenCode (ROCode)

本手册面向日常使用者，按“如何启动、如何工作、如何排查”的顺序介绍当前版本的 ROCode。

## 0. 版本

- 当前版本：`v2026.4.26`
- 当前 CLI 命令：`rocode`

## 1. 先选运行方式

### 1.1 TUI

适合日常在本地仓库里交互式工作：

```bash
rocode tui
```

也可以从源码直接启动：

```bash
cargo run -p rocode -- tui
```

### 1.2 单次运行

适合脚本、自动化、CI：

```bash
rocode run "请检查这个仓库里的高风险改动"
```

常用附加参数：

```bash
rocode run "..." --model <MODEL>
rocode run "..." --session <SESSION_ID>
rocode run "..." --continue
rocode run "..." --fork
rocode run "..." --format json
rocode run "..." --thinking
```

### 1.3 HTTP Server / Web

启动服务：

```bash
rocode serve --hostname 127.0.0.1 --port 3000
```

启动 Web：

```bash
rocode web --hostname 127.0.0.1 --port 3000
```

如果不在目标目录里启动，可以显式指定 workspace：

```bash
rocode web --dir /path/to/workspace
```

当前 Web 正式入口是 `/`，不是历史过渡路由。
如果用户直接双击二进制、且不在终端环境里启动，ROCode 会优先走桌面 Web 启动路径，并在打开浏览器前先确定 workspace 目录；若当前目录不可信，会尝试复用上次目录或弹出系统目录选择框。
图标与品牌源资产当前以 `icons/ROCODE.svg` 为主；桌面分发派生资产位于 `icons/rocode.ico` 与 `icons/rocode.icns`。Web 会使用基于该品牌源生成的 favicon，`windows-msvc` 构建会尝试把 `.ico` 嵌入生成的 `rocode.exe`，Linux 桌面分发可使用 `packaging/linux/rocode.desktop` 模板，macOS 可通过 `./scripts/build_macos_app_bundle.sh release` 组装 Finder 可双击的 `ROCode.app`。

### 1.4 Attach

如果服务已经启动，可以附加：

```bash
rocode attach http://127.0.0.1:3000
```

## 2. 最常见的日常操作

### 2.1 查看模型并刷新 provider catalog

```bash
rocode models
rocode models --refresh
rocode models openrouter --refresh --verbose
```

这组命令会直接反映当前模型目录，而不是只看静态内置列表。
其中 `rocode models --refresh` 会显式强制刷新 `https://models.dev/api.json`；如果你在交互式会话里操作，对应命令是 `/models refresh`。

### 2.2 管理认证

```bash
rocode auth list
rocode auth login --help
rocode auth logout --help
```

如果 provider 连不上，先看 `auth list`，再刷新 `models`。

### 2.3 查看和管理 session

```bash
rocode session list
rocode session list --format json
rocode session show <SESSION_ID>
rocode session delete <SESSION_ID>
```

### 2.4 查看配置

```bash
rocode config
rocode debug paths
rocode debug config
```

如果你不确定当前 runtime 到底读了哪份配置，这三条最有用。

## 3. Workspace / Config 现在是怎么工作的

ROCode 现在区分：

- workspace local authority
- sandbox `.rocode`
- project config
- global config
- shared / isolated workspace mode

常见规则：

- 当前 workspace 下的 `.rocode/` 是本地运行时 authority
- 项目配置入口通常是 `rocode.jsonc` / `rocode.json`
- 全局配置默认在 `~/.config/rocode/rocode.jsonc`
- 如果当前 workspace 是 isolated 模式，global config 的修改不会自动作用于当前 sandbox runtime

如果你只想影响当前项目，优先改当前 workspace 的配置和 `.rocode/`，不要先改全局。

## 4. TUI 里会看到什么

### 4.1 Session 与忙碌状态

- 当 session 正在运行时，普通输入不会插入当前 workflow 中间
- `/abort` 走独立取消请求，只取消当前活动执行边界，不会把 `/abort` 文本塞进正在运行的 prompt
- 如果系统需要你回答，它应通过正式 question UI 发起
- scheduler stage transcript 会被投影到主 session，而不是只藏在内部日志里
- 当前 TUI 主会话视图已经迁入 reratui reactive session subtree，滚动、reasoning、sidebar 与消息流体验已按新渲染边界收口

### 4.2 Slash Command

- TUI、CLI、Web 使用统一 slash command 语义
- 命令缺参数时，会走 question / 参数补全链路
- 不再要求每个命令都必须走旧式静态预注册弹窗
- 如果要终止某个已登记的 agent task，用 `/tasks kill <ID>`；`/abort` 针对的是当前会话执行边界

### 4.3 Skill 浏览与 Hub

当前 TUI 已能查看：

- resolved skill catalog
- source index
- remote distributions
- artifact cache
- lifecycle
- governance timeline
- hub policy

写操作也已经在 TUI 里闭环，包括 install / update / detach / remove / sync。

## 5. Web 里会看到什么

当前 Web 以当前 workspace 为中心：

- 左侧是当前 workspace 的 session tree
- sidebar 已支持多选与删除确认，适合批量清理 session
- settings 会显示 workspace mode / workspace root
- `Settings -> Providers -> Model Overrides` 里的 `Provider ID` 现在是稳定的 provider 下拉，不再依赖浏览器对 `datalist` 的兼容性
- skills 面板会显示 managed skill、distribution、artifact cache、lifecycle、timeline
- isolated workspace 模式下会明确提示“当前不会继承 global config”
- composer 现在默认单行起始、最多扩展到 10 行，并提供按 provider 分组、可过滤的 model picker

如果你在 settings 里改的是全局配置，Web 也会提示这些修改是否影响当前 sandbox runtime。

## 6. Skill Hub 使用方式

### 6.1 先看状态

```bash
rocode skill hub status
rocode skill hub managed
rocode skill hub index
rocode skill hub distributions
rocode skill hub artifact-cache
rocode skill hub policy
rocode skill hub lifecycle
```

### 6.2 远程安装

```bash
rocode skill hub install-plan \
  --source-id <id> \
  --source-kind registry \
  --locator <locator> \
  --skill-name <name>
```

真正写入 workspace：

```bash
rocode skill hub install-apply \
  --session-id <session> \
  --source-id <id> \
  --source-kind registry \
  --locator <locator> \
  --skill-name <name>
```

### 6.3 更新 / 解绑 / 删除

```bash
rocode skill hub update-apply --session-id <session> --source-id <id> --source-kind registry --locator <locator> --skill-name <name>
rocode skill hub detach --session-id <session> --source-id <id> --source-kind registry --locator <locator> --skill-name <name>
rocode skill hub remove --session-id <session> --source-id <id> --source-kind registry --locator <locator> --skill-name <name>
```

### 6.4 Policy

当前 artifact policy 已正式可观测：

```bash
rocode skill hub policy
```

它会显示：

- artifact cache retention
- fetch timeout
- max download bytes
- max extract bytes

## 7. Memory 与 Skill 自进化

### 7.1 这不是“只会调用 skill”

ROCode 当前会把复杂回合中的经验继续往前推进，而不是停留在“这次刚好做完”：

- 如果一个回合出现多工具协同、编辑后验证、错误恢复等模式，运行时可能追加 `System suggestion`，提醒你把它沉淀为 skill。
- 如果当前已加载的 skill 与真实执行步骤出现偏离，运行时也可能提示你用 `skill_manage("patch", ...)` 修正它，而不是继续让旧 skill 漂移。
- `skill_manage` 的创建、补丁、文件更新、guard 结果都会被送入 memory observation，供后续验证和归纳。

### 7.2 Memory 具体记什么

Memory 记录不是随手记一句话。当前系统会保留这几类关键信息：

- `evidence_refs`：证据来自哪个 session、message、tool call、stage
- `trigger_conditions`：这条经验在什么条件下才应被召回
- `boundaries`：什么情况下不该复用
- `normalized_facts`：归一化后的事实与约束

它的目标不是替代 session transcript，而是把可复用经验从原始会话里提纯出来。

### 7.3 它如何避免“记错”或“乱注入”

- 新记录先以 candidate 进入系统，不会立刻变成稳定记忆。
- validation 会检查 scope、evidence、trigger、boundary、重复冲突、过期与不安全内容。
- retrieval 只从 `validated` / `consolidated` 记录里取稳定内容，并通过 retrieval preview 解释为什么会注入当前回合。
- consolidation 会把重复或相近记录收束起来，并把反复出现的 lesson 提升为 pattern / methodology candidate。

### 7.4 在哪里看这些信息

TUI 中可以直接使用：

```text
/memory
/memory preview <query>
/memory show <record_id>
/memory validation <record_id>
/memory conflicts <record_id>
/memory rules
/memory hits run=<run_id>
/memory runs
/memory consolidate candidates limit=10
```

Web 当前在 settings drawer 里提供 Memory 视图，可查看 records、retrieval preview、rule packs、rule hits、validation、conflicts 和 consolidation runs。

## 8. MCP / Agent / Debug

### 8.1 MCP

```bash
rocode mcp list
rocode mcp add --help
rocode mcp connect <NAME>
rocode mcp disconnect <NAME>
rocode mcp auth list
rocode mcp debug <NAME>
```

### 8.2 Agent

```bash
rocode agent list
rocode agent create --help
rocode debug agent <NAME>
```

### 8.3 Debug

常用入口：

```bash
rocode debug paths
rocode debug config
rocode debug skills --help
rocode debug docs validate --help
rocode debug lsp --help
```

如果你在排 skill / provider / workspace / docs 问题，`debug` 基本是第一现场。

## 8. 推荐工作流

### 8.1 本地仓库交互开发

```bash
rocode tui
```

适合：

- 边看代码边交互修改
- 需要 session continuity
- 需要 question / timeline / runtime telemetry

### 8.2 脚本与自动化

```bash
rocode run "..." --format json
```

适合：

- CI
- 批处理
- 外部系统调用

### 8.3 长时间服务化

```bash
rocode serve --hostname 127.0.0.1 --port 3000
```

适合：

- Web
- 外部 HTTP 客户端
- 多会话并行观察

## 9. 故障排查

### 9.1 模型或 provider 不对

按这个顺序查：

```bash
rocode auth list
rocode models --refresh --verbose
rocode config
rocode debug paths
```

### 9.2 当前配置和你想的不一样

先看：

```bash
rocode debug paths
rocode debug config
```

重点确认：

- 当前 project root
- 当前 workspace mode
- 是否存在 `.rocode/`
- 当前 runtime 是否继承 global config

### 9.3 Skill Hub 看不到预期状态

按这个顺序查：

```bash
rocode skill hub index
rocode skill hub distributions
rocode skill hub artifact-cache
rocode skill hub lifecycle
rocode skill hub policy
```

如果需要更细：

```bash
rocode debug skills audit
rocode debug skills timeline
```

### 9.4 MCP 连不上

```bash
rocode mcp list
rocode mcp debug <NAME>
rocode mcp auth list
```

### 9.5 LSP / docs 问题

```bash
rocode debug lsp --help
rocode debug docs validate --help
```

## 10. 继续阅读

- 项目总览：[README.md](/home/biocheming/tests/python/rust/rocode/README.md)
- 文档索引：[docs/README.md](/home/biocheming/tests/python/rust/rocode/docs/README.md)
- Scheduler 示例：[docs/examples/scheduler/README.md](/home/biocheming/tests/python/rust/rocode/docs/examples/scheduler/README.md)
- Context Docs：[docs/examples/context_docs/README.md](/home/biocheming/tests/python/rust/rocode/docs/examples/context_docs/README.md)
- 插件 / skill 示例：[docs/examples/plugins_example/README.md](/home/biocheming/tests/python/rust/rocode/docs/examples/plugins_example/README.md)
