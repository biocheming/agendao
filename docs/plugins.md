# ROCode 插件系统

这份文档只描述当前代码里真实存在的插件加载面。历史兼容字段会标出来，但不再把它们包装成主路径。

---

## 先钉死边界

今天的 ROCode 里，`plugin` 和 `skills` 不是一回事：

- `plugin`：代码插件，走运行时加载器
- `skills`：提示与流程扩展，走独立的 skill 系统

所以 `SKILL.md` 不属于 `plugin` 配置的一种。它应该看 [skills](skills)，不是往 `plugin` 里塞。

---

## 当前会被真实加载的插件类型

当前自动引导的代码插件只有三类：

| 类型 | 配置值 | 加载方式 | 说明 |
|------|--------|----------|------|
| TypeScript / JavaScript 文件 | `file` | 子进程插件宿主 | 本地 `.ts` / `.js` 文件 |
| npm 插件 | `npm` | 子进程插件宿主 | npm 包名或 `pkg@version` |
| Rust 原生插件 | `dylib` | 进程内动态库加载 | 本地 `.so` / `.dylib` / `.dll` |

`PluginConfig` schema 里仍然保留了 `pip` / `cargo` 字段空间，但当前自动加载器不会把它们转成可执行 loader spec。也就是说：

- `pip` / `cargo` 现在是配置空间
- `file` / `npm` / `dylib` 才是当前真实运行面

---

## 插件配置

推荐使用映射格式：

```jsonc
{
  "plugin": {
    "local-example": {
      "type": "file",
      "path": "./plugins/example-plugin.ts"
    },
    "npm-example": {
      "type": "npm",
      "package": "@scope/my-plugin",
      "version": "1.2.3"
    },
    "native-example": {
      "type": "dylib",
      "path": "./plugins/libmy_plugin.so"
    }
  }
}
```

### 兼容的旧写法

列表格式仍然能读，但只应该视为兼容面：

```jsonc
{
  "plugin": [
    "file://./plugins/example-plugin.ts",
    "@scope/my-plugin@1.2.3"
  ]
}
```

这条兼容路径最终仍然只会落到：

- `file://...` -> `file`
- `pkg@version` -> `npm`

新配置不要再继续扩散这个写法。

### `PluginConfig` 关键字段

| 字段 | 说明 |
|------|------|
| `type` | `file` / `npm` / `dylib` |
| `path` | 本地文件或动态库路径 |
| `package` | npm 包名 |
| `version` | npm 版本约束 |
| `runtime` | 运行时覆盖，例如 `bun` |
| `options` | 插件自定义附加参数 |

---

## `pluginPaths` 是什么

`pluginPaths` 不是死字段，也不是摆设。

它是配置加载器的**发现根目录**，会参与自动扫描本地文件插件：

```jsonc
{
  "pluginPaths": {
    "workspace": "./plugins",
    "shared": "~/.rocode/plugins"
  }
}
```

这条路径的职责很窄：

- 提供额外扫描目录
- 自动发现 `file` 类型插件

它不是一个“远程插件市场”配置，也不意味着这些目录里的任意内容都会被神奇解释成完整插件生态。

---

## TypeScript 插件

`file` 和 `npm` 两类插件都走同一个 TypeScript 子进程宿主。

### 运行时要求

当前运行时探测顺序是：

1. `bun`
2. `deno`
3. `node`（要求 `>= 22.6`，因为要用 `--experimental-strip-types`）

也可以用 `ROCODE_PLUGIN_RUNTIME` 强制指定。

### 安装行为

- `file` 插件：直接加载，不做依赖安装
- `npm` 插件：加载前会执行安装步骤

安装命令取决于选中的 JS runtime：

- `bun` -> `bun install`
- `deno` -> `deno install`
- `node` -> `npm install`

所以旧文档那种“如果需要，执行 npm install”的笼统说法不准确。只有 `npm` spec 会触发这条安装链，而且命令不一定是 `npm`。

### 导出形状

TS 插件应该导出一个默认的异步函数。这个函数返回一个对象；对象的键就是插件能力面。

最小例子：

```ts
export default async function ExamplePlugin() {
  return {
    async "chat.headers"(_input: unknown, output: Record<string, unknown> = {}) {
      const headers = (output.headers ?? {}) as Record<string, string>;
      headers["x-rocode-plugin"] = "example";
      return { ...output, headers };
    },

    async "tool.execute.before"(input: unknown, output: unknown) {
      return output;
    },
  };
}
```

也就是说，当前真实接口是这种风格：

- 直接返回 hook name -> async handler
- 可选返回 `tool`
- 可选返回 `auth`

不是旧文档里那种 `hooks: { ... }` 包装层，也不是旧的驼峰事件名和通知名接口。

### 自定义工具

如果插件要暴露工具，返回对象里放一个 `tool` 字段：

```ts
import { z } from "zod";

export default async function ToolPlugin() {
  return {
    tool: {
      echo: {
        description: "Echo input text",
        args: z.object({
          text: z.string(),
        }),
        async execute(args: { text: string }) {
          return { output: args.text };
        },
      },
    },
  };
}
```

宿主会把 `args` 转成 JSON Schema，供工具注册与展示使用。

---

## 当前真实的 Hook 名

TS 插件当前支持的 hook 名是下面这些精确字符串：

### 请求与消息

- `chat.headers`
- `chat.params`
- `chat.message`

### 工具与权限

- `tool.execute.before`
- `tool.execute.after`
- `tool.definition`
- `permission.ask`

### 命令与 shell

- `command.execute.before`
- `shell.env`

### 实验性 hook

- `experimental.chat.system.transform`
- `experimental.chat.messages.transform`
- `experimental.session.compacting`
- `experimental.telemetry.snapshot.updated`
- `experimental.telemetry.stage.summary.updated`
- `experimental.text.complete`

如果你写的是旧的驼峰事件名或通知式事件名，那不是“有点旧”，而是根本对不上当前宿主。

---

## 认证桥

TS 插件还可以声明 `auth`，由宿主桥接成 provider 认证能力。

这条路径支持的能力包括：

- 列出认证方法
- 发起授权
- callback 回填
- `auth.load()` 返回 `apiKey`
- 可选自定义 `fetch` 代理

简化例子：

```ts
export default async function AuthPlugin() {
  return {
    auth: {
      provider: "github-copilot",
      methods: [
        { type: "oauth", label: "GitHub Copilot" },
      ],
      async loader() {
        const apiKey = process.env.GITHUB_COPILOT_TOKEN?.trim();
        return apiKey ? { apiKey } : {};
      },
    },
  };
}
```

当前内建的认证插件有两份：

- `codex-auth`
- `copilot-auth`

---

## Rust 原生插件

`dylib` 插件走进程内加载，适合：

- 需要更强类型边界的扩展
- 高性能路径
- 不想依赖 JS runtime 的场景

最小导出形式：

```rust
struct MyPlugin;

impl rocode_plugin::Plugin for MyPlugin {
    fn name(&self) -> &str { "my-plugin" }
    fn version(&self) -> &str { "1.0.0" }
}

rocode_plugin::declare_plugin!(MyPlugin);
```

配置示例：

```jsonc
{
  "plugin": {
    "my-native-plugin": {
      "type": "dylib",
      "path": "./plugins/libmy_plugin.so"
    }
  }
}
```

这条路径要注意两件事：

- 动态库和主程序 ABI 风险高
- 不要加载不受信任的二进制

---

## 推荐实践

### 什么时候用什么

| 目标 | 推荐方式 |
|------|----------|
| 调整提示、流程、规范 | `skills` |
| 改请求头、请求参数、消息形状 | TS 插件 hook |
| 自定义 OAuth / fetch 代理 | TS 插件 `auth` |
| 高性能或深度 runtime 集成 | `dylib` |

### 现在不该做的事

- 不要再把 `skills` 写成 `plugin` 的一种
- 不要再用旧的 TS hook 事件名
- 不要把 `pip` / `cargo` 当成当前已接通的自动执行面
- 不要继续扩散 `plugin` 列表格式

---

## 参见

- [认证指南](auth)
- [技能系统](skills)
- [配置参考](configuration)
