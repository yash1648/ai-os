# AI-OS Documentation — 25. Performance Benchmarks

## Benchmark Categories

### Kernel Throughput
Measures objectives processed per unit time under the Stage-appropriate scheduler, holding worker execution time constant (using fake/stub workers) to isolate Kernel overhead from LLM latency. Target: Kernel-side overhead (scheduling, manifest construction, validation, event emission) should remain a small fraction of total objective wall-clock time, dominated instead by worker reasoning latency.

### Manifest Construction Latency
Measures time to construct a complete Execution Manifest from the Project Intelligence Layer as a function of repository size (symbol count, file count). Tracked across repository size tiers (small: <1k files, medium: 1k–20k files, large: 20k+ files) to ensure PIL query performance scales sub-linearly through indexing rather than degrading with raw repository scale.

### Review/Guardian Latency
Measures time for the Reviewer and Guardian to evaluate a diff, tracked separately from worker generation time, since these stages are on the critical path for every objective regardless of worker model choice.

### Concurrency Scaling
Measures achieved throughput as concurrent worker count increases, identifying the point at which lock contention (see `15-scheduler.md`) becomes the binding constraint rather than raw worker capacity.

### Rollback Latency
Measures time from failure detection to fully restored repository/state, since this directly bounds how quickly the system recovers from a bad diff and affects overall pipeline resilience under load.

## Methodology

Benchmarks are run against a fixed set of representative synthetic repositories at each size tier, using deterministic fake workers to eliminate LLM latency/cost variance from infrastructure benchmarks. Separate, explicitly labeled benchmarks track end-to-end latency and cost *with* real LLM-backed workers, but these are reported separately and are not used to judge Kernel infrastructure performance, consistent with the separation of concerns described in `23-testing-strategy.md`.

## Reporting

Benchmark results are tracked over time (per release) and regressions beyond a configured threshold (e.g., >10% increase in Kernel overhead at a given repository size tier) are treated as release-blocking, mirroring the release-gating treatment given to test coverage and Guardian self-hosting checks.

## Non-Goals

These benchmarks intentionally do not attempt to measure or optimize the *quality* of worker-generated code — that is the concern of the End-to-End LLM Evaluation suite (`23-testing-strategy.md`), not the performance benchmark suite, which is scoped strictly to infrastructure efficiency.
