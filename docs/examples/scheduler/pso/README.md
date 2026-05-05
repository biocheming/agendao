# PSO Scheduler — Particle Swarm Optimization Topology

A user-defined scheduler topology that applies Particle Swarm Optimization
principles to multi-agent code exploration and decision-making.

## Overview

Three cognitive "particles" independently explore a task from orthogonal
perspectives, then converge through iterative synthesis rounds. Each particle
maintains its unique identity while learning from the global-best result of
previous iterations.

```
request-analysis
       │
       ▼
┌────────────── Iteration 1 ──────────────┐
│  execution-orchestration                │
│    swarm-coordinator                    │
│      ├── particle-alpha  (Architect)    │
│      ├── particle-beta   (Empiricist)   │
│      └── particle-gamma  (Adversary)    │
│  synthesis → global-best summary        │
└─────────────────────────────────────────┘
       │  transcript accumulates
       ▼
┌────────────── Iteration 2 ──────────────┐
│  particles see iteration-1 synthesis    │
│  reflect → re-explore → new findings   │
│  synthesis → updated global-best        │
└─────────────────────────────────────────┘
       │
       ▼
┌────────────── Iteration 3 ──────────────┐
│  final convergence                      │
│  synthesis → converged result           │
└─────────────────────────────────────────┘
```

## Particle Roles

| Particle | Cognitive Signature | Explores For |
|----------|-------------------|--------------|
| **Alpha** — Architect | Structural thinking | Layered abstractions, interface boundaries, separation of concerns |
| **Beta** — Empiricist | Evidence-driven exploration | Actual code paths, data flows, concrete proof |
| **Gamma** — Adversary | Failure-mode thinking | Security holes, race conditions, edge cases, unsafe assumptions |

These three perspectives are deliberately orthogonal — they evaluate the same
solution against different criteria, which makes PSO converge toward results
that satisfy multiple objectives simultaneously.

## When to Use PSO

### Decision Criteria

PSO is justified when **both** conditions hold:

1. **The problem has multiple orthogonal evaluation dimensions** — a single
   perspective will necessarily miss critical information.
2. **A single-pass exploration is likely to converge on a local optimum** —
   iterative refinement is needed to escape blind spots.

If only condition 1 holds, a standard Atlas (single-round coordination) is
sufficient. If only condition 2 holds, a single agent with a larger
`loopBudget` will do.

### Good Fit

#### Large-scale architecture decisions

```
"Our monolith needs to be split into microservices. How should we decompose it?"
```

- Alpha proposes domain boundaries and service topology
- Beta reads actual code, maps real call graphs and data coupling
- Gamma finds distributed transaction risks, data consistency gaps, network partition hazards

Why iteration matters: Alpha's first-round proposal is idealized. In round 2,
Beta targets verification of Alpha's specific boundaries. Gamma attacks the
concrete plan rather than hypotheticals. **Iteration deepens analysis, it
doesn't repeat it.**

#### Complex root cause analysis

```
"Production OOM occurs intermittently during peak hours, recovers after restart."
```

- Alpha analyzes memory management architecture for potential leak paths
- Beta reads code, greps allocation patterns, traces object lifetimes
- Gamma constructs reproducing conditions, looks for race conditions

Why iteration matters: Round 1 produces fragmented clues. In round 2, Beta may
confirm that the path Alpha identified does hold unreleased references. Gamma
may discover that Beta's code segment triggers the issue specifically under
concurrency. **Convergence happens when clues cross-validate.**

#### Security audit

```
"Audit the security of this payment processing system."
```

- Alpha reviews authentication/authorization architecture, trust boundaries
- Beta reads input handling code, finds actual injection surfaces
- Gamma runs OWASP Top 10 checklist, constructs attack vectors

Why iteration matters: Gamma may flag SSRF risk in round 1. In round 2, after
seeing Beta's analysis of actual input handling, Gamma discovers that endpoint
is actually guarded — but a different endpoint Beta overlooked is the real
exposure.

#### Technology migration evaluation

```
"Evaluate the impact of migrating from REST to gRPC."
```

- Alpha analyzes protocol-level architectural differences, compatibility strategies
- Beta surveys actual endpoints, client dependencies, dependency chains
- Gamma identifies breakage points during migration, failure modes of gradual rollout

### Poor Fit — Use Simpler Schedulers

| Scenario | Why PSO is Overkill | Use Instead |
|----------|-------------------|-------------|
| Clear bug fix (known root cause) | Single perspective is sufficient | Hephaestus |
| New CRUD endpoint | Requirements are unambiguous, no decision space | Sisyphus |
| Documentation update | No adversarial perspective needed | Sisyphus |
| Code formatting / renaming | No exploration space at all | Direct execution |
| Small refactor (< 3 files) | Single agent can see everything in one pass | Hephaestus |
| Well-specified feature with clear requirements | No competing evaluation criteria | Sisyphus |

### Quick Heuristic

> **If you would think for 10 minutes before starting the task, and might
> overturn your initial approach midway through — that's a PSO scenario.**
>
> **If you know exactly what to do the moment you see the task — use Sisyphus
> or Hephaestus.**

PSO solves **cognitive convergence** problems, not **execution efficiency**
problems. Using it to speed up simple tasks wastes tokens. Using it to prevent
blind spots in complex decision spaces is the correct application.

## How It Works (Runtime Mechanics)

### Agent tree execution per iteration

Each `execution-orchestration` stage runs the agent tree (`agent_tree.rs`):

1. **Coordinator** executes first — produces an initial draft
2. **3 particles** execute in parallel — each receives the coordinator's draft
   plus the original task, explores from its own cognitive angle
3. **Coordinator** aggregates — synthesizes child outputs into a unified result

### Inter-iteration context propagation

The `synthesis` stage after each iteration writes a global-best summary into
the transcript. Because `sessionProjection: "transcript"` is set, the next
iteration's agents see the full history.

This means:
- **Particle identity** (systemPrompt) = inertia — each particle keeps its cognitive bias
- **Previous synthesis** (in transcript) = global-best — swarm consensus from last round
- **Particle's own previous output** (in transcript) = personal-best — individual history

Each iteration naturally performs the PSO velocity update without explicit
bookkeeping.

### Double aggregation per iteration

Each iteration actually has **two aggregation layers**:

1. Coordinator aggregation (within the agent tree)
2. Synthesis stage aggregation (at the scheduler level)

This accelerates convergence compared to a single aggregation step.

## Profiles

| Profile | Iterations | Per-iteration Budget | Best For |
|---------|-----------|---------------------|----------|
| `pso-3iter` | 3 | `step-limit:12` | Most architecture/audit/migration tasks |
| `pso-5iter` | 5 | `step-limit:10` | Deeply complex tasks, large-scale system design |

## Files

```
example.jsonc              # Scheduler config with pso-3iter and pso-5iter profiles
trees/pso-swarm.json       # 3-particle agent tree (Architect, Empiricist, Adversary)
```

The agent tree is in a separate file so that:
- Different particle compositions can be swapped without changing the scheduler config
- The same swarm tree can be referenced by multiple profiles or stages
- Complex trees don't clutter the stage configuration

## Customization

### Changing particle roles

Edit `trees/pso-swarm.json` to replace the 3 particles. For example, a
data-engineering variant:

```json
{
  "agent": { "name": "swarm-coordinator" },
  "children": [
    { "agent": { "name": "pipeline-architect" }, "prompt": "Data pipeline topology and throughput." },
    { "agent": { "name": "schema-analyst" }, "prompt": "Schema evolution, backward compatibility." },
    { "agent": { "name": "failure-injector" }, "prompt": "Data corruption, late arrivals, schema drift." }
  ]
}
```

### Adjusting iteration count

Add or remove `execution-orchestration` + `synthesis` pairs in the stages
array. More iterations = deeper convergence but higher token cost.

### Mixing with other stages

PSO is compatible with other stage kinds:

```jsonc
"stages": [
  "request-analysis",
  "interview",                    // clarify requirements first
  // ... PSO iterations ...
  { "kind": "execution-orchestration", ... },
  "synthesis",
  { "kind": "execution-orchestration", ... },
  "synthesis",
  "review",                       // review the converged result
  "handoff"                       // hand off to implementation
]
```
