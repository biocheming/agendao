# J-space 启发的任务治理落地计划

状态：Phase 1 完成；Phase 2 契约+权威完成（含两轮审阅修复）；Phase 3–5 核心落地（缺口见 §3bis）；Phase 6 harness 通过隔离/绑定验证，待正式运行；尚未启用默认路由

日期：2026-08-18

范围：skill 运行实验、session task ledger、scheduler/runtime seam、compaction/recovery、
Web/TUI 任务状态视图、同条件 A/B 评测

## 1. 决策摘要

本计划吸收 J-space 中可验证的工程机制，但不复制其意识、自省或私有思维叙事。

最终目标是把 AgenDao 已有但分散的 Todo、ToolBatchSummary、scheduler checkpoint、
session runtime、compaction continuity 和 recovery 收敛成一个服务端权威的任务状态契约：

```text
Goal      什么叫完成
Core      当前必须保持一致的核心约束
Verified  已验证结论、验证器、覆盖范围和证据
Open      未决问题，以及什么证据可以解决它
Next      唯一的下一动作
```

落地必须遵守以下边界：

1. J-space 先作为可选 skill 做实验，不设为默认 skill，不自动注入所有请求。
2. 原生实现不读取或依赖 `.jspace/WORKSPACE.md`；session 数据只能有一个服务端权威。
3. 不持久化隐藏思维、chain-of-thought 或所谓 inner register，只保存用户可审计的任务状态和证据。
4. 所有模型可见任务状态必须通过现有 prompt-surface authority 投影，不新增旁路 prompt 拼接。
5. 所有前端只消费 typed API/event，不自行从自然语言消息推断 ledger 状态。
6. 停滞检测先观察、再提示、最后才允许 scheduler 重新规划；不得直接执行破坏性恢复。
7. J-space README 中的 benchmark 数字不作为上线依据；AgenDao 必须运行自己的同条件实验。

## 2. 当前基础与缺口

| 主题 | AgenDao 已有基础 | 缺口 | 本计划落点 |
| --- | --- | --- | --- |
| 任务列表 | `agendao-types/src/todo.rs` 和 TodoWrite | 没有 Goal、验证证据、settled-by、唯一 Next | `SessionTaskLedger` |
| 工具结果 | `ToolBatchSummary` 已含 goal status、blocked、unresolved、next step | 一次性注入后被消费，不是持久任务状态 | seam reducer 写入 ledger candidate |
| 调度 | Direct/Plan/Coordinate/Verify/Autoresearch 模板 | task shape 没有持续时间、状态承载和恢复风险 | selector 增加 governance class |
| 验证 | Verify Gate、evaluator、checkpoint capability | 没有统一 claim/verifier/coverage/evidence 契约 | typed checkpoint record |
| 运行状态 | per-session runtime state 和 typed frontend events | 只描述当前运行，不描述任务进展 | runtime 增加 ledger summary/progress observation |
| 压缩 | context compaction continuity packet | 未规定 Goal/Verified/Open/Next 不可丢失 | ledger 纳入 continuity packet |
| 恢复 | retry/resume/abort recovery protocol | Resume 主要重新包装原始 prompt | 从 checkpoint + Next 恢复 |
| Skill | `skill_view(name, file_path)` 支持 supporting files 按需加载 | 尚无 J-space 本地实验基线 | 可选 skill 安装与 A/B |

### 对十条 J-space 建议的取舍

| 建议 | 决策 | 落点与边界 |
| --- | --- | --- |
| 压缩产物采用 Goal/Core/Verified/Open/Next | 采纳 | Phase 3 纳入 continuity packet；不让自然语言摘要成为新权威 |
| 完成声明携带 verifier/coverage | 采纳 | Phase 2 typed checkpoint + Phase 3 final delivery gate |
| 所有中间 message part 增加 `✓/?/✗` | 调整后采纳 | 首版只给 ledger claim/checkpoint 建 typed 状态；不改所有 part，避免大迁移和双重真值 |
| 语义停滞检测 | 采纳 | Phase 4；基于 typed fact，不分析隐藏思维或词频 |
| `I/we/let's` 人称控制语法进入 builtin agent | 仅实验 | Phase 6 单独设 prompt variant；不得先写进所有 builtin prompt，也不以代词频率作为质量指标 |
| inner/ledger/outer 三寄存器 | 只采纳边界 | ledger 可审计、outer 可交付；不建立、不存储、不要求暴露 inner/chain-of-thought |
| `Next` 常驻 Web/TUI | 采纳 | Phase 5；来自服务端 ledger revision，不从流式文本猜测 |
| 外部工具/MCP/plugin 内容标记 untrusted | 采纳为独立安全工作流 | 在 ingress 到 prompt-surface 之间保留来源与信任标签；不与权限等级混为一谈 |
| Skill 单入口与编写期验证 | 调整后采纳 | Phase 1 扩展现有 skill guard；J-space 的逐字 Premise 规则只属于该 skill，不强加给所有 skill |
| 预注册证伪条件与多运行评测 | 采纳 | Phase 6；单次结果、词频和厂商分数不得充当上线证据 |

上述取舍刻意区分三种状态：已验证系统事实、待实验的 prompt 假设、第三方项目的解释性叙事。
三者不能写进同一个 typed authority，也不能因为 J-space 自带报告给出正向数字就跳过 AgenDao 自测。

## 3. 实施顺序

实施必须按 Phase 1 到 Phase 6 顺序推进。每个 Phase 通过退出门槛后才能进入下一阶段，
不得一边定义 ledger、一边让多个前端各自发明临时结构。

## Phase 1：安装可选 J-space Skill，建立实验基线

### 当前状态

本地安装已完成：

```text
source:  ../J-space/j-space/
target:  ~/.agendao/skills/j-space/
```

目标目录包含：

- `SKILL.md`
- `modules/`
- `references/`
- `scripts/`
- 上游 `LICENSE` 和 `CITATION.cff`

源文件基线：

```text
SKILL.md sha256: 83ac50482fd6feb79ed5b2b762277bb9a5fe4b419f077cf0d890a99cd75c99e2
jspace.py sha256: 8f9551f3c1000ec239e8131f2ad7751f52ebc760d034bde7cce3a1d3645323bb
```

2026-08-18 验证结果：

- 上游 `scripts/verify_suite.py` 通过：one entry、one premise、nine modules、无版本话术。
- 临时启动本地 AgenDao 服务后，`GET /skill/catalog` 返回 `j-space`。
- catalog 解析位置为 `~/.agendao/skills/j-space/SKILL.md`，状态为 `disabled: false`。
- `SKILL.md`、9 个 module、3 个 reference、3 个 script、`LICENSE` 和 `CITATION.cff`
  均被识别为主入口或 supporting file。

### 实施任务

1. 确认 AgenDao skill catalog 能发现 `j-space`，并能分别加载主入口和 supporting files。
2. 保持手动选择，不加入 builtin agent 默认 skill，不加入 auto selector 强制规则。
3. 第一轮实验允许使用原始 skill 文本；控制器只在隔离测试 workspace 使用。
4. 不把 `.jspace/` 文件解释为 AgenDao session authority，也不把它同步进 session DB。
5. 建立至少三类任务样本：单步、有限多步、长程多工具。
6. 记录 control 与 skill-on 两组的请求、模型、采样参数、工具集、耗时、token 和结果。
7. 用现有 skill guard 验证 frontmatter、链接文件、路径边界和可加载性；缺失的通用校验加到
   guard profile，不把 J-space 专属的 Premise 字节一致规则变成全局格式要求。
8. 为安装副本保存 source、license、citation 和内容 hash；升级必须重新运行 guard 与 smoke A/B。

### 退出标准

- `skills_list`/`skill_view` 可发现并加载 J-space 及指定 module。
- skill guard 能对安装副本给出可审计结果；不因 supporting file 或 license 产生误报。
- 禁用或不选择 J-space 时，serialized prompt 与当前基线一致。
- skill-on 不绕过权限、scheduler、tool registry 和 prompt-surface authority。
- 至少完成一轮同模型、同任务、同工具条件的 smoke A/B；该结果只用于发现集成问题，
  不用于宣称能力提升。

### 2026-08-18 smoke A/B 记录（仅用于发现集成问题，不构成能力结论）

条件：同 server（HTTP attach）、同模型 `deepseek/deepseek-chat`（auth.json deepseek key）、
同任务（median + unittest 三例）、同工具集、同权限观察器（对全部 pending permission 自动
回复 `turn`，两组一致）、同初始 fixture。

| 组 | 会话 | 耗时 | 工具部件 | j-space 引用 | 结果 |
| --- | --- | --- | --- | --- | --- |
| control | ab-control-r1 (ses_b2ff6dfad…) | 25s | 15 | 0 | 3/3 测试通过 |
| skill-on | ab-skill-r1 (ses_08a0c7bb…) | 27s | 9 | 51（skills_list/skill_view 实际调用） | 3/3 测试通过 |

退出标准核对：catalog/detail 发现与 17 个 supporting files ✅；guard warn 4 / error 0（均为
风格启发，非阻断）✅；control 转录零 j-space 内容 ✅（skills_list 结果中的目录枚举不计）；
权限无绕过 ✅；禁用等价基线由单测 `disabled_skill_runtime_catalog_matches_absent_baseline`
钉死 ✅。

集成发现（须在后续 Phase 处理）：

1. **`agendao run "msg" --flags` 会把 flag 静默吞进消息**（MESSAGE 为 trailing var arg）：
   `--attach` 等全部变成消息文本，模式判定退回 Direct 且无任何警告。应在 message 位置参数
   中检测已知 flag 前缀并报错，或强制 flags-before-message 并给出提示。
2. **非交互 run 客户端超时后，服务端 run 仍在等待权限**：CLI 300s 超时退出是纯客户端行为，
   不触发服务端 abort；pending permission 残留至被回复或 300s 服务端超时。客户端放弃时应
   调用 `/abort` 收敛服务端状态。
3. 服务器无请求日志（排查 attach 异常时只能靠进程行为推断）；建议 serve 增加可开关的
   request log。

## Phase 2：定义服务端权威的 SessionTaskLedger

### 建议类型

新增 `agendao-types` typed contract，建议独立文件 `task_ledger.rs`：

```rust
pub struct SessionTaskLedger {
    pub session_id: String,
    pub revision: u64,
    pub goal: Option<TaskGoal>,
    pub core: Vec<CoreConstraint>,
    pub verified: Vec<VerifiedCheckpoint>,
    pub open: Vec<OpenQuestion>,
    pub next: Option<NextAction>,
    pub status: TaskLedgerStatus,
    pub awaiting_interaction: Option<AwaitingInteractionRef>,
    pub updated_at: i64,
}

pub enum TaskLedgerStatus {
    Active,
    AwaitingUser,
    Blocked,
    Interrupted,
    Completed,
}

pub struct AwaitingInteractionRef {
    pub kind: AwaitingInteractionKind,
    pub interaction_id: String,
}

pub struct VerifiedCheckpoint {
    pub id: String,
    pub claim: String,
    pub verifier: VerifierRef,
    pub coverage: VerificationCoverage,
    pub evidence_artifact_ids: Vec<String>,
    pub source_stage_id: Option<String>,
    pub created_at: i64,
}

pub struct OpenQuestion {
    pub id: String,
    pub question: String,
    pub settled_by: String,
    pub opened_at: i64,
    pub closed_by_checkpoint_id: Option<String>,
}
```

具体字段可根据现有 artifact/evaluator 类型调整，但以下不变量不能削弱：

1. `Verified` append-only；修正旧结论使用 supersede 关系，不覆盖历史。
2. checkpoint 必须同时有 claim、verifier、coverage；仅有“测试通过”字符串不合法。
3. Open 必须有稳定 ID 和 `settled_by`；关闭时必须引用同 session 的 checkpoint。
4. active/blocked 状态下 `Next` 不得为空；completed 状态可以为空。
5. awaiting_user 必须携带 interaction kind/id，并使用 typed Next 表达“等待哪一个用户决定”；
   不能只保存一段不可关联权限或问题记录的字符串。
6. interrupted 保留中断前 Next，但其 provenance 必须标记为 pre-interrupt，不得冒充恢复后新计划。
7. Core 允许多条持久约束，但最多两条标记为 live；换入换出必须产生 revision/event。
8. 所有写操作带 expected revision，避免多 agent 并发时静默覆盖。
9. ledger 按 session 隔离，禁止按 cwd 使用一份全局文件。

### 权威与持久化

- 写入权威放在 server/session runtime 一侧，不放在 Web、TUI、CLI 或 skill 脚本。
- session DB/artifact 是持久化真值；runtime memory 只保存当前 snapshot。
- 新 session 默认无 ledger，首次进入 structured/loop governance 时原子创建。
- 已有 session 无需迁移伪数据，读取时返回 empty ledger snapshot。

### API 与事件

建议最小接口：

```text
GET   /session/{id}/task-ledger
PATCH /session/{id}/task-ledger          expected_revision required
POST  /session/{id}/task-ledger/checkpoint
POST  /session/{id}/task-ledger/open
POST  /session/{id}/task-ledger/open/{open_id}/close
```

前端事件只需要一个 canonical replacement event：

```text
task-ledger.replaced { session_id, ledger, cause }
```

`cause` 使用 typed enum 标明 seam/revision 来源，不使用自由文本。不要为每个字段建立一套可能
乱序的局部 patch 事件。

事件分级：

- Web/TUI live tier 接收每个 committed replacement。
- CLI `final_only` 不接收普通进度噪声，但必须接收 awaiting_user、blocked、interrupted、completed
  状态以及 `final response committed` seam 的 replacement。
- ledger 是可审计任务治理状态，不属于 reasoning；任何 tier 都不得因 reasoning filter 将上述边界事件丢弃。
- reconnect 后的 GET snapshot 是最终对账权威，事件流不承担历史重放。

### 退出标准

- HTTP、Unix、Direct 三种 transport 读写同一 authority。
- revision conflict 返回明确错误，不覆盖较新状态。
- 非 checkpoint 不能关闭 Open；缺 verifier/coverage 的 checkpoint 被 typed validation 拒绝。
- session 删除同时删除 ledger；session fork 明确复制 snapshot 还是建立 provenance 引用。
- Rust 单元测试覆盖创建、并发 revision、checkpoint、open/close、fork 和删除。

## Phase 3：把真实执行边界定义为 Seam

### Seam 定义

Seam 是系统已经能够确定的执行边界，不依赖模型输出特殊口令。首版只接受：

- prompt/run started
- tool batch completed
- scheduler node/stage completed
- evaluator gate completed
- compaction before/after
- recovery retry/resume/abort
- final response committed
- user handoff/awaiting-user

文件写入本身不单独成为 seam；由对应 tool batch completion 汇总，避免高频 edit 产生噪声。

### Reducer

新增单一 ledger reducer，将 typed execution facts 转换为候选更新：

- `ToolBatchSummary.unresolved_items` -> Open candidate
- `recommended_next_step` -> Next candidate
- evaluator pass + artifact -> Verified checkpoint
- blocked reasons -> ledger status/observation
- scheduler stage handoff -> Core/Next candidate

模型建议不能直接伪装成已验证事实。只有 evaluator、确定性检查器或明确的用户确认能创建
Verified checkpoint。

Candidate 不是新领域对象或持久化记录。首版 candidate 只存在于单次 seam reducer transaction
中：由当前 typed facts 确定性生成，在同一 transaction 内接受、拒绝或合并，然后立即释放。
只有通过 invariant validation 且成功提交 revision 的结果进入 ledger；禁止新增 candidate 表、
candidate API 或跨 seam candidate queue。

### Prompt 投影

模型可见 ledger 使用固定、紧凑、typed 渲染：

```text
<task-ledger revision="12">
Goal: ...
Core: ...
Verified: latest N checkpoint summaries with evidence refs
Open: ...
Next: ...
</task-ledger>
```

- 投影只能经 prompt-surface authority 插入。
- 默认只带最新 checkpoint 摘要；完整证据通过 artifact/reference 按需加载。
- 禁止把 controller 的 inner register、情绪 marker 或原始 hidden reasoning 注入模型上下文。

### 外部输入来源与信任边界

外部检索、MCP、plugin 和远程 skill 内容需要保留 typed provenance，例如来源类型、资源标识、
获取时间和 `untrusted_external` 标志。该标志表达“内容可能包含面向模型的指令”，不是“文件有害”，
也不自动等价于拒绝、需要权限或扫描命中。

- provenance 在 ingress 建立，并随 tool result/artifact 进入 prompt-surface authority。
- prompt 投影必须明确区分用户指令、系统治理、工具数据和外部文档中的指令性文本。
- 模型建议执行外部文本中的命令时，仍必须经过正常 tool schema、permission 和 workspace 边界。
- 首版只记录、提示和审计，不靠 fake/fraud/injection 关键词正则自动删除内容或封禁来源。
- provenance 丢失时采取保守的 untrusted 默认值，并产生可观测诊断；不得静默升级为 trusted。

这条安全工作流与 ledger 共享 prompt-surface 和 artifact 基础设施，但不写入 Goal/Verified/Open/Next，
避免把输入信任状态错误地变成任务完成状态。

### Final Delivery Gate

在 `final response committed` seam 前增加结构化交付检查，而不是只看 provider `finish_reason`：

1. completed run 必须逐项关联 Goal acceptance criterion 与 checkpoint/evidence，或明确标为未覆盖。
2. active Open 必须出现在未完成边界中；不能用措辞把 Open 隐藏成已完成。
3. `Verified` 必须已有 verifier、coverage 和 evidence ref；final 文本不能反向创建 Verified。
4. outer 输出不得泄漏内部控制 marker、controller 账本语法或隐藏 reasoning。
5. gate 失败先降级为 partial/blocked 并生成具体诊断；不得仅凭英文正则重写用户答案。

首版 gate 校验 typed metadata 与引用完整性。自然语言一致性检查进入 evaluator/A-B 实验，不能成为
不可解释的硬阻断器。

### Compaction 与 Recovery

- `SessionContinuityPacket` 增加 ledger revision、Goal、Open、Next 和 checkpoint refs。
- compaction 前后 ledger fingerprint 必须一致，除非同一过程中产生显式 revision。
- Resume 从最近有效 checkpoint 和 Next 构造恢复上下文；原始 prompt 仅作为背景，不再是唯一依据。
- Retry 保留 Verified，不把失败输出升级为 checkpoint。
- Abort 与现有运行语义保持一致：清空 queued follow-ups、取消 pending permissions，ledger 不得
  残留对应的 awaiting_user；响应继续报告 dropped/cancelled 数量。pending question 的 abort
  行为必须在 Phase 3 核实并显式定义，不能假定它已与 permission 共用生命周期。
- Abort 保留 ledger 历史并把 status 置为 interrupted；保留中断前 Next 作为 pre-interrupt provenance，
  但恢复后的首个有效 seam 必须确认或替换它，才能重新标记为 active Next。

### 退出标准

- 每类 seam 都有 typed event/reducer 单测。
- tool batch 不会重复创建同一个 Open 或 checkpoint，幂等键稳定。
- compaction 前后 Goal/Verified/Open/Next 不丢失。
- recovery 测试证明 Resume 从 checkpoint + Next 继续，而不是从头重新发现。
- prompt snapshot 测试证明 ledger 只有一个注入位置。
- final delivery gate 拒绝无 coverage 的 completed 状态，并保留 partial/blocked 的诚实交付。
- 外部 ingress provenance 能穿过 tool result、artifact 和 prompt projection，且不绕过 permission。

## Phase 4：任务级停滞检测与受控重新规划

### 观察窗口

仅对 structured/loop 任务启用。默认观察最近三个 semantic seam：

- `Next` 三次不变；
- Verified 数量无增长；
- Open 数量每次增长；
- 有新 checkpoint，但 Next 不变；
- 同一 tool/error fingerprint 重复出现。

这些是 observation，不直接等于 failure。

detector 在 awaiting_user 期间静默，不累计 seam 窗口或 wall-clock stall；等待权限/问题回复不是
推理停滞。interrupted 状态也不检测，resume 后首个 seam 只建立新基线，不把 pre-interrupt Next
计入“三次不变”。

### 状态机

```text
healthy -> suspected -> stalled -> replanning -> healthy|blocked
```

- 第一次命中：记录 telemetry，不打断。
- 连续命中：向模型和 UI 提示具体事实，不使用“似乎卡住”之类无证据文案。
- 达到 stalled：scheduler 可以选择改变策略、调用 evaluator、转 empirics 型测试或请求用户。
- 重新规划必须产生新的 Next 或明确 Blocked reason；否则不得声称已恢复。
- 自动重新规划次数受 execution budget 限制，耗尽后进入 Blocked，不无限循环。

### 退出标准

- detector 输入只来自 ledger snapshots 和 typed execution facts。
- direct/fast 任务不会因三个普通工具调用被误报。
- 相同 Next 但 Verified 持续增长的深度任务不会直接判 stalled。
- 重新规划有最大次数、deadline 和取消测试。
- telemetry 能解释是哪条规则触发、覆盖哪些 seam、采取了什么动作。

## Phase 5：Web/TUI Task State 视图

### 信息架构

新增紧凑的 Task State 视图，而不是另一套聊天卡片：

- Goal：一行，显示来源和最后修改者。
- Next：最突出，显示当前 revision。
- Open：稳定编号、settled-by、是否 blocked。
- Verified：checkpoint 编号、claim、verifier、coverage、证据入口。
- Core：默认折叠，只显示 live/parked 状态。
- Progress observation：仅在 suspected/stalled/blocked 时显示。

### 交互约束

- Web/TUI 都通过相同 API 和 replacement event 更新。
- Web 必须扩展现有 `useSessionRuntimeReconcile`（或将其提升为统一 session reconcile），把
  `/session/{id}/task-ledger` 纳入同一套 session 切换、SSE 重连和 visibility 恢复对账；禁止新建
  第二套独立监听器。TUI 同样复用现有 runtime snapshot/reconcile 入口。
- 用户可编辑 Goal/Core/Next，但必须走 revision check，并显示模型/用户/evaluator provenance。
- 用户关闭 Open 仍需选择或创建 checkpoint，UI 不提供“直接标完成”捷径。
- checkpoint evidence 可跳转到 tool call、stage、artifact 或测试输出。
- 默认折叠，避免运营型界面被新的卡片层淹没。

### 退出标准

- 切换 session、SSE 重连、tab 恢复后通过 GET snapshot 对账。
- 两个前端对同一 ledger revision 显示一致。
- 冲突写入向用户展示“状态已更新，请重试”，不静默覆盖。
- mobile/窄终端不截断 Next、Open ID 和 checkpoint status。
- 不显示隐藏推理，不使用“模型正在想什么”的文案。

## Phase 6：A/B 评测与默认策略决策

### 实验组

至少比较：

1. Control：当前 AgenDao，不加载 J-space，不启用 native ledger governance。
2. Skill-only：加载原始 J-space skill，不启用 native ledger。
3. Native-ledger：不加载 J-space，启用 AgenDao typed ledger/seam/recovery。
4. Combined：仅在前三组证明互补后运行，避免默认认为机制可叠加。
5. Pronoun-control probe：只在前四组稳定后，以 prompt variant 测试 `I/we/let's` 控制语法；
   代词计数只能作为行为探针，不能作为能力或质量主指标。

### 控制变量

- 相同 model/provider/model revision
- 相同 prompt、workspace snapshot、工具集合和权限模式
- 相同 scheduler policy、上下文预算、采样参数和超时
- 每个任务至少 5 个随机种子；确定性模型记录重复运行而非伪造 seed
- 失败和人工介入都必须计入，不删除异常样本

### 任务集

- 单步修复：验证治理开销不会伤害短任务。
- 多文件一致性修改：测 Core 广播和完成核对。
- 工具失败后恢复：测诊断携带和空白重试率。
- compaction 后继续：测 Goal/Verified/Open/Next 恢复率。
- 长时间暂停后 resume：测第一动作是否仍与中断前 Next 一致。
- 验证敏感任务：测 coverage checkpoint 是否减少假完成。
- 不可信外部指令：测 provenance、permission 和 prompt precedence 是否阻止越权执行。
- Skill 结构变体：测单入口、缺失 supporting file、非法路径和合法多模块 skill 的 guard 行为。

### 指标

主指标：

- 任务完成率和验收测试通过率
- compaction/recovery 后状态恢复率
- 无证据“完成”率
- 重复工具调用和同错误空白重试次数

次指标：

- 总 token、wall time、provider cost
- 首次有效行动延迟
- 用户介入次数
- ledger 更新冲突和停滞误报率

### 默认启用门槛

只有同时满足以下条件，native ledger 才能进入 auto/loop 默认路径：

- 长任务完成率有稳定改善，置信区间不与明显退化重叠；
- 短任务 token/time 开销在预设预算内；
- compaction/recovery 恢复率提高；
- 无证据完成率下降；
- 没有新增权限绕过、prompt surface 分叉或跨 session 状态污染。

J-space skill 是否保留为用户可选能力，与 native ledger 是否默认启用分开决策。

## 3bis. Phase 2-6 实施记录与已知缺口（2026-08-18）

已落地：typed 契约与 9 条不变量（`agendao-types/src/task_ledger.rs`，12 域测试）；服务端
权威（session metadata 单一真值、CAS、fork 复制 rebind、delete 级联，7 权威测试）；HTTP
5 端点 + Unix JSON-RPC（get_task_ledger / apply_task_ledger_op）+ `task-ledger.replaced`
canonical 事件与 tier 规则（final_only 仅收 awaiting_user/blocked/interrupted/completed）；
seam reducer（`apply_batch` 单事务单 revision，candidate 不过夜不落表）；RunStarted /
RecoveryInterrupted（含 abort 路径）/ ToolBatchCompleted / FinalResponseCommitted 接线；
投影单注入点（conversation_seed，P0.4 同一先例）；typed final gate；停滞观察窗
（awaiting/interrupted 静默不累积、resume 窗口重置、深度任务 verified 增长不误报——
agendao-server 316 测试全绿）。Web：TaskStateCard（Goal/Next/Open/Verified/Blocked，
typed 字段渲染、默认折叠）+ SSE 事件 + `useSessionRuntimeReconcile` 复用 + 删除清理
（154 web 测试全绿）。A/B harness：`scripts/task_governance_ab.py`（纯标准库；三组同条
件、内嵌权限观察器、独立验收重跑、JSONL+summary）。

已知缺口（下一步，均有 harness 复现路径）：

1. **调度路径缺少 ToolBatchCompleted 事实源**：`latest_tool_batch_summary` 只由
   SessionPrompt 直连路径写入；scheduler 原生路径的运行从不产生它，因此 ledger 组冒烟
   中 rev 停留在 1。修复方向：在 SchedulerAgentObserver 的 tool_result 落点
   （scheduler_runner.rs）以服务器侧累计生成 batch 事实，或在写入点增加 PromptHooks
   通知；不得把逐工具调用升级为 seam。
2. **final gate 写入未在真实运行中观察到**：单元路径已测，端到端冒烟未见
   `delivery_gate_report` 落盘（metadata 仅有 task_ledger/session_context_kind）。需要
   针对任务尾部的集成测试钉住执行分支（怀疑与懒加载水合或尾部路径相关）后再定位。
3. 每轮（turn 级）batch seam、evaluator gate seam、compaction fingerprint 形式化、
   TUI Task State 渲染面板：按原计划顺位。

### 2026-08-18 二轮审阅修复（4 实质缺陷 + 2 中危 + 门禁复核）

1. **组间隔离**：三组统一 `--session` 预置会话；`used_prepared_session`（恒真）废除，
   换成真实证据——`sessions_in_fixture_dir`（必须=1）与 `prepared_session_has_messages`，
   summary 输出 `binding_violations`（非 0 即该组数据作废）。
2. **完成门逐项对应**：`VerifiedCheckpoint.covered_criteria` 逐条映射验收项；无 checkpoint
   覆盖且未声明 uncovered 的 criterion → `CriterionNotCovered`（点名首条）；无关 checkpoint
   覆盖不了任何项；`Complete.uncovered` 持久化到 `uncovered_criteria`（重启可审计）；
   `SetStatus(Completed)` 同门同清理语义（清 next/blocked，pending interaction 时拒绝），`SetStatus(Interrupted)`
   同 Interrupt 语义（pre_interrupt 标记）。
3. **TUI 跨会话泄漏与对账**：`reset_for_new_session` 清空 task_ledger（测试钉死）；
   `open_session` 六路 join 增加 `get_task_ledger_async`（HTTP+Direct 同桥），打开即对账；
   `SessionOpenData.task_ledger` 落库带 revision 防回退。
4. **交互多槽**：`awaiting_interaction: Option` → `awaiting_interactions: Vec`（不设静默容量上限）；
   并发第二个权限/问题可入列（不再 `AwaitingInteractionAlreadyActive` 丢弃）；新 op
   `ResolveInteraction` 按 kind+id 移除，清空才回 Active；测试覆盖双交互排队与逐个释放。
5. **cause 修正**：`FinalResponseCommitted` → `StatusChanged`（条件性完成是状态变化）。
6. **watcher 重试**：`seen` 只在 POST 成功后标记，瞬时失败下轮重试。
7. 门禁复核全部通过：`cargo fmt --check` 干净；clippy -D warnings 0 错误（types/server/
   tui/client）；types 81 / server 318 / tui 472 / web 154 测试全过；web eslint 0 问题。
8. 三组冒烟（修复后）：全部 `binding_violations=0`、`dir_sessions=1`、`has_messages=True`；
   ledger 组 rev 9（交互生命周期）、control/skill rev 0（无治理，符合预期）。
9. 注：`docs/plans/` 被 `.gitignore:15` 忽略——普通 commit 不含本计划文档，需
   `git add -f docs/plans/<file>` 或调整 ignore 规则（由维护者决策）。

### 2026-08-18 审阅修复轮（六项严重问题全部闭环）

1. harness 权限观察器改为仅应答本次运行创建的会话（allow-set + 锁）。
2. ledger 组显式 `--session` 绑定预置会话；记录 `used_prepared_session` 证据字段。
3. 完成门在域层封闭：Open 未清不得 Complete（SetStatus(Completed) 同门）；有验收
   criteria 而无 checkpoint 时必须显式 `uncovered`。
4. 交互生命周期接通：权限/问题 pending → AwaitingUser（typed 引用），任一解决路径
   （reply/timeout/通道关闭/abort/guard Drop）→ Active；Interrupt 幂等；RunStarted
   提交后刷新本地副本防旧值回写；FinalResponseCommitted 在 Open 清零且有 checkpoint
   时自动 Complete，否则保持 active 由 gate 报告缺口。
5. 事件在持写锁期间广播（revision 序即事件序）；Web store/SSE/TUI 三处 revision 防回退。
6. TUI 头部消费 ledger Next；stall forget 接入 delete 路由；Web 状态标签 i18n 化、
   样式对齐；计划页首与状态表一致。

验证：cargo fmt --check 干净；clippy -D warnings 0 错误；server 318 + types 81 +
web 154 测试全过。修复后 ledger 组冒烟（真实 deepseek 运行）：used_prepared_session=
true，rev 1→11（权限等待/恢复在真实运行中反复触发），gate 报告如实落地（无
 checkpoint → 不自动完成，报告缺口）。

### 2026-08-18 四轮封闭（状态机、证据代际、三传输对账、指标诚实性）

1. **提交级全局不变量**：单操作与 batch 都在克隆快照上执行，最终快照通过统一验证后才
   原子提交。Completed 不能保留 Next、Open 或 pending interaction；在 Completed 上直接
   SetGoal/OpenQuestion/SetNext 会整体回滚。合法重开必须在同一 batch 中设置 Goal、Next 和
   Active，且清除上一完成态的 `uncovered_criteria`。
2. **证据代际与有效性**：ledger 增加单调 `goal_generation`，checkpoint 由 authority 写入
   当前 generation；完成门只认当前 generation 且未 supersede 的 checkpoint。checkpoint 的
   `covered_criteria` 和 Complete 的 `uncovered` 都必须引用当前 Goal 的真实验收项，旧目标中
   同名的验收项也不能复用旧证据。
3. **完成门单一权威**：FinalResponseCommitted reducer 改用领域层 `completion_ready()`，不再
   自行判断“有 checkpoint 且 Open 为空”。delivery gate 持久化 open、当前有效证据是否为空、
   逐项 missing acceptance criteria 和显式 uncovered criteria。
4. **交互不允许状态侧门**：`SetStatus(Active)` 在 pending interaction 存在时拒绝；Complete
   同样拒绝而非清空。只有逐项 `ResolveInteraction` 或显式 Interrupt 可收束交互。移除原 8 槽
   上限，避免 runtime 已登记而 ledger 静默拒绝第 9 个交互。
5. **TUI 三传输同构对账**：HTTP、Direct、Unix 都实现 task-ledger getter；`--session` eager
   startup 与 session picker 都立即拉取快照。事件和 REST 回执统一经过 SessionStore 的
   session-id + revision 守卫，错误会话、rev0 和旧 revision 均不落库。
6. **Web 审计面补全**：active checkpoint 同样按 goal generation + supersede 过滤；显式展示
   `uncovered_criteria`，避免 UI 把“声明未覆盖”渲染成普通完成。
7. **Phase 6 指标边界**：harness 现已采集进程完成、独立验收、wall time、token、provider
   cost、权限介入和工具错误/修复；timeout 样本不会使 summary 崩溃。恢复/compaction 恢复率、
   空白重试、首次有效动作延迟、revision conflict 与 stall false-positive 仍缺专门任务或事件
   采集，summary 明确列入 `unavailable_metrics`，不得据此宣称正式 Phase 6 评测完成。

本轮回归覆盖：superseded evidence、旧 goal generation、Completed 非法变更原子回滚、合法
reopen、9 个并发 interaction、Active/Complete 侧门、reducer criterion gate、TUI 跨会话与
旧 revision 防回退。

正式 A/B 运行示例：

```bash
python3 scripts/task_governance_ab.py --base-url http://127.0.0.1:3989 \
  --binary <agendao> --model deepseek/deepseek-chat --seeds 5
```

## 4. 预计代码落点

| 模块 | 预计职责 |
| --- | --- |
| `crates/agendao-types/src/task_ledger.rs` | wire/domain typed contract |
| `crates/agendao-server-core/src/runtime_events.rs` | canonical task-ledger event |
| `crates/agendao-server/src/session_runtime/` | per-session ledger authority、snapshot、seam reducer |
| `crates/agendao-server/src/routes/session/` | HTTP/local API、revision validation |
| `crates/agendao-server/src/unix_socket.rs` | Unix transport parity |
| `crates/agendao-session/src/compaction.rs` | continuity projection，不拥有 ledger |
| `crates/agendao-server/src/recovery.rs` | checkpoint + Next recovery contract |
| `crates/agendao-types`/tool result metadata | 外部输入 provenance 与 trust classification |
| `crates/agendao-session` prompt surface/final output | provenance 投影与 final delivery gate |
| `crates/agendao-skill` guard | 通用单入口、链接文件、路径和结构完整性校验 |
| `crates/agendao-orchestrator/src/selector.rs` | governance class 选择 |
| `crates/agendao-orchestrator/src/` | seam emission、budgeted replanning |
| `apps/agendao-web/src/hooks/useSessionRuntimeReconcile.ts` | 扩展现有对账入口以合并 task-ledger snapshot |
| `apps/agendao-web/src/` | Task State store/view；不建立第二套 reconcile hook |
| `crates/agendao-tui-revue/src/` | Task State snapshot/store/view，复用 runtime reconcile 入口 |

实际实现前必须再次核对当前 ownership；若已有 authority 能承载，不新增同义模块。

## 5. 测试矩阵

### Rust

- ledger domain invariant tests
- persistence/fork/delete tests
- HTTP/Unix/Direct transport parity
- revision conflict/concurrent agent tests
- seam reducer idempotency
- evaluator checkpoint evidence coverage
- compaction continuity round-trip
- recovery resume from checkpoint
- stall detector false-positive and budget tests
- awaiting_user/interrupted detector suppression and resume-window reset
- event-tier tests: live replacements and final-only governance boundaries

### Web/TUI

- per-session isolation
- reconnect/catch-up reconciliation
- revision conflict feedback
- checkpoint evidence navigation
- mobile/narrow viewport text fit
- no hidden reasoning rendered

### End-to-end

- two concurrent sessions with independent ledgers
- two agents racing to update Next
- permission wait followed by abort/recovery
- abort during permission wait leaves no awaiting ledger state or zombie pending interaction
- compaction during loop task
- SSE disconnect during checkpoint creation
- server restart followed by resume

## 6. 风险与回滚

| 风险 | 控制 |
| --- | --- |
| ledger 变成第二套 Todo | Todo 继续描述工作项；ledger 只描述任务治理和证据 |
| 模型自然语言污染真值 | 只有 typed tool/evaluator/user action 能写关键字段 |
| prompt 膨胀 | 只投影摘要和 evidence refs，完整内容按需加载 |
| 高频 seam 产生噪声 | 只接受 semantic seam，文件写入合并到 tool batch |
| 多 agent 覆盖 | revision CAS + provenance + conflict event |
| 停滞 detector 无限重试 | budget、deadline、最大重新规划次数、Blocked 终态 |
| J-space skill 与 native ledger 冲突 | 实验分组隔离，默认不组合 |
| 第三方内容升级漂移 | 保存 hash、license、citation；升级重新做 smoke A/B |

回滚单位：

- Skill-only：从选择列表禁用 J-space，不需要服务端迁移。
- Native ledger：停止自动创建和 prompt 投影，但保留历史只读，不删除证据。
- Stall detector：独立配置关闭，ledger 继续工作。
- UI：隐藏 Task State 入口，不影响服务端数据。

## 7. 明确不做

- 不把 J-space 的情绪 marker 变成 AgenDao runtime event。
- 不扫描或保存模型私有 chain-of-thought。
- 不用英文正则决定 checkpoint 是否真实验证。
- 不让 skill Python controller 成为生产 session authority。
- 不因一次 benchmark 提升就默认启用。
- 不用厂商公开分数与 AgenDao 自测分数拼成统一对照表。

## 8. 阶段状态表

| Phase | 内容 | 状态 |
| --- | --- | --- |
| 1 | 可选 J-space skill 安装与实验基线 | 完成（2026-08-18 smoke A/B：control 25s / skill-on 27s，两组任务均 3/3 测试通过；skill-on 经 skills_list/skill_view 官方路径加载，control 转录零 j-space 引用；权限全程走正常请求-应答，无绕过） |
| 2 | SessionTaskLedger typed authority | 完成（2026-08-18：提交级全局不变量、goal generation、current/non-superseded evidence gate、interaction 侧门封闭；server metadata 单一真值 + CAS/fork/delete；HTTP + Unix JSON-RPC + typed event） |
| 3 | Semantic seam、compaction、recovery | 核心完成（交互/AwaitingUser、条件性 Complete、Interrupt 幂等已接通并实测 rev1→11）；剩余缺口见 §3bis（调度路径 batch 事实源、evaluator seam） |
| 4 | 停滞检测与受控重新规划 | v1 完成（观察与记录；自动重规划仍以 Phase 6 证据为门） |
| 5 | Web/TUI Task State | Web 完成（含 generation-aware evidence 与 uncovered 展示）；TUI 头部 Next 和 HTTP/Direct/Unix 启动/切换对账完成，完整面板记入 backlog |
| 6 | 多运行 A/B 与默认策略决策 | harness 基础链路和可得指标采集完成；正式任务矩阵、不可得指标采集与多 seed 运行未完成，不具备默认启用结论 |
