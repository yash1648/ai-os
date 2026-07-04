"""Pydantic models for the constitution engine.

ConstitutionSection and ConstitutionRule represent the dual representation:
prose (full body) and extracted atomic rules from constitution markdown files.
"""

from __future__ import annotations as _annotations

from pydantic import BaseModel, Field


class ConstitutionRule(BaseModel):
    """A single atomic rule extracted from a constitution section.

    Attributes:
        text: The rule text extracted from the markdown (after the ``**Rule:**`` marker).
        source_section: The title of the section this rule was found in.
        line_number: The 1-indexed line number of the rule in the source file.
    """

    text: str = Field(description="The rule text extracted from the markdown")
    source_section: str = Field(description="The title of the section this rule was found in")
    line_number: int = Field(description="1-indexed line number of the rule in the source file")


class ConstitutionSection(BaseModel):
    """A single constitution markdown file with its full prose and extracted rules.

    Attributes:
        title: The section title (from the first ``# `` heading, or the filename stem).
        filename: The basename of the markdown file this section was loaded from.
        body: The full markdown content of the file.
        rules: All rules extracted from this section.
        word_count: Number of whitespace-delimited words in ``body``.
    """

    title: str = Field(description="Section title from first # heading or filename")
    filename: str = Field(description="Basename of the markdown file")
    body: str = Field(description="Full markdown content")
    rules: list[ConstitutionRule] = Field(default_factory=list, description="Extracted rules")
    word_count: int = Field(default=0, description="Number of words in body")
