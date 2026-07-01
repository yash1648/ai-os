# AI-OS Documentation — 15. Scheduler Design

## Purpose

The Scheduler decides, at any given moment, which `READY` objectives should be dispatched to workers, subject to dependency ordering, lock availability, and resource limits. It is analogous to a process scheduler in a traditional OS: it does not decide *what* work exists (that's the Goal Decomposer's job) but decides *when and in what order* existing work runs.

## Inputs

- The dependency graph from the current active Execution Plan(s).
- Current lock table (which files/domains are currently held by in-flight objectives).
- Worker pool capacity (configured concurrency limits, possibly per-domain).
- Objective priority (from the plan) and any deadline metadata.

## Scheduling Algorithm (Stage 1–2 baseline)

1. Compute the set of objectives whose dependencies are all in `DONE` — these are eligible.
2. Filter out objectives whose required file/domain locks are currently held by another in-flight objective.
3. Order remaining eligible objectives by priority, then by plan-declared sequence, then by age (oldest-ready-first) as a tiebreak.
4. Dispatch objectives to available worker slots up to the configured concurrency limit, acquiring the necessary locks atomically before dispatch (all-or-nothing, to avoid partial-lock deadlocks).

## Lock Management

Locks are acquired at the granularity of the objective's `allowed_files` / domain scope. To avoid deadlock:

- Locks for a given objective are acquired in a fixed global ordering (e.g., lexicographic by path), never in an order dependent on runtime timing.
- An objective that cannot acquire all required locks atomically is not partially dispatched; it remains `READY` and is retried on the next scheduling pass.

## Fairness

The Scheduler avoids starvation by incorporating objective age into ordering (see step 3), ensuring a low-priority objective blocked behind a busy domain eventually gets scheduled once contention clears, rather than being perpetually deprioritized by a stream of higher-priority arrivals.

## Concurrency Growth Path

- **Stage 1**: single worker, no real concurrency — the scheduler is a simple FIFO queue.
- **Stage 2**: multiple domain-specialized workers running concurrently, gated by the lock manager described above.
- **Stage 3+**: asynchronous, event-driven dispatch, where scheduling decisions are triggered by events (`ObjectiveReady`, `LockReleased`) rather than polling, and the worker pool may span multiple processes or machines.

## Resource Limits

The Scheduler respects configurable ceilings: max concurrent objectives globally, max concurrent objectives per domain, max token/cost budget consumed per unit time, and max queue depth (beyond which new objective admission is throttled rather than allowed to grow unbounded). Breaching a limit does not fail objectives; it delays dispatch and emits a `SchedulingThrottled` event for observability.
