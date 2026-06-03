# 插件能力矩阵

这份表只回答一个问题：

今天的 AgenDao 代码里，哪些插件类型是**真实接通的运行面**，哪些只是 schema 里留了字段但还没有自动加载链路。

不要把“配置里能写”误解成“运行时已经支持”。

---

## 结论先行

当前普通用户真正应该考虑的插件类型只有两类：

- `file`：本地 TypeScript / JavaScript 文件插件
- `npm`：npm 包插件

`dylib` 也是真支持，但更适合需要原生扩展能力的人。  
`pip` / `cargo` / Python-hosted 插件目前都不应视为已开放、已接通的用户能力。

---

## 能力矩阵

| 插件类型 | 配置值 | 是否真实可用 | Hook 面 | 最小示例 | 是否推荐给普通用户 |
|---|---|---|---|---|---|
| 本地 TS/JS 文件插件 | `file` | 是 | TS 字符串 hook：`chat.headers`、`chat.params`、`chat.message`、`tool.execute.before`、`tool.execute.after`、`tool.definition`、`permission.ask`、`command.execute.before`、`shell.env`，以及若干 `experimental.*` | `{"plugin":{"my-ts":{"type":"file","path":"./plugins/my-plugin.ts"}}}` | 推荐 |
| npm 插件 | `npm` | 是 | 与 `file` 相同，走同一个 TS 子进程宿主 | `{"plugin":{"my-npm":{"type":"npm","package":"@scope/my-plugin","version":"1.2.3"}}}` | 较推荐 |
| Rust 原生动态库插件 | `dylib` | 是 | Rust 原生 `Plugin` / `HookEvent` API，不是 TS 字符串 hook | `{"plugin":{"my-native":{"type":"dylib","path":"./plugins/libmy_plugin.so"}}}` | 不推荐给普通用户 |
| Python 包插件 | `pip` | 否 | 当前没有稳定、接通的用户 hook 面 | schema 可写，但自动加载器不会把它转成可执行 loader spec | 不推荐 |
| Cargo crate 插件 | `cargo` | 否 | 当前没有稳定、接通的用户 hook 面 | schema 可写，但自动加载器不会把它转成可执行 loader spec | 不推荐 |
| Python-hosted 插件 | `py:` | 否 | 仅保留前缀位，尚未作为正式能力接通 | 无 | 不推荐 |

---

## 各类型说明

### `file`

这是最直接的用户扩展方式。

- 本地 `.ts` / `.js` 文件
- 由子进程插件宿主加载
- 适合改 headers、参数、消息形状、权限决策、工具定义等

最小配置：

```jsonc
{
  "plugin": {
    "my-ts-plugin": {
      "type": "file",
      "path": "./plugins/my-plugin.ts"
    }
  }
}
```

最小插件：

```ts
export default async function MyPlugin() {
  return {
    async "chat.headers"(_input: unknown, output: Record<string, unknown> = {}) {
      return output;
    },
  };
}
```

### `npm`

和 `file` 是同一套运行面，只是插件来源从本地文件变成 npm 包。

- 同样使用 TS 字符串 hook
- 运行前可能触发依赖安装
- 适合团队复用或分发插件

### `dylib`

这是原生插件路径。

- 进程内动态库加载
- 用 Rust 实现
- 直接对接 `agendao_plugin::Plugin` 和 `HookEvent`
- 适合高性能、强类型、深度 runtime 集成

这条路径是真支持，但不适合作为普通用户的首选扩展方式。

### `pip` / `cargo`

这两种类型在配置 schema 里仍然保留字段，但**当前自动加载器不会把它们转成可执行 loader spec**。

这意味着：

- 你可以在 JSON 里写出 `type: "pip"` 或 `type: "cargo"`
- 但今天的运行时不会把它当成和 `file` / `npm` / `dylib` 等价的已接通能力

所以它们目前只能算“配置空间”，不能算“真实运行面”。

### `py:`

代码里保留了 `py:` 前缀位，但它是给未来 Python-hosted 插件保留的命名空间，不应理解成现在已经能写、能跑的正式接口。

---

## 推荐顺序

如果你是普通用户，推荐顺序很简单：

1. 只想改提示和流程：优先用 `skills`
2. 需要 hook / auth / request 变换：用 `file` 或 `npm`
3. 需要原生性能或深度集成：再考虑 `dylib`

不要把 `pip` / `cargo` 当成今天已经接通的官方用户扩展主路径。

---

## 参见

- [plugins](plugins)
- [configuration](configuration)
- [examples/plugins_example/README](examples/plugins_example/README)
