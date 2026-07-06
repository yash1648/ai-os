"""Tests for ``intelligence.kernel_client`` — Kernel HTTP client."""

from __future__ import annotations

from unittest import mock

import pytest
import httpx

from intelligence.kernel_client import KernelClient, KernelClientError


# ── Fixtures ─────────────────────────────────────────────────────────────────────


@pytest.fixture
def client() -> KernelClient:
    return KernelClient(base_url="http://test-kernel:8081")


# ── create_objective ─────────────────────────────────────────────────────────────


class TestCreateObjective:
    def test_creates_objective_and_returns_id(self, client: KernelClient) -> None:
        with mock.patch.object(client._client, "post") as mock_post:
            mock_response = mock.Mock()
            mock_response.status_code = 201
            mock_response.json.return_value = {
                "success": True,
                "data": {"id": "obj-abc-123"},
            }
            mock_post.return_value = mock_response

            obj_id = client.create_objective(
                title="Test objective",
                description="A test",
                owner="kernel",
            )

            assert obj_id == "obj-abc-123"
            mock_post.assert_called_once()
            call_kwargs = mock_post.call_args[1]
            assert call_kwargs["json"]["title"] == "Test objective"
            assert call_kwargs["json"]["owner"] == "kernel"
            assert call_kwargs["json"]["priority"] == "medium"

    def test_raises_on_non_201(self, client: KernelClient) -> None:
        with mock.patch.object(client._client, "post") as mock_post:
            mock_response = mock.Mock()
            mock_response.status_code = 500
            mock_response.text = "Internal error"
            mock_post.return_value = mock_response

            with pytest.raises(KernelClientError, match="Kernel returned 500"):
                client.create_objective(title="Fail", description="", owner="kernel")


# ── submit_plan ────────────────────────────────────────────────────────────────────


class TestSubmitPlan:
    def test_submits_all_objectives(self, client: KernelClient) -> None:
        with mock.patch.object(client, "create_objective") as mock_create, \
             mock.patch.object(client, "mark_objective_ready") as mock_ready, \
             mock.patch.object(client, "trigger_dispatch") as mock_dispatch:
            mock_create.side_effect = ["obj-1", "obj-2"]
            mock_ready.return_value = True
            mock_dispatch.return_value = True

            result = client.submit_plan(
                plan_id="plan-test",
                objectives=[
                    {"title": "First", "owning_domain": "kernel", "dependencies": []},
                    {"title": "Second", "owning_domain": "planner", "dependencies": ["obj-1"]},
                ],
            )

            assert result["plan_id"] == "plan-test"
            assert len(result["objective_ids"]) == 2
            assert result["objective_ids"][0] == {"title": "First", "id": "obj-1"}
            assert result["objective_ids"][1] == {"title": "Second", "id": "obj-2"}
            assert result["ready_status"] is True
            assert result["dispatched"] is True
            assert result["errors"] == []
            assert mock_create.call_count == 2
            assert mock_ready.call_count == 2
            assert mock_dispatch.call_count == 1

    def test_collects_partial_errors(self, client: KernelClient) -> None:
        with mock.patch.object(client, "create_objective") as mock_create, \
             mock.patch.object(client, "mark_objective_ready") as mock_ready, \
             mock.patch.object(client, "trigger_dispatch") as mock_dispatch:
            mock_create.side_effect = [
                "obj-1",
                KernelClientError("Kernel returned 500: DB error"),
            ]
            mock_ready.return_value = True
            mock_dispatch.return_value = True

            result = client.submit_plan(
                plan_id="plan-partial",
                objectives=[
                    {"title": "First", "owning_domain": "kernel", "success_criteria": []},
                    {"title": "Failing", "owning_domain": "worker", "success_criteria": []},
                ],
            )

            assert len(result["objective_ids"]) == 1  # First succeeded
            assert result["objective_ids"][0] == {"title": "First", "id": "obj-1"}
            assert result["ready_status"] is False  # Not all objectives succeeded
            assert result["dispatched"] is False  # Short-circuits before dispatch
            assert len(result["errors"]) == 1  # Second failed
            assert result["errors"][0]["title"] == "Failing"
            assert "500" in result["errors"][0]["error"]
            assert mock_create.call_count == 2
            assert mock_ready.call_count == 0  # Never reached due to short-circuit
            assert mock_dispatch.call_count == 0  # Never reached due to short-circuit


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
