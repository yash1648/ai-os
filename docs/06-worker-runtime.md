# AI-OS Documentation — 06. Worker Runtime Specification

## Definition

A Worker is a stateless execution unit — typically, but not necessarily, backed by an LLM — that accepts exactly one Execution Manifest, performs the work described by a single objective, and returns exactly one diff plus one execution report before terminating. Workers hold no memory of prior objectives and cannot communicate with other workers except through Kernel-mediated channels (cross-domain requests, events).

## Lifecycle

1. **Dispatch** — the Kernel Scheduler assigns a ready objective to an available worker slot and constructs the Execution Manifest.
2. **Cold start** — the worker process is instantiated fresh; no prior conversation, memory, or cache from a previous objective is available.
3. **Manifest ingestion** — the worker parses the manifest: constitution excerpts, relevant ADRs, code snapshot, interface contracts, allowed files, forbidden actions, and output schema.
4. **Reasoning** — the worker performs whatever internal reasoning process it needs (chain-of-thought, tool calls to the PIL for additional read-only context, iterative drafting) entirely within its own sandboxed process.
5. **Diff generation** — the worker produces a single unified diff, scoped strictly to `allowed_files`.
6. **Report generation** — the worker produces a structured execution report: summary of changes, self-assessed confidence, tests added/modified, and any assumptions made.
7. **Termination** — the worker process exits. No state survives termination; a retried objective spawns an entirely new worker instance.

## Worker Types (Domain Specializations)

Workers are typically specialized by domain to improve quality and reduce context size, mirroring the Ownership Model:

- Backend Worker
- Frontend Worker
- Database Worker
- Testing Worker
- Documentation Worker
- DevOps Worker

Specialization is a routing hint for the Scheduler, not a security boundary — the Permission Engine enforces allowed-file scope regardless of which worker type is assigned.

## Sandbox Constraints

Workers execute inside an isolated sandbox with:

- Read-only access to the code snapshot provided in the manifest (not the live repository).
- Read-only, rate-limited query access to the Project Intelligence Layer.
- No network access to production systems, package registries, or external services unless explicitly and narrowly granted by policy for the objective.
- No ability to invoke Git operations, filesystem writes outside a scratch directory, or shell commands with side effects beyond the sandbox.

## Output Contract

A worker's output must conform to the manifest's `output_schema`. At minimum:

```yaml
diff: <unified diff text>
report:
  summary: string
  files_changed: [string]
  tests_added: [string]
  assumptions: [string]
  confidence: enum(low, medium, high)
  open_questions: [string]
```

Malformed output (schema violation, diff touching files outside `allowed_files`, diff that fails to apply cleanly) is rejected by the Kernel before it ever reaches the Reviewer — this is a structural check, not a quality judgment.

## Retry Semantics

If a worker fails structurally (malformed output) or fails Review/Guardian, the Kernel may schedule a retry. Each retry is a brand-new, stateless worker instance; it receives an augmented manifest that includes the prior attempt's rejection reason, but it does not inherit any other memory. This keeps failure recovery deterministic and preserves the statelessness invariant.

## Why Statelessness

Persistent, cross-objective worker memory is a major source of drift in agentic systems: a worker's internal model of "how this codebase works" can diverge from ground truth over time, and that divergence is invisible until it causes a defect. By forcing every objective to be handled by a freshly instantiated worker that only knows what the manifest and PIL tell it, AI-OS guarantees that a worker's understanding of the codebase is always as current as the last applied commit.
