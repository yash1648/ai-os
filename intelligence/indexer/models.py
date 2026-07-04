"""Data models for the repository indexer.

Defines the shapes for indexed file metadata and indexer configuration.
"""
from __future__ import annotations

from datetime import datetime
from pathlib import Path

from pydantic import BaseModel, Field


class IndexedFile(BaseModel):
    """Metadata for a single indexed file."""

    name: str = Field(..., description="The file name (e.g. 'main.py').")
    path: str = Field(..., description="Absolute path to the file.")
    rel_path: str = Field(..., description="Path relative to the indexer root.")
    ext: str = Field(..., description="File extension (lower-cased, e.g. '.py').")
    size: int = Field(..., ge=0, description="File size in bytes.")
    modified_at: datetime = Field(..., description="Last modification time.")


class IndexerConfig(BaseModel):
    """Configuration for the repository indexer."""

    root_dir: Path = Field(..., description="Root directory to index.")
    ignore_patterns: list[str] = Field(
        default_factory=lambda: [
            "**/.omo/**",
            "**/target/**",
            "**/node_modules/**",
            "**/__pycache__/**",
            "**/.git/**",
            "**/.venv/**",
        ],
        description="Glob-style patterns to exclude from indexing.",
    )
