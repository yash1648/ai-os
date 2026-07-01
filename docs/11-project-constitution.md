# AI-OS Documentation — 11. Project Constitution Specification

## Purpose

The Project Constitution is the set of rules a project agrees are non-negotiable within a given release cycle. It exists so that architectural intent is enforced mechanically rather than relying on every worker (or every human reviewer) independently remembering and applying it correctly.

## Contents

A constitution typically defines:

- **Language and platform versions** (e.g., Java 21, Node 22).
- **API style** (e.g., REST-only, GraphQL forbidden, versioning scheme).
- **Architecture style** (e.g., Hexagonal Architecture, layered architecture, allowed/forbidden dependency directions between layers).
- **Testing requirements** (e.g., minimum coverage thresholds, mandatory test types for certain change categories).
- **Security baselines** (e.g., mandatory input validation patterns, forbidden libraries).
- **Approval rules** (which change categories require human sign-off — see `human_approval` policy in the Kernel).
- **Deployment policy** (e.g., no direct production deploys from worker-generated diffs without a release-manager approval).

## Format

The Constitution exists in two synchronized representations:

1. **Human-readable prose** (Markdown), for context injection into worker manifests and for onboarding contributors.
2. **Machine-checkable rules**, expressed as structured policy statements consumable by the Architecture Guardian (see `17-architecture-guardian.md`), for example:

```yaml
rule_id: hex-arch-001
description: "Domain layer must not import from infrastructure layer"
type: forbidden_dependency
from: "domain/**"
to: "infrastructure/**"
severity: blocking
```

Both representations are generated from a single source definition to prevent drift between what humans believe the rules say and what the Guardian actually enforces.

## Immutability and Amendment

The Constitution is immutable during normal operation — no worker, Reviewer, or Guardian can alter it. Amendments follow a deliberate, human-driven process:

1. A proposed amendment is written as an RFC (see `28-rfc-process.md`).
2. The amendment is reviewed and approved by designated project maintainers/architects.
3. A new Constitution version is published; the version bump is itself an auditable event (`ConstitutionAmended`).
4. In-flight objectives continue under the Constitution version their manifest was built against; new objectives use the new version — preventing rules from silently changing under a worker mid-execution.

## Injection into Manifests

Only the sections of the Constitution relevant to a given objective's domain and risk profile are injected into its Execution Manifest, keeping manifests compact while ensuring the worker always operates within, and is aware of, the applicable rules.

## Relationship to ADRs

Where the Constitution defines standing rules, ADRs (`12-adr-system.md`) document the reasoning behind specific past decisions. A Constitution rule may reference the ADR that motivated it, giving both the "what" (Constitution) and the "why" (ADR) without conflating the two.
