"""
Execution plan models — the canonical schema for Goal Decomposer output.

An ``ExecutionPlan`` is an immutable, versioned document produced by the
:class:`~planner.decomposer.GoalDecomposer`. It describes a directed acyclic
graph of objectives derived from a single human business objective.

Once frozen, a plan MUST NOT be mutated. Scope changes produce a new plan
version that explicitly supersedes the old one.
"""

from __future__ import annotations

import hashlib
import json
from datetime import datetime, timezone
from enum import Enum
from typing import Optional
from uuid import uuid4

from pydantic import BaseModel, Field


# ── Enums ──────────────────────────────────────────────────────────────────────


class Priority(str, Enum):
    """Objective priority — used by the Kernel for admission and scheduling."""

    critical = "critical"
    high = "high"
    medium = "medium"
    low = "low"


class ObjectiveStatus(str, Enum):
    """Lifecycle status of an objective within the execution plan."""

    pending = "pending"
    eligible = "eligible"  # no unmet dependencies
    in_progress = "in_progress"
    completed = "completed"
    blocked = "blocked"
    cancelled = "cancelled"


class RiskLevel(str, Enum):
    """Severity of an identified risk annotation."""

    critical = "critical"
    high = "high"
    medium = "medium"
    low = "low"
    informational = "informational"


# ── Risk annotation ────────────────────────────────────────────────────────────


class RiskAnnotation(BaseModel):
    """A single identified risk attached to an objective or the plan as a whole.

    .. attribute:: category
        One of ``schema_change``, ``public_interface``, ``security_sensitive``,
        ``external_dependency``, ``performance``, ``ambiguity``, ``other``.
    """

    category: str = Field(
        ...,
        description="Risk category — schema_change, public_interface, security_sensitive, etc.",
    )
    description: str = Field(..., min_length=1, description="Human-readable risk summary.")
    level: RiskLevel = Field(
        ..., description="Severity level — from informational to critical."
    )
    affected_objective_ids: list[str] = Field(
        default_factory=list,
        description="Objective IDs this risk applies to (empty = plan-level).",
    )


# ── Success criteria ───────────────────────────────────────────────────────────


class SuccessCriteria(BaseModel):
    """Concrete, checkable criteria that define when an objective is complete.

    Each criterion MUST be verifiable without human interpretation, e.g.
    "all existing tests pass", "new endpoint documented in OpenAPI spec",
    "coverage ≥ 90% on new code".
    """

    description: str = Field(
        ..., min_length=1, description="Verifiable success criterion."
    )
    verification_hint: str | None = Field(
        None,
        description="Optional hint for how to verify (e.g. 'cargo test', 'pytest --coverage').",
    )


# ── Objective ──────────────────────────────────────────────────────────────────


class Objective(BaseModel):
    """A single decomposable unit of work within an execution plan."""

    id: str = Field(default_factory=lambda: f"obj-{uuid4().hex[:12]}", description="Unique objective ID.")
    title: str = Field(..., min_length=1, description="Concise title.")
    description: str = Field("", description="Extended description of the work.")
    owning_domain: str = Field(
        ..., min_length=1, description="Domain responsible (e.g. 'kernel', 'planner')."
    )
    priority: Priority = Field(Priority.medium, description="Relative priority.")
    dependencies: list[str] = Field(
        default_factory=list,
        description="Objective IDs that must be completed before this one starts.",
    )
    success_criteria: list[SuccessCriteria] = Field(
        default_factory=list,
        description="Verifiable criteria that define completion.",
    )
    risks: list[RiskAnnotation] = Field(
        default_factory=list,
        description="Risks identified for this specific objective.",
    )
    status: ObjectiveStatus = Field(
        ObjectiveStatus.pending,
        description="Current status — set by the Kernel, not the Decomposer.",
    )


# ── Execution Plan ─────────────────────────────────────────────────────────────


class ExecutionPlan(BaseModel):
    """Immutable, versioned decomposition of a human business objective.

    The :class:`~planner.decomposer.GoalDecomposer` produces exactly one
    ``ExecutionPlan`` per ``decompose()`` call. The plan is frozen (immutable)
    after emission — any scope change requires a new plan version.
    """

    plan_id: str = Field(default_factory=lambda: f"plan-{uuid4().hex[:12]}", description="Unique plan identifier.")
    version: int = Field(1, ge=1, description="Monotonically increasing version number.")
    supersedes: str | None = Field(
        None, description="Optional plan_id that this version supersedes."
    )
    objective_description: str = Field(
        ..., min_length=1, description="Original human-supplied business objective."
    )
    rationale: str = Field(
        "", description="Plan-level explanation of the decomposition strategy."
    )
    objectives: list[Objective] = Field(
        ..., min_length=1, description="Non-empty DAG of objectives."
    )
    plan_level_risks: list[RiskAnnotation] = Field(
        default_factory=list,
        description="Risks that span the entire plan, not a single objective.",
    )
    created_at: str = Field(
        default_factory=lambda: datetime.now(timezone.utc).isoformat(),
        description="ISO-8601 timestamp of plan creation.",
    )
    content_hash: str = Field(
        "", description="SHA-256 hex digest of the canonical plan content."
    )

    def model_post_init(self, __context: object) -> None:
        """Auto-compute the content hash after construction."""
        self.content_hash = self._compute_hash()
        return super().model_post_init(__context)

    def _compute_hash(self) -> str:
        """Return a SHA-256 hex digest over all substantive plan fields.

        The hash covers everything except ``plan_id`` and ``created_at``
        so identical decompositions produce the same hash irrespective
        of the UUID or timestamp.
        """
        payload = {
            "version": self.version,
            "supersedes": self.supersedes,
            "objective_description": self.objective_description,
            "rationale": self.rationale,
            "objectives": [
                {
                    "title": o.title,
                    "description": o.description,
                    "owning_domain": o.owning_domain,
                    "priority": o.priority.value,
                    "dependencies": sorted(o.dependencies),
                    "success_criteria": [c.model_dump() for c in o.success_criteria],
                    "risks": [r.model_dump() for r in o.risks],
                }
                for o in sorted(self.objectives, key=lambda x: x.title)
            ],
            "plan_level_risks": [r.model_dump() for r in self.plan_level_risks],
        }
        raw = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
        return hashlib.sha256(raw).hexdigest()

    def freeze(self) -> ExecutionPlan:
        """Return a frozen copy — the plan is now immutable.

        This is a marker; immutability is enforced by convention (no setter
        methods) and by the Kernel rejecting mutated plan IDs.
        """
        return self.model_copy(deep=True)
