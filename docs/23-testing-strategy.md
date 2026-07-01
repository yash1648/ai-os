# AI-OS Documentation — 23. Testing Strategy

## Testing Philosophy

Because AI-OS's core value proposition is *deterministic* orchestration around *probabilistic* workers, testing is split into two clearly separated concerns: (1) verifying the Kernel and surrounding infrastructure behave deterministically and correctly, and (2) verifying the system degrades safely when workers behave unpredictably (which is treated as the expected case, not an edge case).

## Test Categories

### Unit Tests
Cover individual Kernel subsystems in isolation: State Machine transition legality, Permission Engine allow/deny decisions, Scheduler lock-acquisition ordering, Diff Applier atomicity.

### Contract Tests
Verify that every component's input/output conforms to its published JSON Schema (`20-json-schemas.md`) — Execution Manifests, worker outputs, Reviewer/Guardian verdicts, and events are all schema-validated in CI on every change.

### Integration Tests
Exercise the full pipeline (objective → manifest → worker → review → guardian → apply) using deterministic **fake workers** — worker stubs that produce scripted diffs and reports — so that Kernel behavior can be tested without depending on real LLM output, which is non-deterministic by nature.

### Adversarial / Fault-Injection Tests
Deliberately exercise failure paths: workers producing malformed output, diffs touching out-of-scope files, diffs that fail to apply cleanly, Reviewer/Guardian rejections, and mid-execution repository changes from concurrent objectives. Every failure state in `07-state-machine.md` must have at least one corresponding fault-injection test proving the Kernel transitions correctly and rollback succeeds.

### Concurrency Tests
Verify the lock manager's deadlock-avoidance guarantees under high contention (many objectives targeting overlapping domains), and verify fairness properties (no objective starves indefinitely) under sustained load.

### Self-Hosting / Dogfooding Tests
As referenced in `21-repository-layout.md`, changes to the AI-OS repository itself are run through AI-OS's own pipeline in CI, exercising the real Guardian against the real Constitution for the AI-OS project. A regression here is treated as a release-blocking defect.

### End-to-End LLM Evaluation (non-blocking)
A separate, non-release-blocking evaluation suite runs real LLM-backed workers against a fixed benchmark set of objectives on sample repositories, tracking quality metrics (Review pass rate, Guardian pass rate, human-approval outcomes) over time as worker models change. This suite informs worker/model selection but never gates a Kernel release, preserving the separation between infrastructure correctness and model quality.

## Coverage Expectations

Kernel, Permission Engine, State Machine, and Scheduler code require high (≥90%) unit test coverage as a release gate, consistent with the kind of coverage bar AI-OS itself enforces on managed projects via the Constitution. Worker prompt/strategy code, being inherently harder to unit-test meaningfully, is instead governed primarily by the End-to-End LLM Evaluation suite's tracked metrics.
