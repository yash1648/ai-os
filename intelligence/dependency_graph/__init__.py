"""Dependency graph — import-level dependency analysis for project files.

Analyses source files to extract import statements and build a directed
dependency graph.  Supports Python (``import`` / ``from``), Rust (``use``),
TypeScript (ESM ``import``), and JavaScript (``require`` / ESM).

Exports
-------
- :class:`DependencyGraph` — the main parsing and indexing engine
- :class:`DependencyEdge` — a single import edge between files
- :class:`DependencyGraphConfig` — configuration for :class:`DependencyGraph`
- :class:`DependencyGraphStats` — aggregate graph statistics
- :class:`ResolvedDependencies` — incoming and outgoing edges for a query
"""

from intelligence.dependency_graph.engine import DependencyGraph
from intelligence.dependency_graph.models import (
    DependencyEdge,
    DependencyGraphConfig,
    DependencyGraphStats,
    ResolvedDependencies,
)

__all__ = [
    "DependencyEdge",
    "DependencyGraph",
    "DependencyGraphConfig",
    "DependencyGraphStats",
    "ResolvedDependencies",
]
