# AI-OS Documentation — 21. Repository Layout

## Top-Level Structure

```
ai-os/
├── kernel/           # Scheduler, Permission Engine, State Machine, Diff Applier, Rollback Manager
├── planner/          # Goal Decomposer
├── intelligence/     # Project Intelligence Layer: indexes, graphs, search
├── workers/          # Worker runtime + domain-specialized worker configs
├── reviewer/         # Reviewer pipeline stage
├── guardian/         # Architecture Guardian pipeline stage
├── events/           # Event Bus implementation, schemas, subscribers
├── policies/         # Machine-checkable Constitution rules, approval-gate policies
├── plugins/          # Language and framework plugins
├── interfaces/       # Declared interface contracts (Interface Registry source)
├── constitution/     # Human-readable Constitution documents
├── adr/              # Architecture Decision Records
├── schemas/          # Canonical JSON Schemas for all core objects
├── sdk/              # Plugin SDK crate (Plugin trait, governance stubs, example plugin)
├── dashboard/        # Timeline/observability UI (htmx SPA)
├── docker/           # Dockerfiles and Compose configuration for containerized deployment
├── memory/           # Cross-session memory and knowledge persistence
├── docs/             # This documentation set
├── tests/            # Python integration and conformance test suite
└── examples/         # Worked examples, quick-start, cross-domain scenarios
```

## Directory Conventions

- Every top-level directory that produces or consumes a schema-defined object includes a `README.md` linking back to the relevant `docs/` specification page, so the docs and code never drift apart silently.
- `policies/` and `constitution/` are kept separate deliberately: `constitution/` is the human-authored source of truth; `policies/` contains the generated, machine-checkable representation (see `11-project-constitution.md`), and is treated as a build artifact, not hand-edited directly.
- `plugins/` follows a per-plugin subdirectory convention: `plugins/<kind>/<name>/`, e.g. `plugins/language/typescript/`, `plugins/framework/spring-boot/`.
- `adr/` files are named `NNNN-short-title.md` in strict sequential order, never renumbered, to preserve stable references from code comments and other ADRs.

## Ownership Domain Alignment

The Ownership Model (`13-ownership-model.md`) is typically configured so that each top-level `ai-os/` directory (or a defined subset of directories within `workers/` and `plugins/`) maps to its own domain — meaning AI-OS, as a project, governs its own development using the same mechanisms it provides to projects it manages. This self-hosting relationship is treated as a first-class testing strategy: if AI-OS cannot safely manage changes to its own Kernel using its own Guardian and Ownership rules, that is treated as a critical defect.
