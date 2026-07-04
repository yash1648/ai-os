"""Dependency graph — import-level dependency analysis for source files.

The :class:`DependencyGraph` engine scans project files, extracts import
statements using language-specific regex patterns, and builds a directed
graph from source files to their dependencies.

Typical usage::

    from intelligence.dependency_graph.engine import DependencyGraph
    from intelligence.dependency_graph.models import DependencyGraphConfig

    graph = DependencyGraph(DependencyGraphConfig(root_dir=Path(".")))
    total = graph.index_files()
    deps = graph.resolve("intelligence/main.py")
"""

from __future__ import annotations

import re
from pathlib import Path

from intelligence.dependency_graph.models import (
    DependencyEdge,
    DependencyGraphConfig,
    DependencyGraphStats,
    ResolvedDependencies,
)

# ---------------------------------------------------------------------------
# Regex patterns per language
# ---------------------------------------------------------------------------
# Each entry is a list of ``(compiled_pattern, kind_string)``.  Named groups
# capture:
#   * ``module`` — the leaf module or symbol being imported
#   * ``pkg``    — the package prefix in ``from X import Y`` / ``use X::{Y}``

_IMPORT_PATTERNS: dict[str, list[tuple[re.Pattern, str]]] = {
    "python": [
        # ``import os`` → group 1 = ``os``
        (re.compile(r"^import\s+([a-zA-Z_]\w*)", re.MULTILINE), "import"),
        # ``from pathlib import Path`` → group 1 = ``pathlib``
        (re.compile(r"^from\s+([\w.]+)\s+import", re.MULTILINE), "from-import"),
    ],
    "rust": [
        # Simple ``use std::collections::HashMap`` — NOT ``use X::{…}``
        # The ``;?$`` end anchor prevents matching group imports like ``use X::{Y}``.
        (re.compile(r"^use\s+(\w+(?:::\w+)*)(?:\s+as\s+\w+)?;?$", re.MULTILINE), "use"),
        # ``use crate::event_bus::{Event, Bus}``
        (re.compile(r"^use\s+(\w+(?:::\w+)*)::\{(.+)\}", re.MULTILINE), "use-group"),
    ],
    "typescript": [
        # ``import { readFile } from 'fs'``
        (re.compile(r"^import\s+\{[^}]*\}\s+from\s+['\"]([^'\"]+)['\"]", re.MULTILINE), "import"),
        # ``import path from 'path-browserify'``
        (re.compile(r"^import\s+\w+\s+from\s+['\"]([^'\"]+)['\"]", re.MULTILINE), "import"),
        # ``import 'something'`` (side-effect import)
        (re.compile(r"^import\s+['\"]([^'\"]+)['\"]", re.MULTILINE), "import"),
    ],
    "javascript": [
        # ``const fs = require('fs')``
        (re.compile(r"(?:const|let|var)\s+\w+\s*=\s*require\(['\"]([^'\"]+)['\"]\)", re.MULTILINE), "require"),
        # ``import { readFile } from 'fs'``
        (re.compile(r"^import\s+\{[^}]*\}\s+from\s+['\"]([^'\"]+)['\"]", re.MULTILINE), "import"),
        # ``import path from 'path-browserify'``
        (re.compile(r"^import\s+\w+\s+from\s+['\"]([^'\"]+)['\"]", re.MULTILINE), "import"),
    ],
}


def _extract_import_targets(
    lang: str, content: str, file_path: Path
) -> list[DependencyEdge]:
    patterns = _IMPORT_PATTERNS.get(lang, [])
    if not patterns:
        return []

    edges: list[DependencyEdge] = []
    resolved = str(file_path.resolve())

    for pattern, kind in patterns:
        for match in pattern.finditer(content):
            line_num = content[: match.start()].count("\n") + 1

            if kind == "use-group":
                pkg = match.group(1)
                items = [item.strip() for item in match.group(2).split(",")]
                for item in items:
                    edges.append(
                        DependencyEdge(
                            source_file=resolved,
                            source_line=line_num,
                            target=_canonicalise_target(f"{pkg}::{item}"),
                            kind="use",
                        )
                    )
                continue

            target = _canonicalise_target(match.group(1))
            edges.append(
                DependencyEdge(
                    source_file=resolved,
                    source_line=line_num,
                    target=target,
                    kind=kind,
                )
            )

    seen: set[tuple[str, str]] = set()
    unique: list[DependencyEdge] = []
    for e in edges:
        key = (e.target, e.kind)
        if key not in seen:
            seen.add(key)
            unique.append(e)
    return unique


def _canonicalise_target(raw: str) -> str:
    """Strip surrounding whitespace and quotes from a captured module name."""
    return raw.strip().strip("\"'")


# ---------------------------------------------------------------------------
# DependencyGraph
# ---------------------------------------------------------------------------


class DependencyGraph:
    """Import-level dependency graph for a project.

    Scans source files, extracts import statements per language, and
    builds a directed graph where nodes are files and edges are imports.

    .. rubric:: Life-cycle

    1. Instantiate with a :class:`DependencyGraphConfig`.
    2. Call :meth:`index_files` to populate the graph.
    3. Query with :meth:`resolve`, :meth:`dependents_of`, :meth:`dependencies_of`,
       :meth:`stats`, or :meth:`all_edges`.
    """

    def __init__(self, config: DependencyGraphConfig) -> None:
        self._config = config
        # Edges indexed by source file (absolute path → list of edges).
        self._edges_by_source: dict[str, list[DependencyEdge]] = {}
        # Reverse index: target → list of edges pointing to it.
        self._edges_by_target: dict[str, list[DependencyEdge]] = {}
        # Language set (file extension → language).
        self._languages: set[str] = set()

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def index_files(self, file_paths: list[Path] | None = None) -> int:
        """Scan *file_paths* (or auto-discover from the configured *root_dir*)
        and build the dependency graph.

        Returns the total number of dependency edges found.
        """
        if file_paths is None:
            file_paths = self._discover_files()

        total = 0
        for fp in file_paths:
            edges = self._index_file(fp)
            total += len(edges)
        return total

    def resolve(self, target_name: str) -> ResolvedDependencies:
        """Return both incoming (dependents) and outgoing (dependencies)
        edges for *target_name*.

        The *target_name* is matched as a case-insensitive substring against
        the ``target`` field of edges (so ``"event_bus"`` matches
        ``"crate::event_bus"``, ``"kernel.event_bus"``, etc.).
        """
        return ResolvedDependencies(
            dependents=self.dependents_of(target_name),
            dependencies=self.dependencies_of(target_name),
        )

    def dependents_of(self, target_name: str) -> list[DependencyEdge]:
        """Return edges where *target_name* appears in the ``target`` field
        (case-insensitive substring match).

        These are the files that **import** *target_name*.
        """
        lower = target_name.lower()
        result: list[DependencyEdge] = []
        for edges in self._edges_by_source.values():
            for e in edges:
                if lower in e.target.lower():
                    result.append(e)
        return result

    def dependencies_of(self, source_name: str) -> list[DependencyEdge]:
        """Return edges where *source_name* appears in the file *path* or
        in the ``source_file`` field (case-insensitive substring match).

        These are the files that **are imported by** *source_name*.
        """
        lower = source_name.lower()
        result: list[DependencyEdge] = []
        for source, edges in self._edges_by_source.items():
            if lower in source.lower():
                result.extend(edges)
            else:
                for e in edges:
                    if lower in e.source_file.lower():
                        result.append(e)
        return result

    def stats(self) -> DependencyGraphStats:
        """Return aggregate graph statistics."""
        all_nodes: set[str] = set()
        for source, edges in self._edges_by_source.items():
            all_nodes.add(source)
            for e in edges:
                all_nodes.add(e.target)
        edge_count = sum(len(edges) for edges in self._edges_by_source.values())
        return DependencyGraphStats(
            node_count=len(all_nodes),
            edge_count=edge_count,
            languages=sorted(self._languages),
        )

    def all_edges(self) -> list[DependencyEdge]:
        """Return every edge in the graph."""
        result: list[DependencyEdge] = []
        for edges in self._edges_by_source.values():
            result.extend(edges)
        return result

    # ------------------------------------------------------------------
    # Internal — indexing
    # ------------------------------------------------------------------

    def _index_file(self, file_path: Path) -> list[DependencyEdge]:
        """Parse imports from a single file and add them to the graph."""
        if not file_path.is_file():
            return []

        ext = file_path.suffix.lower().lstrip(".")
        lang = self._config.language_parsers.get(ext)
        if lang is None:
            return []

        self._languages.add(lang)

        try:
            content = file_path.read_text(encoding="utf-8", errors="replace")
        except (OSError, UnicodeDecodeError):
            return []

        edges = _extract_import_targets(lang, content, file_path)
        resolved = str(file_path.resolve())
        self._edges_by_source[resolved] = edges

        # Build reverse index.
        for e in edges:
            self._edges_by_target.setdefault(e.target, []).append(e)

        return edges

    # ------------------------------------------------------------------
    # Internal — file discovery
    # ------------------------------------------------------------------

    def _discover_files(self) -> list[Path]:
        """Use the :class:`~intelligence.indexer.indexer.Indexer` to
        list every file under the configured *root_dir*, filtering to
        only those whose extension is recognised.
        """
        from intelligence.indexer.indexer import Indexer
        from intelligence.indexer.models import IndexerConfig

        indexer = Indexer(IndexerConfig(root_dir=self._config.root_dir))
        indexed = indexer.index()
        exts = set(self._config.language_parsers.keys())
        return [
            Path(f.path)
            for f in indexed
            if f.ext.lower().lstrip(".") in exts
        ]
