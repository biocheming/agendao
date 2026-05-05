# plugins_example

文档基线：v2026.5.5（更新日期：2026-05-05）

这个目录放三类扩展示例，但它们不是同一条加载链：

- `skill/`：提示与流程扩展
- `ts/`：TypeScript 插件
- `rust/`：原生 `dylib` 插件示例

不要把这三者混成“plugin 都一样”。

## 1) Skill (Markdown) 是提示词能力

- 文件格式：`SKILL.md`
- 典型放置目录：`.rocode/skills/<skill-name>/SKILL.md`
- 特点：不改运行时代码，主要给模型注入流程和约束

本目录示例：`./skill/SKILL.md`

## 2) TS Plugin 是运行时 Hook / Auth 扩展

- 由 `rocode-plugin` 子进程桥接执行
- 在 `rocode.jsonc` 的 `plugin` 字段中声明
- 当前真实 hook 面是字符串键，例如 `chat.headers`、`tool.definition`

推荐配置（项目根 `rocode.jsonc`）：

```json
{
  "plugin": {
    "example-plugin": {
      "type": "file",
      "path": "./docs/examples/plugins_example/ts/example-plugin.ts"
    }
  }
}
```

示例配置文件见：`./rocode.jsonc.example`

兼容列表写法仍然能读，但这里只保留当前推荐写法，不再继续扩散旧入口。

本目录示例：`./ts/example-plugin.ts`

## 3) Rust 示例是原生 dylib 编译示例

- Rust 代码不会像 TS 插件那样被动态 `import`
- 需要先编译成 `cdylib` / `dylib`，再通过 `plugin.type = "dylib"` 显式配置加载
- 原生插件 API 使用 `HookEvent` 枚举；它和 TS 字符串 hook 面不是一套接口
- 这个目录下的 Rust 代码是入口示例，不是“放进仓库就会自动生效”的插件目录

本目录示例：`./rust/src/lib.rs`

## 推荐实践

- 只想增强提示和流程：优先用 Skill
- 需要动态 hook / auth / custom fetch：用 TS Plugin
- 需要深度性能 / 类型安全 / 核心能力扩展：改 Rust 代码并编译
- 对于大输出插件工具，优先返回摘要文本 + 结构化 metadata，不要把全部结果塞进 `output`
