# AI-OS Documentation — 19. API Specification

## Overview

The Kernel exposes a REST API as the primary integration surface for external tools, dashboards, and CI/CD systems. All privileged mutations flow through this API and are subject to the same Permission Engine rules as internal callers.

## Endpoints

Two servers expose REST APIs:

- **Kernel** (port 8081) — core execution engine, objective lifecycle, scheduler
- **PIL sidecar** (port 8082) — intelligence queries, plan decomposition, admission

### Kernel Endpoints

`POST /api/v1/objectives` — create a new objective; returns its UUID.

`GET /api/v1/objectives` — list all objectives.

`GET /api/v1/objectives/{id}` — retrieve objective state and history.

`POST /api/v1/objectives/{id}/ready` — mark an objective as READY for scheduling.

`POST /api/v1/objectives/{id}/transition` — request a state machine transition.

`DELETE /api/v1/objectives/{id}` — abandon (soft-delete) an objective.

`GET /api/v1/scheduler/status` — scheduler stats (active, queued, dispatched).

`GET /api/v1/scheduler/queue` — peek into the dispatch queue.

`POST /api/v1/scheduler/dispatch` — trigger dispatch of the next queued objective.

`POST /api/v1/validate` — validate a state machine transition without applying it.

`GET /api/v1/events` — SSE stream of real-time events.

`GET /api/v1/events/objective/{id}` — event timeline for one objective.

`GET /api/v1/events/recent` — recent events across all objectives.

`GET /api/v1/events/timeline` — query events by time range.

`GET /api/dashboard/timeline` — dashboard timeline view.

`GET /api/dashboard/objectives` — dashboard objectives list.

`GET /api/dashboard/metrics` — dashboard aggregate metrics.

`GET /api/dashboard/audit-log` — hash-chained audit log.

### PIL Endpoints

`GET /api/v1/health` — PIL health check.

`GET /api/v1/adr/search?q=...&status=...` — search ADR records.

`GET /api/v1/constitution/validate?action=...` — validate text against constitution.

`GET /api/v1/symbol/resolve?name=...&kind=...` — resolve symbols by name.

`GET /api/v1/search/semantic?q=...&top_k=...` — semantic code/document search.

`GET /api/v1/indexer/status` — indexer statistics.

`GET /api/v1/dependency/resolve?name=...` — resolve module dependencies.

`GET /api/v1/dependency/graph` — full dependency graph stats and edges.

#### Plan Workflow Endpoints

```
POST /api/v1/plan/decompose  POST /api/v1/plan/admit  POST /api/v1/plan/submit
    │                              │                         │
    ▼                              ▼                         ▼
ExecutionPlan              AdmissionVerdict          Kernel objectives
```

1. **`POST /api/v1/plan/decompose`** — submit a business objective; returns an `ExecutionPlan` (immutable DAG of objectives with criteria and risks).
   - Uses an OpenAI-compatible LLM when `OPENAI_API_KEY` or `OPENROUTER_API_KEY` is set; otherwise falls back to a deterministic mock for local development.

2. **`POST /api/v1/plan/admit`** — validate a plan against six deterministic checks
   (structural, domain, DAG, constitution, criteria, risks). Returns an
   `AdmissionVerdict`; plans with error-severity issues are rejected.

3. **`POST /api/v1/plan/submit`** — submit an admitted plan to the Kernel.
   Creates each objective via the Kernel's CRUD API, preserving the DAG structure,
   owning domains, and risk annotations. Rejects plans that have not passed
   admission.

## Authentication & Authorization

All endpoints require an authenticated caller identity. Mutating endpoints additionally pass through the Permission Engine using the caller's identity as the "human" actor type; a caller without the appropriate role (e.g., approver, maintainer) receives a `403` with a structured denial reason, mirroring internal Permission Engine denials for consistency.

## Versioning

The API is versioned via URL prefix (`/v1/...`). Breaking changes to the API itself follow the same Interface Registry and Constitution-governed compatibility rules AI-OS enforces on the projects it manages — the system is expected to hold itself to its own standards ("dogfooding" the compatibility policy).

## Error Format

```yaml
error:
  code: string
  message: string
  details: object | null
  request_id: string
```
