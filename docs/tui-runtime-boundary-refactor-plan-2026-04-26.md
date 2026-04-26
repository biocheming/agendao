# TUI Runtime Boundary Refactor Plan

Date: 2026-04-26

## Problem

The TUI adapter currently stores a `BlockingApiClient` in `AppContext` and many UI event/render paths call it directly. `reqwest::blocking` owns an internal Tokio runtime; when it is created or dropped inside the TUI async loop, Tokio can panic with:

```text
Cannot drop a runtime in a context where blocking is not allowed.
```

This violates the ROCode constitution in practice:

- Article 1: the adapter owns runtime behavior instead of delegating all execution to the runtime authority.
- Article 5: UI state and server read-model synchronization are mixed inside arbitrary UI methods.
- Article 9: adapter event/render paths perform side-effecting HTTP calls directly.

## Target Architecture

TUI should be a pure adapter:

- Render reads only local `AppContext` projection state.
- Key events enqueue typed UI commands, not direct HTTP calls.
- A single TUI runtime gateway owns HTTP/SSE IO and writes normalized results back through `UiBridge` events.
- Blocking HTTP client usage is removed from TUI; the gateway uses `AsyncApiClient`.

## Task A: Introduce TuiRuntimeGateway

- Add a gateway task owned by the TUI bridge runtime.
- Give it an async command channel and `AsyncApiClient`.
- Define typed commands for prompt dispatch, session sync, catalog refresh, mode refresh, provider/model edits, status panels, permission/question actions, and config writes.
- Define typed result events emitted back through `UiBridge`.

## Task B: Move Prompt Submission

- Replace `thread::spawn` + `BlockingApiClient::send_prompt` in prompt dispatch with gateway commands.
- Keep optimistic UI updates in `App`, but make network effects gateway-owned.
- Ensure completion/error events are the only way prompt dispatch mutates UI state after submission.

## Task C: Move Catalog And Mode Refresh

- Move model/provider/agent/mode refresh calls into gateway commands.
- Cache mode list in `AppContext` as a read model.
- Make Tab mode switching use cached mode data only; refresh happens asynchronously.

## Task D: Move Render-Time Status Fetches

- Remove all `get_api_client()` calls from render/status-panel methods.
- Status dialogs render cached snapshots.
- Opening or refreshing a status dialog enqueues a gateway fetch.

## Task E: Remove Blocking Runtime From TUI

- Replace the old `crate::api::ApiClient` alias with a real TUI API gateway.
- Keep all HTTP calls behind this gateway; UI event/render paths no longer create, own, or drop HTTP clients.
- Remove `reqwest::blocking` from `rocode-tui`; health probing uses a standard-library TCP probe.
- Use `rocode-client::AsyncApiClient` as the gateway backend. `BlockingApiClient` is not referenced by TUI.
- Add a grep-based guard in docs/review checklist: `rocode-tui/src` must not import `reqwest::blocking`, `block_in_place`, or create ad-hoc Tokio runtimes on render/event paths.

## Implemented Boundary: 2026-04-26

- `crate::api::ApiClient` is now a concrete TUI API gateway instead of a direct client alias.
- The gateway owns a single dedicated `rocode-tui-api-gateway` thread with a current-thread Tokio runtime and `rocode-client::AsyncApiClient`. HTTP I/O is no longer performed by TUI render/event paths and no blocking HTTP client is owned by TUI.
- The temporary bridge containment was removed: no `tokio::task::block_in_place`, no single-worker multi-thread TUI runtime, and no `shutdown_background()` workaround remain in `rocode-tui`.
- `rocode-tui` no longer enables `reqwest`'s `blocking` feature and no longer calls `reqwest::blocking` directly.
- `rocode-client::AsyncApiClient` has been extended with the endpoint coverage needed by TUI, so the TUI gateway uses the same shared Rust HTTP client contract as other async frontends.

## Remaining Architectural Follow-Up

- Move render-time status fetches and catalog/mode refreshes from synchronous gateway calls into typed gateway commands that update cached read models.
- Keep the public API contract in `rocode-client`/`rocode-api`; do not grow a second TUI-specific HTTP API surface.

## Final Verification

The TUI runtime boundary is considered closed when all of the following pass:

```bash
rg -n "BlockingApiClient|reqwest::blocking|block_in_place|new_multi_thread|shutdown_background" crates/rocode-tui/src crates/rocode-tui/Cargo.toml
cargo check -p rocode-tui --manifest-path /home/biocheming/tests/python/rust/rocode/Cargo.toml
cargo check -p rocode --manifest-path /home/biocheming/tests/python/rust/rocode/Cargo.toml
cargo fmt --manifest-path /home/biocheming/tests/python/rust/rocode/Cargo.toml --all --check
```

As of this update, the grep returns no matches and all checks pass.
