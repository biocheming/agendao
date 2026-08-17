# AgenDao 配置参考

AgenDao 通过分层的 JSON/JSONC 配置系统进行配置。本文档描述 `agendao.jsonc` / `agendao.json` 中的配置选项。

---

## 配置文件位置

### 全局配置

```
~/.agendao/agendao.jsonc
~/.agendao/agendao.json
```

如果不存在，AgenDao 会在首次运行时使用默认值。

全局配置与数据统一收在 `~/.agendao`（可用 `AGENDAO_HOME` 覆盖）。

### 项目级配置

AgenDao 从项目目录向上查找配置文件，按以下优先级加载（后者覆盖前者）：

| 来源 | 路径 | 优先级 |
|------|------|--------|
| 远程 well-known | `{url}/.well-known/opencode` | 最低 |
| 全局配置 | `~/.agendao/agendao.jsonc` / `~/.agendao/agendao.json` | 中 |
| 项目 `.agendao` 目录 | `<project>/.agendao/agendao.jsonc` / `<project>/.agendao/agendao.json` | 高 |
| 项目根目录 | `<project>/agendao.jsonc` / `<project>/agendao.json` | 最高 |

此外还支持企业管理的配置目录：

- macOS: `/Library/Application Support/agendao`
- Linux: `/etc/agendao`
- Windows: `%ProgramData%\agendao`

可通过 `AGENDAO_CONFIG_DIR` 环境变量覆盖。

### 配置合并

合并策略为深度合并（deep merge）：

1. 远程 well-known 配置作为基础
2. 全局配置覆盖
3. 项目配置覆盖
4. 项目根配置覆盖

数组类型字段（如 `instructions`）为拼接而非覆盖。

### 启动与运行期重载

配置文件及 Markdown Agent/Command frontmatter 是启动契约。任一层解析失败时，Server、Web 和 TUI 的本地 Server 都会明确启动失败，不会静默退回默认配置。

运行期间不会监视磁盘并隐式改写当前配置。外部编辑配置文件后，必须调用 `POST /config/reload`；这是唯一的磁盘重载入口。重载会先完整解析所有配置层：解析失败时返回 `400` 并保留当前配置；成功后替换配置快照，重建 provider、tool、Agent 和执行模式派生状态，并广播 `config.updated`，Web 与 TUI 随后刷新各自的配置视图。已经开始的 Agent 回合继续使用启动该回合时捕获的执行上下文，新回合使用重载后的配置。

---

## Memory 边界与 Workspace 作用域

当前版本没有单独暴露一个顶层 `memory` 配置块，但 memory 行为并不是无约束默认值。它直接受运行时 workspace authority 影响：

- 当前 workspace root 与 `.agendao/` 决定 memory 的本地身份边界
- shared / isolated workspace mode 会影响允许使用的 memory scope
- retrieval preview、validation、consolidation 都在当前 workspace identity 下进行，不会把别的工作区记录不加区分地注入当前回合

这意味着 memory 的正确打开方式不是“把所有经验堆在一起”，而是：

- 在正确的 workspace 中运行
- 明确当前是 shared 还是 isolated
- 让记录带着 evidence、trigger、boundary 与 workspace identity 进入系统

当前运行时只会把经过 validation / consolidation 的稳定记录用于正式检索注入；candidate 更像待裁决草稿，而不是默认启用的长期记忆。

---

## 顶层结构

```jsonc
{
  "$schema": "https://agendao.dev/schemas/...",
  "theme": "dracula",
  "logLevel": "warn",
  "model": "glm-5.1",
  "small_model": "qwen3.6-plus",
  "default_agent": "code",
  "username": "dev",
  "layout": "auto",
  "snapshot": true,
  "share": "manual",
  "autoshare": false,
  "autoupdate": "notify",

  "keybinds": { ... },
  "tui": { ... },
  "server": { ... },
  "command": { ... },
  "skills": { ... },
  "docs": { ... },
  "watcher": { ... },
  "plugin": { ... },
  "toolImports": [],
  "agent": { ... },
  "mode": { ... },
  "provider": { ... },
  "mcp": { ... },
  "formatter": { ... },
  "lsp": { ... },
  "uiPreferences": { ... },
  "permission": { ... },
  "runtimeBudget": { ... },
  "tools": { ... },
  "webSearch": { ... },
  "enterprise": { ... },
  "compaction": { ... },
  "experimental": { ... },
  "env": { ... },

  "disabledProviders": [],
  "enabledProviders": [],
  "instructions": [],
  "taskCategoryPath": null,
  "skillPaths": {},
  "pluginPaths": {}
}
```

---

## 顶层字段

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `$schema` | string | null | JSON Schema URI，用于编辑器自动补全 |
| `theme` | string | null | TUI 主题名称。内置主题可通过 `Theme::builtin_theme_names()` 查看，格式为 `name@dark` 或 `name@light` |
| `logLevel` | string | `"warn"` | 日志级别。可选 `trace`、`debug`、`info`、`warn`、`error`。也可通过 `RUST_LOG` 环境变量设置 |
| `model` | string | null | 默认模型 ID。如 `glm-5.1`、`qwen3.6-plus`、`kimi-k2.5` |
| `small_model` | string | null | 小型模型 ID，用于轻量任务（摘要、路由） |
| `default_agent` | string | null | 默认 Agent 名称 |
| `username` | string | null | 显示在 TUI 中的用户名 |
| `layout` | string | `"auto"` | 布局模式。可选 `"auto"`、`"stretch"` |
| `snapshot` | boolean | null | 启用文件快照（用于 diff 和回退） |
| `share` | string | null | 分享模式。可选 `"manual"`、`"auto"`、`"disabled"` |
| `autoshare` | boolean | null | 自动分享会话 |
| `autoupdate` | boolean 或 string | null | 自动更新。`true` 启用，`false` 禁用，`"notify"` 仅通知 |
| `taskCategoryPath` | string | null | 任务分类配置路径 |
| `toolImports` | string[] | `[]` | 外部 tool catalog 文件导入列表。支持相对于声明该配置文件的相对路径，也支持绝对路径 |

### Scheduler 运行预算

`runtimeBudget` 是 Scheduler 与其他运行时资源限制的唯一配置入口。Scheduler 相关字段及默认值：

```jsonc
{
  "runtimeBudget": {
    "scheduler_max_model_calls": 32,
    "scheduler_max_tool_calls": 96,
    "scheduler_max_total_tokens": 1048576,
    "scheduler_max_wall_time_ms": 1800000,
    "scheduler_max_parallelism": 4,
    "scheduler_max_graph_nodes": 48,
    "scheduler_max_graph_depth": 16,
    "scheduler_max_loop_iterations": 6,
    "scheduler_max_agent_steps": 16,
    "scheduler_workspace_max_files": 10000,
    "scheduler_workspace_max_total_bytes": 1073741824,
    "scheduler_workspace_min_free_disk_bytes": 536870912,
    "scheduler_workspace_operation_timeout_ms": 30000
  }
}
```

模型请求中的 token 和 timeout 配置只能把对应 Scheduler 上限收紧，不能越过这里的硬预算。

### 外部 Tool Catalog 导入

当外部工具很多时，不建议把所有工具定义直接堆进主 `agendao.json[c]`。推荐做法是：

```jsonc
{
  "toolImports": [
    "./tools/cadd/tools.jsonc",
    "~/.agendao/tools/lab/tools.jsonc"
  ]
}
```

被导入的 `tools.jsonc` 文件用于承载外部工具清单。当前版本支持：

- 主配置声明导入文件路径
- 相对路径按“声明该字段的配置文件所在目录”解析
- 多个导入文件按配置加载顺序合并
- 外部 tool catalog 文件中记录 `source` 与 `catalog` 元数据
- 外部 tool 显式分为两类：
  - `catalog-only`：只有发现/分类能力，没有执行声明
  - `executable`：必须提供 `execution` 块
- `capability` 的 `search` / `describe` action 会把导入的外部 tools 暴露给模型
- `capability` 的 `call` action 只对声明了 `execution` 的外部 tool 提供执行接线
- `catalog-only` tool 会继续保留为“可发现、可描述、不可执行”

#### tools.jsonc 示例

```jsonc
{
  "tools": {
    "dock_pose": {
      "source": {
        "path": "./cadd/molecular_docking/dock_pose.py"
      },
      "catalog": {
        "domain": "cadd",
        "family": "molecular_docking",
        "subfamily": "protein_ligand"
      },
      "execution": {
        "kind": "script_runner",
        "entry": "./runners/dock_pose.py",
        "arguments_schema_ref": "./schemas/dock_pose.schema.json"
      }
    }
  }
}
```

#### 执行声明规则

- 没有 `execution`：按 `catalog-only` 处理
- 有 `execution`：按 `executable` 处理
- 第一版 `execution.kind` 只接受 `script_runner`
- `execution.entry` 是必填项
- `execution.entry` / `execution.arguments_schema_ref` 都按 `tools.jsonc` 所在目录解析相对路径
- `script_runner` 目前通过 `capability` 的 `call` action 运行

#### 目录推断规则

如果 `catalog.domain` / `catalog.family` / `catalog.subfamily` 中有缺失项，配置层会尝试从 `tools/<domain>/<family>/<subfamily>/...` 目录结构保守补齐缺失层级。例如：

- `tools/cadd/molecular_docking/dock_pose.py`
  - 推断 `domain = cadd`
  - 推断 `family = molecular_docking`

显式 `catalog` 字段优先于目录推断。

---

## Provider 配置

`provider` 字段是一个 Provider ID 到配置的映射。每个 Provider 可以包含自定义模型列表、API 密钥、base URL 等。

```jsonc
{
  "provider": {
    "openai": {
      "name": "OpenAI",
      "api_key": "sk-...",
      "models": {
        "gpt-5": {
          "tool_call": true,
          "reasoning": true,
          "limit": { "context": 128000, "output": 16384 }
        }
      }
    },
    "anthropic": {
      "name": "Anthropic",
      "api_key": "sk-ant-..."
    }
  }
}
```

运行时只实现三种 API shape：OpenAI Responses、OpenAI Chat Completions 和 Anthropic
Messages。OpenAI-compatible endpoint 可以配置自定义 `base_url`，但不会启用 Google、Bedrock、
Vertex、Copilot 或 GitLab 的专用协议。

### ProviderConfig 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `name` | string | Provider 显示名称 |
| `id` | string | Provider 标识符 |
| `api_key` | string | API 密钥 |
| `base_url` | string | API 基础 URL |
| `models` | object | 自定义模型定义（见 ModelConfig） |
| `options` | object | Provider 级别的额外选项 |
| `npm` | string | 对应的 npm 包名 |
| `env` | string[] | 用于认证的环境变量名列表 |
| `whitelist` | string[] | 模型白名单（非空时只提供列表中的模型） |
| `blacklist` | string[] | 模型黑名单（永远不提供列表中的模型） |

### ModelConfig 字段

在 `provider.<id>.models.<modelId>` 中定义单个模型的配置：

| 字段 | 类型 | 说明 |
|------|------|------|
| `name` | string | 模型显示名称 |
| `model` | string | 模型 API 标识符 |
| `base_url` | string | 模型级别 API 基础 URL |
| `tool_call` | boolean | 是否支持工具调用 |
| `reasoning` | boolean | 是否支持推理 |
| `attachment` | boolean | 是否支持附件 |
| `temperature` | boolean | 是否支持温度参数 |
| `interleaved` | boolean 或 object | 交错模式支持 |
| `variants` | object | 模型变体（如不同推理等级） |
| `cost` | object | 定价信息（见 ModelCostConfig） |
| `limit` | object | 限制信息（见 ModelLimitConfig） |
| `modalities` | object | 支持的模态 |
| `headers` | object | 自定义请求头 |
| `family` | string | 模型家族 |
| `status` | string | 模型状态 |
| `release_date` | string | 发布日期 |
| `provider` | object | 模型级别 Provider 配置 |

`cost` 子字段：`input`、`output`（每百万 Token 美元价格），可选 `cache_read`、`cache_write`。

`limit` 子字段：`context`（上下文窗口）、`output`（最大输出 Token），可选 `input`。

### Provider 启用/禁用

```jsonc
{
  "disabledProviders": ["internal-test"],
  "enabledProviders": ["openai", "anthropic"]
}
```

- `enabledProviders` 如果非空，只有列表中的 Provider 会被激活
- `disabledProviders` 始终排除指定 Provider

---

## Agent 配置

Agent 定义在 `agent` 字段中，也可以从 `.agendao/agent/` 或 `.agendao/agents/` 目录中的 Markdown 文件加载。`mode` 字段类似，但自动设置 `mode: "primary"`，从 `.agendao/modes/` 加载。

```jsonc
{
  "agent": {
    "code": {
      "name": "Code", "model": "glm-5.1",
      "mode": "primary", "temperature": 0.3,
      "max_steps": 30, "color": "cyan",
      "prompt": "You are an expert software engineer."
    }
  }
}
```

### AgentConfig 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `name` | string | Agent 显示名称 |
| `model` | string | 使用的模型 ID |
| `variant` | string | 模型变体 |
| `temperature` | float | 采样温度 |
| `top_p` | float | Top-p 采样参数 |
| `prompt` | string | 系统 prompt 前缀 |
| `disable` | boolean | 禁用此 Agent |
| `description` | string | Agent 描述 |
| `mode` | string | Agent 模式：`"primary"`、`"subagent"`、`"all"` |
| `hidden` | boolean | 是否在自动补全中隐藏 |
| `options` | object | Agent 级别额外选项 |
| `color` | string | ANSI 显示颜色 |
| `steps` | integer | leaf AgentLoop 最大步数（`max_steps` 的同义配置字段） |
| `max_steps` | integer | leaf AgentLoop 最大步数；不能扩大 Blueprint 或 runtimeBudget 上限 |
| `max_tokens` | integer | 最大输出 Token |
| `permission` | object | 工具权限规则（见 PermissionConfig） |
| `tools` | object | 工具启用/禁用映射 |

### Agent Markdown 文件

在 `.agendao/agents/` 目录放置 Markdown 文件定义 Agent，YAML frontmatter 支持 `name`、`description`、`mode`、`model` 等字段，正文作为 prompt。

CLI 创建：`agendao agent create <name> --description "..." --mode subagent`。

---

## Skills 配置

```jsonc
{
  "skills": {
    "paths": ["./skills", "~/.agendao/skills"],
    "urls": ["https://skills.example.com/index.json"],
    "hub": {
      "artifactCacheRetentionSeconds": 604800, "fetchTimeoutMs": 30000,
      "maxDownloadBytes": 8388608, "maxExtractBytes": 8388608
    }
  }
}
```

| 字段 | 说明 |
|------|------|
| `paths` | 本地技能搜索路径 |
| `urls` | 远程技能索引 URL |
| `hub.artifactCacheRetentionSeconds` | Artifact 缓存保留时间（默认 604800 秒 / 7 天） |
| `hub.fetchTimeoutMs` | 获取超时（默认 30000 毫秒） |
| `hub.maxDownloadBytes` | 最大下载字节（默认 8 MB） |
| `hub.maxExtractBytes` | 最大解压字节（默认 8 MB） |

---

## MCP 服务器配置

```jsonc
{
  "mcp": {
    "filesystem": {
      "command": ["mcp-server-filesystem", "/home/user/projects"],
      "enabled": true, "timeout": 30000
    },
    "remote-server": {
      "url": "https://mcp.example.com/sse",
      "headers": { "Authorization": "Bearer ..." },
      "oauth": { "clientId": "my-id", "scope": "read" }
    },
    "disabled-server": { "enabled": false }
  }
}
```

本地服务器字段：`command`（命令数组）、`environment`/`env`（环境变量）、`enabled`、`timeout`。

远程服务器字段：`url`、`headers`、`enabled`、`timeout`、`oauth`（含 `clientId`、`clientSecret`、`scope`；设为 `false` 禁用 OAuth 自动检测）。

CLI：`agendao mcp add <name> --command <cmd>`、`agendao mcp add <name> --url <url>`、`agendao mcp list/connect/disconnect`。

---

## Plugin 配置

插件运行面只包含 `npm`、`file`、`dylib`。

```jsonc
{
  "plugin": {
    "my-npm": { "type": "npm", "package": "@scope/plugin", "version": ">=1.0" },
    "my-local": { "type": "file", "path": "./plugins/p.ts" },
    "my-native": { "type": "dylib", "path": "./plugins/libp.so" }
  }
}
```

### PluginConfig 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `type` | string | `"npm"`、`"file"` 或 `"dylib"` |
| `package` | string | 包名 |
| `version` | string | 版本约束 |
| `path` | string | 文件路径（`file` 或 `dylib`） |
| `runtime` | string | 运行时覆盖（如 `"python3.11"`） |
| `options` | object | 插件特定选项 |

自动发现路径：`~/.agendao/plugins/`、`<project>/.agendao/plugins/`，以及 `plugin_paths` 中配置的显式路径。

如果你要看一张更硬的“插件类型 -> 是否真实可用 -> hook 面”矩阵，见 [plugins-capability-matrix](plugins-capability-matrix)。

---

## 自定义命令

```jsonc
{
  "command": {
    "review": {
      "template": "Review this code: $ARGUMENTS",
      "description": "Review code", "model": "qwen3.6-plus", "agent": "review"
    }
  }
}
```

| 字段 | 说明 |
|------|------|
| `template` | 模板字符串，`$ARGUMENTS` 被用户输入替换 |
| `description` | 命令描述 |
| `model` | 模型覆盖 |
| `agent` | Agent 覆盖 |

也可从 `.agendao/command/` 或 `.agendao/commands/` 中的 Markdown 文件加载。

---

## TUI 配置

| 字段 | 说明 |
|------|------|
| `sidebar` | 显示侧边栏 |
| `scrollSpeed` | 滚动速度 |
| `scrollAcceleration.enabled` | 滚动加速 |
| `diffStyle` | Diff 显示样式 |

当前 TUI 的主要显示偏好已经收敛到 `uiPreferences`，例如 `showHeader`、`showScrollbar`、`showThinking`、`messageDensity`、`semanticHighlight`。`compact` 现在是消息密度和内容压缩语义，不再是旧的交互前端模式切换。

---

## Server 配置

| 字段 | 默认值 | 说明 |
|------|--------|------|
| `port` | 0（随机） | HTTP 服务端口 |
| `hostname` | `"127.0.0.1"` | 监听地址 |
| `mdns` | false | 启用 mDNS 服务发现 |
| `mdnsDomain` | `"agendao.local"` | mDNS 域名 |
| `cors` | [] | CORS 允许的源列表 |

---

## 键位绑定

`keybinds` 字段包含 60+ 可配置项。常用示例：

```jsonc
{ "keybinds": {
    "leader": "ctrl+s", "appExit": "ctrl+q",
    "inputSubmit": "enter", "inputNewline": "alt+enter",
    "sessionNew": "ctrl+n", "compact": "ctrl+k",
    "modelList": "ctrl+m", "agentList": "ctrl+a"
} }
```

涵盖：应用控制、输入编辑（光标/选择/删除/撤销）、消息导航（翻页/跳转）、会话管理、模型/Agent 切换、TUI 控制（侧边栏/滚动条/主题）。

---

## UI 偏好

| 字段 | 说明 |
|------|------|
| `theme` | TUI 主题 |
| `webTheme` | Web 界面主题 |
| `webMode` | Web 界面模式 |
| `showHeader` | 显示消息头 |
| `showScrollbar` | 显示滚动条 |
| `showTimestamps` | 显示时间戳 |
| `showThinking` | 显示推理过程 |
| `showToolDetails` | 显示工具调用详情 |
| `messageDensity` | 消息密度 |
| `semanticHighlight` | 语义高亮 |
| `tipsHidden` | 隐藏提示 |

---

## 权限配置

每个工具可设置为 `"ask"`（询问）、`"allow"`（允许）或 `"deny"`（禁止）。支持细粒度子规则：

```jsonc
{
  "permission": { "Bash": "ask", "Edit": "allow", "Write": "allow", "Read": "allow" }
}
```

子规则映射：

```jsonc
{ "permission": { "Bash": { "ls": "allow", "rm": "deny" } } }
```

### 工具启用/禁用

`tools` 字段：`{ "Bash": true, "WebSearch": false }`。

---

## Formatter 配置

格式化器在文件写入后自动运行。设为 `false` 禁用所有格式化器。

```jsonc
{
  "formatter": {
    "prettier": { "command": ["prettier", "--write"], "extensions": [".ts", ".tsx"], "disabled": false }
  }
}
```

| 字段 | 说明 |
|------|------|
| `command` | 命令数组（文件名追加为最后参数） |
| `extensions` | 处理的文件扩展名（含前导点） |
| `disabled` | 临时禁用 |
| `environment` | 环境变量 |

---

## LSP 配置

设为 `false` 禁用所有 LSP。

```jsonc
{
  "lsp": {
    "rust-analyzer": {
      "command": ["rust-analyzer"], "extensions": [".rs"],
      "initialization": { "checkOnSave": { "command": "clippy" } }
    }
  }
}
```

| 字段 | 说明 |
|------|------|
| `command` | LSP 服务器启动命令 |
| `extensions` | 关联的文件扩展名 |
| `disabled` | 禁用此服务器 |
| `env` | 环境变量 |
| `initialization` | LSP 初始化选项 |

---

## Web Search 配置

| 字段 | 默认值 | 说明 |
|------|--------|------|
| `base_url` | null | MCP 搜索端点 URL（如 `https://mcp.exa.ai`） |
| `endpoint` | `"/mcp"` | URL 路径 |
| `method` | `"web_search_exa"` | MCP 工具方法名 |
| `defaultSearchType` | null | `"auto"`、`"fast"`、`"deep"` |
| `defaultNumResults` | 8 | 默认结果数量 |
| `options` | null | 传递给 MCP 的额外参数 |

---

## Compaction 配置

| 字段 | 说明 |
|------|------|
| `auto` | 自动压缩上下文 |
| `prune` | 压缩时修剪历史 |
| `reserved` | 预留 Token 数量 |

---

## Watcher 配置

`watcher.ignore`：文件监视忽略列表（如 `["node_modules", ".git", "target"]`）。

## Enterprise 配置

`enterprise.url`：企业服务器 URL。`enterprise.managedConfigDir`：托管配置目录路径。

## Experimental 配置

| 字段 | 说明 |
|------|------|
| `disablePasteSummary` | 禁用粘贴内容摘要 |
| `batchTool` | 启用批量工具调用 |
| `openTelemetry` | 启用 OpenTelemetry 遥测 |
| `primaryTools` | 主工具列表 `["Bash", "Edit", ...]` |
| `continueLoopOnDeny` | 工具被拒绝后继续循环 |
| `mcpTimeout` | MCP 调用默认超时（毫秒） |

---

## 环境变量注入

`env` 字段：键值对映射，注入到所有工具执行中。

## 指令注入

`instructions` 字段：字符串数组，拼接到系统 prompt 中。如 `["Always use 4-space indentation."]`。

---

## Scheduler 配置

Scheduler 不从 `agendao.jsonc` 加载路径。session 创建和 prompt 请求直接携带统一的
`SchedulerChoice`：

```json
{ "scheduler": { "kind": "auto" } }
```

`kind` 可以是 `auto`、`template` 或 `blueprint`。`blueprint` 直接内联当前
`SchedulerBlueprint` schema；不存在路径加载、旧 profile 转换或失败回退。详见
[Scheduler](scheduler) 和 [当前示例](examples/scheduler/README)。

请求仅携带 `agent` 而没有 `scheduler` 时，会生成以该 Agent 为 primary leaf 的 `direct`
Blueprint；这不是另一条执行路径。Web 的 Session Insights 也可以通过 session Blueprint API 管理
当前锁定图，AI Planner 生成的临时 Agent manifest 随图保存，不写回 `agent` 配置。

---

## 参见

- [认证](auth) -- API 密钥和多 Provider 配置
- [安装指南](installation) -- 构建和环境设置
- [Scheduler](scheduler) -- 统一 Blueprint、模板、auto selector 和执行边界
- [Scheduler 示例](examples/scheduler/README) -- 当前内联 Blueprint 示例
