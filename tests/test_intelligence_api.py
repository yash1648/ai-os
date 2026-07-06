"""Tests for the intelligence API endpoints."""

from __future__ import annotations

import json

import pytest
from fastapi.testclient import TestClient

from intelligence.main import create_app
from intelligence.api.state import PilState


@pytest.fixture
def client() -> TestClient:
    """Create a test client for the intelligence API with initialized state."""
    app = create_app()
    # Manually initialize the pil_state (normally done by lifespan)
    app.state.pil_state = PilState(None)  # None uses current working directory
    return TestClient(app)


def test_health_endpoint(client: TestClient) -> None:
    """Test the health endpoint returns success."""
    response = client.get("/api/v1/health")
    assert response.status_code == 200
    data = response.json()
    assert data["success"] is True
    assert data["data"]["status"] == "ok"


def test_plan_decompose_endpoint(client: TestClient) -> None:
    """Test the plan decompose endpoint returns a valid plan."""
    response = client.post(
        "/api/v1/plan/decompose",
        json={
            "objective": "Add user authentication to the web service",
            "context": {},
        },
    )
    assert response.status_code == 200
    data = response.json()
    assert data["success"] is True
    assert "data" in data
    plan = data["data"]
    assert "plan_id" in plan
    assert "objective_description" in plan
    assert plan["objective_description"] == "Add user authentication to the web service"
    assert "objectives" in plan
    assert isinstance(plan["objectives"], list)
    assert len(plan["objectives"]) > 0
    # Check first objective has required fields
    first_obj = plan["objectives"][0]
    assert "id" in first_obj
    assert "title" in first_obj
    assert "owning_domain" in first_obj
    assert "description" in first_obj
    assert "success_criteria" in first_obj
    assert "risks" in first_obj


def test_plan_decompose_with_empty_objective(client: TestClient) -> None:
    """Test that decompose handles empty object (returns validation error)."""
    response = client.post(
        "/api/v1/plan/decompose",
        json={
            "objective": "",
            "context": {},
        },
    )
    # Empty string should fail Pydantic validation (min_length=1)
    assert response.status_code == 422  # Validation error
    data = response.json()
    assert "detail" in data


def test_plan_admit_endpoint(client: TestClient) -> None:
    """Test the plan admit endpoint with a valid plan."""
    # First get a plan to admit
    decompose_resp = client.post(
        "/api/v1/plan/decompose",
        json={
            "objective": "Add API rate limiting",
            "context": {},
        },
    )
    assert decompose_resp.status_code == 200
    decompose_data = decompose_resp.json()
    assert decompose_data["success"] is True
    plan = decompose_data["data"]

    # Now admit the plan
    admit_resp = client.post(
        "/api/v1/plan/admit",
        json={
            "plan": plan,
            "known_domains": ["kernel", "planner", "worker", "dashboard"],
        },
    )
    assert admit_resp.status_code == 200
    admit_data = admit_resp.json()
    assert admit_data["success"] is True
    assert "data" in admit_data
    verdict = admit_data["data"]
    assert "passed" in verdict
    assert "plan_id" in verdict
    assert verdict["plan_id"] == plan["plan_id"]
    assert "issues" in verdict
    assert isinstance(verdict["issues"], list)
    assert "reviewed_at" in verdict


def test_plan_admit_with_invalid_plan(client: TestClient) -> None:
    """Test that admit handles invalid/malformed plans gracefully."""
    response = client.post(
        "/api/v1/plan/admit",
        json={
            "plan": {
                # Missing required fields
                "objective_description": "Test",
            },
            "known_domains": [],
        },
    )
    # Should return validation error (422) for missing required fields
    assert response.status_code == 422  # Validation error
    data = response.json()
    assert "detail" in data


def test_plan_admit_empty_objectives(client: TestClient) -> None:
    """Test that admit rejects plans with empty objectives list."""
    response = client.post(
        "/api/v1/plan/admit",
        json={
            "plan": {
                "plan_id": "test-plan",
                "version": 1,
                "objective_description": "Test objective",
                "rationale": "Test rationale",
                "objectives": [],  # Empty objectives - should fail validation
                "plan_level_risks": [],
                "created_at": "2024-01-01T00:00:00Z",
                "content_hash": "abc123",
            },
            "known_domains": ["kernel"],
        },
    )
    # Empty objectives list should fail Pydantic validation (min_length=1)
    assert response.status_code == 422  # Validation error
    data = response.json()
    assert "detail" in data


def test_plan_admit_with_unknown_domain_warning(client: TestClient) -> None:
    """Test that admitting a plan with unknown domain produces warnings."""
    # First get a plan
    decompose_resp = client.post(
        "/api/v1/plan/decompose",
        json={
            "objective": "Add blockchain integration",
            "context": {},
        },
    )
    assert decompose_resp.status_code == 200
    plan = decompose_resp.json()["data"]

    # Modify an objective to have an unknown domain
    if plan["objectives"]:
        plan["objectives"][0]["owning_domain"] = "unknown-domain-xyz"

    # Admit with known domains that don't include the unknown one
    admit_resp = client.post(
        "/api/v1/plan/admit",
        json={
            "plan": plan,
            "known_domains": ["kernel", "planner"],
        },
    )
    assert admit_resp.status_code == 200
    admit_data = admit_resp.json()
    assert admit_data["success"] is True
    verdict = admit_data["data"]
    # Should have warnings about unknown domain
    warning_issues = [
        issue for issue in verdict["issues"]
        if issue["severity"] == "warning" and issue["category"] == "domain"
    ]
    assert len(warning_issues) > 0
    assert any("unknown-domain-xyz" in issue["message"] for issue in warning_issues)


# ── Plan submit tests ──────────────────────────────────────────────────────────


def test_plan_submit_rejects_without_admission(client: TestClient) -> None:
    """Test that submit rejects a plan without a passing admission verdict."""
    # First get a plan
    decompose_resp = client.post(
        "/api/v1/plan/decompose",
        json={"objective": "Test plan", "context": {}},
    )
    assert decompose_resp.status_code == 200
    plan = decompose_resp.json()["data"]

    # Submit with a failing verdict
    submit_resp = client.post(
        "/api/v1/plan/submit",
        json={
            "plan": plan,
            "verdict": {
                "passed": False,
                "plan_id": plan["plan_id"],
                "issues": [{"severity": "error", "category": "test", "message": "Test error"}],
                "reviewed_at": "2024-01-01T00:00:00Z",
            },
        },
    )
    assert submit_resp.status_code == 200
    data = submit_resp.json()
    assert data["success"] is False
    assert "has not passed admission" in data["error"]


def test_plan_submit_without_verdict_proceeds(client: TestClient) -> None:
    """Test that submit proceeds when no verdict is provided (best-effort)."""
    decompose_resp = client.post(
        "/api/v1/plan/decompose",
        json={"objective": "Test plan", "context": {}},
    )
    assert decompose_resp.status_code == 200
    plan = decompose_resp.json()["data"]

    # Submit without a verdict — should proceed (kernel may be down, handled gracefully)
    submit_resp = client.post(
        "/api/v1/plan/submit",
        json={"plan": plan},
    )
    assert submit_resp.status_code == 200
    data = submit_resp.json()
    # Without a running kernel, it should fail gracefully with an error
    assert data["success"] is False
    assert "error" in data


def test_plan_submit_with_passing_verdict(client: TestClient) -> None:
    """Test that submit accepts a plan with a passing verdict."""
    decompose_resp = client.post(
        "/api/v1/plan/decompose",
        json={"objective": "Test plan", "context": {}},
    )
    assert decompose_resp.status_code == 200
    plan = decompose_resp.json()["data"]

    admit_resp = client.post(
        "/api/v1/plan/admit",
        json={"plan": plan, "known_domains": ["kernel"]},
    )
    assert admit_resp.status_code == 200
    verdict = admit_resp.json()["data"]

    # Submit with the passing verdict — should attempt kernel call and fail gracefully
    submit_resp = client.post(
        "/api/v1/plan/submit",
        json={"plan": plan, "verdict": verdict},
    )
    assert submit_resp.status_code == 200
    data = submit_resp.json()
    # Kernel is not running, so it should fail gracefully
    assert data["success"] is False
    assert isinstance(data.get("error"), str)


if __name__ == "__main__":
    pytest.main([__file__, "-v"])