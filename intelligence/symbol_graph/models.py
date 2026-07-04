"""Pydantic models for the symbol graph module.

Defines data shapes for symbol definitions, cross-file references,
and :class:`SymbolGraph` configuration.
"""
from __future__ import annotations

from pathlib import Path

from pydantic import BaseModel, Field


class SymbolDef(BaseModel):
    """A single symbol definition extracted from a source file.

    Tracks the symbol's name, its syntactic kind, and its exact location
    (file, line, column) so downstream tools can jump to the definition,
    build a call graph, or perform impact analysis.
    """

    name: str = Field(..., description="Symbol name.")
    kind: str = Field(
        ...,
        description=(
            "Symbol kind — one of: ``function``, ``struct``, ``class``, "
            "``enum``, ``trait``, ``const``, ``heading``, ``key``."
        ),
    )
    file_path: str = Field(..., description="Absolute file path.")
    line: int = Field(..., ge=0, description="1-based line number.")
    column: int = Field(..., ge=0, description="0-based column offset.")
    docstring: str | None = Field(
        None,
        description="Associated doc-comment or docstring, if any.",
    )


class SymbolRef(BaseModel):
    """A cross-reference from one symbol to another (e.g. a call site).

    For the MVP the target fields (``to_file``, ``to_line``) may be
    ``None`` when the reference could not be resolved across files.
    """

    name: str = Field(..., description="Referenced symbol name.")
    from_file: str = Field(
        ...,
        description="File in which the reference appears.",
    )
    from_line: int = Field(
        ...,
        ge=0,
        description="Line number of the reference site.",
    )
    to_file: str | None = Field(
        None,
        description="Resolved target file, or ``None`` if unresolved.",
    )
    to_line: int | None = Field(
        None,
        description="Resolved target line, or ``None`` if unresolved.",
    )


class SymbolGraphConfig(BaseModel):
    """Configuration for the :class:`~intelligence.symbol_graph.graph.SymbolGraph`.

    Attributes:
        root_dir: Root directory for file discovery.
        language_parsers: Mapping from file extension (without leading dot)
            to a language identifier used to select the appropriate parser
            or regex patterns.
    """

    root_dir: Path = Field(
        ...,
        description="Root directory for file discovery.",
    )
    language_parsers: dict[str, str] = Field(
        default_factory=lambda: {
            "rs": "rust",
            "py": "python",
            "json": "json",
            "md": "markdown",
        },
        description=(
            "Extension → language name mapping. "
            "The extension should omit the leading dot (e.g. ``'rs'``)."
        ),
    )
