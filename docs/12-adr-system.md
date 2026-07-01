# AI-OS Documentation — 12. ADR System Specification

## Purpose

Architecture Decision Records preserve the *reasoning* behind significant design choices, so future workers (and humans) don't have to rediscover or accidentally reverse decisions whose rationale is no longer visible in the code itself.

## ADR Schema

```yaml
adr_id: string
title: string
status: enum(proposed, accepted, superseded, deprecated)
date: date
context: string          # the situation motivating the decision
decision: string         # what was decided
alternatives_considered:
  - option: string
    rejected_because: string
rationale: string
consequences:
  positive: [string]
  negative: [string]
  neutral: [string]
supersedes: string | null
superseded_by: string | null
tags: [string]
affected_domains: [string]
```

## Lifecycle

1. **Proposed** — an ADR is drafted, typically alongside an RFC for significant changes.
2. **Accepted** — the ADR is approved by relevant maintainers and becomes part of the searchable ADR index.
3. **Superseded** — a later ADR replaces this decision; both records remain, linked via `supersedes` / `superseded_by`, preserving full decision history rather than deleting it.
4. **Deprecated** — the decision no longer applies (e.g., the affected subsystem was removed) but the record is retained for historical context.

## Retrieval

ADRs are retrieved selectively, not injected wholesale into every manifest. The Project Intelligence Layer's ADR Index supports:

- Tag-based lookup (e.g., all ADRs tagged `authentication`).
- Domain-based lookup (all ADRs affecting a given ownership domain).
- Semantic search (natural-language query against ADR content).

Only ADRs relevant to a given objective's scope are included in its Execution Manifest, keeping context bounded while ensuring workers don't unknowingly contradict a documented, still-valid decision.

## Relationship to the Guardian

Where an ADR implies a checkable rule (e.g., "we decided to always use UUIDv7 for primary keys"), that rule should also be encoded as a Constitution rule or Guardian policy so it is mechanically enforced, not merely documented. ADRs that never become enforceable rules remain valuable as historical record but rely on Reviewer/human judgment for compliance.

## Authorship

ADRs may be authored by humans (architects, maintainers) or drafted by a worker as part of an objective's execution report when the objective involved a non-trivial design decision — in the latter case, the draft ADR is treated as a proposed record requiring human acceptance before entering the authoritative index.
