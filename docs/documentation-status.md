# ROCode 文档状态总表

文档日期：`2026-05-15`

本文不是发布日志，也不是今天改了什么的流水账。它的作用只有一个：把 `rocode/docs` 里哪些文档是当前真相、哪些是设计参考、哪些已经主要转为复盘材料，统一说清楚。

## 先看结论

当前 `rocode/docs` 应按三类阅读：

1. 稳定产品文档
   - 面向用户、集成方和日常开发。
   - 默认认为“当前可用”，除非文档内显式标注实验性或规划性边界。
2. 设计/实施参考
   - 解释架构边界、实施顺序和权责收口。
   - 可以指导局部改动，但不能自动当成项目主计划。
3. 示例与 schema
   - 主要用于上手、验证格式和构造配置。
   - 它们说明“怎么写”，不说明“产品是否已经完整做完”。

## 当前产品线状态

### 1. 工具调用准确率 / replay authority

状态：`已完成当前主收口`

结论：

- assistant 历史 replay 现在有共享 authority。
- canonical replay ordering 已钉死：`reasoning -> text -> tool_use -> tool_result -> file`
- session / provider / orchestrator 三条路径都有守护测试。
- downgraded tool-summary 不会再被重新送回 `Role::Tool`。

对应文档：

- `plans/message-replay-authority-refactor.md`
- `context-caching.md`
- `tools.md`

### 2. tool repair / telemetry / trajectory quality

状态：`已进入正式读面`

结论：

- repair summary、repair query snapshot、tool trajectory quality 不再只是内部调试数据。
- persisted session telemetry 已携带这些结构化读面。
- CLI / TUI / Web 都已经能展示 trajectory quality。
- 这条线的定位是“解释工具轨迹质量和修复成本”，不是替代主执行逻辑。

对应文档：

- `commands.md`
- `index.md`
- `README.md`

### 3. prompt caching / context closure / prompt surface

状态：`核心能力已落地，仍有持续硬化空间`

结论：

- 这条线已经不是纯设计稿，已有稳定提示面、context closure、cache diagnostics、runtime snapshot 等正式读面。
- 但相关计划文档仍应被视为架构/实施参考，而不是“下一阶段自动执行单”。

对应文档：

- `context-caching.md`
- `plans/prompt-caching-architecture.md`
- `plans/prompt-caching-implementation-plan.md`
- `plans/prompt-surface-runtime-snapshot-and-ingress-stabilization.md`

### 4. frontend/backend decoupling

状态：`主体完成，后续以边界守护为主`

结论：

- `rocode` 已是产品壳。
- `rocode-cli` / `rocode-tui` / `rocode-web` / `rocode-server` 的角色划分已经清楚。
- 这份蓝图文档现在更像边界守护参考，而不是高频执行清单。

对应文档：

- `plans/frontend-backend-decoupling-blueprint.md`

### 5. provider profile / protocol / transport 清债

状态：`进行中`

结论：

- provider authority、descriptor、protocol family、transport/auth 边界已经明显比早期清楚。
- 但这条线还没有彻底完成结构性收缩，尤其是 provider 内部文件体积和职责混杂问题仍然存在。

对应文档：

- `plans/provider-profile-protocol-transport-refactor-plan.md`

### 6. verifier mode

状态：`主算法与主产品面已落地，剩余主要是 polish`

结论：

- verifier 不再只是概念设计，核心 pairwise / logprob-aware / score-job 路径已经实现。
- 当前剩余项更多是 retry counters、router/frontend 文案、展示 polish，而不是主算法空白。

对应文档：

- `plans/verifier-mode-preset-design.md`
- `plans/verifier-mode-implementation-checklist.md`

## 根目录文档状态

| 文档 | 当前定位 | 状态判断 |
| --- | --- | --- |
| `README.md` | docs 门户与阅读顺序 | 当前 |
| `index.md` | 产品总览 | 当前 |
| `installation.md` | 安装/分发 | 当前 |
| `commands.md` | CLI/斜杠命令参考 | 当前，已补齐 memory/provider/repair 等入口 |
| `tools.md` | 内置工具参考 | 当前，需按 canonical-first 理解高复杂工具 |
| `configuration.md` | 配置与 validation | 当前 |
| `context-caching.md` | 上下文缓存与 prompt surface | 当前 |
| `skills.md` | skill 治理与 hub | 当前 |
| `scheduler.md` | scheduler 全参考 | 当前 |
| `auth.md` | provider 认证 | 当前 |
| `mcp.md` | MCP 管理 | 当前 |
| `hooks.md` | hooks 与事件 | 当前 |
| `plugins.md` | 插件系统说明 | 当前 |
| `plugins-capability-matrix.md` | 插件能力矩阵 | 当前 |
| `agents.md` | agent 系统说明 | 当前 |

## 计划文档状态

| 文档 | 当前定位 | 状态判断 |
| --- | --- | --- |
| `plans/message-replay-authority-refactor.md` | replay authority 复盘与边界说明 | `已完成` |
| `plans/frontend-backend-decoupling-blueprint.md` | 架构边界蓝图 | `主体完成，继续作为边界参考` |
| `plans/global-state-followup-tasklist.md` | startup/global-state 局部清理清单 | `局部 backlog，不是主线` |
| `plans/prompt-caching-architecture.md` | 缓存架构原则 | `长期参考` |
| `plans/prompt-caching-implementation-plan.md` | 缓存实施路线 | `部分落地，仍是局部技术计划` |
| `plans/prompt-surface-runtime-snapshot-and-ingress-stabilization.md` | prompt surface snapshot / ingress 稳定化 | `第一版已落地，后续仍可继续` |
| `plans/provider-profile-protocol-transport-refactor-plan.md` | provider 清债路线 | `进行中` |
| `plans/verifier-mode-preset-design.md` | verifier 设计与现状 | `主体完成，余下 polish` |
| `plans/verifier-mode-implementation-checklist.md` | verifier 落地清单 | `主体完成，余下 polish` |
| `plans/tui-session-graph-sidebar.md` | TUI sidebar 设计草图 | `设计 backlog` |

## 示例与 schema 状态

这些内容默认不作为“当前产品完成度”判断依据：

- `examples/context_docs/*`
- `examples/plugins_example/*`
- `examples/scheduler/*`
- `examples/tasks/*`
- `rocode_config.schema.json`

它们的职责是：

- 提供合法格式样例
- 提供教程入口
- 提供最小配置模板

而不是记录某条主线是否已经完成。

## 如何避免后续再把文档读乱

后续判断某条工作能不能继续自动外推时，优先看：

1. `documentation-status.md`
2. `README.md`
3. 具体根目录产品文档
4. 局部计划文档

如果某份文档只在 `plans/` 里成立，而没有被总览文档承认，它默认只是局部技术参考，不自动构成新的主线阶段。
