# AI-OS Documentation — 08. Event Bus Specification

## Purpose

The Event Bus is the nervous system of AI-OS. Every meaningful action — a state transition, a diff submission, a review outcome, a rollback — is published as an immutable event. The event stream is both the mechanism for loosely coupling components (the Reviewer does not call the Guardian directly; it emits `ReviewPassed` and the Guardian subscribes) and the substrate for the audit trail and timeline UI.

## Design Principles

- **Append-only.** Events are never edited or deleted once published.
- **Total ordering per objective.** Events related to a single objective are strictly ordered; global ordering across objectives is not guaranteed but is not required for correctness.
- **Self-contained payloads.** An event carries enough information to be understood without requiring a join against live system state — critical for long-term auditability after components are upgraded or replaced.
- **At-least-once delivery to subscribers**, with idempotent consumers, to tolerate transient delivery failures without losing signal.

## Core Event Types

| Event | Emitted When |
|---|---|
| `ObjectiveCreated` | An objective enters `DISCOVERED` or `PLANNED`. |
| `PlanGenerated` | The Goal Decomposer emits a new Execution Plan. |
| `PlanApproved` | The Kernel admits a plan into active scheduling. |
| `WorkspaceLocked` | The Scheduler acquires domain/file locks for an objective. |
| `WorkerStarted` | A worker process is dispatched for an objective. |
| `WorkerFinished` | A worker returns output (successful or malformed) and terminates. |
| `DiffGenerated` | A structurally valid diff is produced. |
| `ReviewPassed` / `ReviewFailed` | The Reviewer completes evaluation. |
| `GuardianPassed` / `GuardianFailed` | The Architecture Guardian completes evaluation. |
| `IntegrationStarted` | The Kernel begins applying an approved diff. |
| `MergeCompleted` | The diff is committed to Git. |
| `ObjectiveCompleted` | The objective reaches `DONE` with all success criteria verified. |
| `RollbackStarted` / `RollbackCompleted` | The Rollback Manager reverts repository/state. |
| `HumanApprovalRequested` / `HumanApprovalGranted` / `HumanApprovalDenied` | A gated action awaits or receives a human decision. |

## Event Schema

```yaml
event_id: uuid
type: string
timestamp: iso8601
objective_id: string
actor:
  kind: enum(kernel, worker, reviewer, guardian, human, scheduler)
  id: string
payload: object      # event-type-specific structured data
causation_id: uuid    # the event that directly caused this one, if any
correlation_id: uuid  # groups all events belonging to one plan execution
```

## Consumers

- **Timeline UI / Dashboard** — renders human-readable objective histories.
- **Audit Log** — persists a hash-chained record for compliance and post-mortem analysis.
- **Metrics Pipeline** — aggregates durations, retry counts, and outcome rates from event sequences.
- **Architecture Guardian** — in advanced configurations, subscribes to `DiffGenerated` to begin pre-fetching relevant dependency-graph state ahead of formal evaluation.

## Replay and Reconstruction

Because every state transition has a corresponding event, the entire history of any objective — and, in aggregate, of the whole project — can be reconstructed purely from the event log, independent of the live database state. This property is deliberately relied upon for disaster recovery: rebuilding Kernel state from the event log is a supported operational procedure, not an afterthought.
