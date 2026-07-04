"""Symbol graph — extracts symbol definitions from source files.

Provides :class:`SymbolGraph`, a parser that wraps tree-sitter when
available and falls back to regex-based extraction for environments
without the native grammar bindings.

Typical usage::

    from intelligence.symbol_graph.graph import SymbolGraph
    from intelligence.symbol_graph.models import SymbolGraphConfig

    graph = SymbolGraph(SymbolGraphConfig(root_dir=Path(".")))
    total = graph.index_files()          # auto-discover + parse
    fns = graph.find_by_kind("function")  # filter
"""
from __future__ import annotations

import importlib
import re
from pathlib import Path
from typing import TYPE_CHECKING

from intelligence.symbol_graph.models import SymbolDef, SymbolGraphConfig

if TYPE_CHECKING:
    import tree_sitter as _ts

# ---------------------------------------------------------------------------
# Regex fallback patterns
# ---------------------------------------------------------------------------
# Each entry is ``(compiled_pattern, kind_string)``.  Group *1* captures the
# symbol name, with the sole exception of Markdown headings where group *2*
# holds the heading text.

_RE_PATTERNS: dict[str, list[tuple[re.Pattern, str]]] = {
    "rust": [
        (re.compile(r"(?:pub\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+([a-zA-Z_]\w*)"), "function"),
        (re.compile(r"(?:pub\s+)?struct\s+([A-Z]\w*)"), "struct"),
        (re.compile(r"(?:pub\s+)?enum\s+([A-Z]\w*)"), "enum"),
        (re.compile(r"(?:pub\s+)?trait\s+([A-Z]\w*)"), "trait"),
        (re.compile(r"(?:pub\s+)?const\s+([a-zA-Z_]\w*)"), "const"),
    ],
    "python": [
        (re.compile(r"(?:async\s+)?def\s+([a-zA-Z_]\w*)"), "function"),
        (re.compile(r"class\s+([A-Z]\w*)"), "class"),
    ],
    "json": [
        (re.compile(r'"([a-zA-Z_]\w*)"\s*:'), "key"),
    ],
    "markdown": [
        (re.compile(r"^(#{1,6})\s+(.+)", re.MULTILINE), "heading"),
    ],
}

# Comment markers used for basic docstring extraction in regex mode.
_COMMENT_PREFIX: dict[str, str | tuple[str, ...]] = {
    "rust": "///",
    "python": "#",
}

# ---------------------------------------------------------------------------
# tree-sitter node-type → kind mapping
# ---------------------------------------------------------------------------
# The keys are tree-sitter grammar node types; values are the *kind* strings
# stored in ``SymbolDef.kind``.

_TS_NODE_KINDS: dict[str, str] = {
    # Rust
    "function_item": "function",
    "struct_item": "struct",
    "enum_item": "enum",
    "trait_item": "trait",
    "const_item": "const",
    # Python
    "function_definition": "function",
    "class_definition": "class",
    # JSON
    "pair": "key",
    # Markdown
    "atx_heading": "heading",
}

# For each tree-sitter node type listed above, the field name that holds the
# symbol's name (``None`` means the ``"name"`` field).
_TS_NAME_FIELD: dict[str, str | None] = {
    "pair": "key",
    "atx_heading": "title",
}


# ===================================================================
# SymbolGraph
# ===================================================================


class SymbolGraph:
    """Extract symbol definitions from source files.

    Wraps tree-sitter parsing when the relevant grammar bindings are
    installed, and transparently falls back to line-based regex extraction
    when they are not — keeping the module usable in any Python >= 3.12
    environment with zero required C-extension dependencies.

    .. rubric:: Life-cycle

    1. Instantiate with a :class:`SymbolGraphConfig`.
    2. Call :meth:`index_file` or :meth:`index_files` to populate the graph.
    3. Query with :meth:`find_symbol`, :meth:`find_by_kind`,
       :meth:`get_file_symbols`, :meth:`all_symbols`, or :meth:`count`.
    """

    def __init__(self, config: SymbolGraphConfig) -> None:
        self._config = config
        self._symbols: list[SymbolDef] = []
        self._symbols_by_file: dict[str, list[SymbolDef]] = {}

        # tree-sitter state (best-effort)
        self._ts_available = False
        self._parsers: dict[str, object] = {}      # ext → ts.Parser
        self._ts_languages: dict[str, object] = {}  # ext → Language

        self._init_ts_parsers()

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def index_file(self, file_path: Path) -> list[SymbolDef]:
        """Parse a single *file_path* and return discovered symbols.

        The returned list is also added to the internal index so that
        query methods see it.
        """
        if not file_path.is_file():
            return []

        content = file_path.read_text(encoding="utf-8", errors="replace")
        ext = file_path.suffix.lower().lstrip(".")
        if ext not in self._config.language_parsers:
            return []

        symbols: list[SymbolDef] = []

        # Try tree-sitter first (when a parser was successfully created
        # for this extension during ``__init__``).
        if ext in self._parsers:
            try:
                symbols = self._parse_with_ts(ext, content, file_path)
            except Exception:  # noqa: BLE001
                symbols = []

        # Fall back to regex patterns.
        if not symbols:
            lang = self._config.language_parsers[ext]
            symbols = self._parse_with_regex(lang, content, file_path)

        resolved = str(file_path.resolve())
        self._symbols.extend(symbols)
        self._symbols_by_file.setdefault(resolved, []).extend(symbols)
        return symbols

    def index_files(self, file_paths: list[Path] | None = None) -> int:
        """Parse every file in *file_paths* (or auto-discover from the
        configured *root_dir*) and return the total symbol count.

        When *file_paths* is ``None`` (the default), the method uses the
        :class:`~intelligence.indexer.indexer.Indexer` to walk the
        repository and filters files by the extensions declared in
        :attr:`SymbolGraphConfig.language_parsers`.
        """
        if file_paths is None:
            file_paths = self._discover_files()

        total = 0
        for fp in file_paths:
            total += len(self.index_file(fp))
        return total

    def find_symbol(self, name: str) -> list[SymbolDef]:
        """Case-insensitive substring search over all indexed symbols.

        Returns every symbol whose *name* contains *name* (e.g.
        ``find_symbol("parse")`` will match ``"parse_json"``,
        ``"parse_xml"``, and ``"run_parse"``).
        """
        name_lower = name.lower()
        return [s for s in self._symbols if name_lower in s.name.lower()]

    def find_by_kind(self, kind: str) -> list[SymbolDef]:
        """Return symbols whose :attr:`SymbolDef.kind` matches exactly."""
        return [s for s in self._symbols if s.kind == kind]

    def get_file_symbols(self, file_path: str) -> list[SymbolDef]:
        """Return every symbol defined in *file_path* (resolved)."""
        norm = str(Path(file_path).resolve())
        return list(self._symbols_by_file.get(norm, []))

    def all_symbols(self) -> list[SymbolDef]:
        """Return a copy of every indexed symbol."""
        return list(self._symbols)

    def count(self) -> int:
        """Total number of symbols currently in the index."""
        return len(self._symbols)

    # ------------------------------------------------------------------
    # Internal — tree-sitter initialisation
    # ------------------------------------------------------------------

    def _init_ts_parsers(self) -> None:
        """Best-effort initialisation of tree-sitter parsers.

        Silently skips languages whose grammar bindings are not installed.
        """
        try:
            import tree_sitter as ts  # noqa: F811

            self._ts_available = True
        except ImportError:
            self._ts_available = False
            return

        for ext, lang_name in self._config.language_parsers.items():
            grammar_mod_name = f"tree_sitter_{lang_name}"
            try:
                mod = importlib.import_module(grammar_mod_name)
                # Modern tree-sitter (≥ 0.22) exposes ``.language()``
                # which returns a ``Language`` instance.
                language = mod.language()
                parser = ts.Parser(language)
                self._parsers[ext] = parser
                self._ts_languages[ext] = language
            except (ImportError, AttributeError, TypeError):
                continue

    # ------------------------------------------------------------------
    # Internal — tree-sitter parsing
    # ------------------------------------------------------------------

    def _parse_with_ts(
        self,
        ext: str,
        content: str,
        file_path: Path,
    ) -> list[SymbolDef]:
        """Walk the tree-sitter AST to find definition nodes."""
        import tree_sitter as ts

        parser: ts.Parser = self._parsers[ext]  # type: ignore[assignment]
        encoded = content.encode("utf-8")
        tree = parser.parse(encoded)
        symbols: list[SymbolDef] = []

        def _walk(node: ts.Node) -> None:
            kind = _TS_NODE_KINDS.get(node.type)
            if kind is not None:
                field = _TS_NAME_FIELD.get(node.type, "name")
                name_node = node.child_by_field_name(field)
                if name_node is not None:
                    name = encoded[
                        name_node.start_byte : name_node.end_byte
                    ].decode("utf-8", errors="replace")
                    row, col = name_node.start_point
                    doc = self._extract_ts_docstring(node, encoded)
                    symbols.append(
                        SymbolDef(
                            name=name,
                            kind=kind,
                            file_path=str(file_path.resolve()),
                            line=row + 1,
                            column=col,
                            docstring=doc,
                        )
                    )

            for child in node.children:
                _walk(child)

        try:
            _walk(tree.root_node)
        except Exception:  # noqa: BLE001
            return []

        return symbols

    @staticmethod
    def _extract_ts_docstring(
        node: _ts.Node,  # type: ignore[name-defined]
        encoded: bytes,
    ) -> str | None:
        """Best-effort docstring extraction from tree-sitter AST.

        Looks for a ``comment`` or ``doc_comment`` sibling immediately
        preceding the definition node.
        """
        try:
            prev = node.prev_sibling
            if prev is not None and prev.type in (
                "line_comment",
                "block_comment",
                "comment",
                "doc_comment",
            ):
                text = encoded[prev.start_byte : prev.end_byte].decode(
                    "utf-8", errors="replace"
                )
                return _strip_comment_prefix(text)
        except Exception:  # noqa: BLE001
            return None
        return None

    # ------------------------------------------------------------------
    # Internal — regex-based parsing
    # ------------------------------------------------------------------

    def _parse_with_regex(
        self,
        lang: str,
        content: str,
        file_path: Path,
    ) -> list[SymbolDef]:
        """Extract symbols using line-based regex patterns.

        Used as a fallback when tree-sitter is unavailable or fails.
        """
        patterns = _RE_PATTERNS.get(lang, [])
        if not patterns:
            return []

        symbols: list[SymbolDef] = []
        lines = content.split("\n")

        for pattern, kind in patterns:
            for match in pattern.finditer(content):
                # Position of the whole match (not just the name).
                line_num, col = _offset_to_line_col(content, match.start())

                if kind == "heading":
                    # Markdown heading — the name is in group 2.
                    name = match.group(2).strip()
                else:
                    name = match.group(1)

                doc = self._extract_docstring_regex(content, match, lines, lang)

                symbols.append(
                    SymbolDef(
                        name=name,
                        kind=kind,
                        file_path=str(file_path.resolve()),
                        line=line_num,
                        column=col,
                        docstring=doc,
                    )
                )

        return symbols

    @staticmethod
    def _extract_docstring_regex(
        content: str,  # noqa: ARG004  (kept for future use)
        match: re.Match,
        lines: list[str],
        lang: str,
    ) -> str | None:
        """Basic docstring extraction in regex mode.

        Scans upward from the line containing *match* and collects
        consecutive comment lines into a single docstring.
        """
        prefixes = _COMMENT_PREFIX.get(lang)
        if prefixes is None:
            return None
        if isinstance(prefixes, str):
            prefixes = (prefixes,)

        match_line = content[: match.start()].count("\n")

        doc_parts: list[str] = []
        # Walk backwards from the line *before* the match.
        for i in range(match_line - 1, -1, -1):
            raw = lines[i]
            stripped = raw.strip()
            if not stripped:
                continue  # skip blank lines while collecting
            matched_prefix = None
            for p in prefixes:
                if stripped.startswith(p):
                    matched_prefix = p
                    break
            if matched_prefix is not None:
                doc_parts.append(stripped[len(matched_prefix) :].strip())
            else:
                break  # non-comment line -> stop

        if not doc_parts:
            return None
        doc_parts.reverse()
        return " ".join(doc_parts)

    # ------------------------------------------------------------------
    # Internal — file discovery
    # ------------------------------------------------------------------

    def _discover_files(self) -> list[Path]:
        """Use the :class:`~intelligence.indexer.indexer.Indexer` to
        list every file under the configured *root_dir*, filtering to
        only those whose extension is recognised by the language-parser
        mapping.
        """
        from intelligence.indexer.indexer import Indexer
        from intelligence.indexer.models import IndexerConfig

        indexer = Indexer(
            IndexerConfig(root_dir=self._config.root_dir)
        )
        indexed = indexer.index()
        exts = set(self._config.language_parsers.keys())
        return [
            Path(f.path)
            for f in indexed
            if f.ext.lower().lstrip(".") in exts
        ]


# ===================================================================
# Module-level helpers
# ===================================================================


def _offset_to_line_col(content: str, offset: int) -> tuple[int, int]:
    """Convert a byte/char offset to (1-based line, 0-based column)."""
    line_num = content[:offset].count("\n") + 1
    last_nl = content.rfind("\n", 0, offset)
    col = offset - last_nl - 1 if last_nl != -1 else offset
    return line_num, col


def _strip_comment_prefix(text: str) -> str:
    """Strip common comment markers (``#``, ``//``, ``///``, ``/* … */``)
    from a comment line."""
    text = text.strip()
    if text.startswith("///"):
        text = text[3:].strip()
    elif text.startswith("//!"):
        text = text[3:].strip()
    elif text.startswith("//"):
        text = text[2:].strip()
    elif text.startswith("#"):
        text = text[1:].strip()
    # Strip surrounding /* … */  (single-line only)
    if text.startswith("/*") and text.endswith("*/"):
        text = text[2:-2].strip()
    elif text.startswith("/*"):
        text = text[2:].strip()
    elif text.endswith("*/"):
        text = text[:-2].strip()
    # Strip leading ``*`` on Javadoc-style lines
    if text.startswith("*"):
        text = text[1:].strip()
    return text
