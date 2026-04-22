# ROCode 插件系统

ROCode 的插件系统让你通过额外的工具、Hook、认证桥接、代理和技能来扩展代理能力。插件以独立目录或文件形式组织，通过配置文件声明加载。

---

## 目录

1. [插件类型概览](#插件类型概览)
2. [插件配置](#插件配置)
3. [Skill 插件（Markdown）](#skill-插件markdown)
4. [TypeScript 插件](#typescript-插件)
5. [Rust 原生插件（dylib）](#rust-原生插件dylib)
6. [插件工具桥接](#插件工具桥接)
7. [Hook 系统](#hook-系统)
8. [认证桥接](#认证桥接)
9. [创建自定义插件](#创建自定义插件)
10. [推荐实践](#推荐实践)

---

## 插件类型概览

当前运行时自动引导三种插件入口：

| 类型 | 格式 | 加载方式 | 适用场景 |
|------|------|----------|----------|
| **Skill** | `SKILL.md` | 直接加载 | 增强提示词和流程，不改运行时代码 |
| **TypeScript** | `.ts` 文件 | 子进程桥接 | 动态 Hook、Auth、自定义 fetch |
| **Rust 原生** | `.so` / `.dylib` | in-process `libloading` | 深度性能、类型安全、核心能力扩展 |

配置 schema 中还保留了 `pip` / `cargo` 类型字段，但当前 server 启动流程不会自动把它们交给插件加载器；真正会自动引导的是上表这三类入口。

---

## 插件配置

插件在 `rocode.jsonc`（或 `opencode.jsonc`）的 `plugin` 字段中声明。`plugin` 字段支持两种格式：

### 映射格式

```jsonc
{
  "plugin": {
    "my-plugin": {
      "type": "npm",
      "package": "my-rocode-plugin",
      "version": "1.0.0"
    },
    "local-plugin": {
      "type": "file",
      "path": "./plugins/my-plugin.ts"
    },
    "native-plugin": {
      "type": "dylib",
      "path": "./plugins/libmy_plugin.so"
    }
  }
}
```

### 列表格式（兼容旧写法）

```jsonc
{
  "plugin": [
    "file://./plugins/my-plugin.ts",
    "my-npm-plugin@latest"
  ]
}
```

列表格式自动转换：
- `file://` 前缀 -> `file` 类型插件
- `pkg@version` 格式 -> `npm` 类型插件

列表格式只覆盖历史兼容路径，本质上仍只会落到 `file` 或 `npm` 两种自动加载入口。新配置优先使用映射格式。

### PluginConfig 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `type` | string | 当前自动引导的类型: `file`, `npm`, `dylib` |
| `path` | string | 文件路径（`file` 和 `dylib` 类型） |
| `package` | string | 包名（当前主要用于 `npm`） |
| `version` | string | 版本约束（常用于 `npm`，可选） |
| `runtime` | string | 运行时覆盖（例如 `bun`） |
| `options` | object | 插件专用扩展参数 |

`PluginConfig` 结构体里还定义了 `pip` / `cargo` 相关描述空间，但当前自动引导逻辑不会把它们转成 loader spec；如果文档或示例需要可执行插件，请继续使用 `npm` / `file` / `dylib`。

### 插件路径

通过 `pluginPaths` 配置额外的插件发现目录：

```jsonc
{
  "pluginPaths": {
    "custom": "./plugins"
  }
}
```

---

## Skill 插件（Markdown）

Skill 是纯 Markdown 的提示词能力模块，不改运行时代码，主要给模型注入流程和约束。

### 文件格式

文件名：`SKILL.md`

典型放置目录：`.rocode/skills/<skill-name>/SKILL.md`

### 配置

在 `rocode.jsonc` 中声明技能路径：

```jsonc
{
  "skills": {
    "paths": [
      "./.rocode/skills"
    ]
  }
}
```

### 管理

通过 `skill_manage` 工具管理技能：

| 操作 | 说明 |
|------|------|
| `create` | 创建新技能 |
| `patch` | 修补技能 |
| `edit` | 编辑技能 |
| `write_file` | 写入技能支持文件 |
| `remove_file` | 移除技能支持文件 |
| `delete` | 删除技能 |

通过 `skills_list` 工具列出可用技能，通过 `skill_view` 工具查看技能内容。

### 示例

```
.rocode/skills/
  code-review/
    SKILL.md
    references/
      checklist.md
```

---

## TypeScript 插件

TypeScript 插件通过子进程桥接执行。ROCode 使用检测到的 JS 运行时（bun、deno 或 node）启动插件宿主脚本。

### 加载流程

1. ROCode 检测可用的 JS 运行时（优先级：bun > deno > node）
2. 如果需要，执行 `npm install` 安装依赖
3. 启动宿主脚本作为子进程
4. 通过 JSON-RPC 协议进行双向通信

### 宿主脚本

ROCode 内嵌了宿主脚本（`plugin-host.ts`），插件不需要自己提供宿主。

### 配置

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

或使用列表格式：

```jsonc
{
  "plugin": [
    "file://./plugins/my-plugin.ts"
  ]
}
```

映射格式是当前推荐写法；列表格式主要用于兼容旧项目。

### 插件能力

TypeScript 插件可以注册：

- **自定义工具** -- 暴露给模型的函数
- **Hook** -- 生命周期事件处理器
- **认证桥接** -- 处理 OAuth 或其他认证流程

---

## Rust 原生插件（dylib）

Rust 原生插件以动态库形式加载到 ROCode 进程内，提供最高性能和完整类型安全。

### 入口点

共享库必须导出名为 `rocode_plugin_create` 的函数：

```rust
#[no_mangle]
pub fn rocode_plugin_create() -> Box<dyn rocode_plugin::Plugin> {
    Box::new(MyPlugin)
}
```

便捷宏：

```rust
rocode_plugin::declare_plugin!(MyPlugin);
```

### 安全注意事项

- 共享库**必须**使用与 ROCode 相同的 Rust 编译器版本编译
- Rust 不保证跨版本的稳定 ABI
- 加载不受信任的动态库有任意代码执行风险

### 配置

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

### Plugin trait

每个 Rust 插件实现 `rocode_plugin::Plugin` trait，提供：

- `name()` -- 插件名称
- `version()` -- 插件版本
- Hook 方法（见 Hook 系统）

---

## 插件工具桥接

每个插件注册的自定义工具通过 `PluginTool` 桥接到 ROCode 工具注册表。

### PluginTool 结构

`PluginTool` 持有：

- 工具 ID
- 插件 ID
- 工具描述
- 参数模式 (JSON Schema)
- `PluginLoader` 引用（支持空闲关机后的透明恢复）

### 工具定义

插件通过 `PluginToolDef` 声明工具：

| 字段 | 类型 | 说明 |
|------|------|------|
| `description` | string | 工具描述 |
| `parameters` | JSON Value | 参数的 JSON Schema |

### 执行流程

1. 模型选择调用一个插件工具
2. ROCode 通过 `PluginTool.execute()` 转发请求
3. `PluginLoader` 确保子进程活跃（需要时自动重启）
4. 请求通过 JSON-RPC 发送到插件子进程
5. 结果返回给模型

### 大输出处理

对于会产生大输出的插件工具，建议：
- 把二进制/大文本放到 `attachments` 或外部引用
- 不要直接塞进 `output` 文本
- 避免请求体超限

---

## Hook 系统

插件可以注册 Hook 以响应生命周期事件。Hook 接收 JSON payload 描述事件。

### Hook 事件

| 事件 | 触发时机 |
|------|----------|
| `PreToolUse` | 工具执行前 |
| `PostToolUse` | 工具返回结果后 |
| `PostToolUseFailure` | 工具调用出错后 |
| `PermissionDenied` | 权限请求被拒绝时 |
| `PermissionRequest` | 权限请求时（判定前） |
| `Notification` | 代理通知 |
| `UserPromptSubmit` | 用户提交提示词 |
| `SessionStart` | 会话开始 |
| `SessionEnd` | 会话结束 |
| `Stop` | 模型完成回复 |
| `StopFailure` | 停止序列失败 |
| `SubagentStart` | 子代理启动 |
| `SubagentStop` | 子代理完成 |
| `PreCompact` | 上下文压缩前 |
| `PostCompact` | 上下文压缩后 |

### Hook 上下文

每个 Hook 接收 `HookContext`，包含事件类型和 payload。Hook 返回 `HookOutput` 或 `HookError`。

### Hook 输出

`HookOutput` 可以包含：

- `payload` -- 可选的 JSON 值，影响后续处理

### 阻塞 vs 非阻塞

- **阻塞 Hook**：返回错误时阻止操作
- **非阻塞 Hook**：返回错误时仅记录警告

---

## 认证桥接

ROCode 内建了两个认证桥接插件：

| 桥接 | 说明 |
|------|------|
| `codex-auth` | 处理 Codex 认证流程 |
| `copilot-auth` | 处理 Copilot 认证流程 |

这些桥接在 `rocode-plugin` crate 中内嵌，自动加载。

TypeScript 插件可以通过 `PluginAuthBridge` 注册自定义认证处理器，处理 OAuth 或其他认证协议。

---

## 创建自定义插件

### 创建 Skill 插件

1. 在 `.rocode/skills/` 下创建目录：

```
.rocode/skills/my-skill/
  SKILL.md
```

2. 编写 `SKILL.md`：

```markdown
---
name: my-skill
description: 我的自定义技能
category: custom
---

# My Skill

指令和流程描述...
```

3. 在配置中声明技能路径（如尚未配置）：

```jsonc
{
  "skills": {
    "paths": ["./.rocode/skills"]
  }
}
```

也可以通过 `skill_manage` 工具的 `create` 操作创建。

### 创建 TypeScript 插件

1. 创建 `.ts` 文件：

```typescript
// my-plugin.ts
export default {
  name: "my-plugin",
  version: "1.0.0",
  tools: [
    {
      name: "my_tool",
      description: "自定义工具描述",
      parameters: {
        type: "object",
        properties: {
          input: { type: "string", description: "输入参数" }
        },
        required: ["input"]
      },
      execute: async (args) => {
        return { output: `处理: ${args.input}` };
      }
    }
  ],
  hooks: {
    PostToolUse: [
      {
        matcher: "Edit",
        handler: async (context) => {
          // 工具后处理逻辑
          return {};
        }
      }
    ]
  }
};
```

2. 在配置中声明：

```jsonc
{
  "plugin": {
    "my-plugin": {
      "type": "file",
      "path": "./plugins/my-plugin.ts"
    }
  }
}
```

### 创建 Rust 原生插件

1. 创建 Rust 库项目：

```rust
// src/lib.rs
use rocode_plugin::{Plugin, PluginSystem, HookContext, HookOutput, HookResult};

struct MyPlugin;

impl Plugin for MyPlugin {
    fn name(&self) -> &str { "my-plugin" }
    fn version(&self) -> &str { "1.0.0" }
}

rocode_plugin::declare_plugin!(MyPlugin);
```

2. `Cargo.toml` 配置为 `cdylib`：

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
rocode-plugin = { path = "../rocode/crates/rocode-plugin" }
```

3. 编译并配置：

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

---

## 推荐实践

| 需求 | 推荐方式 |
|------|----------|
| 增强提示和流程 | **Skill** (SKILL.md) -- 最简单，不改代码 |
| 动态 Hook / Auth / 自定义 fetch | **TypeScript 插件** -- 灵活，运行时加载 |
| 深度性能 / 类型安全 / 核心能力扩展 | **Rust 原生插件** -- 编译期集成 |

### 批量工具调用

对于批量工具调用，建议：
- 返回摘要文本 + 结构化 metadata
- 前端按 metadata 做可视化渲染
- 不要把所有输出都塞进纯文本

### 用户交互

如果工具需要向用户提问：
- 保留结构化 `question` 能力
- 不要把所有交互退化为普通文本

### 熔断器

插件子系统内建熔断器（circuit breaker），当插件子进程反复失败时自动停止尝试，防止资源浪费。熔断器会在一段时间后自动尝试恢复。
