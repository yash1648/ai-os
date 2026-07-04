"""
ADR engine — queries and reasons over Architecture Decision Records.

Responsibilities:
  - Parsing and indexing ADR markdown files with frontmatter
  - Searching ADRs by status, tag, date, or content
  - Answering questions grounded in the ADR corpus
  - Detecting conflicts between existing decisions and proposed changes
"""

from __future__ import annotations

from intelligence.adr_engine.engine import AdrEngine
from intelligence.adr_engine.models import AdrRecord, AdrSearchResult

__all__ = [
    "AdrEngine",
    "AdrRecord",
    "AdrSearchResult",
]
