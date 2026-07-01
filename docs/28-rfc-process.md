# AI-OS Documentation — 28. RFC Process

## Purpose

Significant changes to AI-OS's architecture, Kernel behavior, Constitution, or core schemas go through a lightweight but mandatory RFC (Request for Comments) process, ensuring decisions with wide blast radius are deliberated in the open and their rationale is preserved (typically becoming the basis of one or more ADRs, `12-adr-system.md`).

## What Requires an RFC

- Any change to Kernel invariants (`03-project-kernel.md`)
- Any new primary or failure state in the State Machine
- Any change to the core schemas (`20-json-schemas.md`) that isn't purely additive
- Any change to the AI-OS project's own Constitution
- Any new category of mandatory Human Approval Gate (addition or removal)
- Introduction of a new top-level component or major subsystem

## What Does Not Require an RFC

- Bug fixes that restore documented behavior
- New plugins conforming to the existing Plugin SDK
- Additive, backward-compatible schema fields
- Documentation-only changes

## RFC Lifecycle

1. **Draft** — author writes the RFC using the standard template (problem statement, proposed design, alternatives considered, impact on existing invariants/schemas, migration plan).
2. **Discussion** — open comment period; substantive objections must be addressed or explicitly acknowledged and overruled with rationale before proceeding.
3. **Decision** — maintainers accept, reject, or request revision. An accepted RFC results in one or more ADRs capturing the final decision and rationale.
4. **Implementation** — tracked against the accepted RFC; significant deviation during implementation requires a follow-up RFC amendment, not silent scope change.
5. **Archival** — all RFCs (accepted or rejected) are retained permanently; rejected RFCs remain valuable to avoid re-litigating settled questions without new information.

## Template

```markdown
# RFC: <title>

## Problem Statement
## Proposed Design
## Alternatives Considered
## Impact on Existing Invariants / Schemas
## Migration Plan
## Open Questions
```

## Relationship to ADRs

An RFC captures the deliberation; the resulting ADR captures the settled decision in the standardized, retrievable form the Project Intelligence Layer indexes for worker/manifest consumption. Not every ADR originates from an RFC (smaller decisions may be recorded directly), but every RFC that results in an accepted change produces at least one ADR.
