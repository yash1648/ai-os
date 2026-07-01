# AI-OS Documentation — 02. Architecture Overview

## System Layers

AI-OS is organized into five layers, each with a single responsibility. Layers communicate only through well-defined interfaces; no layer reaches "around" the Kernel to touch a resource it does not own.

```
┌─────────────────────────────────────────────┐
│ 1. Intent Layer         (User, Goal Decomposer)
├─────────────────────────────────────────────┤
│ 2. Governance Layer      (Constitution, ADRs, Ownership)
├─────────────────────────────────────────────┤
│ 3. Kernel Layer          (Scheduler, Permissions, State Machine, Policy)
├─────────────────────────────────────────────┤
│ 4. Execution Layer       (Workers, Reviewer, Guardian)
├─────────────────────────────────────────────┤
│ 5. Persistence Layer     (Git, Event Log, Project Intelligence Layer)
└─────────────────────────────────────────────┘
```

## Request Lifecycle

1. A user submits a business-level goal.
2. The **Goal Decomposer** queries the Project Intelligence Layer and produces an **immutable Execution Plan**: a directed graph of objectives with dependencies and success criteria.
3. The **Kernel** admits the plan, validating it against the Constitution and current Ownership Model, and schedules objectives whose dependencies are satisfied.
4. For each ready objective, the Kernel constructs an **Execution Manifest** and dispatches it to an available **Worker**.
5. The Worker returns a **diff** and an **execution report**. The worker then terminates; no state persists.
6. The Kernel routes the diff through the **Reviewer** (correctness, style, tests, performance) and the **Architecture Guardian** (constitutional and boundary compliance).
7. If both pass, and no human approval gate is triggered, the Kernel applies the diff to Git and transitions the objective to `DONE`.
8. If a human approval gate is triggered (schema change, breaking interface, dependency addition, constitutional exception, production deploy), the Kernel pauses the objective in `INTEGRATION` pending explicit human sign-off.
9. Every step emits an event onto the **Event Bus**, producing a complete, replayable audit trail.

## Why a Kernel, Not a Manager

A "manager" agent — a supervisory LLM that coordinates other LLMs via natural-language delegation — inherits all the same probabilistic weaknesses as the workers it supervises. It can still hallucinate authority it doesn't have, approve changes it shouldn't, or lose track of a global invariant.

The Kernel is explicitly **not** an LLM. It is deterministic code implementing a fixed policy: state machine transitions, permission checks, diff validation rules, and event emission. This means the boundary between "things an AI decided" and "things the system enforced" is always visible and auditable — a property that manager-of-agents architectures cannot guarantee.

## Concurrency Model

Multiple objectives may execute concurrently provided:

- They belong to non-overlapping ownership domains, **or**
- They touch only files each objective has been explicitly granted in its Execution Manifest, and the Kernel's lock manager has serialized any overlapping file access.

The Kernel's scheduler is responsible for lock acquisition, deadlock avoidance (via ordered lock acquisition on domain IDs), and fair scheduling across competing objectives.

## Failure Philosophy

AI-OS assumes workers will sometimes produce incorrect, incomplete, or non-compliant output — this is expected, not exceptional. The system is designed so that:

- A failed diff never reaches Git.
- A partially-applied change is always revertible via Kernel-owned rollback.
- Every failure transitions the objective into a named failure state (see `07-state-machine.md`) rather than silently retrying forever or failing open.

This "fail closed, fail loud, fail auditable" posture is the architectural backbone that lets AI-OS scale worker autonomy without scaling risk proportionally.
