# AI-OS Documentation

> LLMs think. The Kernel decides.

This directory is the official design documentation for AI-OS, organized the way large infrastructure projects (Kubernetes, LLVM, Linux) structure theirs: one focused document per subsystem rather than a single monolithic file.

## Index

| # | Document | Covers |
|---|---|---|
| 00 | [Vision](00-vision.md) | Purpose, thesis, guiding values |
| 01 | [Philosophy & Terminology](01-philosophy-and-terminology.md) | OS metaphor, glossary |
| 02 | [Architecture Overview](02-architecture-overview.md) | Layers, request lifecycle, concurrency, failure philosophy |
| 03 | [Project Kernel](03-project-kernel.md) | Kernel responsibilities, subsystems, invariants |
| 04 | [Project Intelligence Layer](04-project-intelligence-layer.md) | Indexes, graphs, query model |
| 05 | [Goal Decomposer](05-goal-decomposer.md) | Plan generation, immutability |
| 06 | [Worker Runtime](06-worker-runtime.md) | Statelessness, sandboxing, output contract |
| 07 | [State Machine](07-state-machine.md) | Primary and failure states |
| 08 | [Event Bus](08-event-bus.md) | Event types, schema, consumers |
| 09 | [Execution Manifest](09-execution-manifest.md) | Manifest schema, construction |
| 10 | [Interface Registry](10-interface-registry.md) | Contracts, blast-radius analysis |
| 11 | [Project Constitution](11-project-constitution.md) | Rules, amendment process |
| 12 | [ADR System](12-adr-system.md) | Decision records, lifecycle |
| 13 | [Ownership Model](13-ownership-model.md) | Domains, cross-domain requests |
| 14 | [Permission Engine](14-permission-engine.md) | Deny-by-default evaluation |
| 15 | [Scheduler](15-scheduler.md) | Dispatch, locking, fairness |
| 16 | [Review Pipeline](16-review-pipeline.md) | Reviewer responsibilities and flow |
| 17 | [Architecture Guardian](17-architecture-guardian.md) | Constitutional/structural enforcement |
| 18 | [Plugin SDK](18-plugin-sdk.md) | Language, framework, Guardian rule plugins |
| 19 | [API Specification](19-api-specification.md) | REST endpoints |
| 20 | [JSON Schemas](20-json-schemas.md) | Canonical object schemas |
| 21 | [Repository Layout](21-repository-layout.md) | Directory structure and conventions |
| 22 | [Development Roadmap](22-development-roadmap.md) | Stages 1–5 |
| 23 | [Testing Strategy](23-testing-strategy.md) | Test categories, coverage |
| 24 | [Security Model](24-security-model.md) | Threat model, guarantees |
| 25 | [Performance Benchmarks](25-performance-benchmarks.md) | Benchmark categories, methodology |
| 26 | [Deployment Guide](26-deployment-guide.md) | Topologies, rollout |
| 27 | [Contributor Guide](27-contributor-guide.md) | Getting started, PR checklist |
| 28 | [RFC Process](28-rfc-process.md) | Governance for architectural change |
| 29 | [Future Research](29-future-research.md) | Open questions |

## Source Documents

This documentation set expands and supersedes the earlier single-file blueprints:
- `AI-OS_Project_Specification_v0.1.md`
- `AI-OS_Complete_Project_Blueprint_v1.0.md`

Those files remain useful as high-level summaries; this `docs/` set is the authoritative, per-subsystem reference going forward.
