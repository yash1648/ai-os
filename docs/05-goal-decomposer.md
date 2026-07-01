# AI-OS Documentation — 05. Goal Decomposer Specification

## Purpose

The Goal Decomposer is the single entry point through which a human business objective becomes a structured, schedulable body of work. It is the only LLM-driven component permitted to produce objectives; everything downstream operates on the structured plan it emits, never on raw natural language.

## Inputs

- A free-form business objective (e.g., "Add multi-tenant support to the billing service").
- Read access to the Project Intelligence Layer (repository index, dependency graph, ownership map, ADRs, constitution).
- Optional constraints supplied by the requester (deadline, priority, excluded domains).

## Outputs

An **Execution Plan**: an immutable, versioned document containing:

- A directed acyclic graph of **objectives** and **sub-objectives**.
- For each objective: title, owning domain, priority, dependencies, success criteria, and identified risks.
- A plan-level rationale explaining the decomposition strategy.
- A plan ID and content hash, used for downstream reference and change detection.

## Decomposition Process

1. **Clarification pass** — the Decomposer checks the objective against the Constitution and existing ADRs for conflicts or ambiguity; if the objective is materially ambiguous (e.g., contradicts a constitutional rule), it returns a clarification request rather than guessing.
2. **Scoping pass** — using the dependency graph and ownership map, the Decomposer identifies which domains are affected and drafts a candidate objective breakdown.
3. **Dependency resolution** — objectives are ordered into a DAG; objectives with no unmet dependencies are marked eligible for immediate scheduling.
4. **Success criteria authoring** — each objective is given concrete, checkable success criteria (e.g., "all existing tests pass," "new endpoint documented in OpenAPI spec," "coverage ≥ 90% on new code"), avoiding vague criteria like "implemented correctly."
5. **Risk annotation** — objectives touching schema, public interfaces, or security-sensitive code are flagged, which downstream causes the Kernel to enforce a human approval gate at the appropriate stage.
6. **Plan freezing** — once emitted, the plan is immutable. Any change in scope requires a new decomposition cycle producing a new plan version, never an in-place mutation of an active plan.

## Immutability Rationale

Allowing execution plans to be silently edited mid-flight is one of the most common sources of coordination failure in multi-agent systems — objectives get reprioritized while workers are mid-execution against the old plan, dependencies become stale, and audit trails become incoherent. AI-OS instead treats a plan revision as a new plan, explicitly superseding the old one, with the Kernel responsible for reconciling in-flight objectives (completing them under the old plan, or cancelling and re-issuing them under the new one, per policy).

## Interaction with the Kernel

The Goal Decomposer does not submit objectives directly for execution. It hands the completed plan to the Kernel's admission control, which independently re-validates the plan (constitutional compliance, ownership resolution, cycle detection in the dependency graph) before accepting it. The Decomposer's output is a proposal; the Kernel's admission is the authority.

## Failure Modes

- **Planning Failure** — the Decomposer cannot produce a valid plan (e.g., irreducible ambiguity, conflicting constraints). The objective is returned to the requester with a structured explanation.
- **Admission Rejection** — the Kernel rejects an otherwise well-formed plan due to a constitutional conflict or unresolvable ownership ambiguity, requiring re-decomposition.
