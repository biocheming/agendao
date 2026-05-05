# ROCode 认证指南

这份文档只写当前代码里已经成立的认证路径，不再把历史兼容面写成主路径。

---

## 先说结论

今天的 ROCode 有四条认证入口，但它们不是一回事：

1. `rocode.jsonc` 里的 `provider.*.apiKey` / `provider.*.models.*.apiKey`
2. 环境变量
3. Server / Web 路径上的 `AuthManager` 与插件认证桥
4. `.well-known/opencode` 远程配置兼容路径

如果你只是想稳定地把 provider 跑起来，优先用前两条。后两条是更窄的运行时集成能力，不应该被当成默认入口。

---

## 方式一：环境变量

这是最直接、最稳定的方式。

```bash
export OPENAI_API_KEY="sk-..."
export OPENROUTER_API_KEY="sk-or-..."
```

对于自定义 provider，推荐把环境变量名显式写进配置，而不是依赖猜测：

```jsonc
{
  "provider": {
    "my-provider": {
      "name": "My Provider",
      "baseURL": "https://api.example.com/v1",
      "env": ["MY_PROVIDER_API_KEY"]
    }
  }
}
```

```bash
export MY_PROVIDER_API_KEY="secret-123"
```

这条路径的好处是边界清楚：密钥留在环境里，配置只保存引用名。

---

## 方式二：直接写入配置

如果你需要把凭证和 provider 定义一起管理，可以在 `rocode.jsonc` 里显式写入：

```jsonc
{
  "provider": {
    "openrouter": {
      "name": "OpenRouter",
      "apiKey": "sk-or-..."
    },
    "custom-provider": {
      "name": "Custom",
      "baseURL": "https://api.example.com/v1",
      "apiKey": "secret-123"
    }
  }
}
```

模型级别也可以单独覆盖：

```jsonc
{
  "provider": {
    "custom-provider": {
      "models": {
        "my-model": {
          "apiKey": "model-specific-secret"
        }
      }
    }
  }
}
```

这条路径适合本地实验或受控环境；共享仓库、CI、多人机器上仍然更推荐环境变量。

---

## `rocode auth` 命令现在到底做什么

`rocode auth` 目前只是一个很窄的 CLI 辅助命令：

- `rocode auth list`
- `rocode auth login <provider> --token ...`
- `rocode auth logout <provider>`

它**只会设置或清除当前进程内的环境变量**，不会持久化到磁盘。

```bash
rocode auth list
rocode auth login openai --token sk-...
rocode auth logout openai
```

执行 `login` 后，CLI 会明确提示：

- 只对当前进程生效
- 想持久化，还是要你自己写 shell profile

另外，`rocode auth login https://...` 这种 well-known URL 登录目前还没有在 Rust CLI 里接通。

### `rocode auth` 当前支持的 provider 子集

这不是完整 provider 目录，只是 CLI helper 目前硬编码支持的那一批：

| Provider ID | 环境变量 |
|-------------|----------|
| `ethnopic` | `ANTHROPIC_API_KEY` |
| `openai` | `OPENAI_API_KEY` |
| `openrouter` | `OPENROUTER_API_KEY` |
| `google` | `GOOGLE_API_KEY` |
| `azure` | `AZURE_OPENAI_API_KEY` |
| `bedrock` | `AWS_ACCESS_KEY_ID` |
| `mistral` | `MISTRAL_API_KEY` |
| `groq` | `GROQ_API_KEY` |
| `xai` | `XAI_API_KEY` |
| `deepseek` | `DEEPSEEK_API_KEY` |
| `cohere` | `COHERE_API_KEY` |
| `together` | `TOGETHER_API_KEY` |
| `perplexity` | `PERPLEXITY_API_KEY` |
| `cerebras` | `CEREBRAS_API_KEY` |
| `deepinfra` | `DEEPINFRA_API_KEY` |
| `vercel` | `VERCEL_API_KEY` |
| `gitlab` | `GITLAB_TOKEN` |
| `github-copilot` | `GITHUB_COPILOT_TOKEN` |

注意两点：

- 这张表描述的是 `rocode auth` 命令，不是“ROCode 全部 provider 能力表”。
- 像 `bedrock`、`vertex` 这类 provider 往往还需要额外上下文配置；`rocode auth` 不会替你补全这些外围条件。

---

## Server / Web 路径上的持久认证

如果你跑的是 server / web 路径，认证面比纯 CLI 多一层 `AuthManager`。

当前 server 会从下面的位置加载持久凭证：

- `ROCODE_DATA_DIR/auth.json`
- 如果没配 `ROCODE_DATA_DIR`，默认是 `~/.local/share/rocode/data/auth.json`

这里的凭证类型包括：

- `api`
- `oauth`
- `wellknown`

这条路径主要服务于：

- Web / server 发起的 OAuth 流程
- provider OAuth callback
- 插件认证桥回填的凭证

它和 `rocode auth login` 不是一条线。今天的 CLI `auth login/logout` 不会去写这个文件。

---

## 插件认证桥

ROCode 的 TypeScript 插件可以声明 `auth`，由 server 侧把它桥接成真实认证流程。

当前这条路径支持的事情包括：

- 列出某个 provider 可用的认证方式
- 发起 OAuth / code 流程
- callback 回填
- `auth.load()` 返回 `apiKey`
- 可选自定义 `fetch` 代理

内建的认证插件目前有两份：

- `codex-auth`
- `copilot-auth`

如果某个 provider 依赖插件认证，这条链路通常发生在 server / web 侧，而不是靠 `rocode auth login` 完成。

---

## `.well-known/opencode` 远程配置

这条能力是存在的，但当前应该被理解为**兼容路径**，不是主路径。

现在的实现会做这些事：

1. 从本地 `auth.json` 里找出 `type: "wellknown"` 的条目
2. 请求 `{url}/.well-known/opencode`
3. 读取返回里的 `config`
4. 把 token 注入到 `provider.env` 包含对应 key 的 provider 上
5. 在内存里缓存 5 分钟

### 需要特别注意的地方

当前 well-known 入口读取的不是 `rocode` 目录，而是历史兼容位置：

- `~/.local/share/opencode/auth.json`

也就是说，今天这块仍然带着旧路径兼容色彩。文档不再把它包装成“ROCode 的标准持久认证入口”。

示例：

```json
{
  "https://corp.example.com": {
    "type": "wellknown",
    "key": "CORP_TOKEN",
    "token": "secret-123"
  }
}
```

远程配置失败不会阻止启动；这条路径只是最低优先级的远端补充。

---

## 一个更稳妥的自定义 provider 配法

如果你自己接第三方或私有 provider，建议按下面这个模式来：

```jsonc
{
  "model": "my-model",
  "provider": {
    "my-provider": {
      "name": "My Provider",
      "baseURL": "https://api.example.com/v1",
      "env": ["MY_PROVIDER_API_KEY"],
      "models": {
        "my-model": {
          "toolCall": true
        }
      }
    }
  }
}
```

```bash
export MY_PROVIDER_API_KEY="secret-123"
rocode run -m my-model "analyze this repo"
```

这样做的好处是：

- 密钥不进仓库
- provider 配置有明确 owner
- 后续导出 artifact 时也更容易保持无密钥

---

## 安全建议

- 优先使用环境变量或外部 secret 注入
- 不要把 `apiKey` 提交到版本控制
- 如果你使用 server / web OAuth，保护好 `ROCODE_DATA_DIR/auth.json`
- 不要把 well-known 兼容文件当成主要 secret store

---

## 参见

- [配置参考](configuration)
- [插件系统](plugins)
