# AI-OS Documentation — 26. Deployment Guide

## Deployment Topologies

### Stage 1: Single-Process (Local / Development)
The Kernel, Scheduler, Reviewer, Guardian, and a single worker run as one process against a local Git repository clone. Appropriate for evaluation, development of AI-OS itself, and small personal projects. No external services required beyond the chosen LLM provider API.

### Stage 2–3: Single-Host, Multi-Component
Kernel and supporting services (PIL, Event Bus) run as separate processes on a single host, communicating over local IPC or a lightweight message broker. Multiple worker processes can run concurrently, bounded by host resources. Appropriate for team-scale usage on small-to-medium repositories.

### Stage 4–5: Distributed
Kernel, PIL, Event Bus, and worker pools are deployed as independently scalable services, typically containerized, with workers potentially distributed across multiple hosts or a job-execution platform. Appropriate for large repositories, high objective throughput, or multi-team/multi-repository governance.

## Prerequisites

- A Git repository (or repositories) AI-OS will manage, with appropriate service-account credentials scoped narrowly to the Kernel's Diff Applier (never exposed to workers — see `24-security-model.md`).
- Access credentials for the chosen LLM provider(s) backing Workers and (optionally) the Reviewer.
- A configured Project Constitution and initial Ownership Model for the target repository.

## Configuration Checklist

1. Author the initial `constitution/` documents and generate the corresponding `policies/` machine-checkable rules.
2. Define Ownership Model domains covering 100% of the repository's paths (no unassigned files).
3. Register known interfaces in `interfaces/` (declarative) and allow the PIL to derive the remainder.
4. Configure Scheduler concurrency limits appropriate to the deployment tier and available worker/LLM capacity.
5. Configure Human Approval Gate routing (who approves schema changes, breaking interface changes, dependency additions, deploys) — this is a mandatory step; AI-OS refuses to activate gated categories without at least one configured approver.
6. Configure metrics/event sinks (dashboard, alerting) appropriate to the deployment tier.

## Rollout Recommendation

New adopters are strongly encouraged to begin in a **shadow mode**: AI-OS runs its full pipeline (planning, worker execution, review, guardian evaluation) but the Kernel's Diff Applier is configured to open a pull request rather than merge directly, regardless of gate configuration. This allows a project to validate Guardian rule correctness and worker output quality against real human review before granting AI-OS direct merge authority.

## Upgrade Policy

Kernel upgrades follow the same Interface Registry-governed compatibility rules AI-OS enforces on managed projects: breaking changes to the Kernel API or manifest schema require a major version bump, a deprecation window for the prior version where feasible, and are never silently auto-applied to a running deployment without operator action.
