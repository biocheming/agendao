# 执行权威矩阵（Execution Authority Matrix）

> 状态：Phase 0 盘点基线（依据 docs/plans/penguin-inspired-core-compression-plan.md 修订版）
>
> 本文是执行入口、prompt 组装、事件流、transport 与旁路的**事实登记表**。
> 每条结论附代码证据（文件:行号）。改变任何一条权威关系时必须同步更新本文。

## 一、执行入口矩阵

标准生产执行链（所有 session prompt 执行的唯一路径）：

```text
HTTP / Unix socket / Local(direct) 三 transport
    ↓ 共享同一 handler 层（见第四节）
routes::session::prompt::session_prompt_inner (prompt.rs:1446)
    ↓ spawn 内：
scheduler_runner::run_scheduler (scheduler_runner.rs:1046)   ← scheduler composition authority
    ↓
SchedulerEngine::run (orchestrator/engine.rs:172)
    ↓ 唯一调用方关系
AgentLoop::run (orchestrator/agent_loop/loop_impl.rs:418)
    ↓
ModelBackend / ToolBackend（Provider / Tool / Permission / Sandbox）
```

| 问题 | 答案 | 证据 |
| --- | --- | --- |
| 谁启动 run | `session_prompt_inner` 内 spawn 的 task | prompt.rs:1446 |
| 谁组装 catalog/policy/blueprint/后端 | `run_scheduler` | scheduler_runner.rs:1046-1255 |
| 谁驱动 model→tool→model | `SchedulerEngine` 内部构造并驱动 `AgentLoop`，无第二处生产构造 | engine.rs:145, engine.rs:286 |
| 谁写状态 | optimistic assistant message 由 prompt route 创建；执行中状态经 `SchedulerAgentObserver`/`SchedulerEventChannel` 回写 | prompt.rs:2037, scheduler_runner.rs:98-193 |
| 谁发事件 | `ExecutionEvent`(引擎) / `AgentLoopObserver`(循环) / `ServerEvent`(总线) 三层，见第三节 | — |
| 谁定终态 | `run_scheduler` 返回 `SchedulerRunOutput`；prompt route 写 `assistant.finish` 与 metadata | scheduler_runner.rs:1263, prompt.rs:2130-2149 |
| 谁负责 cancellation | prompt route 持有 `CancellationToken`，`run_scheduler` 桥接为引擎 `CancellationFlag` | scheduler_runner.rs:1191-1198 |

**"direct path" 语义澄清**：`direct` 是 scheduler 的一个模板（`TemplateId::Direct`，单 agent 无 gate blueprint），不是独立执行循环。显式 agent 请求解析为 Direct 模板，仍走 `run_scheduler`。
证据：`routes/session/scheduler.rs:91-105`（`resolve_effective_scheduler_choice`：explicit agent → `TemplateId::Direct`，否则 `Auto`，二者皆为 `SchedulerChoice`）；Web 端模板列表 `apps/agendao-web/src/lib/webRuntime.ts:12-16`。

## 二、Prompt 组装矩阵

| 入口 | prompt 组装方式 | 是否经 PromptAuthority |
| --- | --- | --- |
| session prompt（三 transport） | `run_scheduler` 内构建 catalog/template/selection，blueprint 校验后由引擎 `PromptAuthority` 构造 prompt surface | 是（engine.rs:185-193） |
| TaskLedger 注入 | prompt route 把 ledger projection 作为 typed context block 追加进 conversation seed（"Single prompt injection point"） | 经 seed，非独立拼接（prompt.rs:2026-2034） |
| conversation seed 重放 | `agendao_session::prompt::replay_provider_messages` 从 session messages 重放 | prompt.rs:1983-1984 |
| GitHub CLI（旁路） | 自行 `template_parameters` + `build_template` + `ValidatedBlueprint`，经引擎 PromptAuthority 但 composition 独立于 `run_scheduler` | 部分（github_scheduler.rs:1-30 imports） |
| provider 诊断 probe | 单发 `"Reply with the single word: OK"`，无 prompt surface、无工具 | routes/provider.rs:1984-2017（诊断调用，非执行旁路） |

## 三、事件矩阵

| 层 | 类型 | 生产者 | 消费者 |
| --- | --- | --- | --- |
| 执行事实 | `ExecutionEvent`（orchestrator/events.rs:5） | `SchedulerEngine`（RunStarted/NodeStarted/Evaluated/RunCompleted/RunFailed…） | `SchedulerEventChannel`→mpsc→`project_scheduler_events` 投影 task（scheduler_runner.rs:1040-1044, 1224-1229） |
| 循环观察 | `AgentLoopObserver`（loop_impl.rs:46） | `AgentLoop` 步进/turn/tool 钩子 | `SchedulerAgentObserver`（scheduler_runner.rs:98）→ step 进度卡/工具记账/ToolBatchCompleted seam |
| 服务事件 | `ServerEvent`（server-core/runtime_events.rs:141） | `broadcast_server_event` / `broadcast_event`（server.rs:700） | 事件总线 → SSE `/event?tier=web`（event_stream.rs）→ 三端客户端 |
| 前端投影 | `FrontendEvent` | `session_runtime/frontend_projection.rs` 把 ServerEvent 投影为 web-tier 词汇（`session.runtime.replaced`、`question.upsert`、`permission.upsert`、`task-ledger.replaced`） | Web `useServerEventStream.ts` / TUI projection_coordinator |
| 输出成形 | `OutputBlock`（SessionEvent/tool/status/queue_item/inspect…） | server 各面（如 scheduler_step 进度卡 scheduler_runner.rs:179-191） | 三端展示层；Web runtime surface（RuntimeSurfaceSection.tsx） |

前端不自行判定终态：Web 端 run_status 取自 canonical `session.runtime.replaced` 事件的 `run_status` 字段；taskLedgers 有 revision 防倒退；断线重连后从 authoritative runtime state 重推导（useServerEventStream.ts:132-143, 113-125, 229-231）。

## 四、Transport 矩阵（parity 对象）

| Transport | 入口代码 | 与 handler 的关系 |
| --- | --- | --- |
| HTTP/SSE | axum routes（routes/mod.rs） | 直接绑定 handler |
| Unix socket | unix_socket.rs:240,482-540 | `handle_prompt` → `local_prompt` → 同一 handler（`:1285` 注释确认共享 SessionManager） |
| Local/direct | local_api.rs:331,981-987 | 直接调用 `super::prompt::session_prompt` / `super::session_crud::execute_shell` |

结论：handler 层三 transport 天然共享；**parity 的实质对象是 transport 序列化与事件流层**（HTTP+SSE vs unix stream vs 本地调用的事件序列/终态可观测一致性），而非业务逻辑。

## 五、Web 路径对账（apps/agendao-web）

事件消费与 read model 的权威关系：

- 事件流：裸 fetch SSE `/event?tier=web`（`useServerEventStream.ts:218-241`，带 Authorization 故不用 EventSource），消费 web-tier 投影词汇（`output_block`、`session.runtime.replaced`、`question.*`、`permission.*`、`task-ledger.replaced`、`session.projection.replaced`、`config.updated`），按字符串 type 分发、无 TS 强类型。
- 执行状态：`useExecutionActivity.ts:43-52` 读正式 read model（`GET /session/{id}/telemetry` + `/insights`）；SSE `output_block` 仅作 live 覆盖层（上限 8 条展示条目）。
- run 终态权威：`session.runtime.replaced` 的 `run_status` + `useSessionRuntimeReconcile.ts:36-40` 的 `GET /session/{id}/runtime` 对账（切换/重连/refocus 触发）。SSE 不重放，重连后从权威状态重推（`useServerEventStream.ts:229-231`）。
- ledger：`task-ledger-slice.ts:9-28` 纯 replacement + revision 单调守卫，无本地推断。
- 无旁路：执行仅经 `POST /session/{id}/prompt` 与 `/command`；composer 只解析 `@文件` 引用，不拼系统 prompt；全部网络出口仅为 SSE fetch 与 `/pty/{id}/connect` WebSocket（均服务端端点）。

受控的本地乐观推断（随后均被 canonical 事件纠正，Phase 4 parity 需覆盖这些下落沿）：

| 位置 | 推断 | 纠正来源 |
| --- | --- | --- |
| `usePromptSubmission.ts:236-238,256-261` | 提交后乐观置 streaming/running；按 prompt REST 响应推断 awaiting_user | `session.runtime.replaced` |
| `useServerEventStream.ts:175,213` | question.removed / permission.resolved 后本地置 running | `session.runtime.replaced` |
| `usePromptSubmission.ts:315-342` | stop 时本地置 cancelling / 失败回 running | runtime 对账 |

跨端契约镜像点：TS `isTranscriptBearingIdentity`（`lib/liveTranscriptState.ts:116-121`）镜像 Rust `LiveSemanticConsumer::is_transcript_bearing_kind()` —— 手工镜像，无自动校验，是 parity 断言的必盯对象。

## 六、旁路登记

| 路径 | 性质 | 处置 |
| --- | --- | --- |
| `crates/agendao-cli/src/github_scheduler.rs` | headless CI composition path（自建 `GithubToolBackend`、极简 catalog、policy、直调 `build_template` + `SchedulerEngine`），由 `agendao github run --event --token` 触发（github.rs:1152,1249） | **已收口（Phase 3，方案 A）**：复用同一 `SchedulerEngine`/`AgentLoop` 内核（非第二套 loop）；model backend 组装收归 `ProviderModelBackend::from_definitions`（与 `run_scheduler` 共享的唯一 ToolId 折叠点，agent_loop/provider.rs）；补齐取消生命周期（SIGINT → `CancellationFlag` → 引擎 select! 分支，取消语义与 session 路径同构）。catalog/policy 保持 headless 特化（单 agent、无 evaluator/observer/事件投影），为产品差异而非语义重复 |
| `routes/provider.rs:1984 test_provider_model` | 单发、无工具诊断调用 | 登记，不计为执行旁路 |
| CLI `generate.rs` / `providers.rs` / `provider_cmd.rs` | 模型目录/bootstrap/配置转换，无运行时模型调用 | 非入口 |

内核唯一性结论：`AgentLoop` 的生产构造点仅 `SchedulerEngine::new`（engine.rs:145）；`SchedulerEngine` 的生产构造点仅 `scheduler_runner.rs` 与 `github_scheduler.rs`，二者共享同一内核与取消语义。标准链无第二套 `model→tool→model` 循环。

## 七、parity 现状基线

已有覆盖（部分）：

- `crates/agendao-server/src/transport_parity_tests.rs` — **三 transport parity（Phase 4 交付）**：同一输入（显式 Direct 模板 + scripted provider）经 local 直调 / unix JSON-RPC 行协议 / HTTP axum router 三种编码路径，运行时断言 (a) 各自终态契约（finish=stop、脚本文本）、(b) 三路 assistant 终态形状（finish/文本/元数据键集合）一致、(c) frontend bus 事件类别序列（相邻去重保序）一致。共享 ScriptedProvider 经 `ProviderRegistry` 注册，覆盖真实 model 解析链（parse_model_string → get_provider → get_model）
- `crates/agendao-server/src/scheduler_runner_progress_tests.rs` — scheduler 进度/观察者
- `crates/agendao-server/tests/submission_protocol_test.rs` — 提交协议集成
- `crates/agendao-server/src/routes/frontend_smoke.rs` — 前端事件注入烟测端点（question/permission/output-block）
- `crates/agendao-server/src/routes/session/prompt/tests.rs` — prompt route 单测
- `apps/agendao-web/scripts/*.mjs` — Web 端浏览器烟测（boot/live-transcript/runtime-surface/session-management 等）
- `scripts/task_governance_ab.py` — harness 级 A/B（驱动真实 binary）

设计要点：parity 测试显式选 Direct 模板（`{"kind": "template", "template": "direct"}`）绕开 Auto 的 AI planning 调用 —— parity 对象是 transport 编码层，不是 planner 的模型选择；planner 自身的行为属于 scheduler 契约域（第九节）。

剩余增量（登记，非 Phase 4 范围）：SSE 断线重连语义、Web/TUI projector 消费细节（第五节"受控的本地乐观推断"表中的下落沿）、steer/permission/question 等待语义（见第九、十节遗留项）。

## 八、旧入口清单

| 入口 | 状态 |
| --- | --- |
| session prompt（三 transport 汇聚） | 已收口（run_scheduler 单链） |
| TaskLedger auto-continuation | 已收口（复用 session_prompt_inner，`SchedulerChoice::Auto` 权威，prompt.rs:286-292） |
| GitHub CLI scheduler | 已收口（Phase 3 方案 A：共享内核 + 共享 model backend 组装 + 补取消生命周期；headless 特化保留） |
| provider probe | 保留（诊断） |
| direct 模板路径遗留 `latest_tool_batch_summary` metadata | 已盘点（Phase 2）：全库无写入无读取，仅剩一处注释引用死名词；注释已改写为不依赖该名词的 seam 顺序说明（prompt.rs:2246-2250）。双写不存在，字段按死代码处理 |

## 九、run_scheduler 契约（Phase 1 固化）

> 由 `crates/agendao-server/src/scheduler_contract_tests.rs` 以行为测试 pin 住；
> 修改 `run_scheduler` 的输入、终态、错误或事件语义前必须先更新本节与对应测试。

### 输入与控制

| 面 | 契约 | 证据 |
| --- | --- | --- |
| 组装职责 | catalog、policy、blueprint、runtime 后端、事件投影、cancellation 桥接（`CancellationToken` → 引擎 `CancellationFlag`）均由 `run_scheduler` 完成 | scheduler_runner.rs:1046-1255, 1191-1198 |
| 模板选择 | `SchedulerChoice::Template { TemplateId::Direct }` 等模板在此展开；blueprint 名统一为 `"session-scheduler"`（组合层命名，模板 id 只决定拓扑） | scheduler_runner.rs:1803, templates.rs:55-68 |
| 取消语义 | 模型调用被 `tokio::select!` 包住（cancellation/deadline 分支），**挂起中的流可被立即取消**；取消后 run promptly 返回归类为取消的 `Err`，且**取消返回后进入静止**（禁止取消后的重试、重规划或补偿性模型调用 —— 硬断言） | loop_impl.rs:484-489；契约测试 `cancelled_run_returns_cancelled_error_and_goes_quiet` |

### 终态与错误

| 面 | 契约 | 证据 |
| --- | --- | --- |
| 成功终态 | `Ok(SchedulerRunOutput)`：`result`（节点结果）、`usage`、`blueprint`、`fingerprint`、`source`、`review`（tool_call/error_tool_call 计数与 used_skill_names 回流记账）全部随终态返回，调用方不得从文本猜测 | 契约测试 `direct_template_completes_with_structured_outcome` |
| 失败终态 | provider 失败沿 `Err(String)` 完整传播，错误分类保留原始因果（终态错误分类的唯一来源） | 契约测试 `provider_failure_propagates_as_error_outcome` |

### 事件面与 lifecycle 边界（核心结论）

`run_scheduler` 产出的执行事实经 `ServerEvent::OutputBlock` 直通投影为
`FrontendEvent::OutputBlockAppended`（scheduler_step 进度卡，block.kind == "session_event"），
三端可经 frontend bus 观测（契约测试 `run_emits_scheduler_step_events_to_frontend_bus`；
Direct transport 装配点为 `ServerState::ensure_frontend_projector`，与 HTTP 启动路径同构）。

**`run_scheduler` 不注册 session runtime**：`register_scheduler_run` 与 run 状态广播由
prompt route 的 session lifecycle 段负责（prompt.rs:2047-2050）；projector 的
`SessionRuntimeReplaced` 依赖 runtime telemetry 快照（frontend_projection.rs:430-438），
route 未注册时不会产出。这就是 scheduler composition 与 session lifecycle 的分界：

```text
composition（run_scheduler）：组装 + 执行事实 + 结构化终态
lifecycle（prompt route）：optimistic assistant message、ledger 注入、runtime 注册、
                          finish/metadata 写入、等待态清理
```

### 已知缺口（后续增量）

- steer、permission/question 等待语义的 contract tests 未覆盖：二者横跨 AgentLoop
  工具调度与 route 层等待态清理，与 Phase 2 lifecycle 盘点耦合，作为 Phase 1 后续
  增量与 Phase 2 一并推进。

## 十、Session lifecycle 权威矩阵（Phase 2 盘点）

> Phase 2 决策门槛：若只有单一 owner 且无重复语义，保留现状并补文档。
> 盘点结果：**四个 lifecycle 域全部单一 owner，无生命周期分叉** —— 不抽取
> lifecycle helper，保留现状，本节即权威登记。

### 1. Assistant message 创建与终态写入

| 操作 | 唯一 owner | 证据 |
| --- | --- | --- |
| optimistic 创建 | `session_prompt_inner` 内 `session.add_assistant_message()` | prompt.rs:2037 |
| 终态写入（三分支） | 同一函数：Ok → `finish: "stop"` + blueprint/usage/metadata + 文本；`Err`+cancelled → `finish: "cancelled"`；`Err` → `finish: "error"`（step limit → `scheduler_resumable` + continuation 提示） | prompt.rs:2122-2223 |

### 2. Ledger 注入、runtime 注册与 continuation

| 操作 | 唯一 owner | 证据 |
| --- | --- | --- |
| Ledger 注入 | typed context block 追加进 conversation seed（单一注入点，无独立拼接） | prompt.rs:2026-2034 |
| runtime 注册 | `register_scheduler_run`（token + ExecutionRecord） | prompt.rs:2047-2050, runtime_control.rs:420 |
| runtime 注销 | `finish_scheduler_run` —— **晚退设计**：response、step seams、verifier、completion/interrupt seam、delivery report 全部 settle 后才 retire 取消权威与 scheduler 子树（防止孤儿节点让 recovery 永久 running） | prompt.rs:2325-2331, runtime_control.rs:469-525 |
| auto-continuation 决策 | `prepare_task_ledger_auto_continuation` 在取消权威仍注册时执行（abort 竞态可在 synthetic request 校验前清掉 marker） | prompt.rs:2306-2323 |

### 3. Usage / telemetry 回流记账

| 操作 | 唯一 owner | 证据 |
| --- | --- | --- |
| assistant usage + metadata | 终态写入分支（见第 1 项） | prompt.rs:2148-2176 |
| session usage 记账 | `record_session_usage`（聚合自 assistant messages） | prompt.rs:2239-2242, session.rs:884-903 |
| telemetry metadata 持久化 | `persist_session_telemetry_metadata` | prompt.rs:2243 |
| review 回流 | `maybe_enqueue_background_review(nudge)`（nudge 字段复制自 `SchedulerRunOutput.review`） | prompt.rs:2093-2103, 2225-2231 |

### 4. Cancellation / permission / question 等待态清理

`abort_session_execution` 是 abort 的唯一清理收口（`abort_prompt` / `abort_session`
两个 handler 与 unix transport 均汇聚于此），按序清理：permission turn → 排队
followup prompts（stop means stop）→ auto-continuation marker（含持久化）→
pending permissions（无 deadline，必须显式解决，否则弹窗与 waiter 留存到重启）→
pending questions（独立交互注册表，scheduler token 已 retire 也要清）→
scheduler token 取消 → prompt runner 取消 → interrupt 标记 → `RecoveryInterrupted`
seam dispatch。证据：cancel.rs:42-129。

### 遗留项处置

`latest_tool_batch_summary`：Phase 0 登记的"双写疑虑"已证伪 —— 全库无写入无读取，
仅 prompt.rs:2246-2250 注释引用死名词。注释已改写为不依赖该名词的 seam 顺序说明。
双写不存在，无需治理动作。

### Phase 1 遗留缺口的 Phase 2 结论

steer / permission / question 等待语义横跨 AgentLoop 与上述清理收口；其 contract
tests 的正确锚点是**本节的清理契约**（abort 后无悬挂等待态）+ AgentLoop 工具调度
（permission 裁决流），作为 Phase 4 parity 测试的组成一并成体系，不在本阶段另立。
