# Resource Optimization Comparative Audit

Scope:
- `../codex`
- `../deepseek-gui`
- `../hermes-agent`
- `../opencode`
- `../holon`
- current `agendao`

Goal:
- identify concrete CPU / memory saving strategies already used by peer agent runtimes
- identify where `agendao` still spends resources continuously on event pull, fallback refresh, and repeated projection work
- derive an actionable optimization roadmap for `agendao`

This audit is evidence-led. Every claim below is tied to concrete local files that currently exist in the workspace.

## 1. Executive Summary

Across the five reference projects, the strongest resource-saving patterns are not "one big optimization". They cluster into six repeatable moves:

1. Event bursts are coalesced before they hit UI state.
2. Long-lived snapshots are reused instead of recomputed on every poll.
3. Expensive catalogs / schemas are cached and served immediately while background init continues.
4. Streaming UIs do not redraw on every token; they batch, threshold, and back off.
5. Runtimes explicitly sleep when queues are empty instead of simulating liveness with periodic polling.
6. Context / tool history is bounded aggressively so memory and downstream compute do not grow linearly with session length.

`agendao` has already adopted part of this playbook:
- server-side output block coalescing
- TUI bridge sleep-until-deadline logic
- debounced session telemetry / question / permission / process refresh
- frontend event capability filtering and session filtering

But `agendao` still has a large periodic-refresh tail, especially in the TUI:
- full session sync every 10s on remote paths
- question fallback sync every 5s
- permission fallback sync every 5s / 15s
- aux dialog refresh every 5s / 15s
- process refresh every 2s while the sidebar is visible
- ad hoc session telemetry snapshot fetches as a repair path

The main architectural reason these loops still exist is that some authorities are still "current-session slot" shaped instead of "per-session event-fed store" shaped. As long as that remains true, `agendao` has to keep fallback polling alive to heal authority gaps.

## 2. Comparative Matrix

| Project | Primary resource strategy | CPU effect | Memory effect | Evidence |
| --- | --- | --- | --- | --- |
| `codex` | coalesce file watch notifications; serve cached MCP tool snapshots during startup; treat stream lag as first-class | reduces event storms and init stalls | avoids duplicated watch payloads and tool catalog rebuilds | `codex-rs/file-watcher/src/lib.rs`, `codex-rs/codex-mcp/src/rmcp_client.rs`, `codex-rs/exec/src/lib.rs` |
| `deepseek-gui` / `kun` | cache-first loop; inflight lifecycle cleanup; bounded request history hygiene; progressive MCP discovery | less repeated request preparation; fewer UI updates; less tool-context overhead | bounded tool output and arg retention; no leaked inflight state | `kun/src/loop/inflight-tracker.ts`, `kun/src/loop/request-history-hygiene.ts`, `kun/README.md` |
| `hermes-agent` | activity-based timeout, not wall-clock; buffered stream consumer; duplicate prevention; completion notification instead of polling | avoids killing active jobs; reduces token-by-token UI churn; reduces resend churn | less duplicate session/message state | `gateway/stream_consumer.py`, `gateway/session.py`, `RELEASE_v0.8.0.md` |
| `opencode` | memoized tool/schema lowering; server-side event filtering by workspace; cache breakpoint budgeting | less repeated schema build and less irrelevant event fan-out | avoids duplicate tool definition objects and over-marked cache metadata | `packages/llm/src/tool.ts`, `packages/server/src/handlers/event.ts`, `packages/llm/src/protocols/anthropic-messages.ts` |
| `holon` | queue + sleep/wake runtime; cached poll views by activity marker; quiescence windows; explicit idle/asleep posture | avoids busy loops and redundant poll recomputation | avoids rebuilding poll snapshots when storage marker unchanged | `src/run_once.rs`, `src/runtime/scheduler_executor.rs` |
| `agendao` | debounced sync + coalesced output + bridge sleep | partial win | partial win | `crates/agendao-tui/src/app/*.rs`, `crates/agendao-server/src/routes/mod.rs`, `crates/agendao-server/src/routes/event_stream.rs` |

## 3. What The Other Projects Are Doing Well

### 3.1 Codex

#### A. Coalesce file watcher storms before subscriber delivery

`codex-rs/file-watcher/src/lib.rs` does two useful things:
- changed paths are deduplicated in a `BTreeSet`
- `ThrottledWatchReceiver` emits at most once per interval

This is a classic "compress hot upstream noise before state consumers see it" move. It saves CPU in the render / downstream sync path and saves memory by holding one merged batch instead of many tiny events.

#### B. Use startup snapshots so tool catalogs do not block on cold async init

`codex-rs/codex-mcp/src/rmcp_client.rs` and the corresponding tests in `connection_manager_tests.rs` show a very practical pattern:
- if the live MCP client is still initializing, return a cached tool snapshot immediately
- do not block `list_all_tools()` if a startup snapshot exists

This reduces cold-start latency and prevents a UI or planner from repeatedly waiting on expensive remote tool discovery.

#### C. Make lag visible and bounded

`codex-rs/exec/src/lib.rs` has an explicit lag message:
- `in-process app-server event stream lagged; dropped N events`

That matters because once lag is observable, the system can resync surgically instead of hiding the problem and letting downstream consumers compensate with blunt polling.

#### D. Bounded-context discipline is treated as a rule, not an afterthought

`codex/AGENTS.md` explicitly enforces:
- no history rewrite
- avoid frequent context changes that cause cache misses
- everything injected into model context must have a bounded size and hard cap

This is primarily model-compute optimization, but it also indirectly controls local memory growth because long-lived session structures do not keep accumulating unbounded prompt cargo.

### 3.2 DeepSeek GUI / Kun

#### A. Cache-first loop by construction

`deepseek-gui/kun/README.md` documents the runtime as:
- immutable prompt prefix
- bounded TTL/LRU caches
- inflight tracking
- explicit context compaction

This is important because Kun does not treat cache as an analytics afterthought. It shapes the entire loop around a stable prefix and bounded hot state.

#### B. Inflight lifecycle is a strict authority with guaranteed cleanup

`kun/src/loop/inflight-tracker.ts` is intentionally minimal:
- `begin`
- `end`
- `run` with `finally`
- `abortAll`

That simplicity matters. It avoids leaked "running" state, leaked timers, and leaked UI spinner truth.

#### C. Request history hygiene aggressively bounds tool output and arguments

`kun/src/loop/request-history-hygiene.ts` does several high-value things:
- caps tool result lines / bytes / approximate tokens
- caps completed tool-call arguments
- omits base64 blobs
- selects only signal lines from long output
- bounds array sizes

This is both a memory strategy and a downstream compute strategy. It prevents retained session history from becoming a replay tax.

#### D. Progressive MCP discovery prevents massive tool catalogs from entering every turn

`kun/README.md` describes `mcp_search`, `mcp_describe`, `mcp_call`, `mcp_refresh_catalog`.

This is nominally token optimization, but it also reduces local CPU work in request construction and tool registry projection because the full catalog does not need to be re-materialized into every model request.

#### E. Config-level hard bounds exist for large payloads

`src/shared/app-settings-kun.ts` bounds:
- max tool result lines
- max tool result bytes
- max tool result tokens
- max argument bytes / tokens
- max array items
- compaction thresholds

That is a strong pattern: resource caps live in config normalization, not as scattered magic limits.

### 3.3 Hermes Agent

#### A. Stream consumer batches edits instead of updating every token

`gateway/stream_consumer.py` has a real pacing system:
- `edit_interval`
- `buffer_threshold`
- queue draining
- `await asyncio.sleep(0.05)` yield to avoid busy-looping
- adaptive behavior when platforms flood-control or do not support edits well

This is exactly the kind of streaming discipline that keeps CPU and outbound transport churn under control.

#### B. Use activity-based timeouts, not wall-clock timeouts

`RELEASE_v0.8.0.md` and `gateway/session.py` show a strong idea:
- active long-running work should not be treated as "timed out"
- idle sessions can expire/reset

This prevents pointless restarts and compensating recovery work, which are often more expensive than letting an active process finish.

#### C. Completion notification beats polling

Hermes explicitly added `notify_on_complete` for background processes instead of making the agent poll for completion.

That is a direct CPU optimization. If a task can signal completion, the runtime should sleep until it does.

#### D. Duplicate prevention is treated as a first-class optimization

Hermes repeatedly mentions:
- gateway dedup
- partial stream guard
- duplicate delivery prevention

Duplicate suppression is not just a correctness fix. It is resource control: duplicate sends trigger duplicate storage, duplicate render work, and duplicate downstream refresh.

### 3.4 OpenCode

#### A. Memoize tool definitions and codecs

`packages/llm/src/tool.ts` precomputes and retains:
- tool definition
- decode / encode helpers
- projection helpers

This avoids rebuilding schema objects and JSON Schema projections per invocation.

#### B. Filter event streams on the server before they hit the client

`packages/server/src/handlers/event.ts`:
- subscribes to one event stream
- filters by workspace directory and workspace ID before delivery

That reduces event fan-out and makes transport cost proportional to relevance.

#### C. Enforce provider-specific cache budgets centrally

`packages/llm/src/protocols/anthropic-messages.ts`:
- caps Anthropic cache breakpoints at four
- drops overflow markers centrally
- keeps tools high in the cache hierarchy

That is a good example of "budget at the lowering boundary". It avoids generating invalid or needlessly expensive request shapes.

#### D. Memo utility is explicit and resettable

`packages/console/core/src/util/memo.ts` provides:
- lazy load once
- explicit reset with optional cleanup

This is a lightweight pattern `agendao` can reuse for expensive dialog / catalog projections that are currently recomputed periodically.

### 3.5 Holon

#### A. Sleep/wake is part of the runtime model, not a patch

Holon's public model and scheduler explicitly use:
- queue
- sleep
- wake
- idle / asleep posture

That immediately cuts wasted CPU because "nothing to do" becomes an actual runtime state.

#### B. Poll views are cached behind an activity marker

`src/run_once.rs` caches `CachedPollView` and only recomputes the expensive view if the storage/activity marker changes.

This is one of the clearest direct lessons for `agendao`: if your fallback path must poll, at least do not rebuild the full snapshot when nothing changed.

#### C. Completion is stabilized through a quiescence window

Holon waits for the same candidate completion state to survive a quiescence window before finalizing.

That prevents thrash:
- no eager finalize/reopen loops
- no premature resync
- fewer "finished, wait, no actually not" transitions

#### D. Batch accounting is explicit

`run_once.rs` also tracks batched command items and provider cache usage. Holon does not just do work; it records where the work is aggregating.

## 4. What `agendao` Already Does Right

### 4.1 The TUI bridge is no longer a naive busy loop

`crates/agendao-tui/src/bridge/mod.rs` already uses:
- `next_tick_deadline(now)`
- `ui_bridge.notified()`
- `tokio::select!`
- "do not draw if nothing meaningful changed"

This is a major improvement. It is codex-style: sleep until either a deadline or an actual event arrives.

### 4.2 Server-side output block coalescing exists

`crates/agendao-server/src/routes/mod.rs` contains a `LiveSnapshotCoalescer` with a substantial test surface. `event_stream.rs` also batches output blocks with `EVENT_OUTPUT_BLOCK_BATCH_MS = 16`.

That means `agendao` already knows how to collapse noisy delta streams into full-so-far snapshots before they hit the frontend.

### 4.3 Frontend bus filtering now exists in one place

`crates/agendao-server/src/session_runtime/frontend_subscription.rs` centralizes:
- event → session identity extraction
- event → subscription capability checks

That is good for both correctness and performance: one filter authority prevents each transport from doing its own wasteful reinterpretation.

### 4.4 TUI refresh paths are at least debounced

`crates/agendao-tui/src/app/app.rs` shows:
- session sync debounce: `180ms`
- session telemetry debounce: `120ms`
- question sync debounce: `40ms`
- permission sync debounce: `40ms`
- process refresh debounce: `120ms`

This already prevents the most obvious feedback loops.

### 4.5 Workspace file indexing has a hard refresh interval

`crates/agendao-tui/src/file_index.rs` only refreshes a root if:
- root and depth match
- last refresh is older than 5s

That is better than rescanning every frame.

## 5. Where `agendao` Still Burns CPU / Memory

### 5.1 Periodic fallback loops are still a major steady-state tax

The current TUI remote path still schedules periodic work even when frontend events exist:

- full session sync every `10s`
- question fallback sync every `5s`
- permission fallback sync every `5s` or `15s`
- aux refresh every `5s` or `15s`
- process refresh every `2s` while sidebar is visible
- perf log every `10s`

Evidence:
- `crates/agendao-tui/src/app/app.rs`
- `crates/agendao-tui/src/app/runtime.rs`
- `crates/agendao-tui/src/app/event_loop.rs`

This means the remote TUI still has a background metronome even when nothing semantically changed.

### 5.2 Session telemetry snapshot fetch is still used as a repair path

`queue_session_telemetry_refresh()` and `spawn_queued_session_telemetry_refresh()` still exist in `crates/agendao-tui/src/app/sync.rs`.

Cost:
- spawns a fresh OS thread
- performs a full telemetry fetch
- reapplies a large snapshot

This is acceptable as a narrow repair path, but still expensive if it becomes the common path for authority healing.

### 5.3 Question / permission remote fallback still exists

Even after question/permission event improvements, `event_loop.rs` still schedules:
- `sync_question_requests()`
- `sync_permission_requests()`

for non-direct paths.

This means the authority is not yet fully closed by `FrontendEvent` on every transport.

### 5.4 Process refresh is still timer-driven

When the session sidebar is visible, `event_loop.rs` keeps refreshing process stats every 2 seconds through:
- `agendao_core::process_registry::global_registry().refresh_stats()`

That is expensive because process registry scans are not free, and the refresh does not appear to be event-driven or activity-driven.

### 5.5 Aux dialog refresh is still clock-driven

`event_loop.rs` periodically refreshes:
- session list
- skill list
- LSP status
- MCP dialog

This is efficient enough at small scale, but on large workspaces and large skill catalogs it becomes repeated IO / parsing / projection work triggered by time rather than invalidation.

### 5.6 `session_runtime` is still effectively a current-session slot

The recent `ToolCallUpsert` work narrowed fallback sharply, but the core limit remains:
- `AppContext.session_runtime` is still one `Option<SessionRuntimeState>`
- not a per-session authority map

Consequences:
- pure event-driven background session runtime is still hard
- some fallback polling remains necessary
- authority mismatch must still be repaired instead of naturally coexisting

This is the single biggest structural reason `agendao` cannot yet eliminate all periodic runtime healing.

### 5.7 `refresh_attached_sessions()` and status dialog refresh still fan out after sync

After session sync and telemetry refresh, `event_loop.rs` often triggers:
- `refresh_attached_sessions()`
- `refresh_active_status_dialog()`

These are reasonable, but today they are often chained after broader sync operations. That makes expensive UI recomputation piggyback on already-expensive fetches.

### 5.8 Event-stream parsing still does repeated JSON work

`crates/agendao-server/src/routes/event_stream.rs` does:
- raw string filter checks
- JSON parse for filter decisions
- JSON parse again into typed events

This is not catastrophic, but it is waste on a hot path. A typed bus or envelope carrying session identity separately would be cheaper.

### 5.9 Lagged event handling still degrades into indirect resync cost

In `event_stream.rs`, lagged broadcast events are mostly skipped or flushed, but the downstream frontend often compensates through later fallback sync paths.

This is worse than a targeted repair path because:
- a small local lag can trigger a large remote snapshot reload later
- lost authority continuity becomes "fetch a lot again"

### 5.10 File indexing is interval-bounded but still scan-based

`file_index.rs` avoids per-frame scanning, but it still does a `WalkDir` rebuild every 5 seconds once a refresh condition hits. On very large repos this is still heavier than a watcher-backed incremental index.

## 6. What `agendao` Should Learn From The Others

### Priority 0: Shrink fallback polling from periodic to diagnostic-only

Borrow from:
- Holon's activity-marker cache
- Hermes's completion-notification bias
- Codex's lag-is-explicit pattern

Change:
- question / permission / runtime / process repair should not run on fixed cadence by default
- instead, run only when:
  - event stream lag is observed
  - transport reconnect occurs
  - explicit authority gap is detected
  - user manually opens a view that has never been hydrated

Desired outcome:
- `5s / 10s / 15s` fallback clocks disappear from the hot path
- fallback becomes a rare repair mechanism, not a standing tax

### Priority 0: Replace single-slot runtime authority with per-session runtime authority

Borrow from:
- Holon's queue/state model
- OpenCode's location-scoped event model

Change:
- store runtime snapshots per session in TUI context
- current session view just reads one selected entry

Why this matters:
- background session events can update without corrupting foreground authority
- `ToolCallUpsert` and future lifecycle events can become fully pure event-driven
- fallback telemetry fetches shrink further because "wrong session loaded" stops being a category

### Priority 0: Make process refresh activity-driven, not 2-second timer-driven

Borrow from:
- Hermes's activity-based timeout logic
- Holon's sleep/idle posture

Change:
- refresh process stats only while:
  - a process actually changed recently
  - a running tool / process exists
  - user explicitly focuses the process panel
- use exponential backoff toward idle

Expected effect:
- a quiet session sidebar stops causing constant process scans

### Priority 1: Introduce cached poll views / snapshot markers for unavoidable repair paths

Borrow from:
- Holon's `CachedPollView`

Change:
- when `agendao` must perform fallback sync, cache the last authority projection with a version marker
- if marker unchanged, do not rebuild full attached sessions / status projections / dialog blocks

Expected effect:
- fallback no longer implies full recomputation

### Priority 1: Move dialog refresh from cadence-driven to invalidation-driven

Borrow from:
- OpenCode `memo()`
- Codex cached startup snapshots

Change:
- session list, skill list, MCP dialog, model dialog, LSP view should maintain:
  - cached snapshot
  - dirty bit
  - optional TTL
- refresh when opened, invalidated, or explicitly requested

Expected effect:
- less background work when dialogs are open but underlying data is unchanged

### Priority 1: Treat event lag as a targeted repair trigger

Borrow from:
- Codex's explicit lag warning

Change:
- when frontend bus or SSE reports lag/dropped count:
  - mark only the affected authority dirty
  - schedule one scoped resync
  - suppress broader polling for a cooldown window

This is cheaper than allowing every lag incident to degrade into repeated generic full syncs.

### Priority 1: Stop spawning ad hoc threads for telemetry repair

Current path:
- `spawn_queued_session_telemetry_refresh()` uses `std::thread::spawn`

Better path:
- reuse a bounded async worker pool or semaphore-limited task path

Reason:
- repeated thread creation is noisy
- bounded concurrency is easier to observe and cap

### Priority 2: Convert scan-based file index to watcher-backed incremental index

Borrow from:
- Codex throttled file watcher

Change:
- maintain a watcher-backed workspace index
- coalesce file events
- periodically rebuild only on watcher failure or root change

This will matter most on large repos with active sidebars.

### Priority 2: Avoid repeated JSON parse in SSE hot path

Borrow from:
- OpenCode's typed event stream boundary

Change:
- broadcast typed frontend event envelopes internally
- include session identity and capability hints outside the JSON payload

Effect:
- filtering becomes cheaper
- transports stop reparsing the same content just to decide whether to drop it

### Priority 2: Apply stricter bounded-history hygiene to retained UI/runtime artifacts

Borrow from:
- Kun request history hygiene

`agendao` already does a lot on provider-side context closure and cache diagnostics, but the same philosophy should be applied to frontend-retained artifacts:
- cap retained tool-progress detail in live UI state
- cap huge JSON block retention in long session views
- collapse stale intermediate snapshots once final blocks exist

This is primarily a memory optimization.

## 7. Recommended `agendao` Optimization Roadmap

### Phase A: Kill standing timers where event authority already exists

Targets:
- question fallback sync
- permission fallback sync
- aux sync for open dialogs
- process refresh timer

Rule:
- no periodic refresh unless there is no reliable event authority for that domain

### Phase B: Upgrade TUI state authorities from "current-session slot" to "per-session store"

Targets:
- runtime
- projection
- question queue
- permission queue
- diff state

Rule:
- if the server already emits a per-session event, the TUI should store it per session, not force it through a global slot

### Phase C: Add activity markers and dirty bits to expensive views

Targets:
- session telemetry view
- attached sessions
- status dialog blocks
- skill hub / MCP / LSP dialogs

Rule:
- recompute only when source authority version changes

### Phase D: Replace "poll because maybe" with "repair because evidence"

Targets:
- lagged event recovery
- reconnect recovery
- authority mismatch repair

Rule:
- diagnostics should drive repair
- clocks should not drive authority by default

## 8. The Single Most Important Difference

The strongest peer runtimes do not merely "optimize faster". They reduce the number of times the system has to ask the same question at all.

That is the main remaining gap in `agendao`.

Today, `agendao` has already improved the event path substantially, but several frontend and repair flows still answer:

> "I do not fully trust the last event path, so I will ask the server again in 2s / 5s / 10s."

Codex, Kun, Hermes, OpenCode, and Holon each solve this in different ways, but the shared lesson is the same:

- cache the answer
- version the answer
- bound the answer
- sleep until something actually changes

When `agendao` finishes that last step, its CPU profile will flatten sharply in idle and near-idle sessions, and memory growth will become much easier to reason about.

## 9. Concrete Next Moves For `agendao`

Recommended order:

1. Convert TUI runtime/projection authority to per-session maps.
2. Replace question/permission/process periodic fallback with lag-triggered repair.
3. Add dirty-bit + memoized snapshot caches for open dialogs and status panels.
4. Replace thread-spawned telemetry repair with bounded async workers.
5. Add watcher-backed incremental workspace file index.
6. Move frontend/internal event buses toward typed envelopes to avoid repeated JSON parse/filter work.

If only one item is funded, do item 1. It unlocks most of the rest.
