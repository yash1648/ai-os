"""
Data models for the semantic search module.

Defines document, result, and configuration types used throughout
the FAISS / fallback search engine.
"""

from pydantic import BaseModel, Field


class SearchDocument(BaseModel):
    """A single document or chunk indexed for semantic search.

    When the original content exceeds 512 words it is split into
    overlapping chunks — each chunk shares the same ``id`` and
    ``file_path`` but carries a distinct ``chunk_index``.
    """

    id: str
    """Unique document identifier (shared across chunks of the same source)."""

    title: str
    """Human-readable title or heading."""

    content: str
    """Raw text content (a full document or a single chunk)."""

    source: str
    """Origin category — one of ``"adr"``, ``"doc"``, or ``"constitution"``."""

    file_path: str
    """Filesystem path relative to the project root."""

    chunk_index: int = 0
    """Position of this chunk within the original document (0-based)."""


class SearchResult(BaseModel):
    """A single search hit pairing a document with its relevance score."""

    document: SearchDocument
    """The matched document or chunk."""

    score: float = Field(ge=0.0, le=1.0)
    """Similarity score in [0, 1] — higher is more relevant."""


class SearchConfig(BaseModel):
    """Configuration for the semantic search engine."""

    embedding_model: str = "all-MiniLM-L6-v2"
    """Sentence-transformer model name for the FAISS backend."""

    index_type: str = "flat"
    """FAISS index type (currently only ``"flat"`` is supported)."""

    top_k: int = 10
    """Default number of results returned by :meth:`SemanticSearch.search`."""

    score_threshold: float = 0.0
    """Minimum similarity score for a result to be included."""
