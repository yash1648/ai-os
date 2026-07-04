"""
Constitution engine — parse and index constitution rules from markdown files.

The constitution is a directory of markdown files that encode project policies.
This module:
  - Loads and indexes constitution rules from the ``constitution/`` directory
  - Provides read-only query and search over sections and rules
  - Exposes the dual representation: full prose (section body) and atomic rules

Public API:
  - :class:`ConstitutionEngine` — the indexing and query interface
  - :class:`ConstitutionSection` — a single markdown file's content + rules
  - :class:`ConstitutionRule` — a single atomic rule extracted from a section
"""

from intelligence.constitution_engine.engine import ConstitutionEngine
from intelligence.constitution_engine.models import ConstitutionRule, ConstitutionSection

__all__ = [
    "ConstitutionEngine",
    "ConstitutionRule",
    "ConstitutionSection",
]
