"""
PIL API response models — mirror the shapes the kernel PilClient expects.

Each endpoint returns ``{"success": bool, "data": T}`` where *T* is one of
the item types below (or a list thereof).
"""

from __future__ import annotations

from pydantic import BaseModel, Field


# ── Generic wrapper ──────────────────────────────────────────────────────────


class PilResponse(BaseModel):
    """Generic PIL API response wrapper matching ``PilResponse<T>``."""

    success: bool = Field(..., description="Whether the request succeeded.")
    data: list | dict | None = Field(None, description="Response payload.")


# ── Health ────────────────────────────────────────────────────────────────────


class PilHealth(BaseModel):
    """Health-check payload matching ``PilHealth { status: String }``."""

    status: str = Field(..., description="Server status (e.g. 'ok').")


# ── ADR search ────────────────────────────────────────────────────────────────


class AdrSearchResultItem(BaseModel):
    """Single ADR search hit matching ``AdrSearchResult`` in the kernel client.

    Fields map from ``AdrRecord`` + ``AdrSearchResult`` engine models:
      - ``id`` ← ``AdrRecord.adr_id``
      - ``content`` ← ``AdrRecord.body``
    """

    id: str = Field(..., description="ADR identifier (e.g. 'ADR-001').")
    title: str = Field(..., description="ADR title.")
    status: str = Field(..., description="Decision status.")
    date: str | None = Field(None, description="Date (ISO-8601) or null.")
    tags: list[str] = Field(default_factory=list, description="Categorisation tags.")
    content: str = Field(..., description="Full markdown body.")


# ── Constitution validate ────────────────────────────────────────────────────


class ConstitutionSectionItem(BaseModel):
    """Single constitution section matching ``ConstitutionSection`` in the kernel.

    Fields:
      - ``content`` ← ``ConstitutionSection.body``
      - ``rules`` ← ``[r.text for r in ConstitutionSection.rules]``
    """

    title: str = Field(..., description="Section title.")
    content: str = Field(..., description="Full markdown body.")
    rules: list[str] = Field(
        default_factory=list, description="Extracted atomic rule texts."
    )


# ── Symbol resolve ────────────────────────────────────────────────────────────


class SymbolDefItem(BaseModel):
    """Single symbol definition matching ``SymbolDef`` in the kernel client."""

    name: str = Field(..., description="Symbol name.")
    kind: str = Field(..., description="Symbol kind (function, struct, class, …).")
    file_path: str = Field(..., description="Absolute file path.")
    line: int = Field(..., ge=1, description="1-based line number.")
    column: int = Field(..., ge=0, description="0-based column offset.")


# ── Semantic search ───────────────────────────────────────────────────────────


class SemanticSearchResultItem(BaseModel):
    """Single search hit matching ``SemanticSearchResult`` in the kernel."""

    title: str = Field(..., description="Document or chunk title.")
    content: str = Field(..., description="Matched text content.")
    file_path: str = Field(..., description="Filesystem path.")
    score: float = Field(..., ge=0.0, le=1.0, description="Similarity in [0, 1].")


# ── Dependency graph ────────────────────────────────────────────────────────────


class DependencyEdgeItem(BaseModel):
    """Single dependency edge matching ``DependencyEdge`` in the kernel client."""

    source_file: str = Field(..., description="Absolute path of the importing file.")
    source_line: int = Field(..., ge=1, description="Line number of the import.")
    target: str = Field(..., description="Imported module or symbol name.")
    kind: str = Field(..., description="Import kind (import, from-import, use, …).")


class DependencyGraphStatsItem(BaseModel):
    """Aggregate graph statistics matching ``DependencyGraphStats``."""

    node_count: int = Field(..., ge=0, description="Unique file nodes.")
    edge_count: int = Field(..., ge=0, description="Total dependency edges.")
    languages: list[str] = Field(
        default_factory=list, description="Languages present in the graph."
    )


class ResolvedDependenciesItem(BaseModel):
    """Incoming and outgoing edges for a symbol or file query."""

    dependents: list[DependencyEdgeItem] = Field(
        default_factory=list, description="Files that import the target."
    )
    dependencies: list[DependencyEdgeItem] = Field(
        default_factory=list, description="Files the target imports from."
    )


# ── Plan / Decomposer ───────────────────────────────────────────────────────────


class SuccessCriteriaItem(BaseModel):
    """Verifiable success criterion matching ``SuccessCriteria`` in the planner."""

    description: str = Field(..., min_length=1, description="Verifiable success criterion.")
    verification_hint: str | None = Field(None, description="Optional hint for verification.")


class RiskAnnotationItem(BaseModel):
    """Identified risk matching ``RiskAnnotation`` in the planner."""

    category: str = Field(..., description="Risk category.")
    description: str = Field(..., description="Human-readable risk summary.")
    level: str = Field(..., description="Severity level.")
    affected_objective_ids: list[str] = Field(default_factory=list, description="Affected objective IDs.")


class ObjectiveItem(BaseModel):
    """Single objective within an execution plan."""

    id: str = Field(..., description="Unique objective ID.")
    title: str = Field(..., min_length=1, description="Objective title.")
    description: str = Field("", description="Extended description.")
    owning_domain: str = Field(..., description="Responsible domain.")
    priority: str = Field("medium", description="Relative priority.")
    dependencies: list[str] = Field(default_factory=list, description="Dependency objective IDs.")
    success_criteria: list[SuccessCriteriaItem] = Field(default_factory=list)
    risks: list[RiskAnnotationItem] = Field(default_factory=list)
    status: str = Field("pending", description="Current lifecycle status.")


class ExecutionPlanItem(BaseModel):
    """Immutable execution plan produced by the Goal Decomposer."""

    plan_id: str = Field(..., description="Unique plan identifier.")
    version: int = Field(1, ge=1, description="Plan version.")
    supersedes: str | None = Field(None, description="Superseded plan ID.")
    objective_description: str = Field(..., description="Original business objective.")
    rationale: str = Field("", description="Decomposition strategy rationale.")
    objectives: list[ObjectiveItem] = Field(..., min_length=1, description="DAG of objectives.")
    plan_level_risks: list[RiskAnnotationItem] = Field(default_factory=list)
    created_at: str = Field(..., description="ISO-8601 timestamp.")
    content_hash: str = Field("", description="SHA-256 content digest.")


class DecomposeRequest(BaseModel):
    """Request body for ``POST /api/v1/plan/decompose``."""

    objective: str = Field(..., min_length=1, description="Business objective to decompose.")
    context: dict | None = Field(None, description="Optional PIL-gathered context.")


class DecomposeResponse(BaseModel):
    """Response body for the decompose endpoint."""

    success: bool = Field(..., description="Whether decomposition succeeded.")
    data: ExecutionPlanItem | None = Field(None, description="The execution plan, if successful.")
    error: str | None = Field(None, description="Error message, if decomposition failed.")
