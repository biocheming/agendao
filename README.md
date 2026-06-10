<p align="right">
  <strong>English</strong> | <a href="./README_CN.md">中文</a>
</p>

<p>
  <img src="icons/logo.svg" alt="AgenDao" width="280" />
</p>

# AgenDao

> **Turn input, execution, orchestration, output, and feedback into one living flow instead of five stitched-together features.**

Most AI coding tools focus on the question of "how to do it": how to generate code, call tools, or chain reasoning steps. AgenDao focuses on a different question: **when a software task runs for hours, spans multiple sessions, crosses multiple frontends, and goes through forks, rewinds, compaction, and replay, what keeps the whole system on the same working thread?**

The answer is not only model capability. It is governance.

Current version: `v2026.6.10`

---

## AgenDao In One Sentence

AgenDao belongs to the same broad category as other AI agent tools: a local coding-agent runtime. But its design center is not "make the model smarter". It is **make the system flow better, and make it easier to trust**.

Its core diagnosis is simple: **system drift does not mainly come from weak models; it comes from input, execution, state, output, and feedback splitting into multiple competing authorities.**

So AgenDao is not about adding yet another "smarter agent" layer. It is about making the entire chain, from prompt input to the next prompt, obey the same governance model.

---

## The Dao Canon Of AgenDao

AgenDao is not designed by working backward from a feature list. It is designed from one governing statement:

> **Every semantic domain must have exactly one authority; that authority must close the yin-yang loop and participate in the five-phase flow.**

Here, yin and yang are not decorative metaphors. They are runtime constraints:

- **Yang**: input, triggering, execution, expansion, display
- **Yin**: convergence, normalization, stabilization, accounting, recycling, verification

Pure yang becomes agitation: input and execution exist, but there is no carrying layer and no return path.

Pure yin becomes stagnation: rules and authority exist, but nothing is truly triggered or delivered.

That is why AgenDao asks every product path to satisfy three conditions:

1. One authority
2. A yin-yang pairing
3. A five-phase flow

---

## AgenDao Through The Five Phases

### Wood: input

`prompt`, attachments, slash commands, history, citations, and mode switching all belong to **wood**.

The law of wood is: **growth is precious; uncontrolled branching is not**.

AgenDao therefore rejects:

- text, attachments, hints, and outbound payloads splitting into multiple input truths
- input components that only hold visible text while the real outbound content lives elsewhere
- each frontend inventing its own prompt-surface semantics

### Fire: execution

The LLM loop, permission adjudication, tool scheduling, optimistic submit, cancel, interrupt, and retry all belong to **fire**.

The law of fire is: **ignition is precious; many furnaces are not**.

AgenDao therefore requires:

- every `model -> tool -> model` cycle to be driven by one execution kernel
- permission decisions to happen at one adjudication point
- tool scheduling, fallback, normalization, and name repair to be implemented once

### Earth: orchestration and carrying

Configuration, session state, context management, the serialized prompt surface, and cross-frontend side-effect routing all belong to **earth**.

The law of earth is: **unification is precious; fractured ground is not**.

This is AgenDao's center. Wood, fire, metal, and water may all be strong, but if earth is unstable, the whole path breaks.

AgenDao therefore requires:

- one live source of truth for configuration
- one owner for every state domain
- all side effects to route through orchestration
- the serialized prompt surface to be constructed by one authority

### Metal: output

Assistant responses, tool output, scheduler stages, reasoning presentation, message projection, and event grammar all belong to **metal**.

The law of metal is: **form is precious; competing blades are not**.

So AgenDao does not treat "a lot of things came out" as success. Output must have structure, priority, and one authoritative grammar.

### Water: feedback and return flow

Telemetry, cache, memory, compaction, replay, resend, workflow usage, and session usage all belong to **water**.

The law of water is: **return and storage are precious; visible-but-unusable residue is not**.

AgenDao therefore rejects:

- display without reinjection
- telemetry write paths with no hot-path consumers
- cache and usage semantics that diverge across frontends

---

## Generation, Not Accumulation

AgenDao is not trying to stuff more features into one agent. It is trying to restore a living chain:

1. **Wood generates fire**: input can be ignited directly by the execution kernel
2. **Fire generates earth**: execution state returns into one orchestration layer
3. **Earth generates metal**: session state, context, and prompt surface become one output grammar
4. **Metal generates water**: output settles into telemetry, cache, memory, usage, and replay
5. **Water generates wood**: what was settled feeds the next input instead of dying in logs and sidebars

If a system can accept input, run, and display output but cannot naturally feed the next turn, it is not actually closed-loop.

---

## Constraint, Not Hostility

The five phases also define governance boundaries:

- **Metal constrains wood**: rules, tips, and format hints must not overpower the input itself
- **Wood constrains earth**: input variants must not multiply until they break a single authority
- **Earth constrains water**: governance may shape feedback, but must not reduce it to decoration
- **Water constrains fire**: telemetry, cache, and memory may restrain wasteful execution, but may not replace true execution semantics
- **Fire constrains metal**: runtime events may enrich output, but may not dissolve its final form

This is why many AgenDao choices are not "more complexity for its own sake". They exist to prevent the familiar failure mode:

> More features arrive, and the system becomes less coherent.

---

## How This Becomes Engineering

This language is not just philosophy. It changes how code is split, who owns state, how frontends read data, and how return-flow is consumed.

- For every new capability, ask **earth** first: which crate, which state domain, which authority owns it?
- For every new interaction, ask **wood** next: does input still flow through the same prompt authority, or did a second draft model quietly appear?
- For every new runtime path, ask **fire**: who ignites it, who cancels it, who adjudicates permissions, who accounts for its running state?
- For every new display surface, ask **metal**: does it reuse the existing event grammar and message projection, or invent a second output structure that only looks similar?
- For every new telemetry, cache, or memory path, ask **water**: who will actually consume it on the next turn?

In AgenDao, the real bad smell is not "the code feels inelegant". It is this:

- one semantic domain has two truths
- one result is display-only and never feeds back
- one frontend starts copying middle-layer authority for convenience

---

## Three Long-Horizon Capabilities

### Memory: keep what deserves to remain

- new material first enters as a candidate, then passes validation, conflict checking, and consolidation before becoming a real record
- retrieval shows a preview first so the system can explain why it is being injected
- durable memory records stay separate from temporary session material, so drafts do not pollute long-term context

### Skills: grow, but also converge

- the usage ledger tells which skills are actually used
- negative entropy surfaces long-idle skills that should be reviewed or retired
- semantic conflict and composition relationships prevent the same capability from growing twice under different names
- runtime gating and proposal review are distinct: visible for inspection does not mean executable at runtime

### Context caching: protect a stable prompt surface

AgenDao's cache strategy is not "add a few cache fields". It is to make the prompt surface stable, explainable, and diagnosable over time. It records prompt-surface fingerprints, cache evidence, and the context-closure contract, and it splits request, live, and workflow usage into separate ledgers. See [docs/context-caching.md](docs/context-caching.md).

---

## Runtime Boundaries

AgenDao is a complete local coding-agent runtime. CLI, TUI, and Web are not three separate products. They are three reading surfaces over the same underlying terrain: one session authority, one scheduler authority, one tool authority, one provider authority, one skill authority, one memory authority, one telemetry authority.

- providers do not rely on npm package names or historical aliases; `ProviderProfile`, descriptors, validation, and runtime profiles share one authority model
- sessions do not merge live context, child-workflow cost, and accumulated usage into one blurry number; CLI, TUI, and Web all read request, live, and workflow ledgers plus the context-closure contract
- fork and subsession boundaries are explicit: children receive only explicit packets; parents absorb only results and summaries
- config, providers, schedulers, and the skill tree have one shared read-only explanation surface for "what is actually in effect right now"
- Web, TUI, and Server align message synchronization on authoritative `session.updated`; streaming `output_block` and the persisted final message do not diverge as separate truths
- all three frontends share one event contract

Built-in scheduler presets: `sisyphus` · `prometheus` · `atlas` · `hephaestus` · `verifier`

---

## Entry Points

### `agendao`

The full TUI, and the best entry point for sustained work.

### `agendao run`

Single-shot execution for scripts, CI, and batch tasks.

### `agendao serve` / `agendao web`

Server and Web entry points for scenarios that need a long-lived observability surface.

### `agendao attach`

Attach to a session already maintained by the server.

### `agendao acp`

Agent Client Protocol server entry point.

---

## Quick Start

### Requirements

- Rust stable
- Cargo
- Git

### Build

```bash
cargo build -p agendao
```

If you also need the Web frontend:

```bash
npm --prefix apps/agendao-web install
cargo build -p agendao
```

### Run

```bash
cargo run -p agendao --                      # default TUI
cargo run -p agendao -- tui --socket         # Unix socket
cargo run -p agendao -- tui --attach-url http://127.0.0.1:3000  # HTTP attach
cargo run -p agendao -- run "review the riskiest changes in this repo"
cargo run -p agendao -- serve --hostname 127.0.0.1 --port 3000
cargo run -p agendao -- web --hostname 127.0.0.1 --port 3000
```

### Local install

```bash
./scripts/install-local.sh release ~/.local
```

---

## Internal Topology

- `crates/agendao` - the product distribution shell and the only formal distribution entry
- `crates/agendao-cli` / `crates/agendao-tui` / `apps/agendao-web` - three frontend surfaces over the same runtime authority
- `crates/agendao-server` - HTTP, SSE, and runtime control; the cross-frontend observability and scheduling surface
- `crates/agendao-session` - the session domain model, prompt-surface organization, and context continuity; the heaviest earth-and-water layer
- `crates/agendao-orchestrator` - scheduler and orchestration authority; the fire-and-earth core
- `crates/agendao-provider` - provider profiles, transport, descriptors, and cache; the boundary around prompt-surface and usage semantics
- `crates/agendao-skill` - skill authority, hub, distribution, and guard
- `crates/agendao-memory` - memory validation, retrieval, conflict handling, and promotion; the layer that turns output into reusable return-flow

More detail: [docs/README.md](docs/README.md)

---

## Developer Commands

```bash
cargo fmt --all
cargo check
cargo check -p agendao -p agendao-cli -p agendao-server -p agendao-tui
```

Release versioning:

```bash
./scripts/release-date.sh 2026-05-17
./scripts/sync_version.sh
```

---

## Next

- User guide: [USER_GUIDE.md](USER_GUIDE.md)
- Documentation index: [docs/README.md](docs/README.md)
- Context caching: [docs/context-caching.md](docs/context-caching.md)
- Changelog: [CHANGELOG.md](CHANGELOG.md)

---

## Acknowledgements

AgenDao's architecture has been shaped by the broader open-source AI agent community. Special thanks to [OpenCode](https://github.com/anomalyco/opencode), [Hermes Agent](https://github.com/stitionai/hermes-agent), [Codex](https://github.com/openai/codex), [Holon](https://github.com/holon-run/holon), and [LLM-as-a-Verifier](https://github.com/llm-as-a-verifier) for their earlier exploration.
