# AI-OS Documentation — 19. API Specification

## Overview

The Kernel exposes a REST API as the primary integration surface for external tools, dashboards, and CI/CD systems. All privileged mutations flow through this API and are subject to the same Permission Engine rules as internal callers.

## Endpoints

### Objectives & Plans

`POST /plans` — submit a business objective to the Goal Decomposer; returns a proposed Execution Plan.

`POST /plans/{plan_id}/admit` — request Kernel admission of a plan into active scheduling.

`GET /objectives/{objective_id}` — retrieve current state, history, and manifest reference for an objective.

`POST /objective` — (legacy/simple path) directly submit a single well-formed objective, bypassing the Goal Decomposer, for advanced/manual use.

### Execution

`POST /worker/start` — (internal/administrative) manually trigger dispatch of a `READY` objective; primarily used in testing and Stage 1 single-worker deployments.

`POST /review` — submit a diff for Review/Guardian evaluation outside the normal Kernel-orchestrated flow (used for local development / pre-flight checks).

`POST /rollback` — trigger a Kernel rollback for a given objective or commit range.

### Approvals

`GET /approvals/pending` — list objectives awaiting a Human Approval Gate decision.

`POST /approvals/{objective_id}/grant` — record human approval.

`POST /approvals/{objective_id}/deny` — record human denial, with required justification text.

### Observability

`GET /timeline` — retrieve the event-derived timeline for a plan or objective.

`GET /interfaces` — query the Interface Registry.

`GET /interfaces/{interface_id}/consumers` — blast-radius lookup.

`GET /metrics` — retrieve aggregate metrics (see `19-api-specification.md` metrics section and `24-testing-strategy.md`).

`GET /audit/{objective_id}` — retrieve the full, hash-chained audit trail for an objective.

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
