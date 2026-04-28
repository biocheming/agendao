# Scheduler Examples

This directory contains formal external scheduler profile examples for ROCode.

> **Tutorial & User Guide**: See [`SCHEDULER_GUIDE.md`](SCHEDULER_GUIDE.md) for a
> comprehensive guide covering all scheduler concepts, configuration, and usage patterns.
>
> **Verifier Guide**: See [`VERIFIER.md`](VERIFIER.md) for a detailed explanation of
> the verifier preset, canonical score-job algorithm, trajectory evidence, artifacts,
> and configuration advice.

## Files

### Schema

- `scheduler-profile.schema.json`
  - Formal schema for the generic scheduler profile file
  - Public orchestrator surface: `sisyphus`, `prometheus`, `atlas`, `hephaestus`, `verifier`
  - Supports per-stage policy overrides via the `stageOverride` schema
  - Exposes inline iterative workflows through `workflow`, `workflowPath`, and `iterativeWorkflow`

### Public OMO examples

- `sisyphus.example.jsonc`
  - Public OMO-aligned delegation-first example
- `prometheus.example.jsonc`
  - Public OMO-aligned planning-first example
- `atlas.example.jsonc`
  - Public OMO-aligned coordination example
- `hephaestus.example.jsonc`
  - Public OMO-aligned autonomous execution example
- `verifier.simple.example.jsonc`
  - Minimal verifier example for users who only want to keep passing candidates and pick the best one
- `verifier.example.jsonc`
  - Public verifier example showing both inline `workflow` and external `workflowPath`
- `verifier.workflow.jsonc`
  - External verifier workflow file referenced by `verifier-custom`
- `VERIFIER.md`
  - Detailed verifier guide covering the problem, algorithm, ROCode implementation,
    artifacts, fallback behavior, and tuning advice

### Agent tree files

- `trees/coordinator-tree.json`
  - Reusable coordinator agent tree with frontend-dev and backend-dev children
- `trees/deep-worker-tree.jsonc`
  - Reusable deep-worker agent tree with code-explorer and docs-researcher children (JSONC format)
- `trees/pso-swarm.json`
  - 3-particle swarm agent tree (Architect, Empiricist, Adversary) for PSO topology

### User-defined topology examples

- `pso.example.jsonc`
  - Particle Swarm Optimization topology using iterative multi-agent convergence
  - See [`PSO.md`](PSO.md) for detailed usage guide, applicable scenarios, and customization

### Related workflow examples

- [`../autoresearch_example/book-authoring.autoresearch.jsonc`](../autoresearch_example/book-authoring.autoresearch.jsonc)
  - Runnable autoresearch workflow example outside this scheduler subdirectory
- [`../autoresearch_example/book-authoring-jsonc-verify.sh`](../autoresearch_example/book-authoring-jsonc-verify.sh)
  - Companion verify script used by the checked-in autoresearch example
- [`../autoresearch_example/book-authoring-jsonc-guard.sh`](../autoresearch_example/book-authoring-jsonc-guard.sh)
  - Companion guard script used by the checked-in autoresearch example

Each example file contains two profiles:
- A **`*-default`** profile using plain stage strings (preset defaults)
- A **`*-custom`** profile demonstrating per-stage overrides

## Per-Stage Overrides

Stage entries in the `stages` array can be either:

1. **Plain string** — uses preset defaults for that stage:
   ```jsonc
   "stages": ["request-analysis", "route", "execution-orchestration"]
   ```

2. **Object with overrides** — customize individual stage policies:
   ```jsonc
   "stages": [
     "request-analysis",
     {
       "kind": "execution-orchestration",
       "toolPolicy": "allow-all",
       "loopBudget": "step-limit:10",
       "sessionProjection": "transcript",
       "childSession": true,
       "agentTree": {
         "agent": { "name": "coordinator" },
         "children": [
           { "agent": { "name": "worker-a" }, "prompt": "Do A." },
           { "agent": { "name": "worker-b" }, "prompt": "Do B." }
         ]
       }
     },
     "synthesis"
   ]
   ```

### Override fields

| Field | Type | Description |
|-------|------|-------------|
| `kind` | `string` | **Required.** Stage kind (e.g. `"plan"`, `"execution-orchestration"`). |
| `toolPolicy` | `"allow-all"` \| `"allow-read-only"` \| `"disable-all"` | Tool access policy. |
| `loopBudget` | `"unbounded"` \| `"step-limit:N"` | Max LLM loop iterations. |
| `sessionProjection` | `"hidden"` \| `"transcript"` | Whether stage output is visible. |
| `childSession` | `boolean` | Create an isolated child session. |
| `agentTree` | `AgentTreeNode` \| `string` | Per-stage agent tree (overrides profile-level). Accepts an inline object or a file path string. |
| `agents` | `string[]` | Agent name filter. |
| `skillList` | `string[]` | Skills available to this stage. |

Omitted fields fall through to the preset → hardcoded default chain.

### Override priority

```
per-stage JSON override  →  preset function override  →  hardcoded default
```

## Agent Tree File Paths

Both profile-level and per-stage `agentTree` fields accept either:

1. **Inline object** — the agent tree definition directly in the config:
   ```jsonc
   "agentTree": {
     "agent": { "name": "deep-worker" },
     "children": [
       { "agent": { "name": "code-explorer" }, "prompt": "Explore code." }
     ]
   }
   ```

2. **File path string** — a relative path to an external JSON/JSONC file:
   ```jsonc
   "agentTree": "./trees/coordinator-tree.json"
   ```

File paths are resolved relative to the config file's directory. The referenced
file must contain a valid `AgentTreeNode` JSON/JSONC object. JSONC features
(comments, trailing commas) are supported in external tree files.

### Example directory layout

```
project/
├── rocode.jsonc                          # schedulerPath → ./scheduler.jsonc
├── scheduler.jsonc                       # main scheduler config
└── trees/
    ├── coordinator-tree.json             # reusable agent tree
    └── deep-worker-tree.jsonc            # another tree (with comments)
```

### Benefits

- **Reuse**: The same agent tree file can be referenced by multiple profiles or stages.
- **Readability**: Keeps the main scheduler config compact when trees are large or deeply nested.
- **Separation of concerns**: Agent team composition can be managed independently from stage orchestration policy.

### Example files

- `trees/coordinator-tree.json` — A coordinator with frontend-dev and backend-dev children.
- `trees/deep-worker-tree.jsonc` — A deep-worker with code-explorer and docs-researcher children (JSONC format).

## Current Scope

These examples reflect the current implementation scope:

- external JSON / JSONC config parsing exists in `rocode-orchestrator`
- public preset profiles are:
  - `sisyphus`
  - `prometheus`
  - `atlas`
  - `hephaestus`
  - `verifier`
- named orchestrators are presets over the shared scheduler profile kernel, not separate execution engines
- workflow-backed profiles can inline iterative workflow config via `workflow` or load it from `workflowPath`
- `Sisyphus` currently defaults to stages:
  - `request-analysis`
  - `route`
  - `execution-orchestration`
- `Prometheus` currently defaults to stages:
  - `request-analysis`
  - `route`
  - `interview`
  - `plan`
  - `review`
  - `handoff`
- `Atlas` currently defaults to stages:
  - `request-analysis`
  - `execution-orchestration`
  - `synthesis`
- `Hephaestus` currently defaults to stages:
  - `request-analysis`
  - `execution-orchestration`
- `Verifier` currently defaults to stages:
  - `request-analysis`
  - `execution-orchestration`

## Current Behavioral Notes

These public examples now assume the following runtime semantics:

- `Prometheus`
  - planner-only workflow
  - blocking interview questions should use the formal `question` tool / question-card flow
  - review stays enabled by default before handoff
- `Atlas`
  - coordination / delegation / verification preset
  - QA `Gate Decision` YES/NO checks are Atlas internal rubric, not a user questionnaire
  - use the `question` tool only for real user decision blockers, not for Atlas's own QA responsibility
- `Hephaestus`
  - autonomous deep-worker preset
  - failure recovery follows a clearer `3-Level Escalation Protocol`
- `Verifier`
  - workflow-backed verifier preset over the autonomous execution kernel
  - use when explicit multi-candidate comparison is worth the extra judge cost
  - canonical mode keeps multiple candidates and selects the winner through score jobs over `(pair, criterion, repetition)`
  - with `useLogprobs=true`, score jobs compute A-T score-token expected reward; artifacts expose score-job matrix, logprob status, pair means, and round-robin win counts
- `Sisyphus`
  - execution-oriented single-loop preset
  - favors bounded execution with final delivery normalization rather than planner-style interview flow

## Stage Capability Observability

Scheduler stage runtime metadata distinguishes between capability pool and
runtime activation:

- `available_skill_count`
- `available_agent_count`
- `available_category_count`
  - these describe the stage's accessible capability pool only
  - they do not mean all listed capabilities were used for the current task
- `active_skills`
- `active_agents`
- `active_categories`
  - these describe runtime-verified activation only
  - they should be populated from concrete scheduling evidence such as:
    - delegated agent selection
    - delegated category selection
    - explicit skill loading

The authority boundary is strict:

- scheduler/orchestration runtime owns the semantic meaning of `active_*`
- TUI / CLI / Web consume and render these fields
- adapters must not infer "used capabilities" from the full available pool
- generic tool activity, question flow, summaries, and stage prose do not count
  as capability activation by themselves

These examples do not yet cover the full future scheduler system described in long-form plans.

## Intended Usage

These files are intended to be referenced externally by a future `schedulerPath` field in `rocode.json` / `rocode.jsonc`.

Example:

```jsonc
{
  "schedulerPath": "./docs/examples/scheduler/sisyphus.example.jsonc"
}
```

## Validation

The checked-in public examples should stay aligned with the scheduler runtime authority:

- they should parse through `SchedulerConfig::load_from_file(...)`
- their default profile should resolve successfully
- their `orchestrator` and `stages` should match the corresponding public preset defaults in code
