# Changelog

## 2026.5.17

- 同步工作区版本到 `v2026.6.3`，更新工作区版本、Web 根包版本与相关 lock 文件中的 AgenDao 自身条目。
- 刷新 README、文档首页、安装页、文档状态总表与示例基线，把产品说明收敛到“当前整体能力与边界”，不再用阶段性补丁总结替代产品文档。

## 2026.5.15

- 同步工作区版本到 `v2026.5.15`，更新工作区版本、Web 根包版本与相关 lock 文件中的 AgenDao 自身条目。
- replay authority 主线完成当前收口：assistant 历史 replay 进入共享 authority，canonical ordering 与 raw replay shape 优先级被明确写死，session/provider/orchestrator 三条路径都有回归守护。
- tool repair / trajectory quality 进入正式可见读面：persisted telemetry 现在稳定携带 repair summary、repair query snapshot 与 tool trajectory quality，CLI/TUI/Web 都能显示。
- 文档系统完成一次通盘整理：`agendao/docs` 新增统一状态总表，README/命令参考/工具参考/上下文缓存文档按“当前真相、局部技术计划、示例文档”重新分层，不再依赖口头复盘判断哪些文档 still authoritative。

## 2026.5.12

- 同步工作区版本到 `v2026.5.12`，更新 AgenDao 自有包元数据、Web 根包版本与相关 lock 文件中的 AgenDao 自身条目。
- 修复最终 assistant message 在不同前端间的同步不一致：Web 现在会把 history 与 live block 的 message/reasoning id 统一归一，避免已落库的最终文本被 stale live snapshot 覆盖。
- `agendao-server` 现在会真正广播 `session.updated` 事件，而不只是记录 telemetry；TUI 能在 `prompt.final`、`prompt.completed`、`prompt.scheduler.completed` 之后触发 authoritative session sync。
- 最近这一轮收口后，AgenDao 的产品面已经从“终端聊天 + 多 provider”扩展为统一 authority 的本地编码智能体运行时：provider descriptor / config validation、session telemetry 三本账、context closure diagnostics、scheduler continuity、skill hub 治理与 memory consolidation 都已经进入正式读写路径。
- skill hub 已具备远程 discoverability 的基础形态：支持多 source 聚合搜索、stale index 提示、默认 registry 建议、trust / maintenance metadata，以及为 agent 侧 search → install 流程返回可直接消费的结构化结果。
- 子 session / agent handoff 在 thinking-sensitive provider 上的边界更清晰：shared sanitizer、effective thinking mode 与 continuation boundary 逻辑完成收口，避免不同协议族和子 agent 路径各自维护近似实现。

## 2026.5.8

- 同步工作区版本到 `v2026.5.8`，更新 AgenDao 自有包元数据、Web 根包版本与相关 lock 文件中的 AgenDao 自身条目。
- provider 治理线完成收口：typed `ProviderProfile` 统一承接 protocol family / shape / transport / usage / cache 语义；Web/TUI/CLI 通过独立 provider descriptor 和 config validation 读面解释当前生效配置。
- session / context ownership 完成收口：telemetry 明确区分 `request_context_tokens`、`live_context_tokens`、`workflow_cumulative_tokens`，并新增 `context_closure_contract` 与 `prompt_surface_state_snapshot` 诊断 sidecar。
- run loop 现在持有 owner-local request-view checkpoint 语义，能结合 exact model limits 在执行中决定 continue / compact request view / block，而不再只靠 pre-run / post-run 两头治理。
- external adapter 不再允许集成方自行猜测 session id；必须先走 owner-local `/external-adapter/session/provision` 创建受控 session，再绑定 verify / replay / run。
- 显式 full-history fork 完成 contract 收口：冻结 fork policy、导入历史只读、usage / revert / recovery 保持 local-only，child/subsession 则继续走 packet handoff 与 result/summary 回收语义。
- skill 治理从“安装和读取”扩展到 usage ledger、negative entropy、semantic conflict、composition relationship、proposal inbox 和统一 runtime gate；`retired` skill 对 inspection 仍可见，但不再进入 runtime catalog。

## 2026.4.29

- 同步工作区版本到 `v2026.4.29`，更新 AgenDao 自有包元数据、Web 根包版本与相关 lock 文件中的 AgenDao 自身条目。
- Scheduler session continuity 完成闭环治理：server 产出 markdown projection 与 JSON packet，orchestrator 通过 typed metadata 与 version gate 原子装载。
- 新增受控 hydration 能力：`scheduler_context_hydrate` 按 Source Anchors 回查同会话消息，`scheduler_memory_hydrate` 按 Memory Anchors 回查持久化记忆详情。
- Hydration 调用结果沉淀到 scheduler stage metadata，记录 hydrated / rejected / missing ids、截断参数与 evidence 开关，供前端和 telemetry 审计。
- 刷新文档入口与 scheduler 指南，明确 scheduler continuity 在上下文治理、授权召回和跨会话记忆上的当前行为。

## 2026.4.26

- 同步工作区版本到 `v2026.4.26`，更新 AgenDao 自有包元数据、Web 根包版本与相关 lock 文件中的 AgenDao 自身条目。
- `agendao web` 默认改为优先使用内嵌 Web 资源；仅在显式设置 `AGENDAO_WEB_DIST` 时才走外部 `dist/` 覆盖。
- `agendao-server/build.rs` 新增 Web 资源增量构建机制：只有 `apps/agendao-web` 源码缺失或变更时，才会自动触发 `npm run build` 并重新内嵌。
- 刷新 README、安装页、命令参考和文档入口，统一当前 Web 分发/构建机制与版本基线描述。

## 2026.4.21

- 同步当前版本基线到 `v2026.4.21`，统一工作区根配置、Web 包版本、文档入口、安装说明与示例文档中的版本标识。
- 收紧版本同步脚本的作用范围：仅更新 `agendao` 自身白名单文件，以及 lock 文件里的 `agendao-*` 包版本，不再有误改第三方依赖版本的风险。

## 2026.4.18

- 同步版本基线到 `v2026.4.18`，统一工作区根文档、安装说明、使用手册、Web 包版本与锁文件中的版本标识。
- 刷新文档入口与安装说明，使当前桌面 Web 启动路径、workspace 解析顺序、图标链路和正式前端入口描述保持一致。
- 清理文档索引中的陈旧路径与命名表述，减少根目录与 `docs/` 之间的跳转歧义。

## 2026.4.17

- 完成 TUI reratui 迁移主线收口：Phase 0-5 已按当前 hybrid app shell 边界结束，session subtree、消息渲染与热点交互已进入稳定态。
- 大幅更新 Web 界面：统一消息阅读节奏、收紧 sidebar / composer / header 密度、补齐更轻的 copy/footer 语法，并把 tool / status / structured block 纳入统一显示体系。
- Web composer 新增可检索 model picker，按 provider 分组展示模型、上下文窗口与能力 badge；输入框改为单行起始、最多 10 行增长。
- Web sidebar 新增 session 多选、批量删除与确认弹层，减少误删并提升会话管理效率。
- provider 模型读面补齐 capabilities，下游 Web/TUI 可直接消费视觉、音频、PDF、附件、tool-call、reasoning 等能力信息。
- CLI 新增 `agendao web --dir` / `agendao serve --dir`；无参数且非终端环境启动时，会自动走桌面 Web 启动路径，并先确定 workspace，再打开浏览器。
- Web 入口新增正式 `favicon`，为后续桌面安装包 / app bundle / shortcut icon 链路提供基础品牌资产。
- 仓库图标源资产统一落在 `icons/`；Web 改为消费派生 favicon，`windows-msvc` 编译会尝试嵌入 `agendao.ico`，并新增 Linux `agendao.desktop` 模板。
- 新增 `icons/agendao.icns`、macOS `AgenDao.iconset` 与 `scripts/build_macos_app_bundle.sh`，可组装带图标的 `AgenDao.app`。
- 文档与计划同步到 `v2026.4.17`，移除旧的假入口，并把当前 TUI/Web 状态改为与实现一致的描述。
