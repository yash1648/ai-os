# AI-OS Documentation — 07. State Machine Specification

## Primary States

```
DISCOVERED → PLANNED → READY → EXECUTING → REVIEW → INTEGRATION → DONE
```

| State | Meaning |
|---|---|
| `DISCOVERED` | Objective identified (by Goal Decomposer or manual entry) but not yet incorporated into an admitted plan. |
| `PLANNED` | Objective is part of an admitted Execution Plan, with dependencies and success criteria defined. |
| `READY` | All dependencies satisfied; eligible for scheduling. |
| `EXECUTING` | A worker has been dispatched and is actively producing a diff. |
| `REVIEW` | A diff has been submitted and is undergoing Reviewer and Guardian evaluation. |
| `INTEGRATION` | Diff has passed Review and Guardian; awaiting Kernel application (and human approval, if gated). |
| `DONE` | Diff applied to Git; success criteria verified; objective closed. |

## Failure States

Each primary state has an associated failure path, entered when that stage cannot successfully complete:

| Failure State | Entered From | Typical Cause |
|---|---|---|
| `PLANNING_FAILURE` | `DISCOVERED` | Decomposer cannot produce a valid plan; irreducible ambiguity. |
| `PERMISSION_FAILURE` | `READY` / `EXECUTING` | Permission Engine denies a requested scope. |
| `EXECUTION_FAILURE` | `EXECUTING` | Worker produces malformed output or times out. |
| `REVIEW_FAILURE` | `REVIEW` | Reviewer rejects the diff (correctness, style, tests). |
| `INTEGRATION_FAILURE` | `INTEGRATION` | Guardian rejects the diff, or apply fails against updated base. |
| `HUMAN_REJECTED` | `INTEGRATION` | A required human approval gate results in explicit rejection. |
| `ROLLBACK` | any post-`EXECUTING` state | Downstream failure after partial application; Kernel reverts to last known-good state. |

## Transition Rules

- Transitions are strictly forward, or into a failure state; there is no direct transition from `DONE` back to an earlier state — remediation of a completed objective requires a new objective.
- From any failure state, the Kernel may re-enter `READY` (retry) if within configured retry limits, or transition to a terminal `ABANDONED` state if retries are exhausted or a human explicitly closes the objective without resolution.
- `ROLLBACK` always resolves to either `READY` (retry permitted) or `ABANDONED` (retry exhausted or manually closed) — never directly to `DONE`.

## Guarantees

- Every transition is recorded as an event (see `08-event-bus.md`) with a timestamp, actor (Kernel, Worker ID, Reviewer, Guardian, or human identity), and reason.
- The full transition history of any objective is queryable and forms the basis of the audit trail.
- No objective may skip a state; even a trivially simple change passes through `REVIEW` and `INTEGRATION` rather than being fast-tracked, preserving a uniform guarantee across all changes regardless of perceived risk.

## Idempotency

Because retries spawn new stateless workers, the state machine is designed so that re-entering `EXECUTING` from a failure state is always safe: the new worker attempt operates against the current repository snapshot (post-rollback, if applicable) and does not depend on any partial artifact from the failed attempt.
