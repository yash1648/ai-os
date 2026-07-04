"""
Semantic search — vector‑based retrieval over code, docs, and ADRs.

Uses embeddings to enable natural‑language queries across the workspace:

* FAISS + sentence‑transformers (primary) — 384‑dim embeddings with
  ``IndexFlatIP`` for cosine similarity.
* sklearn TF‑IDF (first fallback) — cosine similarity on term vectors.
* Pure‑Python keyword (last resort) — term‑frequency overlap scoring.

Every public method works correctly regardless of which ML libraries
are installed — the engine auto‑detects the best available backend.
"""

from .models import SearchConfig, SearchDocument, SearchResult
from .searcher import SemanticSearch

__all__ = [
    "SemanticSearch",
    "SearchConfig",
    "SearchDocument",
    "SearchResult",
]
