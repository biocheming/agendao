# AgenDao 文档状态总表

文档日期：`2026-08-26`

本文说明 `agendao/docs` 里每份文档的用途和当前状态，帮助用户快速定位需要的信息。

## 产品文档

| 文档 | 用途 | 状态 |
| --- | --- | --- |
| `README.md` | docs 门户与阅读顺序 | 当前 |
| `index.md` | 产品总览 | 当前 |
| `installation.md` | 安装、升级、卸载 | 当前 |
| `commands.md` | CLI 与斜杠命令参考 | 当前 |
| `tools.md` | 内置工具参考 | 当前 |
| `configuration.md` | 配置格式与验证 | 当前 |
| `context-caching.md` | 上下文缓存与提示面 | 当前 |
| `sandbox.md` | 沙箱执行边界与平台隔离 | 当前 |
| `skills.md` | Skill 治理与 Hub | 当前 |
| `scheduler.md` | Scheduler 模板与编排 | 当前 |
| `auth.md` | Provider 认证 | 当前 |
| `mcp.md` | MCP 服务管理 | 当前 |
| `hooks.md` | Hooks 与事件 | 当前 |
| `plugins.md` | 插件系统 | 当前 |
| `plugins-capability-matrix.md` | 插件能力矩阵 | 当前 |
| `agents.md` | Agent 系统 | 当前 |
| `architecture.md` | 架构总览 | 当前 |
| `task-ledger.md` | `/goal` 命令与 TaskLedger | 当前 |
| `execution-authorities.md` | 执行入口/prompt/事件/transport/旁路权威矩阵 | 当前 |

## 示例与 Schema

以下内容用于上手和验证格式：

- `examples/context_docs/*` — context_docs schema 与示例
- `examples/plugins_example/*` — Skill / TS plugin / Rust 扩展示例
- `examples/scheduler/*` — SchedulerBlueprint 示例
- `examples/configuration/*` — Provider / Permission / Ollama 配置示例
- `agendao_config.schema.json` — 配置 JSON Schema

## 阅读顺序建议

1. `index.md` — 了解 AgenDao 是什么
2. `installation.md` — 安装
3. `commands.md` + `tools.md` — 日常使用
4. `configuration.md` — 自定义配置
5. `skills.md` + `scheduler.md` — 进阶能力
6. 其余文档按需查阅
