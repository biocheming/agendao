# AgenDao 配置参考

AgenDao 通过分层的 JSON/JSONC 配置系统进行配置。本文档描述 `agendao.jsonc` / `agendao.json` 中的配置选项。

---

## 配置文件位置

### 全局配置

```
~/.config/agendao/agendao.jsonc
~/.config/agendao/agendao.json
```

如果不存在，AgenDao 会在首次运行时使用默认值。

### 项目级配置

AgenDao 从项目目录向上查找配置文件，按以下优先级加载（后者覆盖前者）：

| 来源 | 路径 | 优先级 |
|------|------|--------|
| 远程 well-known | `{url}/.well-known/opencode` | 最低 |
| 全局配置 | `~/.config/agendao/agendao.jsonc` / `~/.config/agendao/agendao.json` | 中 |
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
  "smallModel": "qwen3.6-plus",
  "defaultAgent": "code",
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
  "composition": { ... },
  "provider": { ... },
  "mcp": { ... },
  "formatter": { ... },
  "lsp": { ... },
  "uiPreferences": { ... },
  "permission": { ... },
  "tools": { ... },
  "webSearch": { ... },
  "enterprise": { ... },
  "compaction": { ... },
  "experimental": { ... },
  "env": { ... },

  "disabledProviders": [],
  "enabledProviders": [],
  "instructions": [],
  "schedulerPath": null,
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
| `smallModel` | string | null | 小型模型 ID，用于轻量任务（摘要、路由） |
| `defaultAgent` | string | null | 默认 Agent 名称 |
| `username` | string | null | 显示在 TUI 中的用户名 |
| `layout` | string | `"auto"` | 布局模式。可选 `"auto"`、`"stretch"` |
| `snapshot` | boolean | null | 启用文件快照（用于 diff 和回退） |
| `share` | string | null | 分享模式。可选 `"manual"`、`"auto"`、`"disabled"` |
| `autoshare` | boolean | null | 自动分享会话 |
| `autoupdate` | boolean 或 string | null | 自动更新。`true` 启用，`false` 禁用，`"notify"` 仅通知 |
| `schedulerPath` | string | null | 调度器配置文件路径（相对于项目根） |
| `taskCategoryPath` | string | null | 任务分类配置路径 |
| `toolImports` | string[] | `[]` | 外部 tool catalog 文件导入列表。支持相对于声明该配置文件的相对路径，也支持绝对路径 |

### 外部 Tool Catalog 导入

当外部工具很多时，不建议把所有工具定义直接堆进主 `agendao.json[c]`。推荐做法是：

```jsonc
{
  "toolImports": [
    "./tools/cadd/tools.jsonc",
    "~/.config/agendao/tools/lab/tools.jsonc"
  ]
}
```

被导入的 `tools.jsonc` 文件用于承载外部工具清单。当前配置层支持：

- 主配置声明导入文件路径
- 相对路径按“声明该字段的配置文件所在目录”解析
- 多个导入文件按配置加载顺序合并
- 外部 tool catalog 文件中记录 `source` 与 `catalog` 元数据
- 外部 tool 显式分为两类：
  - `catalog-only`：只有发现/分类能力，没有执行声明
  - `executable`：必须提供 `execution` 块

当前版本这一步只完成了**配置层设计与解析能力**。运行时 registry 接线可以在后续阶段再做。

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

#### 目录推断规则

如果 `catalog.domain` / `catalog.family` 没有显式填写，配置层会尝试从 `tools/<domain>/<family>/...` 目录结构做保守推断。例如：

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
    "zhipuai": {
      "name": "Zhipu AI",
      "apiKey": "zhipu-...",
      "whitelist": ["glm-5.1"]
    },
    "alibaba-cn": {
      "name": "Alibaba Cloud Bailian",
      "models": {
        "qwen3.6-plus": {
          "toolCall": true,
          "reasoning": true,
          "limit": { "context": 128000, "output": 16384 }
        }
      }
    },
    "kimi-for-coding": {
      "name": "Moonshot Kimi",
      "whitelist": ["kimi-k2.5"]
    },
    "ollama": {
      "name": "Ollama",
      "baseURL": "http://localhost:11434"
    }
  }
}
```

### ProviderConfig 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `name` | string | Provider 显示名称 |
| `id` | string | Provider 标识符 |
| `apiKey` | string | API 密钥（别名：`apikey`） |
| `baseURL` | string | API 基础 URL（别名：`baseUrl`、`api`） |
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
| `model` | string | 模型 API 标识符（别名：`id`） |
| `apiKey` | string | 模型级别 API 密钥 |
| `baseURL` | string | 模型级别 API 基础 URL |
| `toolCall` | boolean | 是否支持工具调用（别名：`tools`） |
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
| `releaseDate` | string | 发布日期 |
| `provider` | object | 模型级别 Provider 配置 |

`cost` 子字段：`input`、`output`（每百万 Token 美元价格），可选 `cacheRead`、`cacheWrite`。

`limit` 子字段：`context`（上下文窗口）、`output`（最大输出 Token），可选 `input`。

### Provider 启用/禁用

```jsonc
{
  "disabledProviders": ["groq", "cerebras"],
  "enabledProviders": ["zhipuai", "alibaba-cn", "kimi-for-coding"]
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
      "maxSteps": 30, "color": "cyan",
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
| `topP` | float | Top-p 采样参数 |
| `prompt` | string | 系统 prompt 前缀 |
| `disable` | boolean | 禁用此 Agent |
| `description` | string | Agent 描述 |
| `mode` | string | Agent 模式：`"primary"`、`"subagent"`、`"all"` |
| `hidden` | boolean | 是否在自动补全中隐藏 |
| `options` | object | Agent 级别额外选项 |
| `color` | string | ANSI 显示颜色 |
| `steps` | integer | 最大步数 |
| `maxSteps` | integer | 最大步数（别名） |
| `maxTokens` | integer | 最大输出 Token |
| `permission` | object | 工具权限规则（见 PermissionConfig） |
| `tools` | object | 工具启用/禁用映射 |

### Agent Markdown 文件

在 `.agendao/agents/` 目录放置 Markdown 文件定义 Agent，YAML frontmatter 支持 `name`、`description`、`mode`、`model` 等字段，正文作为 prompt。

CLI 创建：`agendao agent create <name> --description "..." --mode subagent`。

---

## Composition 配置（Skill Tree）

```jsonc
{
  "composition": {
    "skillTree": {
      "enabled": true, "separator": "/", "tokenBudget": 4000,
      "truncationStrategy": "priority",
      "root": {
        "nodeId": "root", "markdownPath": "./docs/skills/root.md",
        "children": [
          { "nodeId": "arch", "markdownPath": "./docs/skills/arch.md" }
        ]
      }
    }
  }
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `enabled` | boolean | 启用 Skill Tree |
| `root` | object | 根节点（递归 `nodeId`、`markdownPath`、`children`） |
| `separator` | string | 节点路径分隔符 |
| `tokenBudget` | integer | Token 预算 |
| `truncationStrategy` | string | 截断策略 |

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

当前真实接通的插件运行面是 `npm`、`file`、`dylib`。

`pip` / `cargo` 字段仍然保留在 schema 里，用于兼容和未来扩展，但今天的自动加载器不会把它们转成可执行 loader spec，所以不要把它们当成已经接通的用户插件主路径。

```jsonc
{
  "plugin": {
    "my-npm": { "type": "npm", "package": "@scope/plugin", "version": ">=1.0" },
    "my-local": { "type": "file", "path": "./plugins/p.ts" },
    "my-native": { "type": "dylib", "path": "./plugins/libp.so" }
  }
}
```

也支持旧版数组格式 `["pkg@ver", "file://./plugins/my-plugin.ts"]`。

### PluginConfig 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `type` | string | 当前真实运行面：`"npm"`、`"file"`、`"dylib"`；兼容 / 预留字段：`"pip"`、`"cargo"` |
| `package` | string | 包名 |
| `version` | string | 版本约束 |
| `path` | string | 文件路径（`file` 或 `dylib`） |
| `runtime` | string | 运行时覆盖（如 `"python3.11"`） |
| `options` | object | 插件特定选项 |

自动发现路径：`~/.config/agendao/plugins/`、`~/.agendao/plugins/`、`<project>/.agendao/plugins/`，以及 `pluginPaths` 中配置的自定义路径。

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
| `subtask` | 作为子任务执行 |

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
| `recentModels` | 最近使用的模型列表 `[{provider, model}]` |
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
| `baseUrl` | null | MCP 搜索端点 URL（如 `https://mcp.exa.ai`） |
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

调度器通过外部 JSON/JSONC 文件配置，在 `agendao.jsonc` 中引用：

```jsonc
{
  "schedulerPath": "./scheduler.jsonc"
}
```

### Scheduler Profile 结构

每个 profile 包含 `orchestrator`（preset 名）、`stages`（阶段列表）、`agentTree`（agent 树）和 `skillTree`（知识树）。stages 可以是字符串或带 override 的对象。`agentTree` 支持内联对象或外部文件路径。详见 [Scheduler 示例](examples/scheduler/README)。

当前 checked-in scheduler 示例不再平铺在一个目录里，而是按语义分组：

- `examples/scheduler/presets/`
  - `sisyphus`、`prometheus`、`atlas`、`hephaestus` 这些公开内置 preset
- `examples/scheduler/verifier/`
  - verifier 的最小配置、完整 profile 和外置 workflow
- `examples/scheduler/pso/`
  - PSO 这类用户自定义 topology
- `examples/scheduler/autoresearch/`
  - 嵌入 scheduler profile 的 workflow 级 autoresearch 示例

如果你从仓库里的示例起步，推荐把对应目录复制到项目内，再让 `schedulerPath` 指向项目里的副本。因为这些示例经常依赖同目录下的相对路径文件，比如 `workflowPath`、`agentTree` 或 `trees/`。

---

## 参见

- [认证](auth) -- API 密钥和多 Provider 配置
- [安装指南](installation) -- 构建和环境设置
- [Scheduler 示例](examples/scheduler/README) -- 调度器 presets / verifier / pso / autoresearch 示例入口
- [Scheduler 指南](examples/scheduler/SCHEDULER_GUIDE) -- 完整调度器使用教程
