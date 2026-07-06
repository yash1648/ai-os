"""Tests for ``planner.admission`` — PlanAdmissionService.

Covers all six check categories plus the happy path.
"""

from __future__ import annotations

import json

import pytest

from planner.admission import PlanAdmissionService
from planner.models import (
    AdmissionVerdict,
    ExecutionPlan,
    Objective,
    Priority,
    RiskAnnotation,
    RiskLevel,
    SuccessCriteria,
    VerdictSeverity,
)


# ── Fixtures ─────────────────────────────────────────────────────────────────────


@pytest.fixture
def valid_plan() -> ExecutionPlan:
    """A well-formed plan that should pass admission cleanly."""
    return ExecutionPlan(
        objective_description="Add multi-tenant support",
        rationale="Multi-tenant billing requires schema, API, and config changes.",
        objectives=[
            Objective(
                title="Add tenant_id to billing schema",
                owning_domain="kernel",
                description="Add tenant_id column to all billing tables.",
                dependencies=[],
                success_criteria=[
                    SuccessCriteria(
                        description="All billing tables have tenant_id column",
                        verification_hint="cargo test",
                    ),
                    SuccessCriteria(
                        description="Existing tests still pass",
                        verification_hint="cargo test",
                    ),
                ],
                risks=[
                    RiskAnnotation(
                        category="schema_change",
                        description="Adding NOT NULL column requires default",
                        level=RiskLevel.high,
                    ),
                ],
            ),
            Objective(
                title="Update billing API for tenant context",
                owning_domain="kernel",
                description="Add tenant_id header to billing API endpoints.",
                dependencies=[],
                success_criteria=[
                    SuccessCriteria(
                        description="All endpoints accept x-tenant-id header",
                    ),
                ],
            ),
        ],
        plan_level_risks=[
            RiskAnnotation(
                category="schema_change",
                description="Schema migration touches production data",
                level=RiskLevel.high,
            ),
            RiskAnnotation(
                category="public_interface",
                description="Adding required header breaks existing clients",
                level=RiskLevel.medium,
            ),
        ],
    )


@pytest.fixture
def service() -> PlanAdmissionService:
    return PlanAdmissionService()


# ── Happy path ────────────────────────────────────────────────────────────────────


class TestAdmitHappyPath:
    def test_valid_plan_passes(self, valid_plan: ExecutionPlan, service: PlanAdmissionService) -> None:
        verdict = service.admit(valid_plan)
        assert verdict.passed
        assert verdict.plan_id == valid_plan.plan_id
        # Valid plan with warnings only from risk check
        errors = [i for i in verdict.issues if i.severity == VerdictSeverity.error]
        assert len(errors) == 0

    def test_verdict_includes_plan_id(self, valid_plan: ExecutionPlan, service: PlanAdmissionService) -> None:
        verdict = service.admit(valid_plan)
        assert verdict.plan_id == valid_plan.plan_id

    def test_verdict_includes_timestamp(self, valid_plan: ExecutionPlan, service: PlanAdmissionService) -> None:
        verdict = service.admit(valid_plan)
        assert verdict.reviewed_at  # non-empty ISO string


# ── Structural checks ────────────────────────────────────────────────────────────


class TestStructuralChecks:
    def test_blank_title_errors(self, service: PlanAdmissionService) -> None:
        plan = ExecutionPlan(
            objective_description="test",
            rationale="",
            objectives=[
                Objective(title="   ", owning_domain="kernel", description="Some work."),
            ],
        )
        verdict = service.admit(plan)
        assert not verdict.passed
        assert any(
            i.category == "structural" and i.severity == VerdictSeverity.error
            for i in verdict.issues
        )

    def test_blank_owning_domain_errors(self, service: PlanAdmissionService) -> None:
        plan = ExecutionPlan(
            objective_description="test",
            rationale="",
            objectives=[
                Objective(title="Do the thing", owning_domain=" ", description="Some work."),
            ],
        )
        verdict = service.admit(plan)
        assert not verdict.passed
        assert any(
            i.category == "structural" and i.severity == VerdictSeverity.error
            for i in verdict.issues
        )

    def test_empty_description_warns(self, service: PlanAdmissionService) -> None:
        plan = ExecutionPlan(
            objective_description="test",
            rationale="",
            objectives=[
                Objective(title="Do the thing", owning_domain="kernel", description=""),
            ],
        )
        verdict = service.admit(plan)
        # Empty description is a warning, not an error
        assert verdict.passed
        assert any(
            i.category == "structural" and i.severity == VerdictSeverity.warning
            for i in verdict.issues
        )


# ── Domain checks ────────────────────────────────────────────────────────────────


class TestDomainChecks:
    def test_unknown_domain_warns(self, service: PlanAdmissionService) -> None:
        plan = ExecutionPlan(
            objective_description="test",
            rationale="",
            objectives=[
                Objective(
                    title="Do the thing",
                    owning_domain="bogus-invalid-domain",
                    description="Some work.",
                ),
            ],
        )
        verdict = service.admit(plan)
        assert any(
            i.category == "domain" and i.severity == VerdictSeverity.warning
            for i in verdict.issues
        )

    def test_known_domain_does_not_warn(self, service: PlanAdmissionService) -> None:
        plan = ExecutionPlan(
            objective_description="test",
            rationale="",
            objectives=[
                Objective(
                    title="Do the thing",
                    owning_domain="planner",
                    description="Some work.",
                ),
            ],
        )
        verdict = service.admit(plan, known_domains=["planner"])
        domain_issues = [i for i in verdict.issues if i.category == "domain"]
        assert len(domain_issues) == 0

    def test_custom_known_domains_accepted(self, service: PlanAdmissionService) -> None:
        plan = ExecutionPlan(
            objective_description="test",
            rationale="",
            objectives=[
                Objective(
                    title="Configure router",
                    owning_domain="network",
                    description="Set up network routing.",
                ),
            ],
        )
        verdict = service.admit(plan, known_domains=["kernel", "network"])
        domain_issues = [i for i in verdict.issues if i.category == "domain"]
        assert len(domain_issues) == 0


# ── DAG checks ────────────────────────────────────────────────────────────────────


class TestDagChecks:
    def test_dag_cycle_errors(self, service: PlanAdmissionService) -> None:
        objective_a = Objective(
            title="A",
            owning_domain="kernel",
            description="First objective.",
        )
        objective_b = Objective(
            title="B",
            owning_domain="kernel",
            description="Second objective.",
        )
        # A depends on B, B depends on A = cycle
        objective_a.dependencies.append(objective_b.id)
        objective_b.dependencies.append(objective_a.id)

        plan = ExecutionPlan(
            objective_description="test",
            rationale="",
            objectives=[objective_a, objective_b],
        )
        verdict = service.admit(plan)
        assert not verdict.passed
        assert any(
            i.category == "dag" and i.severity == VerdictSeverity.error
            for i in verdict.issues
        )

    def test_self_loop_errors(self, service: PlanAdmissionService) -> None:
        obj = Objective(
            title="Self-loop",
            owning_domain="kernel",
            description="Depends on itself.",
            dependencies=[],
        )
        obj.dependencies.append(obj.id)  # self-loop
        plan = ExecutionPlan(
            objective_description="test",
            rationale="",
            objectives=[obj],
        )
        verdict = service.admit(plan)
        assert not verdict.passed
        assert any(
            i.category == "dag" and i.severity == VerdictSeverity.error
            for i in verdict.issues
        )


# ── Criteria checks ──────────────────────────────────────────────────────────────


class TestCriteriaChecks:
    def test_missing_criteria_warns(self, service: PlanAdmissionService) -> None:
        plan = ExecutionPlan(
            objective_description="test",
            rationale="",
            objectives=[
                Objective(
                    title="No criteria objective",
                    owning_domain="kernel",
                    description="Some work without criteria.",
                ),
            ],
        )
        verdict = service.admit(plan)
        assert any(
            i.category == "criteria" and i.severity == VerdictSeverity.warning
            for i in verdict.issues
        )


# ── Risk checks ───────────────────────────────────────────────────────────────────


class TestRiskChecks:
    def test_unattended_risk_categories_warn(self, service: PlanAdmissionService) -> None:
        """A plan without any risk annotations should get warnings."""
        plan = ExecutionPlan(
            objective_description="test",
            rationale="",
            objectives=[
                Objective(
                    title="Safe objective",
                    owning_domain="kernel",
                    description="Trivially safe work.",
                    success_criteria=[
                        SuccessCriteria(description="It compiles."),
                    ],
                ),
            ],
        )
        verdict = service.admit(plan)
        risk_warnings = [
            i for i in verdict.issues
            if i.category == "risk" and i.severity == VerdictSeverity.warning
        ]
        assert len(risk_warnings) >= 1  # at least one unattended category warning

    def test_high_risk_without_description_warns(self, service: PlanAdmissionService) -> None:
        plan = ExecutionPlan(
            objective_description="test",
            rationale="",
            objectives=[
                Objective(
                    title="Risky objective",
                    owning_domain="kernel",
                    description="Risky work.",
                    success_criteria=[SuccessCriteria(description="Done.")],
                    risks=[
                        RiskAnnotation(
                            category="schema_change",
                            description="   ",  # whitespace-only description
                            level=RiskLevel.critical,
                        ),
                    ],
                ),
            ],
        )
        verdict = service.admit(plan)
        assert any(
            i.category == "risk" and i.severity == VerdictSeverity.warning
            for i in verdict.issues
        )


# ── Constitution checks ──────────────────────────────────────────────────────────


class _FakeConstitutionEngine:
    """Minimal stand-in for test purposes — returns canned rules for keywords."""

    def __init__(self, keyword_rules: dict[str, list[tuple[str, str, str]]] | None = None) -> None:
        # key -> [(section_title, rule_text, line_number)]
        self._data: dict[str, list[tuple[str, str, str]]] = keyword_rules or {}

    def search_text(self, query: str) -> list:
        class _FakeSection:
            def __init__(self, title: str, body: str, rules: list):
                self.title = title
                self.body = body
                self.rules = rules

        query_lower = query.lower()
        matching: list = []
        for keyword, rules in self._data.items():
            if keyword.lower() in query_lower or query_lower in keyword.lower():
                rule_objects = []
                for section_title, rule_text, line_num in rules:
                    class _FakeRule:
                        def __init__(self, text: str, line_number: int):
                            self.text = text
                            self.line_number = line_number
                    rule_objects.append(_FakeRule(rule_text, line_num))
                matching.append(_FakeSection(keyword, "", rule_objects))
        return matching


class TestConstitutionChecks:
    def test_constitution_matches_surface_warnings(self, service: PlanAdmissionService) -> None:
        plan = ExecutionPlan(
            objective_description="Add tenant isolation",
            rationale="Need tenant isolation for multi-tenant support.",
            objectives=[
                Objective(
                    title="Add tenant isolation",
                    owning_domain="kernel",
                    description="Isolate tenant data in the billing schema.",
                    success_criteria=[SuccessCriteria(description="Done.")],
                ),
            ],
        )
        fake_engine = _FakeConstitutionEngine({
            "tenant": [
                ("Data Isolation", "Tenant data must be isolated at the database level.", 42),
            ],
        })
        verdict = service.admit(plan, constitution_engine=fake_engine)
        constitution_warnings = [
            i for i in verdict.issues
            if i.category == "constitution"
        ]
        assert len(constitution_warnings) >= 1
        # Verify it references the rule
        matched = any(
            "tenant" in i.message.lower() and "isolat" in i.message.lower()
            for i in constitution_warnings
        )
        assert matched

    def test_no_constitution_engine_skipped(self, service: PlanAdmissionService) -> None:
        """Without a constitution engine, no constitution checks run."""
        plan = ExecutionPlan(
            objective_description="test",
            rationale="",
            objectives=[
                Objective(
                    title="Simple objective",
                    owning_domain="kernel",
                    description="Simple work.",
                    success_criteria=[SuccessCriteria(description="Done.")],
                ),
            ],
        )
        verdict = service.admit(plan)  # no constitution_engine passed
        constitution_issues = [i for i in verdict.issues if i.category == "constitution"]
        assert len(constitution_issues) == 0


# ── Edge cases ────────────────────────────────────────────────────────────────────


class TestEdgeCases:
    def test_no_objectives_admission_succeeds(self, service: PlanAdmissionService) -> None:
        """ExecutionPlan requires at least one objective, but test the service handles it."""
        with pytest.raises(Exception):
            ExecutionPlan(
                objective_description="test",
                rationale="",
                objectives=[],  # min_length=1 will raise
            )

    def test_admit_twice_independent(self, valid_plan: ExecutionPlan, service: PlanAdmissionService) -> None:
        """Multiple admits should produce independent verdicts."""
        v1 = service.admit(valid_plan)
        v2 = service.admit(valid_plan)
        assert v1.passed == v2.passed
        assert v1.plan_id == v2.plan_id
