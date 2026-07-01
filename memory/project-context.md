# AI-OS — Sisyphus Memory

## Project Identity
- **Name**: AI-OS
- **Directory**: `/home/grim/Projects/ai-os-docs`
- **Purpose**: Deterministic operating-system-inspired runtime for AI-assisted software engineering
- **Core Thesis**: "LLMs think. The Kernel decides."

## Architecture (5 Layers)
1. **Intent Layer** — User + Goal Decomposer (LLM-driven, produces structured plans)
2. **Governance Layer** — Constitution + ADRs + Ownership Model (machine-checkable rules)
3. **Kernel Layer** — Scheduler, State Machine, Permission Engine, Diff Applier (deterministic CORE)
4. **Execution Layer** — Workers, Reviewer, Guardian (LLM-backed but stateless)
5. **Persistence Layer** — Git, Event Log, Project Intelligence Layer

## Lifecycle
```
Goal → Plan (Goal Decomposer) → Manifest (Kernel) → Worker → Diff → Reviewer + Guardian → Apply (Kernel) → Done
```

## Tech Stack (Polyglot — Best Tool Per Subsystem)
| Layer | Technology | Reason |
|---|---|---|
| **Kernel** | Rust (tokio, axum, serde, sqlx, git2, clap, thiserror, tracing, dashmap, notify, parking_lot) | Memory safe, fast, perfect for deterministic state machine, great async, excellent CLI tooling |
| **Goal Decomposer** | Python (Pydantic, Instructor, OpenAI SDK, LiteLLM, Jinja2, NetworkX, orjson) | Best LLM tooling ecosystem |
| **Intelligence Layer** | Python (Tree-sitter, NetworkX, FAISS, Sentence Transformers, ripgrep, LlamaIndex optional) | Code indexing, semantic search, dependency graphs |
| **Workers** | Python (initially) | Easy LLM integration, stateless per objective |
| **Dashboard** | Next.js + TypeScript (TailwindCSS, shadcn/ui, TanStack Query, Zustand, React Flow, Monaco, Socket.io) | Observability, approval gates, timeline |
| **Database** | SQLite (MVP) → PostgreSQL (prod) | Zero-config → production scale |
| **Vector Search** | FAISS (MVP) → pgvector (prod) | Embedding similarity search |
| **Parsing** | Tree-sitter (never AI for parsing) | Safe, deterministic, multi-language AST |
| **Event Bus** | Rust tokio channels → Redis Streams → NATS | In-process → distributed |
| **Communication** | Traits (Stage 1) → gRPC (Stage 3+); REST for external; WebSocket for real-time | Contract-first, evolve later |
| **Plugin System** | Rust traits | `trait LanguagePlugin` with parse, extract_dependencies, check_dependency_rule |
| **Observability** | tracing (MVP) → Prometheus + Grafana (prod) | Structured logs first, metrics later |
| **Config** | TOML | Standard, readable |
| **CLI** | clap (Rust) | Natural for Rust project |

## Repository Layout
```
ai-os/
├── kernel/          # Rust — Scheduler, State Machine, Permission Engine, Diff Applier, Rollback
├── planner/         # Python — Goal Decomposer
├── intelligence/    # Python — Code indexing, PIL
├── workers/         # Python — Worker runtime + configs
├── dashboard/       # Next.js — Timeline, approvals, metrics
├── sdk/             # Client SDKs
├── plugins/         # Language plugins (java/, python/, rust/, typescript/)
├── schemas/         # Canonical JSON Schemas
├── constitution/    # Human-readable Constitution documents
├── adr/             # Architecture Decision Records
├── memory/          # AI memory (this file)
├── examples/        # Worked examples
├── tests/           # Integration + conformance tests
├── docs/            # The existing documentation set
└── docker/          # Docker Compose for integration tests
```

## Build Order (Stage 1 — Single-Process Kernel)

### Phase 1 — Foundation
1. ✅ Repository scaffold (kernel/, schemas/, events/workers/, reviewer/)
2. JSON Schemas (objective.json, manifest.json, worker-output.json, event.json, verdict.json)
3. **State Machine Engine** — pure logic, no I/O, 14 states, forward + failure transitions
4. **In-process Event Bus** — tokio::sync::broadcast channels

### Phase 2 — Core Loop
5. Objective model + SQLite storage
6. Execution Manifest construction (simplified — PIL stubbed)
7. Stateless Worker (CLI-based, manifest in → diff + report out)
8. Reviewer (basic: checks diff scope, files_changed match)

### Phase 3 — Apply & Recover
9. Diff Applier (git2 commit with structured message)
10. Rollback Manager (git2 snapshot/restore)

### Phase 4 — Integration
11. End-to-end loop wiring
12. Basic metrics collection (in-memory ring buffer + /metrics endpoint)

## Key Architectural Invariants
- Kernel is **deterministic** — never calls an LLM directly
- Workers are **stateless** — fresh process per objective, no cross-objective memory
- All writes are **mediated by the Kernel** — workers produce diffs only
- Every state transition emits an **auditable event**
- **Rollback is always possible** until `DONE`
- **All interfaces are traits** — Kernel depends on abstractions, not concretions
