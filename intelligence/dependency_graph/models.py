"""Pydantic models for the dependency graph module.

Defines data shapes for import-level dependencies between files
and the overall dependency graph structure.
"""

from __future__ import annotations

from pathlib import Path

from pydantic import BaseModel, Field


class DependencyEdge(BaseModel):
    """A single dependency from *source_file* importing from *target*.

    The *kind* field describes the type of import (e.g. ``"import"`` for
    ``import X``, ``"from-import"`` for ``from X import Y``).
    """

    source_file: str = Field(
        ..., description="Absolute path of the file that contains the import."
    )
    source_line: int = Field(..., ge=1, description="1-based line number of the import.")
    target: str = Field(
        ...,
        description=(
            "The imported module or symbol name, e.g. ``'os'``, ``'.models'``, "
            "``'crate::event_bus'``."
        ),
    )
    kind: str = Field(
        default="import",
        description=(
            "Import kind — one of ``'import'``, ``'from-import'``, "
            "``'use'``, ``'use-group'``."
        ),
    )


class DependencyGraphStats(BaseModel):
    """Aggregate statistics for the dependency graph."""

    node_count: int = Field(..., ge=0, description="Number of unique files in the graph.")
    edge_count: int = Field(..., ge=0, description="Number of dependency edges.")
    languages: list[str] = Field(
        default_factory=list,
        description="Distinct languages found in the graph (e.g. ``['python', 'rust']``).",
    )


class ResolvedDependencies(BaseModel):
    """Dependencies and dependents for a single symbol or file query."""

    dependents: list[DependencyEdge] = Field(
        default_factory=list,
        description="Files that import the target (incoming edges).",
    )
    dependencies: list[DependencyEdge] = Field(
        default_factory=list,
        description="Files the target imports from (outgoing edges).",
    )


class DependencyGraphConfig(BaseModel):
    """Configuration for the :class:`~intelligence.dependency_graph.engine.DependencyGraph`.

    Attributes:
        root_dir: Root directory for file discovery.
        language_parsers: Mapping from file extension (without leading dot)
            to a language identifier used to select the appropriate import
            regex patterns.
    """

    root_dir: Path = Field(
        ..., description="Root directory for file discovery."
    )
    language_parsers: dict[str, str] = Field(
        default_factory=lambda: {
            "py": "python",
            "rs": "rust",
            "ts": "typescript",
            "tsx": "typescript",
            "js": "javascript",
        },
        description=(
            "Extension → language name mapping. "
            "The extension should omit the leading dot (e.g. ``'py'``)."
        ),
    )
