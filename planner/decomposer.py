"""
Goal Decomposer — the single entry point through which a human business
objective becomes a structured, schedulable ``ExecutionPlan``.

The decomposer runs six sequential passes, each building on the previous:

1. **Clarify** — check for constitution/ADR conflicts or ambiguity.
2. **Scope** — identify affected domains and draft objective breakdown.
3. **Resolve dependencies** — order objectives into a DAG.
4. **Author criteria** — attach concrete, verifiable success criteria.
5. **Annotate risks** — flag schema, interface, or security-sensitive objectives.
6. **Freeze** — emit the immutable plan with a content hash.

The decomposer NEVER submits plans directly to the Kernel. Its output is a
*proposal*; the Kernel's admission control independently validates it.
"""

from __future__ import annotations

import json
from typing import Any

from planner.llm import BaseLlmClient, MockLlmClient
from planner.models import (
    ExecutionPlan,
    Objective,
    Priority,
    RiskAnnotation,
    RiskLevel,
    SuccessCriteria,
)


# ── Default system prompts (each pass gets its own) ────────────────────────────

SYSTEM_CLARIFY = """\
You are a planning analyst reviewing a business objective. Your job is to:
1. Identify any conflicts with existing ADRs or constitutional rules.
2. Flag ambiguous or underspecified language.
3. Determine if the objective needs clarification before decomposition can proceed.

Respond in JSON with fields:
  "needs_clarification": bool,
  "clarification_requests": [str],
  "conflicts_found": [str],
  "is_clear": bool
"""

SYSTEM_SCOPE = """\
You are a technical architect decomposing a business objective into concrete,
atomic work items. Given the objective and its context:

1. Identify which domains are affected (e.g. kernel, planner, worker, dashboard).
2. Draft a candidate breakdown of 2-6 objectives.
3. Each objective must be atomic (one unit of work), assigned to one domain.

Respond in JSON with fields:
  "rationale": str,
  "objectives": [
    {
      "title": str,
      "owning_domain": str,
      "description": str,
      "dependencies": [str]  // objective titles this depends on
    }
  ]
"""

SYSTEM_CRITERIA = """\
You are a QA engineer defining success criteria for development objectives.
Each criterion must be concrete, verifiable, and unambiguous.

For each objective, produce 2-4 success criteria covering:
- Correctness (does it work?)
- Testing (is it tested?)
- Documentation (is it documented?)
- Quality (style, linting, coverage where applicable)

Avoid vague criteria like "implemented correctly" or "works as expected".

Respond in JSON with fields:
  "objectives": [
    {
      "title": str,
      "criteria": [
        {"description": str, "verification_hint": str | null}
      ]
    }
  ]
"""

SYSTEM_RISKS = """\
You are a security and architecture reviewer identifying risks in an execution plan.
Flag objectives that touch:

- schema_change: Database or API schema modifications
- public_interface: Public API or SDK changes
- security_sensitive: Authentication, authorization, encryption
- external_dependency: Third-party service or library integration
- performance: High-throughput or latency-sensitive paths
- ambiguity: Requirements that remain underspecified

Assign a level: critical, high, medium, low, or informational.

Respond in JSON with fields:
  "plan_level_risks": [{"category": str, "description": str, "level": str}],
  "objectives": [
    {
      "title": str,
      "risks": [{"category": str, "description": str, "level": str, "affected_objective_ids": [str]}]
    }
  ]
"""


# ── GoalDecomposer ─────────────────────────────────────────────────────────────


class GoalDecomposer:
    """Six-pass decomposition engine for business objectives.

    Usage::

        decomposer = GoalDecomposer(llm_client=my_client)
        plan = decomposer.decompose(
            objective="Add multi-tenant support to the billing service",
            context={"adr_dir": "docs/adr", "domains": ["kernel", "planner"]},
        )
    """

    def __init__(self, llm_client: BaseLlmClient | None = None) -> None:
        self._llm = llm_client or MockLlmClient()

    # ── Public API ──────────────────────────────────────────────────────────

    def decompose(
        self,
        objective: str,
        *,
        context: dict[str, Any] | None = None,
    ) -> ExecutionPlan:
        """Run the full 6-pass decomposition pipeline.

        Args:
            objective: Free-form business objective (e.g. "Add user authentication").
            context: Optional dict carrying ADR search results, ownership data,
                or other PIL-gathered context.

        Returns:
            A frozen ``ExecutionPlan`` — the Decomposer's proposal.
            The Kernel independently validates this before admission.

        Raises:
            ValueError: If the objective fails the clarification pass (irreducible ambiguity).
        """
        ctx = context or {}
        clarification = self._clarify(objective, ctx)

        if clarification.get("needs_clarification", False):
            msg = "; ".join(clarification.get("clarification_requests", ["Objective is ambiguous"]))
            raise ValueError(
                f"Objective requires clarification before decomposition: {msg}"
            )

        scope_result = self._scope(objective, ctx, clarification)

        # Build a working plan from the scope pass, then enhance it.
        working = self._build_plan(objective, scope_result)

        criteria_result = self._author_criteria(working, ctx)
        working = self._attach_criteria(working, criteria_result)

        risks_result = self._annotate_risks(working, ctx)
        working = self._attach_risks(working, risks_result)

        # Resolve dependencies — topological ordering is handled by the
        # Kernel on admission, but we ensure the DAG is well-formed here.
        self._validate_dag(working)

        plan = working.freeze()
        return plan

    # ── Six passes ─────────────────────────────────────────────────────────

    def _clarify(self, objective: str, context: dict[str, Any]) -> dict[str, Any]:
        """Pass 1 — check for constitution/ADR conflicts and ambiguity."""
        context_str = self._format_context(context)
        resp = self._llm.complete(
            system_prompt=SYSTEM_CLARIFY,
            user_prompt=(
                f"Business objective: {objective}\n\n"
                f"Relevant context:\n{context_str}\n\n"
                "Analyse the objective and return your assessment as JSON."
            ),
            request_json=True,
            temperature=0.2,
        )
        return resp.parsed if isinstance(resp.parsed, dict) else {}

    def _scope(
        self, objective: str, context: dict[str, Any], clarification: dict[str, Any]
    ) -> dict[str, Any]:
        """Pass 2 — identify affected domains and draft objective breakdown."""
        context_str = self._format_context(context)
        resp = self._llm.complete(
            system_prompt=SYSTEM_SCOPE,
            user_prompt=(
                f"Business objective: {objective}\n\n"
                f"Clarification findings: {json.dumps(clarification, indent=2)}\n\n"
                f"Context:\n{context_str}\n\n"
                "Decompose this objective into concrete work items as JSON."
            ),
            request_json=True,
            temperature=0.3,
        )
        return resp.parsed if isinstance(resp.parsed, dict) else {"objectives": [], "rationale": ""}

    def _author_criteria(
        self, plan: ExecutionPlan, context: dict[str, Any]
    ) -> dict[str, Any]:
        """Pass 4 — attach concrete, verifiable success criteria to each objective."""
        objectives_text = "\n".join(
            f"- {o.title} (domain: {o.owning_domain}): {o.description}"
            for o in plan.objectives
        )
        resp = self._llm.complete(
            system_prompt=SYSTEM_CRITERIA,
            user_prompt=(
                f"Objectives:\n{objectives_text}\n\n"
                "For each objective, provide concrete success criteria as JSON."
            ),
            request_json=True,
            temperature=0.3,
        )
        return resp.parsed if isinstance(resp.parsed, dict) else {}

    def _annotate_risks(
        self, plan: ExecutionPlan, context: dict[str, Any]
    ) -> dict[str, Any]:
        """Pass 5 — flag schema, interface, or security-sensitive objectives."""
        objectives_text = "\n".join(
            f"- {o.title} (domain: {o.owning_domain}): {o.description}"
            for o in plan.objectives
        )
        resp = self._llm.complete(
            system_prompt=SYSTEM_RISKS,
            user_prompt=(
                f"Objectives:\n{objectives_text}\n\n"
                "Identify risks for each objective and the plan overall as JSON."
            ),
            request_json=True,
            temperature=0.3,
        )
        return resp.parsed if isinstance(resp.parsed, dict) else {}

    # ── Internal helpers ───────────────────────────────────────────────────

    def _build_plan(self, objective: str, scope: dict[str, Any]) -> ExecutionPlan:
        """Construct an ExecutionPlan from the scope pass output."""
        rationale = scope.get("rationale", "")

        raw_objectives: list[dict] = scope.get("objectives", [])
        objectives: list[Objective] = []
        # First pass — create all objectives
        title_to_id: dict[str, str] = {}
        for raw in raw_objectives:
            obj = Objective(
                title=raw.get("title", "Untitled objective"),
                owning_domain=raw.get("owning_domain", "unknown"),
                description=raw.get("description", ""),
            )
            objectives.append(obj)
            title_to_id[obj.title] = obj.id

        # Second pass — resolve dependency titles to IDs
        for raw, obj in zip(raw_objectives, objectives):
            dep_titles: list[str] = raw.get("dependencies", [])
            for dep_title in dep_titles:
                dep_id = title_to_id.get(dep_title)
                if dep_id and dep_id != obj.id:
                    obj.dependencies.append(dep_id)

        return ExecutionPlan(
            objective_description=objective,
            rationale=rationale,
            objectives=objectives,
        )

    def _attach_criteria(self, plan: ExecutionPlan, criteria: dict[str, Any]) -> ExecutionPlan:
        """Merge success criteria from the LLM response into the plan objectives."""
        raw_objectives: list[dict] = criteria.get("objectives", [])
        title_to_obj = {o.title: o for o in plan.objectives}

        for raw in raw_objectives:
            title = raw.get("title", "")
            obj = title_to_obj.get(title)
            if obj is None:
                continue
            raw_criteria: list[dict] = raw.get("criteria", [])
            for rc in raw_criteria:
                obj.success_criteria.append(
                    SuccessCriteria(
                        description=rc.get("description", ""),
                        verification_hint=rc.get("verification_hint"),
                    )
                )
        return plan

    def _attach_risks(self, plan: ExecutionPlan, risks: dict[str, Any]) -> ExecutionPlan:
        """Merge risk annotations from the LLM response into the plan and objectives."""
        # Plan-level risks
        for raw in risks.get("plan_level_risks", []):
            plan.plan_level_risks.append(
                RiskAnnotation(
                    category=raw.get("category", "other"),
                    description=raw.get("description", ""),
                    level=RiskLevel(raw.get("level", "informational")),
                )
            )

        # Per-objective risks
        title_to_obj = {o.title: o for o in plan.objectives}
        for raw in risks.get("objectives", []):
            title = raw.get("title", "")
            obj = title_to_obj.get(title)
            if obj is None:
                continue
            for rr in raw.get("risks", []):
                obj.risks.append(
                    RiskAnnotation(
                        category=rr.get("category", "other"),
                        description=rr.get("description", ""),
                        level=RiskLevel(rr.get("level", "informational")),
                        affected_objective_ids=rr.get("affected_objective_ids", []),
                    )
                )
        return plan

    def _validate_dag(self, plan: ExecutionPlan) -> None:
        """Validate that the objective dependency graph is a DAG.

        Raises:
            ValueError: If a cycle is detected.
        """
        obj_by_id = {o.id: o for o in plan.objectives}
        visited: set[str] = set()
        in_stack: set[str] = set()

        def _has_cycle(obj_id: str, path: list[str]) -> bool:
            visited.add(obj_id)
            in_stack.add(obj_id)
            obj = obj_by_id.get(obj_id)
            if obj:
                for dep_id in obj.dependencies:
                    if dep_id not in visited:
                        if _has_cycle(dep_id, path + [dep_id]):
                            return True
                    elif dep_id in in_stack:
                        cycle_path = " -> ".join(path + [dep_id])
                        raise ValueError(
                            f"Cycle detected in objective dependencies: {cycle_path}"
                        )
            in_stack.discard(obj_id)
            return False

        for obj in plan.objectives:
            if obj.id not in visited:
                _has_cycle(obj.id, [obj.id])

    def _format_context(self, context: dict[str, Any]) -> str:
        """Format the optional context dict for inclusion in LLM prompts."""
        if not context:
            return "(none provided)"
        return json.dumps(context, indent=2, default=str)
