# Skill 系统完整参考

ROCode 的 Skill 系统提供能力的完整生命周期管理：从本地创建、远程分发、artifact 缓存到托管生命周期状态机。系统以 `SkillGovernanceAuthority` 为唯一治理入口，遵循 ROCode 宪法中的"唯一权限裁决"原则。

---

## 目录

- [架构概览](#架构概览)
- [核心治理：SkillGovernanceAuthority](#核心治理skillgovernanceauthority)
- [Skill Authority -- 本地读写权威](#skill-authority----本地读写权威)
- [Skill Hub -- 远程分发与托管](#skill-hub----远程分发与托管)
- [12 状态生命周期状态机](#12-状态生命周期状态机)
- [分发记录（Distribution Record）](#分发记录distribution-record)
- [Artifact 缓存](#artifact-缓存)
- [Artifact Policy 配置](#artifact-policy-配置)
- [Guard 引擎](#guard-引擎)
- [Sync Planner -- 同步规划](#sync-planner----同步规划)
- [Governance Timeline -- 治理时间线](#governance-timeline----治理时间线)
- [CLI 命令参考](#cli-命令参考)
- [数据类型参考](#数据类型参考)

---

## 架构概览

```
SkillGovernanceAuthority
  |
  +-- SkillAuthority              -- 本地 skill 读写（唯一写入点）
  +-- SkillHubStore               -- Hub 状态持久化
  +-- SkillSyncPlanner            -- 同步规划（diff + action）
  +-- SkillGuardEngine            -- 安全守卫（block / warn / pass）
  +-- SkillDistributionResolver   -- 分发解析（registry / git / archive）
  +-- SkillArtifactStore          -- Artifact 缓存管理
  +-- SkillLifecycleCoordinator   -- 生命周期状态推算
```

所有读写命令都通过 `rocode-server` 的 `/skill/hub/*` 路由进入 authority，不在 CLI 侧直接执行副作用。

---

## 核心治理：SkillGovernanceAuthority

`SkillGovernanceAuthority` 是 skill 系统的唯一治理入口。它组合了所有子组件，对外提供统一的操作接口。

### 构造

```rust
pub fn new(base_dir: impl Into<PathBuf>, config_store: Option<Arc<ConfigStore>>) -> Self
```

### 核心方法

| 方法 | 返回类型 | 说明 |
|------|---------|------|
| `skill_authority()` | `&SkillAuthority` | 本地 skill 读写权威 |
| `hub_store()` | `Arc<SkillHubStore>` | Hub 状态存储 |
| `sync_planner()` | `Arc<SkillSyncPlanner>` | 同步规划器 |
| `guard_engine()` | `Arc<SkillGuardEngine>` | 安全守卫引擎 |
| `distribution_resolver()` | `Arc<SkillDistributionResolver>` | 分发解析器 |
| `artifact_store()` | `Arc<SkillArtifactStore>` | Artifact 缓存 |
| `lifecycle()` | `Arc<SkillLifecycleCoordinator>` | 生命周期协调器 |
| `governance_snapshot()` | `SkillHubSnapshot` | 当前治理状态快照 |
| `managed_skills()` | `Vec<ManagedSkillRecord>` | 所有托管 skill 列表 |
| `distributions()` | `Vec<SkillDistributionRecord>` | 所有分发记录 |
| `artifact_cache()` | `Vec<SkillArtifactCacheEntry>` | Artifact 缓存条目 |
| `artifact_policy()` | `SkillHubPolicy` | 当前 artifact 策略 |

### 设计原则

- 唯一写入点：所有 skill 的副作用操作经由 authority 中转
- 只读查询：适配层可自由查询任何子组件
- 治理时间线：所有操作记录到 timeline，可审计
- Guard 保护：写操作经过 guard engine 校验

---

## Skill Authority -- 本地读写权威

`SkillAuthority` 是本地 skill 文件的唯一读写权威。它直接管理 `.rocode/skills/` 目录下的 skill 文件。

### 支持的操作

| 操作 | 对应类型 | 说明 |
|------|---------|------|
| 创建 skill | `CreateSkillRequest` | 创建新 skill |
| 编辑 skill | `EditSkillRequest` | 修改 skill 内容 |
| 补丁更新 | `PatchSkillRequest` | 部分字段更新 |
| 删除 skill | `DeleteSkillRequest` | 删除 skill |
| 写入文件 | `WriteSkillFileRequest` | 写入 skill 内的文件 |
| 删除文件 | `RemoveSkillFileRequest` | 删除 skill 内的文件 |

所有写操作返回 `SkillGovernedWriteResult`，包含 guard report。

---

## Skill Reflection 与自进化闭环

ROCode 当前的 skill 系统不只负责“安装和读取”，还负责把运行中的经验回流为更好的 skill 与 methodology。

### 运行时提示

- 当一个回合呈现“编辑后验证”“错误恢复”“多工具协同”“多轮用户引导收敛”等特征时，运行时会把该回合判定为 skillworthy candidate，并追加保存提示。
- 这个提示不是要求把整段对话原样保存，而是要求提炼出可复用的触发条件、步骤、验证方式和边界。

### Reflection 视图

- 如果一个 skill 在会话中被实际使用，运行时可以生成 Skill Usage Reflection。
- 反思内容会把 skill 中记录的方法步骤与真实 tool calls 并排呈现，用于判断 skill 是否已经过时、缺步、顺序失真或验证环节不足。
- 当偏差明显时，建议通过 `skill_manage("patch", ...)` 修补，而不是为微小变体频繁改写。

### 写入后的 Memory Linkage

`skill_manage` 不是孤立文件操作。写入结果会进入 memory authority 的 observation 链：

- `create` 可以生成与 skill 绑定的 methodology promotion 记录
- `patch` / `edit` / supporting file 变化会留下 linked skill observation
- guard 未通过或存在治理告警时，系统会保留 skill feedback lesson，供后续 consolidation 使用

### 从 Lesson 到 Methodology

skill 自进化并不止于“写回一个 SKILL.md”：

1. session 中的复杂执行先形成 lesson / pattern / candidate 级别的 memory 记录
2. 验证通过后，重复 lesson 会被 consolidation 聚成 pattern
3. 结构化 pattern 会被提升为 methodology candidate
4. 已链接 skill 的 methodology candidate 会反过来成为后续 patch / refine 的依据

这样 skill、session 和记忆不是三套分离系统，而是单向可审计的能力沉淀链。

## Skill 治理收口：不只会增，还会减，还会重新组织

这一轮计划之后，ROCode 的 skill 治理不再停留在“装上就算存在”。现在正式收口成五条线：

### 1. Usage Ledger

- authority 会维护 canonical skill usage / write ledger
- 账本同时记录 runtime 使用、报错、create / patch / edit / supporting file 写入、install / update / detach / remove 等写路径
- Web、CLI、TUI 的 inspection 都以这份账本为基础，而不是各端自己拼 usage 统计

### 2. Negative Entropy

- 很久未使用、长期零复用、被更稳定方法取代、或只剩历史包袱的 skill，会进入 negative entropy 诊断
- 这不是直接删；它先产出 owner-local `review_candidate` / `retired` 建议，再由治理动作决定
- vitality 判断现在不只看 usage ledger，还会消费 memory / proposal 的正向演化信号，避免刚被方法学吸收的 skill 被误判成纯噪音

### 3. Semantic Conflict

- 治理层会检查功能重叠、关键词重叠、tool / stage 条件重叠等 overlap 迹象
- 语义冲突不会直接覆盖现有 skill；它先形成只读 diagnostics，再通过 review sync / vitality set 落到 owner judgement

### 4. Composition Relationship / Capability Group

- skill 之间现在可以形成只读 relationship candidate，而不是永远把重复能力平铺为多个孤岛
- accepted relationship 和 active capability group 会进入后续 prompt hint / proposal retargeting
- methodology proposal 可以被 retarget 到 canonical composition target，避免重复地为近亲 skill 生成碎片化 patch

### 5. Runtime Gate

- inspection 和 runtime 已经分层：skill 可以继续在 detail / history / reflection 里可见，但不一定继续允许进入 runtime catalog
- `retired` skill 会被 runtime gate 阻止加载；`review_candidate` 仍可继续运行
- tool / server / prompt reflection 现在都通过统一 `SkillRuntimeResolver` 读取这条语义，不再各处散补“能不能加载”

---

## Skill Hub -- 远程分发与托管

Skill Hub 是远程 skill distribution、artifact cache 和 managed lifecycle 的统一入口。

### 源类型（SkillSourceKind）

| 类型 | 说明 |
|------|------|
| `Bundled` | 内置 skill（随 ROCode 发行） |
| `LocalPath` | 本地路径 |
| `Git` | Git 仓库 |
| `Archive` | 压缩包 |
| `Registry` | 远程 registry |

### 分发解析器类型

| 类型 | 说明 |
|------|------|
| `Bundled` | 内置 |
| `LocalPath` | 本地路径 |
| `RegistryIndex` | Registry 索引 |
| `RegistryManifest` | Registry manifest |
| `ArchiveManifest` | Archive manifest |
| `GitCheckout` | Git 检出 |

### Artifact 类型

| 类型 | 说明 |
|------|------|
| `RegistryPackage` | Registry 包 |
| `GitCheckout` | Git 检出 |
| `Archive` | 压缩包 |
| `LocalSnapshot` | 本地快照 |

### 安装流程

```
install-plan    ->  生成安装计划（不执行）
                    |
                    v
install-apply   ->  执行安装（写入工作区）
```

**install-plan** 阶段：

1. 从指定 source 解析 skill 的分发信息
2. 获取或下载 artifact
3. 运行 guard 检查
4. 生成安装计划（包含 distribution、artifact cache、guard report）

**install-apply** 阶段：

1. 执行安装计划中的写操作
2. 更新 lifecycle 状态为 `Installed`
3. 记录 governance timeline

### 更新流程

```
update-apply    ->  检查新版本 -> 下载 -> guard -> 安装
```

与安装流程类似，但需要已有安装记录。更新会将 lifecycle 从 `UpdateAvailable` 推进到 `Installed`。

### 卸载流程

| 操作 | 效果 |
|------|------|
| `detach` | 保留文件，解除托管关系 |
| `remove` | 删除文件，移除记录 |

---

## 12 状态生命周期状态机

`SkillManagedLifecycleState` 定义了托管 skill 的完整生命周期：

```
                    [远程来源]
                        |
                        v
                   +---------+
                   | Indexed |  源索引中发现
                   +----+----+
                        |
                        v
                  +-----------+
                  | Resolved  |  解析到具体 artifact
                  +----+------+
                       |
                       v
                   +---------+
                   | Fetched |  artifact 下载/缓存完成
                   +----+----+
                        |
                        v
               +----------------+
               | PlannedInstall |  安装计划生成
               +----+-----------+
                    |
                    v
               +-----------+
            +->| Installed | <--------+
            |  +-----+-----+          |
            |        |                 |
            |        v                 |
            |  +---------------+       |
            |  |UpdateAvailable|       |
            |  +-------+-------+       |
            |          |               |
            |          v               |
            |  +--------+  (update)   |
            |  |Diverged|------------>+
            |  +---+----+
            |      |
            |      v
            |  +-------------+
            |  |  Detached   |  保留文件，解除托管
            |  +------+------+
            |         |
            |         v
            |  +--------------+
            |  |RemovePending |
            |  +------+------+
            |         |
            |         v
            |  +---------+
            |  | Removed |
            |  +---------+
            |
            |  (失败路径)
            |  +-----------------+
            +->|ResolutionFailed |  解析失败
            |  +-----------------+
            |  +------------+
            +->| FetchFailed |  下载失败
            |  +------------+
            |  +------------+
            +->| ApplyFailed |  安装失败
               +------------+
```

### 状态说明

| 状态 | 触发条件 | 后续动作 |
|------|---------|---------|
| `Indexed` | 在源索引中发现 skill | 开始解析 |
| `Resolved` | 成功解析到具体 artifact | 开始下载 |
| `Fetched` | artifact 下载完成，缓存就绪 | 生成安装计划 |
| `PlannedInstall` | 安装计划生成（guard 通过） | 执行安装 |
| `Installed` | 安装成功，文件在工作区 | 正常运行状态 |
| `UpdateAvailable` | 检测到新版本（revision 不同） | 决定是否更新 |
| `Diverged` | 本地文件被修改或删除 | 需要人工干预 |
| `Detached` | 解除托管但保留文件 | 可重新托管或删除 |
| `RemovePending` | 标记待删除 | 确认后删除 |
| `Removed` | 已完全删除 | 终态 |
| `ResolutionFailed` | 分发解析失败 | 重试或报告 |
| `FetchFailed` | artifact 下载失败 | 重试或报告 |
| `ApplyFailed` | 安装执行失败 | 重试或报告 |

### 状态推算（managed_runtime_state）

`SkillLifecycleCoordinator` 根据当前记录推算运行时状态：

```rust
pub fn managed_runtime_state(
    &self,
    record: &ManagedSkillRecord,
    latest_revision: Option<&str>,
) -> SkillManagedLifecycleState
```

推算逻辑：

1. 如果 `deleted_locally` 或 `locally_modified` 为 true -> `Diverged`
2. 如果 `installed_revision` != `latest_revision` -> `UpdateAvailable`
3. 否则 -> `Installed`

---

## 分发记录（Distribution Record）

每个远程安装的 skill 都有一个 `SkillDistributionRecord`，记录其来源、版本和安装状态。

### 结构

```rust
pub struct SkillDistributionRecord {
    pub distribution_id: String,               // 唯一标识
    pub source: SkillSourceRef,                // 来源引用
    pub skill_name: String,                    // skill 名称
    pub release: SkillDistributionRelease,     // 发布版本信息
    pub resolution: SkillDistributionResolution, // 解析结果
    pub installed: Option<SkillInstalledDistribution>, // 安装信息
    pub lifecycle: SkillManagedLifecycleState,  // 当前生命周期状态
}
```

### SkillSourceRef

```rust
pub struct SkillSourceRef {
    pub source_id: String,          // 来源标识
    pub source_kind: SkillSourceKind, // 来源类型
    pub locator: String,            // 定位符（URL、路径等）
    pub revision: Option<String>,   // 版本修订
}
```

### SkillDistributionRelease

| 字段 | 说明 |
|------|------|
| `version` | 语义化版本 |
| `revision` | 修订标识 |
| `checksum` | 校验和 |
| `manifest_path` | manifest 文件路径 |
| `published_at` | 发布时间戳 |

### SkillInstalledDistribution

| 字段 | 说明 |
|------|------|
| `installed_at` | 安装时间戳 |
| `workspace_skill_path` | 工作区内的 skill 路径 |
| `installed_revision` | 安装的版本修订 |
| `local_hash` | 本地文件哈希 |

---

## Artifact 缓存

`SkillArtifactCacheEntry` 记录已下载的 artifact 的缓存状态。

### 结构

```rust
pub struct SkillArtifactCacheEntry {
    pub artifact: SkillArtifactRef,          // artifact 引用
    pub cached_at: i64,                       // 缓存时间
    pub local_path: String,                   // 本地存储路径
    pub extracted_path: Option<String>,       // 解压路径
    pub status: SkillArtifactCacheStatus,     // 缓存状态
    pub error: Option<String>,                // 错误信息
}
```

### SkillArtifactCacheStatus

| 状态 | 说明 |
|------|------|
| `Fetched` | 已下载 |
| `Extracted` | 已解压 |
| `Failed` | 处理失败 |
| `Evicted` | 已驱逐（过期清理） |

### 缓存驱逐

`reconcile_artifact_cache_policy` 方法根据 `SkillHubPolicy` 中配置的保留时间清理过期缓存。

---

## Artifact Policy 配置

Artifact policy 通过 `rocode.jsonc` 的 `skills.hub` 字段配置：

```jsonc
{
  "skills": {
    "hub": {
      "artifactCacheRetentionSeconds": 604800,   // 7 天
      "fetchTimeoutMs": 30000,                    // 30 秒
      "maxDownloadBytes": 8388608,                // 8 MB
      "maxExtractBytes": 8388608                  // 8 MB
    }
  }
}
```

### SkillHubPolicy 字段

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `artifact_cache_retention_seconds` | `u64` | 604800 | artifact 缓存保留时间（秒） |
| `fetch_timeout_ms` | `u64` | 30000 | 下载超时（毫秒） |
| `max_download_bytes` | `u64` | 8388608 | 单次下载最大字节数 |
| `max_extract_bytes` | `u64` | 8388608 | 解压后最大字节数 |

> authority 会把当前生效值暴露到 `/skill/hub/policy`，CLI/TUI/Web 都应读取这一正式读面，而不是各端自己解析配置文件。

---

## Guard 引擎

`SkillGuardEngine` 在写操作前执行安全检查，可以阻止、警告或放行操作。

### Guard 状态

| 状态 | 说明 |
|------|------|
| `Passed` | 检查通过，允许操作 |
| `Warn` | 有警告但允许操作 |
| `Blocked` | 阻止操作 |

### Guard Report

```rust
pub struct SkillGuardReport {
    pub skill_name: String,
    pub status: SkillGuardStatus,
    pub violations: Vec<SkillGuardViolation>,
    pub scanned_at: i64,
}
```

### Guard Violation

```rust
pub struct SkillGuardViolation {
    pub rule_id: String,              // 规则标识
    pub severity: SkillGuardSeverity, // 严重程度
    pub message: String,              // 违规消息
    pub file_path: Option<String>,    // 涉及的文件路径
}
```

### 严重程度

| 级别 | 说明 |
|------|------|
| `Info` | 信息提示 |
| `Warn` | 警告 |
| `Error` | 错误（可能阻止操作） |

---

## Sync Planner -- 同步规划

`SkillSyncPlanner` 对比源索引和本地状态，生成同步计划。

### SkillSyncAction

| 动作 | 说明 |
|------|------|
| `Install` | 需要安装 |
| `Update` | 需要更新 |
| `SkipLocalModification` | 跳过（本地有修改） |
| `SkipDeletedLocally` | 跳过（本地已删除） |
| `RemoveManaged` | 需要移除托管 |
| `Noop` | 无需操作 |

### SkillSyncPlan

```rust
pub struct SkillSyncPlan {
    pub source_id: String,
    pub entries: Vec<SkillSyncEntry>,
}
```

### 远程安装计划

`SkillRemoteInstallPlan` 包含完整的安装上下文：

```rust
pub struct SkillRemoteInstallPlan {
    pub source_id: String,
    pub distribution: SkillDistributionRecord,
    pub entry: SkillRemoteInstallEntry,
}
```

`SkillRemoteInstallAction` 只有两个值：`Install`（首次安装）或 `Update`（版本更新）。

---

## Governance Timeline -- 治理时间线

所有 skill 操作都会记录到 governance timeline，提供完整的审计追踪。

### Timeline 种类（SkillGovernanceTimelineKind）

| 种类 | 说明 |
|------|------|
| `ManagedSnapshot` | 托管状态快照 |
| `SourceIndexRefreshed` | 源索引刷新 |
| `SourceResolved` | 源解析完成 |
| `ArtifactFetched` | Artifact 下载完成 |
| `ArtifactEvicted` | Artifact 缓存驱逐 |
| `ArtifactFetchFailed` | Artifact 下载失败 |
| `RemoteInstallPlanned` | 远程安装计划生成 |
| `RemoteUpdatePlanned` | 远程更新计划生成 |
| `LifecycleTransitioned` | 生命周期状态转换 |
| `Create` | 创建 skill |
| `Patch` | 补丁更新 skill |
| `Edit` | 编辑 skill |
| `Delete` | 删除 skill |
| `WriteFile` | 写入 skill 文件 |
| `RemoveFile` | 删除 skill 文件 |
| `HubInstall` | Hub 安装 |
| `HubUpdate` | Hub 更新 |
| `HubDetach` | Hub 解除托管 |
| `HubRemove` | Hub 删除 |
| `SyncPlanCreated` | 同步计划创建 |
| `SyncApplyCompleted` | 同步执行完成 |
| `GuardBlocked` | Guard 阻止操作 |
| `GuardWarned` | Guard 发出警告 |

### Timeline 状态

| 状态 | 说明 |
|------|------|
| `Info` | 信息 |
| `Success` | 成功 |
| `Warn` | 警告 |
| `Error` | 错误 |

### Timeline 条目

```rust
pub struct SkillGovernanceTimelineEntry {
    pub entry_id: String,
    pub kind: SkillGovernanceTimelineKind,
    pub created_at: i64,
    pub skill_name: Option<String>,
    pub source_id: Option<String>,
    pub actor: Option<String>,
    pub title: String,
    pub summary: String,
    pub status: SkillGovernanceTimelineStatus,
    pub managed_record: Option<ManagedSkillRecord>,
    pub guard_report: Option<SkillGuardReport>,
    pub payload: serde_json::Value,
}
```

---

## CLI 命令参考

所有 CLI 命令通过 `rocode skill hub` 前缀调用。

### 查询命令

| 命令 | 说明 |
|------|------|
| `rocode skill hub status` | 查看当前托管 skill 状态 |
| `rocode skill hub managed` | 查看 managed provenance 记录 |
| `rocode skill hub usage` | 查看 canonical usage / write ledger |
| `rocode skill hub negative-entropy` | 查看 negative entropy 诊断 |
| `rocode skill hub semantic-conflicts` | 查看 semantic overlap 诊断 |
| `rocode skill hub index` | 查看 source index 缓存 |
| `rocode skill hub distributions` | 列出所有分发记录 |
| `rocode skill hub artifact-cache` | 查看 artifact 缓存状态 |
| `rocode skill hub policy` | 查看当前 artifact 策略 |
| `rocode skill hub lifecycle` | 查看所有 skill 的生命周期状态 |

### 安装/更新命令

| 命令 | 说明 |
|------|------|
| `rocode skill hub install-plan --source-id <id> --source-kind registry --locator <locator> --skill-name <name>` | 生成安装计划（不执行） |
| `rocode skill hub install-apply --session-id <session> --source-id <id> --source-kind registry --locator <locator> --skill-name <name>` | 执行安装 |
| `rocode skill hub update-apply --session-id <session> --source-id <id> --source-kind registry --locator <locator> --skill-name <name>` | 执行更新 |

### 治理动作命令

| 命令 | 说明 |
|------|------|
| `rocode skill hub review-candidates-sync --session-id <session>` | 把当前 negative-entropy review candidate 同步到 workspace-local vitality |
| `rocode skill hub semantic-conflict-review-sync --session-id <session>` | 把当前 semantic conflict review candidate 同步到 workspace-local vitality |
| `rocode skill hub vitality-set --session-id <session> --skill-name <name> --state <state> --summary <text>` | 显式设置某个 skill 的 vitality judgement |

### 卸载命令

| 命令 | 说明 |
|------|------|
| `rocode skill hub detach --session-id <session> --source-id <id> --source-kind registry --locator <locator> --skill-name <name>` | 解除托管（保留文件） |
| `rocode skill hub remove --session-id <session> --source-id <id> --source-kind registry --locator <locator> --skill-name <name>` | 完全删除 |

## Proposal Inbox

memory consolidation 产出的 methodology candidate 现在可以进一步生成 skill evolution proposal。它们不会直接改写 `SKILL.md`，而是进入单独的 proposal inbox：

- CLI: `rocode skill proposal list --status draft`
- CLI: `rocode skill proposal show <id>`
- CLI: `rocode skill proposal approve <id>`
- CLI: `rocode skill proposal reject <id>`
- Web / TUI: proposal review 面板

proposal notice 属于 runtime hint，不会污染 model-visible prompt surface；它只提醒用户有新的治理材料待裁决。

---

## 数据类型参考

### SkillSourceRef

| 字段 | 类型 | 说明 |
|------|------|------|
| `source_id` | `string` | 来源标识 |
| `source_kind` | `SkillSourceKind` | 来源类型（Bundled/LocalPath/Git/Archive/Registry） |
| `locator` | `string` | 定位符 |
| `revision` | `string?` | 版本修订 |

### SkillArtifactRef

| 字段 | 类型 | 说明 |
|------|------|------|
| `artifact_id` | `string` | artifact 标识 |
| `kind` | `SkillArtifactKind` | artifact 类型 |
| `locator` | `string` | 定位符 |
| `checksum` | `string?` | 校验和 |
| `size_bytes` | `u64?` | 大小（字节） |

### ManagedSkillRecord

| 字段 | 类型 | 说明 |
|------|------|------|
| `skill_name` | `string` | skill 名称 |
| `source` | `SkillSourceRef?` | 来源引用 |
| `installed_revision` | `string?` | 已安装的版本 |
| `local_hash` | `string?` | 本地文件哈希 |
| `last_synced_at` | `i64?` | 上次同步时间 |
| `locally_modified` | `bool` | 是否有本地修改 |
| `deleted_locally` | `bool` | 是否已本地删除 |

### BundledSkillManifest

| 字段 | 类型 | 说明 |
|------|------|------|
| `bundle_id` | `string` | bundle 标识 |
| `entries` | `BundledSkillManifestEntry[]` | 包含的 skill 条目 |

### SkillSourceIndexSnapshot

| 字段 | 类型 | 说明 |
|------|------|------|
| `source` | `SkillSourceRef` | 源引用 |
| `updated_at` | `i64` | 更新时间 |
| `entries` | `SkillSourceIndexEntry[]` | 索引条目 |
