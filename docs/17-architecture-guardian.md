# AI-OS Documentation — 17. Architecture Guardian Specification

## Purpose

The Architecture Guardian is the mechanical enforcer of the Project Constitution and Ownership Model. Where the Reviewer asks "is this good code," the Guardian asks "is this change *allowed here*, regardless of how good it is." A perfectly written diff that violates a domain boundary or introduces a forbidden dependency is rejected by the Guardian even if the Reviewer would have approved it.

## Checks Performed

1. **Constitutional rule compliance** — evaluates the diff against every machine-checkable rule in the current Constitution (e.g., forbidden dependency directions, required architecture style conformance, forbidden libraries).
2. **Domain boundary compliance** — verifies every touched file falls within the objective's granted `allowed_files`, re-deriving this independently from the Ownership Model rather than trusting the manifest at face value (defense against manifest construction errors or drift).
3. **Forbidden dependency introduction** — re-derives the project's dependency graph including the proposed diff and checks for newly introduced edges that violate declared layering rules (e.g., domain layer importing infrastructure layer).
4. **Interface compatibility** — for any touched registered interface, determines whether the change is backward-compatible; if not, checks the interface's `compatibility.breaking_change_policy` and either rejects, routes to human approval, or requires an accompanying deprecation path.
5. **Constitutional exception detection** — flags any change that would require a formally logged exception to the Constitution (e.g., a temporary allowance below the normal coverage threshold), routing these to a Human Approval Gate rather than silently permitting them.

## Output

```yaml
verdict: enum(pass, fail, requires_human_approval)
violations:
  - rule_id: string
    description: string
    severity: enum(blocking, requires_approval)
    evidence: string
```

## Independence from the Reviewer

The Guardian does not consume the Reviewer's findings as input to its own verdict — it operates on the diff and the current (re-queried) Project Intelligence Layer state directly. This independence is deliberate: a shared blind spot between Reviewer and Guardian would defeat the purpose of having two separate gates.

## Relationship to Human Approval

The Guardian is the component that determines *whether* a Human Approval Gate should be triggered based on Constitution-defined categories (schema migrations, breaking interface changes, new dependencies, constitutional exceptions, production deploys). The Guardian itself never grants approval — it only routes. Only an authenticated human decision, recorded via `HumanApprovalGranted` / `HumanApprovalDenied`, can clear a gate.

## Extensibility

New constitutional rules and dependency policies can be added without modifying Guardian code, provided they can be expressed in the structured policy format described in `11-project-constitution.md`. Rules requiring genuinely novel static-analysis capability (e.g., a new kind of taint analysis) require a Guardian plugin — see `18-plugin-sdk.md`.
