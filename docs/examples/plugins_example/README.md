# plugins_example

文档基线：v2026.4.26（更新日期：2026-04-26）

这个目录是示例合集，用来回答你这个问题：

- 可以放 `markdown skill` 示例
- 可以放 `TypeScript plugin` 示例
- 也可以放 `Rust` 扩展示例

结论：这个做法是对的，但三者的加载方式不一样。

## 本轮补充（v2026.4.26）

- 对于会产生大输出的插件工具，建议把二进制 / 大文本放到 `attachments` 或外部引用，不要直接塞进 `output` 文本，避免请求体超限。
- 对于批量工具调用，建议返回摘要文本 + 结构化 metadata，前端按 metadata 做可视化渲染。
- 如果工具会向用户提问，推荐让前端保留结构化 question 能力，而不是把所有交互都退化成普通文本。

## 1) Skill (Markdown) 是提示词能力

- 文件格式：`SKILL.md`
- 典型放置目录：`.rocode/skills/<skill-name>/SKILL.md`
- 特点：不改运行时代码，主要给模型注入流程和约束

本目录示例：`./skill/SKILL.md`

## 2) TS Plugin 是运行时 Hook / Auth 扩展

- 由 `rocode-plugin` 子进程桥接执行
- 在配置文件里通过 `plugin` 映射或兼容列表声明（入口是 `rocode.jsonc`）

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

兼容旧写法：

```json
{
  "plugin": [
    "file://./docs/examples/plugins_example/ts/example-plugin.ts"
  ]
}
```

本目录示例：`./ts/example-plugin.ts`

## 3) Rust 示例是原生 dylib 编译示例

- Rust 代码不会像 TS 插件那样被动态 `import`
- 需要先编译成 `cdylib` / `dylib`，再通过 `plugin.type = "dylib"` 显式配置加载
- 这个目录下的 Rust 代码是原生插件入口示例，不是“放进仓库就会自动生效”的插件目录

本目录示例：`./rust/src/lib.rs`

## 推荐实践

- 只想增强提示和流程：优先用 Skill
- 需要动态 hook / auth / custom fetch：用 TS Plugin
- 需要深度性能 / 类型安全 / 核心能力扩展：改 Rust 代码并编译
