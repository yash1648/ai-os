"""
Query route definitions for the PIL sidecar.

All intelligence endpoints are wired to their respective engines via
:class:`~intelligence.api.state.PilState`, which is attached to
``request.app.state.pil_state`` during server startup.
"""

from __future__ import annotations

from fastapi import APIRouter, Query, Request

from fastapi import APIRouter, HTTPException, Query, Request

from intelligence.api.models import (
    AdmitRequest,
    AdmitResponse,
    AdmissionIssueItem,
    AdmissionVerdictItem,
    AdrSearchResultItem,
    ConstitutionSectionItem,
    DecomposeRequest,
    DecomposeResponse,
    DependencyEdgeItem,
    DependencyGraphStatsItem,
    ExecutionPlanItem,
    ObjectiveItem,
    PilHealth,
    ResolvedDependenciesItem,
    RiskAnnotationItem,
    SemanticSearchResultItem,
    SubmitRequest,
    SubmitResponse,
    SuccessCriteriaItem,
    SymbolDefItem,
)
from intelligence.api.state import PilState

router = APIRouter(prefix="/api/v1", tags=["intelligence"])


# ── helpers ──────────────────────────────────────────────────────────────────


def _get_state(request: Request) -> PilState:
    """Retrieve the lazily-initialised engine state from the application."""
    return request.app.state.pil_state


# ── health ────────────────────────────────────────────────────────────────────


@router.get("/health")
async def health(request: Request) -> dict:
    """PIL health check — returns ``{"success": true, "data": {"status": "ok"}}``."""
    return {"success": True, "data": {"status": "ok"}}


# ── ADR search ────────────────────────────────────────────────────────────────


@router.get("/adr/search")
async def adr_search(
    request: Request,
    q: str = Query("", description="Free-text search query."),
    status: str | None = Query(None, description="Filter by ADR status."),
) -> dict:
    """Search ADRs by keyword and optional status filter.

    The kernel's ``PilClient.search_adr()`` passes ``q`` and optional ``status``
    as query parameters and expects a ``PilResponse<Vec<AdrSearchResult>>``.
    """
    state = _get_state(request)
    engine = state.adr_engine

    # Narrow by status first if provided.
    if status:
        records = engine.search_by_status(status)
    else:
        records = engine.all()

    # If there is a text query, filter further by keyword search.
    if q.strip():
        results = engine.search_text(q)
        matched_ids = {r.record.adr_id for r in results}
        records = [r for r in records if r.adr_id in matched_ids]

    items = [
        AdrSearchResultItem(
            id=r.adr_id,
            title=r.title,
            status=r.status,
            date=r.date.isoformat() if r.date else None,
            tags=r.tags,
            content=r.body,
        )
        for r in records
    ]

    return {"success": True, "data": [m.model_dump() for m in items]}


# ── constitution validate ─────────────────────────────────────────────────────


@router.get("/constitution/validate")
async def constitution_validate(
    request: Request,
    action: str = Query(..., description="Action or plan text to validate."),
) -> dict:
    """Search constitution sections that may be relevant to *action*.

    The kernel's ``PilClient.validate_constitution()`` passes ``action`` as a
    query parameter and expects a ``PilResponse<Vec<ConstitutionSection>>``.
    """
    state = _get_state(request)
    engine = state.constitution_engine

    sections = engine.search_text(action)
    items = [
        ConstitutionSectionItem(
            title=s.title,
            content=s.body,
            rules=[r.text for r in s.rules],
        )
        for s in sections
    ]

    return {"success": True, "data": [m.model_dump() for m in items]}


# ── symbol resolve ────────────────────────────────────────────────────────────


@router.get("/symbol/resolve")
async def symbol_resolve(
    request: Request,
    name: str = Query(..., description="Symbol name to resolve (substring match)."),
    kind: str | None = Query(None, description="Optional symbol-kind filter."),
) -> dict:
    """Resolve a symbol by name across the workspace.

    The kernel's ``PilClient.resolve_symbol()`` passes ``name`` and optional
    ``kind`` and expects a ``PilResponse<Vec<SymbolDef>>``.
    """
    state = _get_state(request)
    graph = state.symbol_graph

    symbols = graph.find_symbol(name)
    if kind:
        symbols = [s for s in symbols if s.kind == kind]

    items = [
        SymbolDefItem(
            name=s.name,
            kind=s.kind,
            file_path=s.file_path,
            line=s.line,
            column=s.column,
        )
        for s in symbols
    ]

    return {"success": True, "data": [m.model_dump() for m in items]}


# ── semantic search ───────────────────────────────────────────────────────────


@router.get("/search/semantic")
async def search_semantic(
    request: Request,
    q: str = Query(..., description="Natural-language query string."),
    top_k: int = Query(10, ge=1, le=100, description="Maximum results."),
) -> dict:
    """Semantic search across all indexed workspace documents.

    The kernel's ``PilClient.search_semantic()`` passes ``q`` and optional
    ``top_k`` and expects a ``PilResponse<Vec<SemanticSearchResult>>``.
    """
    state = _get_state(request)
    searcher = state.semantic_search

    results = searcher.search(q, top_k=top_k)
    items = [
        SemanticSearchResultItem(
            title=r.document.title,
            content=r.document.content,
            file_path=r.document.file_path,
            score=r.score,
        )
        for r in results
    ]

    return {"success": True, "data": [m.model_dump() for m in items]}


# ── indexer status (internal / debugging) ─────────────────────────────────────


@router.get("/indexer/status")
async def indexer_status(request: Request) -> dict:
    """Return basic indexer statistics for debugging."""
    state = _get_state(request)
    idx = state.indexer
    return {
        "success": True,
        "data": {
            "total_files": idx.count(),
            "extensions": sorted({f.ext for f in idx.files}),
        },
    }


# ── dependency graph ────────────────────────────────────────────────────────────


@router.get("/dependency/resolve")
async def dependency_resolve(
    request: Request,
    name: str = Query(..., description="Module or file name to query (substring match)."),
) -> dict:
    """Resolve dependencies and dependents for a given module/file name.

    The kernel's ``PilClient`` calls this to understand what imports a
    particular module and what it imports in turn.
    """
    state = _get_state(request)
    graph = state.dependency_graph
    resolved = graph.resolve(name)
    return {
        "success": True,
        "data": ResolvedDependenciesItem(
            dependents=[
                DependencyEdgeItem(
                    source_file=e.source_file,
                    source_line=e.source_line,
                    target=e.target,
                    kind=e.kind,
                )
                for e in resolved.dependents
            ],
            dependencies=[
                DependencyEdgeItem(
                    source_file=e.source_file,
                    source_line=e.source_line,
                    target=e.target,
                    kind=e.kind,
                )
                for e in resolved.dependencies
            ],
        ).model_dump(),
    }


@router.get("/dependency/graph")
async def dependency_graph_endpoint(
    request: Request,
) -> dict:
    """Return full dependency graph statistics and all edges.

    Useful for programmatic analysis and dashboard visualisation.
    """
    state = _get_state(request)
    graph = state.dependency_graph
    s = graph.stats()
    edges = graph.all_edges()
    return {
        "success": True,
        "data": {
            "stats": DependencyGraphStatsItem(
                node_count=s.node_count,
                edge_count=s.edge_count,
                languages=s.languages,
            ).model_dump(),
            "edges": [
                DependencyEdgeItem(
                    source_file=e.source_file,
                    source_line=e.source_line,
                    target=e.target,
                    kind=e.kind,
                )
                for e in edges
            ],
        },
    }


# ── plan / decompose ────────────────────────────────────────────────────────────


@router.post("/plan/decompose")
async def plan_decompose(body: DecomposeRequest, request: Request) -> DecomposeResponse:
    """Decompose a business objective into an execution plan.

    Uses the GoalDecomposer with the configured LLM client (OpenAI-compatible
    when ``OPENAI_API_KEY`` or ``OPENROUTER_API_KEY`` is set, otherwise a
    deterministic mock for local development).  Produces an immutable
    ``ExecutionPlan`` proposal.  The Kernel independently validates this
    before admission.
    """
    try:
        from planner.decomposer import GoalDecomposer

        state = _get_state(request)
        decomposer = GoalDecomposer(llm_client=state.llm_client)
        plan = decomposer.decompose(body.objective, context=body.context or {})
    except ValueError as e:
        return DecomposeResponse(success=False, error=str(e))
    except Exception as e:
        return DecomposeResponse(success=False, error=f"Unexpected error: {e}")

    return DecomposeResponse(
        success=True,
        data=ExecutionPlanItem(
            plan_id=plan.plan_id,
            version=plan.version,
            supersedes=plan.supersedes,
            objective_description=plan.objective_description,
            rationale=plan.rationale,
            objectives=[
                ObjectiveItem(
                    id=o.id,
                    title=o.title,
                    description=o.description,
                    owning_domain=o.owning_domain,
                    priority=o.priority.value,
                    dependencies=o.dependencies,
                    success_criteria=[
                        SuccessCriteriaItem(
                            description=c.description,
                            verification_hint=c.verification_hint,
                        )
                        for c in o.success_criteria
                    ],
                    risks=[
                        RiskAnnotationItem(
                            category=r.category,
                            description=r.description,
                            level=r.level.value,
                            affected_objective_ids=r.affected_objective_ids,
                        )
                        for r in o.risks
                    ],
                    status=o.status.value,
                )
                for o in plan.objectives
            ],
            plan_level_risks=[
                RiskAnnotationItem(
                    category=r.category,
                    description=r.description,
                    level=r.level.value,
                    affected_objective_ids=r.affected_objective_ids,
                )
                for r in plan.plan_level_risks
            ],
            created_at=plan.created_at,
            content_hash=plan.content_hash,
        ),
    )


# ── plan / admit ──────────────────────────────────────────────────────────────────


@router.post("/plan/admit")
async def plan_admit(body: AdmitRequest, request: Request) -> AdmitResponse:
    """Validate and admit a decomposed execution plan.

    Runs structural, domain, DAG, criteria, risk, and constitution-alignment
    checks on the submitted plan.  Returns an ``AdmissionVerdict`` — admission
    passes only when there are zero error-severity issues.
    """
    try:
        from planner.admission import PlanAdmissionService
        from planner.models import (
            ExecutionPlan as PlannerExecutionPlan,
            Objective as PlannerObjective,
            RiskAnnotation as PlannerRiskAnnotation,
            RiskLevel as PlannerRiskLevel,
            SuccessCriteria as PlannerSuccessCriteria,
        )

        # Reconstruct the planner model from the API item.
        objectives: list[PlannerObjective] = []
        for oi in body.plan.objectives:
            obj = PlannerObjective(
                id=oi.id,
                title=oi.title,
                description=oi.description,
                owning_domain=oi.owning_domain,
                priority=oi.priority,
                dependencies=oi.dependencies,
                success_criteria=[
                    PlannerSuccessCriteria(
                        description=c.description,
                        verification_hint=c.verification_hint,
                    )
                    for c in oi.success_criteria
                ],
                risks=[
                    PlannerRiskAnnotation(
                        category=r.category,
                        description=r.description,
                        level=PlannerRiskLevel(r.level),
                        affected_objective_ids=r.affected_objective_ids,
                    )
                    for r in oi.risks
                ],
                status=oi.status,
            )
            objectives.append(obj)

        plan = PlannerExecutionPlan(
            plan_id=body.plan.plan_id,
            version=body.plan.version,
            supersedes=body.plan.supersedes,
            objective_description=body.plan.objective_description,
            rationale=body.plan.rationale,
            objectives=objectives,
            plan_level_risks=[
                PlannerRiskAnnotation(
                    category=r.category,
                    description=r.description,
                    level=PlannerRiskLevel(r.level),
                    affected_objective_ids=r.affected_objective_ids,
                )
                for r in body.plan.plan_level_risks
            ],
            created_at=body.plan.created_at,
            content_hash=body.plan.content_hash,
        )

        state = _get_state(request)
        service = PlanAdmissionService()
        verdict = service.admit(
            plan,
            known_domains=body.known_domains,
            constitution_engine=state.constitution_engine,
        )
    except Exception as e:
        return AdmitResponse(success=False, error=f"Admission error: {e}")

    return AdmitResponse(
        success=True,
        data=AdmissionVerdictItem(
            passed=verdict.passed,
            plan_id=verdict.plan_id,
            issues=[
                AdmissionIssueItem(
                    severity=i.severity.value,
                    category=i.category,
                    objective_id=i.objective_id,
                    message=i.message,
                    rule_ref=i.rule_ref,
                )
                for i in verdict.issues
            ],
            reviewed_at=verdict.reviewed_at,
        ),
    )


# ── plan / submit ──────────────────────────────────────────────────────────────────


@router.post("/plan/submit")
async def plan_submit(body: SubmitRequest, request: Request) -> SubmitResponse:
    """Submit an admitted execution plan to the Kernel for execution.

    Creates each objective from the plan in the Kernel via
    ``POST /api/v1/objectives``.  When an admission verdict is provided
    and ``passed`` is false, the request is rejected with a 400-like
    error response.
    """
    # Reject plans that have not passed admission
    if body.verdict is not None and not body.verdict.passed:
        return SubmitResponse(
            success=False,
            error=(
                f"Plan '{body.plan.plan_id}' has not passed admission "
                f"(found {len(body.verdict.issues)} issue(s)). "
                "Run POST /api/v1/plan/admit and resolve errors first."
            ),
        )

    try:
        from intelligence.kernel_client import KernelClient

        client = KernelClient()
        objectives_dicts = [o.model_dump() for o in body.plan.objectives]
        result = client.submit_plan(
            plan_id=body.plan.plan_id,
            objectives=objectives_dicts,
            plan_level_risks=[r.model_dump() for r in body.plan.plan_level_risks],
        )
    except Exception as e:
        return SubmitResponse(success=False, error=f"Submission error: {e}")

    has_errors = len(result.get("errors", [])) > 0
    return SubmitResponse(
        success=not has_errors,
        data=result,
    )
