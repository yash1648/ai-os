# AI-OS Documentation — 09. Execution Manifest Specification

## Purpose

The Execution Manifest is the complete, self-contained "system call" a worker receives. It is constructed by the Kernel (with content sourced from the Project Intelligence Layer) and is the *only* source of context and authority a worker has for a given objective — a worker has no ambient access to anything not included in its manifest.

## Schema

```yaml
manifest_id: string
objective_id: string
plan_id: string
worker_type: enum(backend, frontend, database, testing, docs, devops, generic)

constitution:
  version: string
  sections: [string]          # relevant excerpts only, not the full document

objective:
  title: string
  description: string
  success_criteria: [string]
  priority: enum(low, medium, high, critical)

context:
  code_snapshot_ref: string    # Git ref / content-addressed snapshot ID
  relevant_symbols: [string]
  relevant_files: [string]

adr_refs: [string]             # IDs of ADRs relevant to this objective, retrieved by PIL search

interfaces:
  - id: string
    signature: string
    version: string
    compatibility_notes: string

allowed_files: [string]        # glob patterns; diff MUST NOT touch files outside this set
forbidden_actions: [string]    # e.g. "no schema migrations", "no new dependencies"

output_schema: string          # reference to the required diff+report schema version

deadline: iso8601 | null
retry_context:                 # present only on retries
  previous_attempt_id: string
  rejection_reason: string
```

## Construction Process

1. The Kernel Scheduler determines an objective is `READY` and selects a worker slot.
2. The PIL is queried for: relevant symbols and files (via dependency/symbol graph traversal from the objective's stated scope), applicable ADRs (via semantic + tag search), and current interface contracts for any interfaces the objective's scope touches.
3. The Constitution Store returns only the sections relevant to the objective's domain and risk profile (e.g., a purely frontend objective does not receive database migration policy text), keeping manifest size bounded.
4. The Permission Engine computes `allowed_files` from the Ownership Model and the objective's declared scope, and computes `forbidden_actions` from applicable policies.
5. The completed manifest is content-hashed and attached to the `WorkerStarted` event for auditability.

## Design Rationale: Minimal Sufficient Context

Manifests are deliberately scoped to the *minimum sufficient context* for the objective, rather than maximal context. This serves three goals: it bounds token cost and latency, it reduces the chance a worker's output is influenced by irrelevant or stale information, and it makes each worker's decision space auditable — reviewers can inspect exactly what a worker did and did not know when it made a given change.

## Manifest Immutability

Once dispatched, a manifest is not altered mid-execution. If circumstances change (e.g., a concurrent objective completes and updates a file the current worker is also touching), the Kernel does not attempt to hot-patch the in-flight manifest; instead, the resulting diff is validated against the *current* repository state at submission time, and a conflict results in `INTEGRATION_FAILURE` with a scheduled retry against a freshly constructed manifest.
