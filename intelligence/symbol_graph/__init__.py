"""Symbol graph — tree-sitter based code parser.

Extracts functions, structs, classes, enums, traits, constants, JSON keys,
and Markdown headings from source files using tree-sitter (when available)
or a regex-based fallback.

Exports
-------
- :class:`SymbolGraph` — the main parsing and indexing engine
- :class:`SymbolDef` — a single symbol definition
- :class:`SymbolRef` — a cross-file symbol reference
- :class:`SymbolGraphConfig` — configuration for :class:`SymbolGraph`
"""

from intelligence.symbol_graph.graph import SymbolGraph
from intelligence.symbol_graph.models import SymbolDef, SymbolRef, SymbolGraphConfig

__all__ = [
    "SymbolDef",
    "SymbolGraph",
    "SymbolGraphConfig",
    "SymbolRef",
]
