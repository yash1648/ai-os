"""Lightweight HTTP client for the Kernel API.

Used by the intelligence layer to submit plans to the Kernel
for execution.
"""

from __future__ import annotations

import os

import httpx


class KernelClient:
    """HTTP client for the Kernel REST API.

    Reads ``KERNEL_BASE_URL`` from the environment (defaults to
    ``http://localhost:8081``).
    """

    def __init__(self, base_url: str | None = None) -> None:
        self.base_url = (base_url or os.getenv("KERNEL_BASE_URL", "http://localhost:8081")).rstrip("/")
        self._client = httpx.Client(timeout=30.0)

    # ── Objectives ──────────────────────────────────────────────────────────

    def create_objective(
        self,
        title: str,
        description: str,
        owner: str,
        priority: str = "medium",
        dependencies: list[str] | None = None,
        success_criteria: list[str] | None = None,
        tags: list[str] | None = None,
    ) -> str:
        """Create a single objective in the Kernel and return its ID.

        Raises:
            KernelClientError: If the Kernel returns a non-2xx status.
        """
        body = {
            "title": title,
            "description": description,
            "owner": owner,
            "priority": priority,
            "dependencies": dependencies or [],
            "success_criteria": success_criteria or [],
            "tags": tags or [],
        }
        resp = self._client.post(f"{self.base_url}/api/v1/objectives", json=body)
        if resp.status_code != 201:
            raise KernelClientError(
                f"Kernel returned {resp.status_code} for create_objective: {resp.text}"
            )
        data = resp.json()
        return data.get("data", {}).get("id", "")

    def mark_objective_ready(self, objective_id: str) -> bool:
        """Mark an objective as READY for scheduling.

        Returns:
            True if successfully marked ready, False otherwise.
        """
        resp = self._client.post(
            f"{self.base_url}/api/v1/objectives/{objective_id}/ready"
        )
        return resp.status_code == 200

    def trigger_dispatch(self) -> bool:
        """Trigger the scheduler to dispatch the next ready objective.

        Returns:
            True if an objective was dispatched, False if queue is empty or at capacity.
        """
        resp = self._client.post(f"{self.base_url}/api/v1/scheduler/dispatch")
        if resp.status_code != 200:
            return False
        data = resp.json()
        return data.get("data", {}).get("dispatched") is not None

    def submit_plan(
        self,
        plan_id: str,
        objectives: list[dict],
        plan_level_risks: list[dict] | None = None,
    ) -> dict:
        """Submit all objectives from an admitted plan to the Kernel.

        Each objective is created via ``POST /api/v1/objectives``, then marked
        READY, and finally the scheduler dispatch is triggered to start
        execution.

        Returns:
            A dict with:
            - ``plan_id`` — the submitted plan ID.
            - ``objective_ids`` — list of ``{title, id}`` pairs created.
            - ``ready_status`` — whether all objectives were marked ready.
            - ``dispatched`` — whether an objective was dispatched to a worker.
            - ``errors`` — list of ``{title, error}`` for any failures.
        """
        results: list[dict] = []
        errors: list[dict] = []

        # Step 1: Create all objectives
        for obj in objectives:
            try:
                tags = [plan_id, obj.get("owning_domain", "unknown")]
                obj_id = self.create_objective(
                    title=obj.get("title", "Untitled"),
                    description=obj.get("description", ""),
                    owner=obj.get("owning_domain", "unknown"),
                    priority=obj.get("priority", "medium"),
                    dependencies=obj.get("dependencies", []),
                    success_criteria=[c.get("description", "") for c in obj.get("success_criteria", [])],
                    tags=tags,
                )
                results.append({"title": obj["title"], "id": obj_id})
            except KernelClientError as e:
                errors.append({"title": obj.get("title", ""), "error": str(e)})

        # If any objective failed to create, we can't proceed
        if errors:
            return {
                "plan_id": plan_id,
                "objective_ids": results,
                "ready_status": False,
                "dispatched": False,
                "errors": errors,
            }

        # Step 2: Mark all objectives as READY
        ready_count = 0
        for obj in results:
            if self.mark_objective_ready(obj["id"]):
                ready_count += 1

        # Step 3: Trigger scheduler dispatch to start execution
        dispatched = self.trigger_dispatch()

        return {
            "plan_id": plan_id,
            "objective_ids": results,
            "ready_status": ready_count == len(results),
            "dispatched": dispatched,
            "errors": errors,
        }


class KernelClientError(Exception):
    """Raised when a Kernel API call fails."""
