# AI-OS Documentation — 22. Development Roadmap

## Stage 1 — Single-Process Kernel

Goal: prove the core loop (objective → manifest → worker → review → apply) works end-to-end in the simplest possible deployment.

Deliverables:
- State Machine engine (primary + failure states)
- Objective model and storage
- Execution Manifest construction (manual/simplified PIL stub)
- A single stateless worker implementation
- Reviewer (basic correctness/style checks)
- Rollback (Git-ref based)
- Basic metrics collection

## Stage 2 — Domain Ownership

Goal: support multiple concurrent workers safely via ownership and interface tracking.

Deliverables:
- Ownership Model + Permission Engine
- Backend and Frontend domain-specialized workers
- Interface Registry (initial version)
- Cross-domain request flow

## Stage 3 — Event-Driven Runtime

Goal: move from a synchronous, polling core loop to a fully event-driven runtime, enabling asynchronous, non-blocking scheduling.

Deliverables:
- Event Bus (durable, at-least-once delivery)
- Persistent timeline / dashboard backend
- Asynchronous worker dispatch
- Audit log derived from event stream

## Stage 4 — Project Intelligence Layer

Goal: replace manifest-construction stubs with a real, continuously updated intelligence layer.

Deliverables:
- Repository indexing pipeline
- Symbol graph and dependency graph
- ADR engine (index + retrieval)
- Constitution engine (dual prose/machine representation)
- Semantic search index

## Stage 5 — Scalable, Governed Execution

Goal: production-grade, multi-team, multi-repository operation.

Deliverables:
- Distributed worker execution (multi-process/multi-machine)
- Plugin SDK and initial language/framework plugin set
- Policy packs (organization-level Constitution templates)
- Full dashboard (timeline, metrics, approval queue)
- Enterprise governance features (SSO, role-based approval routing, compliance export)

## Cross-Cutting Workstreams (All Stages)

- **Testing strategy** (`23-testing-strategy.md`) evolves alongside each stage — Stage 1 emphasizes unit and integration tests of the Kernel state machine; Stage 5 adds chaos/rollback testing under concurrent multi-worker load.
- **Security hardening** (`24-security-model.md`) is revisited at each stage boundary, since new capabilities (plugins, distributed execution) each introduce new attack surface.
- **Documentation** is maintained as a first-class deliverable per stage, not a post-hoc addition — each stage's PR checklist includes updates to the relevant `docs/` pages.

## Sequencing Rationale

Stages are ordered to de-risk the highest-uncertainty architectural bets first (does the Kernel-mediated, stateless-worker loop actually work at all?) before investing in scale-oriented capabilities (distribution, plugins) whose value depends entirely on the core loop being sound.
