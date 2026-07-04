"""Data models for the ADR engine.

Defines the shapes for parsed ADR records and search results.
"""

from __future__ import annotations

from datetime import datetime
from typing import Any

from pydantic import BaseModel, Field


class AdrRecord(BaseModel):
    """A single parsed Architecture Decision Record."""

    adr_id: str = Field(..., description="ADR identifier (e.g. '0001').")
    title: str = Field(..., description="Title of the decision.")
    status: str = Field(..., description="Decision status (e.g. 'accepted', 'proposed', 'deprecated').")
    date: datetime | None = Field(None, description="Date the ADR was authored or last updated.")
    tags: list[str] = Field(default_factory=list, description="Tags categorising the ADR.")
    affected_domains: list[str] = Field(
        default_factory=list, description="Domains or components affected by this decision."
    )
    body: str = Field(..., description="Full markdown body (everything after the frontmatter).")
    file_path: str = Field(..., description="Absolute path to the source markdown file.")
    frontmatter: dict[str, Any] = Field(
        default_factory=dict, description="Raw YAML frontmatter key-value pairs."
    )


class AdrSearchResult(BaseModel):
    """Result of a text search against the ADR corpus."""

    record: AdrRecord = Field(..., description="The matched ADR record.")
    match_reason: str = Field(..., description="Human-readable explanation of why this record matched.")
