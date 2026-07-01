# AI-OS Documentation — 03. Project Kernel Internals

## Role

The Project Kernel is the only component in AI-OS with authority to mutate the repository, transition objective state, or apply policy decisions. It is intentionally "boring": a deterministic, well-tested piece of infrastructure with no generative capability of its own. The Kernel never writes code, never makes architectural judgment calls, and never resolves ambiguity — it only enforces rules that have already been decided (by the Constitution, by policies, or by a human).

## Kernel Responsibilities

1. **Objective lifecycle management** — creating, transitioning, and terminating objectives according to the state machine.
2. **Scheduling** — deciding which ready objectives may begin execution, subject to dependency graphs, locks, and resource limits.
3. **Permission enforcement** — validating that a worker's requested action (file write, cross-domain request, tool invocation) is within its granted scope.
4. **Diff validation** — structurally verifying that a submitted diff only touches files listed in the objective's `allowed_files`, applies cleanly against the current snapshot, and does not exceed configured size/blast-radius limits.
5. **Policy evaluation** — checking whether a diff or transition triggers a human approval gate.
6. **Rollback** — reverting the repository and objective state to the last known-good checkpoint on failure or rejection.
7. **Metrics collection** — recording duration, retries, token usage, cost, and outcome for every objective.
8. **Auditing** — persisting an immutable, timestamped record of every decision the Kernel makes.
9. **Event emission** — publishing every state transition and decision onto the Event Bus.

## Kernel Subsystems

### Scheduler
Maintains a dependency graph of objectives and a queue of ready-to-run work. Assigns objectives to available worker slots, respecting concurrency limits and domain locks. See `15-scheduler.md`.

### Permission Engine
Resolves, for each requested action, whether the current objective's manifest grants sufficient scope. Denies by default. See `14-permission-engine.md`.

### State Machine Engine
Enforces legal transitions between objective states and records the full transition history. See `07-state-machine.md`.

### Diff Applier
Applies a validated, reviewed, and approved diff atomically to the Git working tree, and creates the corresponding commit with a structured message linking back to the objective ID, worker ID, and review/guardian decision IDs.

### Rollback Manager
Snapshots repository state (via Git refs) at the start of every objective and restores it on failure, producing a `RollbackStarted` / `RollbackCompleted` event pair.

### Audit Log
An append-only, hash-chained log of every Kernel decision, sufficient to reconstruct the full history of any objective from creation to completion (or failure) without relying on external systems.

## Kernel Invariants

These properties must hold at all times, and are treated as correctness bugs (not policy violations) if broken:

- No diff is ever applied to Git without having passed both Reviewer and Guardian, unless explicitly exempted by an approved human override with a logged justification.
- No objective transitions to `DONE` while any of its declared success criteria are unmet.
- No worker process can invoke a Kernel-privileged operation directly; all requests pass through the Permission Engine.
- Every state transition has a corresponding event on the Event Bus — the audit log and event stream must always be reconstructable from one another.
- Rollback is always possible for any objective that has not yet reached `DONE`.

## Non-Responsibilities

The Kernel explicitly does **not**: generate code, interpret natural language objectives, decide *what* should be built, resolve architectural ambiguity, or evaluate code quality subjectively. These are the responsibilities of the Goal Decomposer, Workers, Reviewer, and Guardian respectively — components whose judgments the Kernel merely records and enforces.
