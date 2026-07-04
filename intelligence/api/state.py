"""
Application state — lazily initialised engine instances.

Each engine is created and indexed on first access so the server starts
quickly even when ML dependencies (sentence-transformers, FAISS) are
present but heavy.
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import Optional

from intelligence.adr_engine.engine import AdrEngine
from intelligence.constitution_engine.engine import ConstitutionEngine
from intelligence.dependency_graph.engine import DependencyGraph
from intelligence.dependency_graph.models import DependencyGraphConfig
from intelligence.indexer.indexer import Indexer
from intelligence.indexer.models import IndexerConfig
from intelligence.semantic_search.models import SearchConfig, SearchDocument
from intelligence.semantic_search.searcher import SemanticSearch
from intelligence.symbol_graph.graph import SymbolGraph
from intelligence.symbol_graph.models import SymbolGraphConfig


class PilState:
    """Holds all PIL engine instances, lazily initialised.

    The *project_root* determines where the engines look for data::

        state = PilState(Path("/path/to/project"))
        adrs = state.adr_engine.search_text("kernel")   # indexes on first call
    """

    def __init__(self, project_root: str | Path | None = None) -> None:
        if project_root is None:
            project_root = Path.cwd()
        self._root = Path(project_root).resolve()

        # Engine instances (None = not yet initialised)
        self._adr: Optional[AdrEngine] = None
        self._constitution: Optional[ConstitutionEngine] = None
        self._symbol_graph: Optional[SymbolGraph] = None
        self._dependency_graph: Optional[DependencyGraph] = None
        self._semantic_search: Optional[SemanticSearch] = None
        self._indexer: Optional[Indexer] = None

    # ------------------------------------------------------------------
    # Public helpers
    # ------------------------------------------------------------------

    @property
    def root(self) -> Path:
        """Project root directory passed at construction."""
        return self._root

    # ------------------------------------------------------------------
    # Lazy engine properties
    # ------------------------------------------------------------------

    @property
    def adr_engine(self) -> AdrEngine:
        """Lazily initialised ADR engine, rooted at ``root / "docs/adr"``."""
        if self._adr is None:
            adr_dir = self._find_dir(["docs/adr", "adr", "decisions"])
            self._adr = AdrEngine(adr_dir)
            self._adr.index()
        return self._adr

    @property
    def constitution_engine(self) -> ConstitutionEngine:
        """Lazily initialised constitution engine, rooted at ``root / "constitution"``."""
        if self._constitution is None:
            const_dir = self._find_dir(["constitution", "policy"])
            self._constitution = ConstitutionEngine(const_dir)
            self._constitution.index()
        return self._constitution

    @property
    def symbol_graph(self) -> SymbolGraph:
        """Lazily initialised symbol graph over the entire project tree."""
        if self._symbol_graph is None:
            cfg = SymbolGraphConfig(root_dir=self._root)
            self._symbol_graph = SymbolGraph(cfg)
            self._symbol_graph.index_files()
        return self._symbol_graph

    @property
    def dependency_graph(self) -> DependencyGraph:
        """Lazily initialised dependency graph over the project tree."""
        if self._dependency_graph is None:
            cfg = DependencyGraphConfig(root_dir=self._root)
            self._dependency_graph = DependencyGraph(cfg)
            self._dependency_graph.index_files()
        return self._dependency_graph

    @property
    def indexer(self) -> Indexer:
        """Lazily initialised file indexer over the project tree."""
        if self._indexer is None:
            cfg = IndexerConfig(root_dir=self._root)
            self._indexer = Indexer(cfg)
            self._indexer.index()
        return self._indexer

    @property
    def semantic_search(self) -> SemanticSearch:
        """Lazily initialised semantic search engine.

        On first access, documents are collected from the ADR engine,
        constitution engine, and a sample of indexed project files.
        """
        if self._semantic_search is None:
            self._semantic_search = SemanticSearch(SearchConfig())
            self._populate_semantic_search()
        return self._semantic_search

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------

    def _find_dir(self, candidates: list[str]) -> Path:
        """Return the first existing directory under *root* from *candidates*,
        falling back to ``root / candidates[0]`` regardless of existence.
        """
        for rel in candidates:
            candidate = self._root / rel
            if candidate.is_dir():
                return candidate
        return self._root / candidates[0]

    def _populate_semantic_search(self) -> None:
        """Feed documents from ADRs, constitution, and indexed files into the
        semantic search engine so the ``/search/semantic`` endpoint returns
        meaningful results.
        """
        docs: list[SearchDocument] = []

        # ADR documents
        for record in self.adr_engine.all():
            docs.append(
                SearchDocument(
                    id=record.adr_id,
                    title=record.title,
                    content=record.body,
                    source="adr",
                    file_path=record.file_path,
                )
            )

        # Constitution documents
        for section in self.constitution_engine.all_sections():
            docs.append(
                SearchDocument(
                    id=section.filename,
                    title=section.title,
                    content=section.body,
                    source="constitution",
                    file_path=str(self._root / "constitution" / section.filename),
                )
            )

        # Indexed source files — include a representative sample
        # (limit body to first 4 KB to avoid exploding token counts).
        indexed_files = self.indexer.files
        for f in indexed_files:
            path = Path(f.path)
            try:
                content = path.read_text(encoding="utf-8", errors="replace")[:4096]
            except (OSError, UnicodeDecodeError):
                continue
            docs.append(
                SearchDocument(
                    id=f.rel_path,
                    title=f.name,
                    content=content,
                    source="doc",
                    file_path=f.path,
                )
            )

        if docs:
            self._semantic_search.index_documents(docs)

    def refresh(self) -> dict[str, int]:
        """Re-index all engines and return per-engine document/symbol counts.

        Useful after workspace changes to keep the PIL state current.
        """
        counts: dict[str, int] = {}

        self._adr = None
        counts["adr"] = self.adr_engine.count()

        self._constitution = None
        counts["constitution"] = self.constitution_engine.count()

        self._symbol_graph = None
        counts["symbols"] = self.symbol_graph.count()

        self._indexer = None
        counts["indexed_files"] = self.indexer.count()

        self._dependency_graph = None
        counts["dep_edges"] = self.dependency_graph.stats().edge_count

        self._semantic_search = None
        counts["search_docs"] = self.semantic_search.count()

        return counts
