"""ConstitutionEngine — parse and index constitution rules from markdown files.

The constitution is a directory of markdown files that encode project policies.
Each file is a "section" with a title (first ``# `` heading) and zero or more
atomic rules marked with ``**Rule:**`` (or ``- **Rule:**``). This engine
provides read-only indexing, search, and retrieval of those rules.
"""

from __future__ import annotations as _annotations

import re
from pathlib import Path

from intelligence.constitution_engine.models import ConstitutionRule, ConstitutionSection

# Pattern matching a rule marker at the start of a line:
#   optional whitespace, optional "- " list prefix, then "**Rule:**" followed by the rule text.
_RULE_PATTERN = re.compile(r"^\s*(?:-\s+)?\*\*Rule:\*\*\s*(.*)", re.MULTILINE)


class ConstitutionEngine:
    """Indexes and queries constitution markdown files.

    Usage::

        engine = ConstitutionEngine(Path("constitution"))
        count = engine.index()
        for section in engine.all_sections():
            ...
    """

    def __init__(self, constitution_dir: Path) -> None:
        """Store the path to the constitution directory.

        The directory is not read until :meth:`index` is called.
        """
        self._constitution_dir = constitution_dir
        self._sections: list[ConstitutionSection] = []
        self._sections_by_title: dict[str, ConstitutionSection] = {}

    # ------------------------------------------------------------------
    # Indexing
    # ------------------------------------------------------------------

    def index(self) -> int:
        """Scan *constitution_dir* for ``*.md`` files and index every section.

        For each file:
        1. Read the full content.
        2. Extract the title from the first ``# `` heading (or use the filename
           stem as a fallback).
        3. Find every line matching the ``**Rule:**`` or ``- **Rule:**`` pattern
           and create a :class:`ConstitutionRule` with the 1-indexed line number.
        4. Count whitespace-delimited words in the body.
        5. Build and store a :class:`ConstitutionSection`.

        Returns the number of sections indexed.
        """
        self._sections.clear()
        self._sections_by_title.clear()

        if not self._constitution_dir.is_dir():
            return 0

        md_files = sorted(self._constitution_dir.glob("*.md"))
        if not md_files:
            return 0

        for md_path in md_files:
            section = self._parse_file(md_path)
            self._sections.append(section)
            self._sections_by_title[section.title.lower()] = section

        return len(self._sections)

    def _parse_file(self, path: Path) -> ConstitutionSection:
        """Parse a single markdown file into a :class:`ConstitutionSection`."""
        body = path.read_text(encoding="utf-8")
        lines = body.splitlines()

        # --- title -----------------------------------------------------------
        title = self._extract_title(body, path)

        # --- rules -----------------------------------------------------------
        rules: list[ConstitutionRule] = []
        for line_index, line in enumerate(lines, start=1):
            match = _RULE_PATTERN.match(line)
            if match:
                rule_text = match.group(1).strip()
                if rule_text:
                    rules.append(
                        ConstitutionRule(
                            text=rule_text,
                            source_section=title,
                            line_number=line_index,
                        )
                    )

        # --- word count ------------------------------------------------------
        word_count = len(body.split())

        return ConstitutionSection(
            title=title,
            filename=path.name,
            body=body,
            rules=rules,
            word_count=word_count,
        )

    @staticmethod
    def _extract_title(body: str, path: Path) -> str:
        """Extract the section title from the first ``# `` heading.

        Falls back to the filename stem (without ``.md``) if no heading is
        found.
        """
        for line in body.splitlines():
            stripped = line.strip()
            if stripped.startswith("# "):
                return stripped.removeprefix("# ").strip()
        return path.stem

    # ------------------------------------------------------------------
    # Querying
    # ------------------------------------------------------------------

    def get_section(self, title: str) -> ConstitutionSection | None:
        """Look up a section by title (case-insensitive).

        Returns ``None`` if no matching section is found.
        """
        return self._sections_by_title.get(title.lower())

    def search_rules(self, keyword: str) -> list[ConstitutionRule]:
        """Return every rule whose ``text`` contains *keyword* (case-insensitive).

        Returns an empty list when no rules match.
        """
        lower = keyword.lower()
        return [rule for section in self._sections for rule in section.rules if lower in rule.text.lower()]

    def search_text(self, query: str) -> list[ConstitutionSection]:
        """Search section titles and bodies for *query* (case-insensitive).

        Returns every section where the *query* appears in the title or the
        full body markdown.
        """
        lower = query.lower()
        return [
            section
            for section in self._sections
            if lower in section.title.lower() or lower in section.body.lower()
        ]

    def all_sections(self) -> list[ConstitutionSection]:
        """Return all indexed sections."""
        return list(self._sections)

    def all_rules(self) -> list[ConstitutionRule]:
        """Return every extracted rule across all sections."""
        return [rule for section in self._sections for rule in section.rules]

    def count(self) -> int:
        """Return the number of indexed sections."""
        return len(self._sections)
