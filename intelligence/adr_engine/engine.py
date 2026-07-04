"""Core ADR engine — parses, indexes, and retrieves Architecture Decision Records.

Usage::

    from pathlib import Path
    from intelligence.adr_engine.engine import AdrEngine

    engine = AdrEngine(Path("adr"))
    count = engine.index()
    record = engine.get_by_id("0001")
    results = engine.search_text("kernel")
"""

from __future__ import annotations

from datetime import date as date_type
from datetime import datetime
from pathlib import Path
from typing import Any

from intelligence.adr_engine.models import AdrRecord, AdrSearchResult

try:
    import yaml as _yaml

    _HAS_YAML = True
except ImportError:  # pragma: no cover
    import json as _json

    _HAS_YAML = False


# -- helpers ------------------------------------------------------------------


def _parse_date(raw: Any) -> datetime | None:
    """Coerce *raw* (from YAML frontmatter) into ``datetime | None``.

    YAML ``date: 2026-07-02`` parses as ``datetime.date``, so we normalise that
    to a zero-time ``datetime``.
    """
    if raw is None:
        return None
    if isinstance(raw, datetime):
        return raw
    if isinstance(raw, date_type):
        return datetime(raw.year, raw.month, raw.day)
    if isinstance(raw, str):
        # Try a few common ISO-ish formats.
        for fmt in ("%Y-%m-%d", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S"):
            try:
                return datetime.strptime(raw, fmt)
            except ValueError:
                continue
    return None


def _parse_frontmatter(text: str) -> tuple[dict[str, Any], str]:
    """Split *text* into ``(frontmatter_dict, body_str)``.

    Expects the standard Jekyll-style ``--- ... ---`` frontmatter block at the
    start of the file.  If the block is absent or malformed the whole file is
    treated as body and an empty dict is returned.
    """
    stripped = text.lstrip("\ufeff")  # strip BOM if present
    if not stripped.startswith("---"):
        return {}, text

    # Find the closing ``---``
    end_idx = stripped.find("---", 3)
    if end_idx == -1:
        return {}, text

    fm_block = stripped[3:end_idx].strip()
    body = stripped[end_idx + 3 :].lstrip("\n")

    if not fm_block:
        return {}, body

    if _HAS_YAML:
        try:
            frontmatter: dict[str, Any] = _yaml.safe_load(fm_block) or {}
        except _yaml.YAMLError:
            frontmatter = {}
    else:  # pragma: no cover
        # Fallback: parse manually or via json (keys are simple scalars).
        try:
            frontmatter = dict(line.split(":", 1) for line in fm_block.splitlines() if ":" in line)
        except Exception:
            frontmatter = {}

    if not isinstance(frontmatter, dict):
        frontmatter = {}

    return frontmatter, body


def _filename_to_adr_id(path: Path) -> str:
    """Return the stem of the ADR file, e.g. ``ADR-001`` from ``ADR-001.md``."""
    return path.stem


# -- engine -------------------------------------------------------------------


class AdrEngine:
    """Read-only index of Architecture Decision Records.

    Call :meth:`index` once to populate, then use the retrieval methods.
    Calling :meth:`index` again re-scans the entire directory.
    """

    def __init__(self, adr_dir: Path) -> None:
        self._adr_dir = adr_dir
        self._records: list[AdrRecord] = []
        # Lookup maps — by frontmatter adr_id AND by filename stem.
        self._by_id: dict[str, AdrRecord] = {}

    # -- indexing -------------------------------------------------------------

    def index(self) -> int:
        """Scan ``adr_dir`` for ``*.md`` files and populate the index.

        Returns the number of successfully parsed records.
        """
        self._records.clear()
        self._by_id.clear()

        if not self._adr_dir.is_dir():
            return 0

        for md_file in sorted(self._adr_dir.glob("*.md")):
            record = self._parse_file(md_file)
            if record is not None:
                self._records.append(record)
                self._by_id[record.adr_id] = record
                # Also index by filename stem (e.g. "ADR-001").
                stem_id = _filename_to_adr_id(md_file)
                if stem_id != record.adr_id:
                    self._by_id[stem_id] = record

        return len(self._records)

    def _parse_file(self, path: Path) -> AdrRecord | None:
        """Parse a single markdown file into an :class:`AdrRecord`.

        Returns ``None`` for unreadable files.
        """
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            return None

        frontmatter, body = _parse_frontmatter(text)

        # Extract well-known fields with defaults.
        adr_id = str(frontmatter.get("adr_id", _filename_to_adr_id(path)))
        title = str(frontmatter.get("title", ""))
        status = str(frontmatter.get("status", "unknown"))
        date = _parse_date(frontmatter.get("date"))

        tags_raw = frontmatter.get("tags", [])
        if isinstance(tags_raw, list):
            tags = [str(t) for t in tags_raw]
        else:
            tags = [str(tags_raw)] if tags_raw else []

        af_raw = frontmatter.get("affected_domains", [])
        if isinstance(af_raw, list):
            affected_domains = [str(d) for d in af_raw]
        else:
            affected_domains = [str(af_raw)] if af_raw else []

        return AdrRecord(
            adr_id=adr_id,
            title=title,
            status=status,
            date=date,
            tags=tags,
            affected_domains=affected_domains,
            body=body,
            file_path=str(path.resolve()),
            frontmatter=frontmatter,
        )

    # -- retrieval ------------------------------------------------------------

    def get_by_id(self, adr_id: str) -> AdrRecord | None:
        """Look up a record by its ADR ID.

        Matches against the frontmatter ``adr_id`` field *or* the filename stem
        (e.g. ``"ADR-001"``).
        """
        return self._by_id.get(adr_id)

    def search_by_tag(self, tag: str) -> list[AdrRecord]:
        """Return records whose ``tags`` list contains *tag* (case-insensitive)."""
        lower = tag.lower()
        return [r for r in self._records if any(t.lower() == lower for t in r.tags)]

    def search_by_status(self, status: str) -> list[AdrRecord]:
        """Return records matching *status* (case-insensitive)."""
        lower = status.lower()
        return [r for r in self._records if r.status.lower() == lower]

    def search_text(self, query: str) -> list[AdrSearchResult]:
        """Basic keyword search over ``title`` + ``body``.

        Returns records where *query* is a case-insensitive substring of either
        the title or the body, with a ``match_reason`` explaining which field
        matched.
        """
        lower = query.lower()
        results: list[AdrSearchResult] = []

        for record in self._records:
            reasons: list[str] = []
            if lower in record.title.lower():
                reasons.append("query found in title")
            if lower in record.body.lower():
                reasons.append("query found in body")

            if reasons:
                results.append(
                    AdrSearchResult(record=record, match_reason="; ".join(reasons))
                )

        return results

    def all(self) -> list[AdrRecord]:
        """Return every indexed record, in file-sorted order."""
        return list(self._records)

    def count(self) -> int:
        """Return the number of indexed records."""
        return len(self._records)
