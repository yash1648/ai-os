"""Plan admission — validates an execution plan before Kernel submission.

The :class:`PlanAdmissionService` runs a suite of deterministic checks against
a frozen ``ExecutionPlan`` and returns an ``AdmissionVerdict``.  Checks are
grouped into six categories:

1. **Structural** — every objective has a non-empty title, description, domain.
2. **Domains** — each ``owning_domain`` is recognised.
3. **DAG** — the dependency graph is acyclic.
4. **Constitution** — surfacing relevant constitutional rules per objective.
5. **Criteria** — every objective has at least one success criterion.
6. **Risks** — high-severity risk categories produce warnings when unmitigated.
"""

from __future__ import annotations

from datetime import datetime, timezone
from typing import Any

from planner.models import (
    AdmissionIssue,
    AdmissionVerdict,
    ExecutionPlan,
    RiskLevel,
    VerdictSeverity,
)

# ── Helpers ─────────────────────────────────────────────────────────────────────

_DEFAULT_KNOWN_DOMAINS: list[str] = [
    "kernel",
    "planner",
    "worker",
    "dashboard",
]

_RISK_CATEGORIES_REQUIRING_ATTENTION: set[str] = {
    "schema_change",
    "public_interface",
    "security_sensitive",
    "external_dependency",
}


# ── PlanAdmissionService ─────────────────────────────────────────────────────────


class PlanAdmissionService:
    """Deterministic plan admission validator.

    Usage::

        service = PlanAdmissionService()
        verdict = service.admit(plan, known_domains=["kernel", "planner"])
        if not verdict.passed:
            for issue in verdict.issues:
                print(f"[{issue.severity}] {issue.message}")
    """

    def __init__(self) -> None:
        self._issues: list[AdmissionIssue] = []

    # ── Public API ───────────────────────────────────────────────────────────

    def admit(
        self,
        plan: ExecutionPlan,
        *,
        known_domains: list[str] | None = None,
        constitution_engine: Any = None,
    ) -> AdmissionVerdict:
        """Run all admission checks against *plan*.

        Args:
            plan: A frozen execution plan to validate.
            known_domains: List of valid domain identifiers.  Defaults to
                ``["kernel", "planner", "worker", "dashboard"]``.
            constitution_engine: Optional ``ConstitutionEngine`` instance for
                constitution-alignment checks.  Skipped when ``None``.

        Returns:
            An ``AdmissionVerdict`` with ``passed=True`` iff no error-severity
            issues were found.
        """
        self._clear()
        domains = known_domains or _DEFAULT_KNOWN_DOMAINS

        self._check_structural(plan)
        self._check_domains(plan, domains)
        self._check_dag(plan)
        self._check_criteria(plan)
        self._check_risks(plan)
        if constitution_engine is not None:
            self._check_constitution(plan, constitution_engine)

        errors = [i for i in self._issues if i.severity == VerdictSeverity.error]
        return AdmissionVerdict(
            passed=len(errors) == 0,
            plan_id=plan.plan_id,
            issues=list(self._issues),
            reviewed_at=datetime.now(timezone.utc).isoformat(),
        )

    # ── Individual checks ───────────────────────────────────────────────────

    def _check_structural(self, plan: ExecutionPlan) -> None:
        """Verify every objective has non-empty title, owning_domain, description."""
        for obj in plan.objectives:
            if not obj.title.strip():
                self._error("structural", obj.id, "Objective title is empty.")
            if not obj.owning_domain.strip():
                self._error("structural", obj.id, "Objective owning_domain is empty.")
            if not obj.description.strip():
                self._warning("structural", obj.id, "Objective description is empty.")

    def _check_domains(self, plan: ExecutionPlan, known_domains: list[str]) -> None:
        """Flag objectives whose owning_domain is not in the known list."""
        known = {d.lower() for d in known_domains}
        for obj in plan.objectives:
            if obj.owning_domain.lower() not in known:
                self._warning(
                    "domain",
                    obj.id,
                    f"Objective assigned to unknown domain '{obj.owning_domain}'. "
                    f"Known domains: {', '.join(sorted(known))}.",
                )

    def _check_dag(self, plan: ExecutionPlan) -> None:
        """Validate the objective dependency graph is acyclic."""
        obj_by_id = {o.id: o for o in plan.objectives}
        visited: set[str] = set()
        in_stack: set[str] = set()

        def _visit(obj_id: str, path: list[str]) -> None:
            visited.add(obj_id)
            in_stack.add(obj_id)
            obj = obj_by_id.get(obj_id)
            if obj:
                for dep_id in obj.dependencies:
                    if dep_id not in visited:
                        _visit(dep_id, path + [dep_id])
                    elif dep_id in in_stack:
                        cycle = " -> ".join(path + [dep_id])
                        self._error(
                            "dag",
                            obj_id,
                            f"Cycle detected in dependency graph: {cycle}.",
                        )
            in_stack.discard(obj_id)

        for obj in plan.objectives:
            if obj.id not in visited:
                _visit(obj.id, [obj.id])

    def _check_criteria(self, plan: ExecutionPlan) -> None:
        """Warn when an objective has no success criteria defined."""
        for obj in plan.objectives:
            if not obj.success_criteria:
                self._warning(
                    "criteria",
                    obj.id,
                    f"Objective '{obj.title}' has no success criteria defined.",
                )

    def _check_risks(self, plan: ExecutionPlan) -> None:
        """Warn about high-impact risk categories that lack annotations.

        Flags plan-level objectives touching schema/interface/security/third-party
        categories when no corresponding risk annotations exist.  Also flags
        individual objectives whose risk level is critical or high.
        """
        # Categories that should always be surfaced.
        observed_categories: set[str] = set()
        for r in plan.plan_level_risks:
            observed_categories.add(r.category)

        unattended = _RISK_CATEGORIES_REQUIRING_ATTENTION - observed_categories
        if unattended:
            self._warning(
                "risk",
                None,
                f"Plan lacks risk annotations for: {', '.join(sorted(unattended))}. "
                "Consider reviewing these categories.",
            )

        # Critical / high risks should have affected_objective_ids or mitigation.
        for obj in plan.objectives:
            for r in obj.risks:
                if r.level in (RiskLevel.critical, RiskLevel.high):
                    if not r.description.strip():
                        self._warning(
                            "risk",
                            obj.id,
                            f"{r.level.value}-severity risk '{r.category}' "
                            "has no description.",
                        )

    def _check_constitution(self, plan: ExecutionPlan, engine: Any) -> None:
        """Search the constitution for rules relevant to each objective.

        This is a keyword-based advisory scan — it surfaces potentially relevant
        sections but does NOT block admission.  The Kernel performs authoritative
        constitution enforcement.
        """
        # Collect significant keywords from the plan.
        keywords: set[str] = set()
        for obj in plan.objectives:
            for word in obj.title.split():
                if len(word) > 3:
                    keywords.add(word.lower())
            for word in obj.description.split():
                if len(word) > 3:
                    keywords.add(word.lower())

        for kw in sorted(keywords):
            sections = engine.search_text(kw)
            for section in sections:
                for rule in section.rules:
                    if kw in rule.text.lower():
                        self._warning(
                            "constitution",
                            None,
                            f"Keyword '{kw}' matches constitution rule: {rule.text}",
                            rule_ref=f"{section.title} (line {rule.line_number})",
                        )

    # ── Issue helpers ───────────────────────────────────────────────────────

    def _clear(self) -> None:
        self._issues.clear()

    def _error(
        self,
        category: str,
        objective_id: str | None,
        message: str,
        rule_ref: str | None = None,
    ) -> None:
        self._issues.append(
            AdmissionIssue(
                severity=VerdictSeverity.error,
                category=category,
                objective_id=objective_id,
                message=message,
                rule_ref=rule_ref,
            )
        )

    def _warning(
        self,
        category: str,
        objective_id: str | None,
        message: str,
        rule_ref: str | None = None,
    ) -> None:
        self._issues.append(
            AdmissionIssue(
                severity=VerdictSeverity.warning,
                category=category,
                objective_id=objective_id,
                message=message,
                rule_ref=rule_ref,
            )
        )
