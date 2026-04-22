# Changelog

## 2026.4.22

- 同步工作区版本到 `v2026.4.22`，仅更新 ROCode 自有包元数据、Web 根包版本与 `Cargo.lock`/`package-lock.json` 中的 ROCode 自身条目。
- 收紧 `scripts/sync_version.sh`：不再自动重写 README、安装指南和示例文档，避免脚本误改历史日期、版本示例或第三方依赖说明。
- 逐份刷新根文档、命令说明、安装页、调度/插件/任务文档与插件示例，补齐 `/abort` 的控制通道语义、agent task 取消边界，以及当前插件引导行为。

## 2026.4.21

- 同步当前版本基线到 `v2026.4.21`，统一工作区根配置、Web 包版本、文档入口、安装说明与示例文档中的版本标识。
- 收紧版本同步脚本的作用范围：仅更新 `rocode` 自身白名单文件，以及 lock 文件里的 `rocode-*` 包版本，不再有误改第三方依赖版本的风险。

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
- CLI 新增 `rocode web --dir` / `rocode serve --dir`；无参数且非终端环境启动时，会自动走桌面 Web 启动路径，并先确定 workspace，再打开浏览器。
- Web 入口新增正式 `favicon`，为后续桌面安装包 / app bundle / shortcut icon 链路提供基础品牌资产。
- 仓库图标源资产统一落在 `icons/`；Web 改为消费派生 favicon，`windows-msvc` 编译会尝试嵌入 `rocode.ico`，并新增 Linux `rocode.desktop` 模板。
- 新增 `icons/rocode.icns`、macOS `ROCode.iconset` 与 `scripts/build_macos_app_bundle.sh`，可组装带图标的 `ROCode.app`。
- 文档与计划同步到 `v2026.4.17`，移除旧的假入口，并把当前 TUI/Web 状态改为与实现一致的描述。
