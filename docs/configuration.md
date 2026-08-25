# AgenDao 配置指南

这份文档只描述当前 Rust 配置 schema 实际接受的字段。配置文件是严格解析的：未知字段会报错，不会被静默忽略。

## 先做最小配置

如果只是想运行一个 provider，先从下面这个结构开始，再按需增加字段：

```jsonc
{
  "$schema": "https://agendao.dev/schemas/agendao_config.schema.json",
  "model": "my-provider/my-model",
  "provider": {
    "my-provider": {
      "name": "My provider",
      "base_url": "https://api.example.com/v1",
      "npm": "@ai-sdk/openai-compatible",
      "api_style": "openai-compatible",
      "api_shape": "chat-completions",
      "transport": "bearer",
      "models": {
        "my-model": {
          "model": "my-model",
          "tool_call": true,
          "reasoning": true
        }
      }
    }
  }
}
```

密钥不要复制进仓库。优先使用 provider 对应的环境变量或认证存储；直接写入 `api_key` 只适合本机临时配置。

完整的可复制样例见 [`docs/examples/configuration/`](examples/configuration/)。

## 配置文件在哪里

全局配置：

```text
~/.agendao/agendao.json
~/.agendao/agendao.jsonc
```

`AGENDAO_HOME` 可以改变全局目录。项目配置从项目目录向上查找：

1. 远程 well-known 配置（最低优先级）
2. 全局配置
3. `<project>/.agendao/agendao.json[c]`
4. `<project>/agendao.json[c]`（最高优先级）

同名对象深度合并；`instructions` 等数组按项目规则合并。若同时存在 `.json` 和 `.jsonc`，当前加载器优先 `.json`。

JSONC 允许注释；JSON 不允许注释。配置发生变化后不会被后台自动监视，使用 `POST /config/reload` 或重启 Server 才会刷新运行时 provider、tool 和 Agent 状态。

## 命名规则：不要混用两套写法

当前配置 schema 的字段名以代码为准，常见字段如下：

| 正确字段 | 不要写成 |
|---|---|
| `api_key` | `apiKey` |
| `base_url` | `baseURL` |
| `small_model` | `smallModel` |
| `default_agent` | `defaultAgent` |
| `toolImports` | `tool_imports` |
| `uiPreferences` | `ui_preferences` |
| `logLevel` | `log_level` |

顶层字段中确实存在两种历史命名风格，这是 schema 的兼容现状；但 provider 和 model 字段应使用表中的 snake_case。`ProviderConfig` 和 `ModelConfig` 都是 `deny_unknown_fields`，所以拼错会直接导致配置加载失败。

## Provider 配置

`provider` 是“provider ID → provider 配置”的对象，`model` 使用 `provider-id/model-id` 形式最清楚：

```jsonc
{
  "model": "deepseek/deepseek-v4-flash",
  "provider": {
    "deepseek": {
      "name": "DeepSeek",
      "base_url": "https://api.deepseek.com",
      "npm": "@ai-sdk/openai-compatible",
      "api_style": "openai-compatible",
      "api_shape": "chat-completions",
      "transport": "bearer",
      "models": {
        "deepseek-v4-flash": {
          "name": "DeepSeek V4 Flash",
          "model": "deepseek-v4-flash",
          "tool_call": true,
          "reasoning": true
        }
      }
    }
  }
}
```

常用 `ProviderConfig` 字段：

| 字段 | 含义 |
|---|---|
| `name` | 显示名称 |
| `id` | 可选的 provider 标识覆盖 |
| `api_key` | 内联密钥；不建议提交到仓库 |
| `base_url` | API 根地址 |
| `models` | 自定义模型表 |
| `options` | provider 额外运行参数 |
| `npm` | `@ai-sdk/openai`、`@ai-sdk/openai-compatible` 或 `@ai-sdk/anthropic` |
| `api_style` | `openai-responses`、`openai-compatible` 或 `anthropic-compatible` |
| `api_shape` | `responses`、`chat-completions` 或 `messages` |
| `transport` | 通常为 `bearer` |
| `usage_shape` | usage 计费字段解析方式 |
| `env` | 自定义认证环境变量名列表 |
| `whitelist` / `blacklist` | 模型过滤 |

模型常用字段：

```jsonc
{
  "models": {
    "my-model": {
      "model": "vendor-model-id",
      "tool_call": true,
      "reasoning": true,
      "attachment": false,
      "reasoning_effort": "high",
      "timeout_secs": 120,
      "stream_stall_timeout_secs": 120,
      "options": { "temperature": 0.2 },
      "limit": { "context": 128000, "output": 16384 }
    }
  }
}
```

`reasoning_effort` 是统一的用户侧档位，不是所有协议都会原样接受。可选值为
`none`、`minimal`、`low`、`medium`、`high`、`xhigh`、`max`、`ultra`；协议适配器会
根据模型声明的 wire vocabulary 只向下收敛到最近的较弱档位，不会静默升级成本。
TUI/Web 的模型编辑器中选择 `Auto` 会清除模型级覆盖并恢复 provider/model 默认值；
session composer 下方的 effort 选择只覆盖下一次 prompt（选择 `Auto` 则继承模型配置）。
例如 DeepSeek V4 的 `xhigh` 会映射为 `max`，不支持 `minimal` 的 OpenAI reasoning
模型会映射为 `low`。未设置时保持未设置，让 provider 默认行为生效；显式 `none`
只会在协议支持关闭档位时发送。

多模态附件也遵循同一条链路：会话先保存带 MIME、文件名和来源的附件，再由协议适配器
选择 wire 形状。OpenAI Chat Completions 使用 `image_url` 和 data-URL 形式的
`input_audio`；OpenAI Responses 使用 `input_image` / `input_file`；Anthropic Messages
使用原生 `image` / PDF `document` block。协议或模型不支持的音频、视频不会被伪装成
图片或文件，而会产生明确的降级警告（能力预检仍是最终裁决点）。

运行时只承认三类协议：OpenAI Responses、OpenAI Chat Completions、Anthropic Messages。所谓“兼容 provider”必须明确写出 `api_style` / `api_shape`，不要把其他 SDK 的字段名直接粘进来。

启用/禁用 provider：

```jsonc
{
  "enabled_providers": ["deepseek", "ollama"],
  "disabled_providers": ["temporary-test"]
}
```

`enabled_providers` 非空时只启用其中的 provider；`disabled_providers` 永远排除对应 provider。

## Permission：配置规则和 session 模式是两回事

### 全局/Agent 配置中的 `permission`

这里是“权限类型 → allow / ask / deny”的规则表，默认是 `ask`：

```jsonc
{
  "permission": {
    "Read": "allow",
    "Grep": "allow",
    "Edit": "ask",
    "Write": "ask",
    "Bash": "ask",
    "WebFetch": "deny"
  }
}
```

也可以按匹配目标细化：

```jsonc
{
  "permission": {
    "Bash": {
      "ls": "allow",
      "git status": "allow",
      "rm *": "deny"
    }
  }
}
```

规则含义：

- `allow`：匹配时自动放行；
- `deny`：匹配时自动拒绝；
- `ask`：交给 permission engine 和交互弹窗；
- 同一 pattern 命中多条规则时，后合并的匹配规则优先；一次请求包含多个 patterns 时，结果按 `deny > ask > allow` 合并。

### session 弹窗里的选项

这是运行时针对当前请求的决定：

| 选项 | 实际效果 |
|---|---|
| Allow once | 只允许这一次请求 |
| Allow for this turn | 本轮匹配请求自动允许；下一轮清掉 |
| Allow for this session | 本 session 内匹配请求自动允许 |
| Deny this request | 只拒绝当前请求 |
| Trust workspace for this session | 切换到 `trusted_workspace` |
| Full access for this session | 切换到 `unsandboxed_yolo` |

“匹配请求”包括权限类型以及资源/范围/匹配器，不一定等于整个工具永久放行。不同权限类别支持的生命周期也不同：InspectRead 通常只有 once；WorkspaceWrite、ExternalAccess 支持 once / turn / session；DangerousExec 通常只有 once。

permission 弹窗等待用户时没有固定 deadline，不会在 5 分钟后自动拒绝；等待仍可通过 abort/cancel
结束。Scheduler 的 `max_wall_time_ms` 只计算活跃执行时间，不计算这段人机等待。

### session permission mode

session 的 typed mode 不是 `~/.agendao/agendao.json` 顶层 `permission` 的替代品，而是 session 级运行设置：

| mode | 行为 |
|---|---|
| `default` | 不做全局自动放行，继续走已有授权、规则、插件和弹窗；InspectRead 在当前实现中会直接放行 |
| `trusted_workspace` | 自动放行工作区范围内的读取和写入；外部目录、网络和危险执行仍需单独判断 |
| `unsandboxed_yolo` | 在 AgenDao permission 层自动批准所有请求，不再弹 permission 对话框 |

> permission mode 只作用在 **permission 层**。模型可达执行的 **sandbox 物理层**由唯一 `SandboxExecutionBoundary` 决定，不受 permission mode 直接改写：`trusted_workspace` 不放开任何超出 `default` 的 sandbox 宽度；`unsandboxed_yolo` 是**显式退出 sandbox**（经 session 层授权触达 native 通道），不是 "sandbox YOLO"。二者区别见 `sandbox.md`。

它可以通过 session UI，或 API `PATCH /session/{id}/permission` 设置：

```json
{ "mode": "trusted_workspace" }
```

## Agent、Tools、Skills 和 MCP

Agent map 的真实字段是：

```jsonc
{
  "agent": {
    "code": {
      "name": "Code",
      "mode": "primary",
      "model": "deepseek/deepseek-v4-flash",
      "prompt": "You are a careful software engineer.",
      "max_steps": 30,
      "max_tokens": 16384,
      "permission": { "Bash": "ask" },
      "tools": { "Bash": true, "WebSearch": false }
    }
  }
}
```

不要使用旧文档里的 `systemPrompt`、`maxSteps`、`allowedTools`、`topP`；当前 schema 对应的是 `prompt`、`max_steps`、`tools`、`top_p`。

技能路径：

```jsonc
{ "skills": { "paths": ["./.agendao/skills", "~/.agendao/skills"], "disabled": ["old-skill"] } }
```

MCP 是名称到服务器配置的映射；本地服务器使用 `command` 数组，远程服务器使用 `url`。外部脚本工具不要堆进主配置，使用 `toolImports`：

```jsonc
{ "toolImports": ["./tools/catalog.jsonc"] }
```

## 其他常用字段

```jsonc
{
  "theme": "dracula",
  "logLevel": "info",
  "snapshot": true,
  "share": "manual",
  "layout": "auto",
  "instructions": ["Use 4-space indentation."],
  "uiPreferences": {
    "showThinking": true,
    "showToolDetails": true,
    "messageDensity": "comfortable"
  },
  "runtimeBudget": {
    "scheduler_max_model_calls": 32,
    "scheduler_max_tool_calls": 96,
    "scheduler_max_total_tokens": 1048576
  }
}
```

高级字段（formatter、lsp、webSearch、multimodal、externalAdapter、enterprise、compaction、experimental 等）请以 [schema](agendao_config.schema.json) 为准；不要从旧版本或其他项目的配置中整块复制。

## 验证与排错

先验证 JSON/JSONC 是否能被当前二进制加载：

```bash
agendao config
```

检查运行时策略：

```bash
agendao config validation
```

provider 能否真正工作，至少要同时满足：字段能解析、provider profile 合法、认证存在、endpoint 可达、模型 ID 正确。`agendao config` 成功只证明“配置可解析”，不证明远端模型可用。

如果出现 `unknown field apiKey` 或 `unknown field baseURL`，先改成 `api_key` / `base_url`；如果 provider 连不上，先单独检查 DNS、密钥、endpoint 和模型 ID，不要靠继续堆字段“试出来”。

## 相关入口

- [配置 examples](examples/configuration/)
- [认证](auth.md)
- [Agent](agents.md)
- [外部工具 examples](examples/tools/README.md)
- [配置 schema](agendao_config.schema.json)
# Sandbox deployment environment

`AGENDAO_SANDBOX_ENV_ALLOW_EXACT` is an optional comma-separated list of exact
environment variable names that the deployment administrator has confirmed are
false positives for sandbox secret-name heuristics. It does not override the
hard denylist: AgenDao internal tokens and passwords remain unavailable even if
listed. The default is empty.
