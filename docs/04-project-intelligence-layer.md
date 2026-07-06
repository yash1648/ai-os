# AI-OS Documentation — 04. Project Intelligence Layer (PIL)

## Purpose

The Project Intelligence Layer is the system of record for everything about a repository that a worker needs to know but should not have to rediscover by scanning source files. Its existence is what allows workers to remain stateless and context-bounded: instead of loading an entire repository into a prompt, a worker queries the PIL for exactly the slice of knowledge relevant to its objective.

## Subsystems

### Repository Index
A structured, continuously updated index of files, directories, and ownership tags. Supports fast lookup by path, domain, and change history.

### Symbol Graph
Cross-referenced index of functions, classes, types, and modules, including definition sites and all call/reference sites. Enables blast-radius queries such as "what breaks if this function's signature changes?"

### Dependency Graph
A directed graph of module- and package-level dependencies, used to detect forbidden dependency introductions and circular dependency risk before a diff is even generated.

### Interface Registry
Tracks every declared interface (API contract, internal module boundary, event schema) along with its owner, consumers, version, and compatibility status. See `10-interface-registry.md`.

### ADR Index
A searchable, tagged index of Architecture Decision Records, retrievable by topic, affected component, or keyword, so that only relevant ADRs are injected into a given Execution Manifest.

### Constitution Store
The current, versioned Project Constitution, exposed as both a machine-checkable rule set (for the Guardian) and human-readable prose (for worker context injection).

### Ownership Map
The authoritative mapping from file/module to owning domain, used by the Permission Engine to evaluate cross-domain requests.

### Semantic Search Index
An embedding-based search index over code, documentation, ADRs, and commit history, enabling the Goal Decomposer and Workers to retrieve conceptually relevant context even when exact symbol names are unknown.

## Query Model

Workers and the Goal Decomposer never scan the filesystem directly. All context retrieval flows through PIL query APIs:

- `getSymbol(name, scope)`
- `getDependents(module)`
- `getInterface(id)`
- `getOwnership(path)`
- `searchADRs(query, tags)`
- `getConstitution(section)`
- `semanticSearch(query, k)`

This indirection is deliberate: it bounds the amount of context injected into any single worker, keeps queries auditable, and allows the PIL's indexing strategy to evolve independently of worker implementations.

## Freshness and Consistency

The PIL is updated synchronously whenever the Kernel applies a diff to Git — index updates are part of the same transaction boundary as the commit, so no worker ever queries a PIL that reflects a state older than the last successfully applied change. During concurrent objective execution, the PIL exposes point-in-time snapshots keyed to the repository ref a given Execution Manifest was built from, preventing workers from reasoning about a moving target.

### Plan Decomposition & Admission

The PIL hosts the **planner subsystem** with three workflow endpoints:

| Endpoint | Description |
|---|---|
| `POST /api/v1/plan/decompose` | Business objective → `ExecutionPlan` using LLM decomposition |
| `POST /api/v1/plan/admit` | Deterministic validation (structural, domain, DAG, constitution, criteria, risks) |
| `POST /api/v1/plan/submit` | Admitted plan → Kernel objectives (calls Kernel CRUD API) |

The decomposition uses an OpenAI-compatible LLM when configured, or a deterministic mock for local development. Admission is purely deterministic (no LLM) — an `AdmissionVerdict` with zero error-severity issues is required before submission.

## Relationship to the Kernel

The PIL is read-heavy and advisory: it informs decisions but does not make them. The Kernel does not trust worker-reported understanding of PIL data; the Guardian independently re-derives the facts it needs (e.g., dependency graph state) from the PIL at validation time, rather than trusting the worker's manifest-time snapshot, to guard against staleness or manipulation.
