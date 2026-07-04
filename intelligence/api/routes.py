"""
Query route definitions for the PIL sidecar.

All intelligence endpoints are wired to their respective engines via
:class:`~intelligence.api.state.PilState`, which is attached to
``request.app.state.pil_state`` during server startup.
"""

from __future__ import annotations

from fastapi import APIRouter, Query, Request

from intelligence.api.models import (
    AdrSearchResultItem,
    ConstitutionSectionItem,
    PilHealth,
    SemanticSearchResultItem,
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
