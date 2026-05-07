# Model Context Protocol (MCP)

Model Context Protocol 是一个基于 JSON-RPC 2.0 的协议，用于将 ROCode 连接到外部工具、资源和提示词服务器。MCP 服务器扩展了智能体的能力范围 -- 无需修改 ROCode 本身即可暴露文件系统、数据库、API、浏览器自动化等能力。

---

## 目录

1. [MCP 概念](#mcp-概念)
2. [传输方式](#传输方式)
3. [配置 MCP 服务器](#配置-mcp-服务器)
4. [配置字段参考](#配置字段参考)
5. [环境变量展开](#环境变量展开)
6. [CLI 命令](#cli-命令)
7. [OAuth 认证](#oauth-认证)
8. [MCP 工具在会话中的使用](#mcp-工具在会话中的使用)
9. [重连机制](#重连机制)
10. [常用 MCP 服务器示例](#常用-mcp-服务器示例)

---

## MCP 概念

MCP 定义了服务器可以提供的三种原语：

- **工具 (Tools)** -- 模型可以调用的函数（类似于内置工具如 `bash` 或 `read`）
- **资源 (Resources)** -- URI 寻址的数据源，模型可以读取
- **提示词 (Prompts)** -- 服务器暴露的可复用提示词模板

ROCode 在握手阶段从已连接的 MCP 服务器发现工具、资源和提示词，将它们包装为原生 `PluginTool` 实例，使其对查询循环透明可用。

---

## 传输方式

MCP 服务器通过以下两种传输方式之一进行通信：

### 本地命令（stdio）

默认传输方式。ROCode 将服务器作为子进程启动，通过其 stdin/stdout 使用换行分隔的 JSON-RPC 2.0 通信。

配置示例：

```jsonc
{
  "mcp": {
    "filesystem": {
      "command": ["npx", "-y", "@modelcontextprotocol/server-filesystem", "/home/user/projects"],
      "type": "local"
    }
  }
}
```

### 远程 URL（HTTP / SSE）

适用于作为独立 HTTP 服务运行的服务器。需要 `url` 字段。

配置示例：

```jsonc
{
  "mcp": {
    "remote-tools": {
      "url": "https://mcp.example.com/sse",
      "type": "remote"
    }
  }
}
```

---

## 配置 MCP 服务器

MCP 服务器在 `rocode.jsonc` 配置文件的 `mcp` 字段中声明。这是一个从服务器名称到 `McpServerConfig` 的映射。

### 项目级配置

在项目根目录创建 `rocode.jsonc`：

```jsonc
{
  "mcp": {
    "filesystem": {
      "type": "local",
      "command": ["npx", "-y", "@modelcontextprotocol/server-filesystem", "${HOME}/projects"],
      "enabled": true
    },
    "github": {
      "type": "local",
      "command": ["npx", "-y", "@modelcontextprotocol/server-github"],
      "environment": {
        "GITHUB_PERSONAL_ACCESS_TOKEN": "${GITHUB_TOKEN}"
      }
    },
    "remote-api": {
      "type": "remote",
      "url": "https://mcp.example.com/sse",
      "headers": {
        "Authorization": "Bearer ${API_TOKEN}"
      }
    }
  }
}
```

### 简写形式

可以仅用 `false` 禁用一个已有的服务器配置：

```jsonc
{
  "mcp": {
    "filesystem": false
  }
}
```

---

## 配置字段参考

每个 `McpServerConfig` 条目支持以下字段：

### 通用字段

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `type` | string | 否 | 传输类型: `"local"`（本地命令）或 `"remote"`（远程 URL）。省略时根据是否有 `command` 或 `url` 自动推断 |
| `enabled` | boolean | 否 | 是否启用。默认 `true`。设为 `false` 禁用 |
| `timeout` | u64 | 否 | 超时时间（毫秒） |

### 本地服务器字段

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `command` | string[] | 是 | 启动命令及其参数 |
| `environment` | map | 否 | 额外环境变量 |
| `env` | map | 否 | `environment` 的旧名别名 |

### 远程服务器字段

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `url` | string | 是 | MCP 服务器 URL |
| `headers` | map | 否 | 请求头 |
| `oauth` | object/boolean | 否 | OAuth 配置。设为 `false` 禁用 OAuth 自动检测 |

### OAuth 配置

| 字段 | 类型 | 说明 |
|------|------|------|
| `clientId` | string | OAuth 客户端 ID |
| `clientSecret` | string | OAuth 客户端密钥 |
| `scope` | string | OAuth scope |

---

## 环境变量展开

所有字符串字段（`command` 参数、`environment` 值、`url`、`headers` 值）支持 Shell 风格的变量展开：

| 模式 | 行为 |
|------|------|
| `${VAR_NAME}` | 替换为环境变量 `VAR_NAME` 的值。未设置时保持原样 |
| `${VAR_NAME:-default}` | 如果 `VAR_NAME` 已设置则使用其值，否则使用 `default` |

示例：

```jsonc
{
  "mcp": {
    "my-server": {
      "command": ["npx", "-y", "my-mcp-server", "--token", "${MY_API_TOKEN:-demo}"],
      "environment": {
        "DATA_DIR": "${HOME:-/tmp}/my-server-data"
      }
    }
  }
}
```

---

## CLI 命令

通过 `rocode mcp` 子命令管理 MCP 服务器：

### 列出服务器

```bash
rocode mcp list
```

输出示例：

```
MCP servers:

  filesystem           connected    tools=12 resources=4
  github               connected    tools=8  resources=0
  remote-api           failed       tools=0  resources=0
    error: connection refused
```

### 添加服务器

```bash
# 远程服务器
rocode mcp add my-server --url https://mcp.example.com/sse

# 本地服务器
rocode mcp add filesystem --command npx --arg -y --arg @modelcontextprotocol/server-filesystem

# 指定超时
rocode mcp add slow-server --url https://slow.example.com/sse --timeout 30000
```

### 连接/断开

```bash
rocode mcp connect my-server
rocode mcp disconnect my-server
```

### 调试

```bash
rocode mcp debug my-server
```

---

## OAuth 认证

对于需要 OAuth 认证的远程 MCP 服务器，ROCode 提供内建的 OAuth 流程支持。

### 启动认证

```bash
rocode mcp auth my-server --authenticate
```

### 获取授权 URL

```bash
rocode mcp auth my-server
```

返回授权 URL，在浏览器中打开并完成授权。

### 回调

授权完成后，传入回调 code：

```bash
rocode mcp auth my-server --code <authorization-code>
```

### 列出 OAuth 服务器

```bash
rocode mcp auth list
```

### 移除凭证

```bash
rocode mcp logout my-server
```

---

## MCP 工具在会话中的使用

MCP 服务器暴露的每个工具自动以原始名称注册为 `PluginTool`，模型可以直接调用，无需特殊语法。

例如，如果 `filesystem` MCP 服务器暴露了 `read_file` 和 `write_file` 工具，模型在会话中可以直接使用这些工具，就像使用内置的 `read` 和 `write` 一样。

---

## 重连机制

当 MCP 服务器断开或连接失败时，ROCode 自动启动后台重连循环：

- 初始重试延迟：**1 秒**
- 退避系数：每次失败后 **2x**
- 最大延迟：**60 秒**

循环在服务器成功连接后立即退出。如果服务器再次断开，可以启动新的重连循环。

服务器状态：

| 状态 | 含义 |
|------|------|
| `connected` | 活跃连接，报告工具数量 |
| `connecting` | 连接尝试中 |
| `disconnected` | 干净断开或尚未尝试 |
| `failed` | 上次尝试失败，已安排重试 |

使用 `rocode mcp connect <name>` 可以取消正在进行的重连循环并立即启动新的连接尝试。

---

## 常用 MCP 服务器示例

### 文件系统

```jsonc
{
  "mcp": {
    "filesystem": {
      "type": "local",
      "command": ["npx", "-y", "@modelcontextprotocol/server-filesystem", "${HOME}/projects"]
    }
  }
}
```

### GitHub

```jsonc
{
  "mcp": {
    "github": {
      "type": "local",
      "command": ["npx", "-y", "@modelcontextprotocol/server-github"],
      "environment": {
        "GITHUB_PERSONAL_ACCESS_TOKEN": "${GITHUB_TOKEN}"
      }
    }
  }
}
```

### PostgreSQL

```jsonc
{
  "mcp": {
    "postgres": {
      "type": "local",
      "command": ["npx", "-y", "@modelcontextprotocol/server-postgres", "${DATABASE_URL}"]
    }
  }
}
```

### Brave Search

```jsonc
{
  "mcp": {
    "brave-search": {
      "type": "local",
      "command": ["npx", "-y", "@modelcontextprotocol/server-brave-search"],
      "environment": {
        "BRAVE_API_KEY": "${BRAVE_API_KEY}"
      }
    }
  }
}
```

### Python (uvx)

```jsonc
{
  "mcp": {
    "git": {
      "type": "local",
      "command": ["uvx", "mcp-server-git", "--repository", "${PWD}"]
    }
  }
}
```

### 本地 HTTP 服务器

```jsonc
{
  "mcp": {
    "my-local-mcp": {
      "type": "remote",
      "url": "http://localhost:3001/sse"
    }
  }
}
```

### 带OAuth的远程服务器

```jsonc
{
  "mcp": {
    "saas-api": {
      "type": "remote",
      "url": "https://mcp.saas.com/sse",
      "oauth": {
        "clientId": "my-client-id",
        "scope": "read write"
      }
    }
  }
}
```
