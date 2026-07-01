# AI-OS Documentation — 16. Review Pipeline Specification

## Purpose

No diff reaches Git without passing through two independent evaluation stages: the Reviewer (quality) and the Architecture Guardian (structural/constitutional compliance). Separating these concerns prevents a single evaluator from having to simultaneously optimize for "is this good code" and "is this allowed here" — two questions with different failure costs and different appropriate strictness levels.

## Pipeline Flow

```
Worker → Diff + Report
   ↓
Kernel structural validation (schema, allowed_files, applies cleanly)
   ↓
Reviewer  → ReviewPassed / ReviewFailed
   ↓ (only if passed)
Architecture Guardian → GuardianPassed / GuardianFailed
   ↓ (only if passed)
Human Approval Gate (if triggered by policy)
   ↓
Kernel Diff Applier → Git commit
```

A failure at any stage halts progression; the objective transitions to the corresponding failure state (`07-state-machine.md`) rather than proceeding with a partial pass.

## Reviewer Responsibilities

- **Correctness** — does the diff plausibly achieve the stated objective and success criteria? Are there obvious logic errors?
- **Style** — does the change conform to project style/lint configuration?
- **Testing** — are new/changed behaviors covered by tests consistent with the Constitution's testing requirements?
- **Performance** — does the change introduce obviously problematic performance characteristics (e.g., N+1 queries, unbounded loops over large collections)?
- **Maintainability** — is the change reasonably clear, appropriately documented, and consistent with surrounding code idioms?

The Reviewer may itself be LLM-backed, but — like Workers — operates statelessly per review request and produces a structured verdict, not free-form commentary alone.

## Reviewer Output

```yaml
verdict: enum(pass, fail)
findings:
  - category: enum(correctness, style, testing, performance, maintainability)
    severity: enum(blocking, warning, info)
    description: string
    location: string
confidence: enum(low, medium, high)
```

Only `blocking` severity findings cause a `fail` verdict; `warning`/`info` findings are recorded but do not block progression (they are surfaced in the execution report and dashboard for human visibility).

## Guardian Independence

The Architecture Guardian re-evaluates the diff independently of the Reviewer's verdict — it does not trust "this looks architecturally fine" commentary from the Reviewer. See `17-architecture-guardian.md` for its specific checks.

## Retry Handling

On `REVIEW_FAILED` or `INTEGRATION_FAILURE` (Guardian rejection), the Kernel may schedule a retry: a new stateless worker instance receives an augmented manifest including the structured findings from the failed review, so it can address them directly, without inheriting any other prior context.

## Escalation

If an objective exhausts its configured retry budget without passing Review and Guardian, it transitions to `ABANDONED` and is surfaced to a human for manual intervention — AI-OS does not loop indefinitely on a stuck objective.
