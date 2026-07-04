"""
Unit tests for the Goal Decomposer (planner module).

All tests use ``MockLlmClient`` so no real LLM API is called.
"""

from __future__ import annotations

import json

import pytest

from planner.decomposer import GoalDecomposer
from planner.llm import MockLlmClient
from planner.models import (
    ExecutionPlan,
    Objective,
    Priority,
    RiskLevel,
    SuccessCriteria,
)


# ── Fixtures ──────────────────────────────────────────────────────────────────


@pytest.fixture
def mock_client() -> MockLlmClient:
    return MockLlmClient(
        responses={
            # Each key is a unique substring that appears in exactly ONE pass's prompt.
            "return your assessment": json.dumps({
                "needs_clarification": False,
                "clarification_requests": [],
                "conflicts_found": [],
                "is_clear": True,
            }),
            "concrete work items": json.dumps({
                "rationale": "Multi-tenant billing requires schema, API, and config changes.",
                "objectives": [
                    {
                        "title": "Add tenant_id to billing schema",
                        "owning_domain": "kernel",
                        "description": "Add tenant_id column to all billing tables.",
                        "dependencies": [],
                    },
                    {
                        "title": "Update billing API for tenant context",
                        "owning_domain": "kernel",
                        "description": "Add tenant_id header to billing API endpoints.",
                        "dependencies": ["Add tenant_id to billing schema"],
                    },
                    {
                        "title": "Add tenant configuration UI",
                        "owning_domain": "planner",
                        "description": "Settings page for per-tenant billing config.",
                        "dependencies": ["Add tenant_id to billing schema"],
                    },
                ],
            }),
            "success criteria as JSON": json.dumps({
                "objectives": [
                    {
                        "title": "Add tenant_id to billing schema",
                        "criteria": [
                            {"description": "All billing tables have tenant_id column", "verification_hint": "cargo test"},
                            {"description": "Existing tests still pass", "verification_hint": "cargo test"},
                            {"description": "Migrations are reversible", "verification_hint": None},
                        ],
                    },
                    {
                        "title": "Update billing API for tenant context",
                        "criteria": [
                            {"description": "All endpoints accept x-tenant-id header", "verification_hint": None},
                            {"description": "Tests cover tenant-scoped queries", "verification_hint": "cargo test"},
                        ],
                    },
                    {
                        "title": "Add tenant configuration UI",
                        "criteria": [
                            {"description": "Settings page lists all tenants", "verification_hint": None},
                            {"description": "Admin can modify per-tenant config", "verification_hint": None},
                        ],
                    },
                ],
            }),
            "risks for each objective": json.dumps({
                "plan_level_risks": [
                    {"category": "schema_change", "description": "Schema migration touches production data", "level": "high"},
                ],
                "objectives": [
                    {
                        "title": "Add tenant_id to billing schema",
                        "risks": [
                            {"category": "schema_change", "description": "Adding NOT NULL column requires default", "level": "high", "affected_objective_ids": []},
                        ],
                    },
                    {
                        "title": "Update billing API for tenant context",
                        "risks": [
                            {"category": "public_interface", "description": "Adding required header breaks existing clients", "level": "medium", "affected_objective_ids": []},
                        ],
                    },
                ],
            }),
        }
    )


@pytest.fixture
def decomposer(mock_client: MockLlmClient) -> GoalDecomposer:
    return GoalDecomposer(llm_client=mock_client)


# ── ExecutionPlan model tests ────────────────────────────────────────────────


class TestExecutionPlan:
    def test_plan_id_generated(self) -> None:
        plan = ExecutionPlan(
            objective_description="Test objective",
            objectives=[Objective(title="Task", owning_domain="kernel")],
        )
        assert plan.plan_id.startswith("plan-")
        assert len(plan.plan_id) == len("plan-") + 12

    def test_content_hash_computed(self) -> None:
        plan = ExecutionPlan(
            objective_description="Test objective",
            objectives=[Objective(title="Task", owning_domain="kernel")],
        )
        assert len(plan.content_hash) == 64  # SHA-256 hex

    def test_content_hash_deterministic(self) -> None:
        plan1 = ExecutionPlan(
            objective_description="Same objective",
            objectives=[Objective(title="Task", owning_domain="kernel")],
        )
        plan2 = ExecutionPlan(
            objective_description="Same objective",
            objectives=[Objective(title="Task", owning_domain="kernel")],
        )
        assert plan1.content_hash == plan2.content_hash

    def test_content_hash_differs_on_change(self) -> None:
        plan1 = ExecutionPlan(
            objective_description="Original",
            objectives=[Objective(title="Task", owning_domain="kernel")],
        )
        plan2 = ExecutionPlan(
            objective_description="Changed",
            objectives=[Objective(title="Task", owning_domain="kernel")],
        )
        assert plan1.content_hash != plan2.content_hash

    def test_freeze_returns_copy(self) -> None:
        plan = ExecutionPlan(
            objective_description="Test",
            objectives=[Objective(title="Task", owning_domain="kernel")],
        )
        frozen = plan.freeze()
        assert frozen.plan_id == plan.plan_id
        assert frozen.content_hash == plan.content_hash

    def test_objective_id_generated(self) -> None:
        obj = Objective(title="Test", owning_domain="kernel")
        assert obj.id.startswith("obj-")

    def test_default_priority(self) -> None:
        obj = Objective(title="Test", owning_domain="kernel")
        assert obj.priority == Priority.medium


# ── GoalDecomposer decomposition tests ───────────────────────────────────────


class TestGoalDecomposer:
    def test_decompose_success(self, decomposer: GoalDecomposer) -> None:
        plan = decomposer.decompose("Add multi-tenant support to the billing service")
        assert isinstance(plan, ExecutionPlan)
        assert plan.objective_description == "Add multi-tenant support to the billing service"
        assert len(plan.objectives) == 3
        assert plan.plan_id.startswith("plan-")
        assert len(plan.content_hash) == 64

    def test_decompose_objectives_have_domains(self, decomposer: GoalDecomposer) -> None:
        plan = decomposer.decompose("Add multi-tenant support to the billing service")
        for obj in plan.objectives:
            assert obj.owning_domain in ("kernel", "planner")

    def test_decompose_objectives_have_criteria(self, decomposer: GoalDecomposer) -> None:
        plan = decomposer.decompose("Add multi-tenant support to the billing service")
        for obj in plan.objectives:
            assert len(obj.success_criteria) >= 1
            for c in obj.success_criteria:
                assert isinstance(c, SuccessCriteria)
                assert len(c.description) > 0

    def test_decompose_objectives_have_risks(self, decomposer: GoalDecomposer) -> None:
        plan = decomposer.decompose("Add multi-tenant support to the billing service")
        # At least one objective should have a risk annotation
        all_risks = [r for obj in plan.objectives for r in obj.risks]
        assert len(all_risks) >= 1

    def test_decompose_plan_level_risks(self, decomposer: GoalDecomposer) -> None:
        plan = decomposer.decompose("Add multi-tenant support to the billing service")
        assert len(plan.plan_level_risks) >= 1
        assert plan.plan_level_risks[0].category == "schema_change"

    def test_decompose_dag_resolved(self, decomposer: GoalDecomposer) -> None:
        """Dependency titles from LLM should be resolved to objective IDs."""
        plan = decomposer.decompose("Add multi-tenant support to the billing service")
        schema_obj = next(o for o in plan.objectives if "tenant_id" in o.title)
        api_obj = next(o for o in plan.objectives if "API" in o.title)
        assert schema_obj.id in api_obj.dependencies

    def test_decompose_rationale_present(self, decomposer: GoalDecomposer) -> None:
        plan = decomposer.decompose("Add multi-tenant support to the billing service")
        assert len(plan.rationale) > 0

    def test_decompose_immutable(self, decomposer: GoalDecomposer) -> None:
        plan = decomposer.decompose("Add multi-tenant support to the billing service")
        # The plan should have a frozen content hash
        original_hash = plan.content_hash
        # Content hash should not change on re-freeze
        frozen = plan.freeze()
        assert frozen.content_hash == original_hash


# ── MockLlmClient tests ──────────────────────────────────────────────────────


class TestMockLlmClient:
    def test_mock_returns_canned_response(self) -> None:
        client = MockLlmClient(responses={"test": '{"key": "value"}'})
        resp = client.complete("system", "this is a test")
        assert "key" in resp.content

    def test_mock_request_json_parses(self) -> None:
        client = MockLlmClient(responses={"test": '{"key": "value"}'})
        resp = client.complete("system", "test", request_json=True)
        assert resp.parsed == {"key": "value"}

    def test_mock_call_logged(self) -> None:
        client = MockLlmClient(responses={"test": "response"})
        client.complete("sys", "test")
        assert len(client.call_log) == 1
        assert client.call_log[0] == ("sys", "test")

    def test_mock_add_response(self) -> None:
        client = MockLlmClient()
        client.add_response("new_key", '{"result": "ok"}')
        resp = client.complete("system", "use new_key here", request_json=True)
        assert resp.parsed == {"result": "ok"}


# ── DAG validation tests ─────────────────────────────────────────────────────


class TestDagValidation:
    def test_acyclic_dag_passes(self) -> None:
        plan = ExecutionPlan(
            objective_description="Test",
            objectives=[
                Objective(id="obj-a", title="A", owning_domain="kernel", dependencies=[]),
                Objective(id="obj-b", title="B", owning_domain="kernel", dependencies=["obj-a"]),
                Objective(id="obj-c", title="C", owning_domain="kernel", dependencies=["obj-b"]),
            ],
        )
        decomposer = GoalDecomposer(mock_client := MockLlmClient())
        # Should not raise
        decomposer._validate_dag(plan)

    def test_cycle_raises(self) -> None:
        plan = ExecutionPlan(
            objective_description="Test",
            objectives=[
                Objective(id="obj-a", title="A", owning_domain="kernel", dependencies=["obj-b"]),
                Objective(id="obj-b", title="B", owning_domain="kernel", dependencies=["obj-a"]),
            ],
        )
        decomposer = GoalDecomposer(mock_client := MockLlmClient())
        with pytest.raises(ValueError, match="Cycle detected"):
            decomposer._validate_dag(plan)

    def test_self_loop_raises(self) -> None:
        plan = ExecutionPlan(
            objective_description="Test",
            objectives=[
                Objective(id="obj-a", title="A", owning_domain="kernel", dependencies=["obj-a"]),
            ],
        )
        decomposer = GoalDecomposer(mock_client := MockLlmClient())
        with pytest.raises(ValueError, match="Cycle detected"):
            decomposer._validate_dag(plan)


# ── Clarification pass tests ─────────────────────────────────────────────────


class TestClarification:
    def test_clear_objective_passes(self, mock_client: MockLlmClient) -> None:
        decomposer = GoalDecomposer(llm_client=mock_client)
        plan = decomposer.decompose("Add multi-tenant support to the billing service")
        assert plan is not None

    def test_ambiguous_objective_raises(self) -> None:
        client = MockLlmClient(
            responses={
                "ambiguous": json.dumps({
                    "needs_clarification": True,
                    "clarification_requests": ["Objective is too vague. Please specify which module."],
                    "conflicts_found": [],
                    "is_clear": False,
                }),
            }
        )
        decomposer = GoalDecomposer(llm_client=client)
        with pytest.raises(ValueError, match="requires clarification"):
            decomposer.decompose("ambiguous")


# ── Priority enum tests ──────────────────────────────────────────────────────


class TestPriority:
    def test_priority_values(self) -> None:
        assert Priority.critical.value == "critical"
        assert Priority.high.value == "high"
        assert Priority.medium.value == "medium"
        assert Priority.low.value == "low"

    def test_priority_order(self) -> None:
        ordered = [Priority.critical, Priority.high, Priority.medium, Priority.low]
        assert all(isinstance(p, Priority) for p in ordered)
