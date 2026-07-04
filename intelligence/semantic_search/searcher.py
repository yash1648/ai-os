"""
Semantic search engine — FAISS + sentence-transformers with graceful fallback.

Backend priority
----------------
1. **FAISS** (primary) — ``sentence-transformers`` for 384‑dim embeddings,
   ``faiss.IndexFlatIP`` for cosine similarity via inner product on
   normalised vectors.
2. **TF‑IDF** (first fallback) — ``sklearn.feature_extraction.text.TfidfVectorizer``
   with cosine similarity.
3. **Keyword** (last resort) — pure‑Python term‑frequency overlap scoring.

Every method works correctly regardless of which (if any) ML libraries are
installed.
"""

from __future__ import annotations

import re
from typing import Optional

from .models import SearchConfig, SearchDocument, SearchResult

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

_CHUNK_MAX_WORDS = 512
"""Maximum words per chunk before splitting."""

_CHUNK_OVERLAP = 256
"""Overlap in words between consecutive chunks."""

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _chunk_text(text: str, max_words: int = _CHUNK_MAX_WORDS, overlap: int = _CHUNK_OVERLAP) -> list[str]:
    """Split *text* into overlapping word‑count chunks.

    Documents shorter than *max_words* are returned as a single chunk.
    Consecutive chunks overlap by *overlap* words to avoid cutting
    sentences in semantically important places.
    """
    words = text.split()
    if len(words) <= max_words:
        return [text]

    chunks: list[str] = []
    stride = max_words - overlap
    start = 0

    while start < len(words):
        end = min(start + max_words, len(words))
        chunks.append(" ".join(words[start:end]))
        if end == len(words):
            break
        start += stride

    return chunks


def _tokenize(text: str) -> list[str]:
    """Lower‑case word tokeniser for the pure‑Python keyword fallback."""
    return [m.group() for m in re.finditer(r"[a-zA-Z0-9_]+", text.lower())]


# ---------------------------------------------------------------------------
# Backend detection
# ---------------------------------------------------------------------------


def _detect_backend() -> str:
    """Return the name of the best backend available on this machine."""
    try:
        import sentence_transformers  # noqa: F401
        import faiss  # noqa: F401

        return "faiss"
    except ImportError:
        pass

    try:
        import sklearn  # noqa: F401

        return "tfidf"
    except ImportError:
        pass

    return "keyword"


# ---------------------------------------------------------------------------
# Engine
# ---------------------------------------------------------------------------


class SemanticSearch:
    """In‑memory semantic search engine with automatic fallback.

    Parameters
    ----------
    config : SearchConfig
        Embedding model name, index type, default ``top_k`` and
        ``score_threshold``.

    Attributes
    ----------
    _docs : list[SearchDocument]
        All indexed chunks (flat list — each chunk is a separate entry).
    _backend : str
        ``"faiss"``, ``"tfidf"``, or ``"keyword"``, detected once at
        construction.
    """

    def __init__(self, config: SearchConfig) -> None:
        self.config = config
        self._docs: list[SearchDocument] = []
        self._backend: str = _detect_backend()

        # Backend-specific state (lazy initialised)
        self._faiss_index = None          # faiss.IndexFlatIP
        self._embedding_model = None      # SentenceTransformer
        self._tfidf_vectorizer = None     # TfidfVectorizer
        self._tfidf_matrix = None         # sparse matrix
        self._keyword_index: list[set[str]] = []

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def index_documents(self, docs: list[SearchDocument]) -> int:
        """Index a list of documents, splitting long ones into chunks.

        Existing index state is **replaced** — call ``clear()`` first if
        you want additive indexing.

        Returns
        -------
        int
            Total number of chunks stored (may be larger than ``len(docs)``
            if chunking occurred).
        """
        chunks: list[SearchDocument] = []
        for doc in docs:
            parts = _chunk_text(doc.content)
            for i, part_text in enumerate(parts):
                chunks.append(
                    SearchDocument(
                        id=doc.id,
                        title=doc.title,
                        content=part_text,
                        source=doc.source,
                        file_path=doc.file_path,
                        chunk_index=i,
                    )
                )

        self._docs = chunks
        texts = [c.content for c in chunks]

        if self._backend == "faiss":
            self._index_faiss(texts)
        elif self._backend == "tfidf":
            self._index_tfidf(texts)
        else:
            self._index_keyword(texts)

        return len(chunks)

    def search(self, query: str, top_k: Optional[int] = None) -> list[SearchResult]:
        """Return documents ranked by relevance to *query*.

        Parameters
        ----------
        query : str
            Natural‑language query string.
        top_k : int or None
            Override for ``config.top_k``.  ``None`` uses the config
            default.

        Returns
        -------
        list[SearchResult]
            Results sorted by descending score, filtered by
            ``config.score_threshold``.
        """
        if not query.strip() or not self._docs:
            return []

        k = top_k if top_k is not None else self.config.top_k
        if k <= 0:
            return []

        if self._backend == "faiss":
            scored = self._search_faiss(query, k)
        elif self._backend == "tfidf":
            scored = self._search_tfidf(query, k)
        else:
            scored = self._search_keyword(query, k)

        threshold = self.config.score_threshold
        results: list[SearchResult] = []
        for idx, score in scored:
            if score < threshold:
                continue
            results.append(
                SearchResult(
                    document=self._docs[idx],
                    score=max(0.0, min(1.0, score)),
                )
            )
        return results

    def count(self) -> int:
        """Return the number of indexed chunks."""
        return len(self._docs)

    def clear(self) -> None:
        """Remove all indexed documents and reset backend state."""
        self._docs.clear()
        self._faiss_index = None
        self._embedding_model = None
        self._tfidf_vectorizer = None
        self._tfidf_matrix = None
        self._keyword_index.clear()

    # ------------------------------------------------------------------
    # FAISS backend
    # ------------------------------------------------------------------

    def _load_embedding_model(self):
        """Lazy‑load the SentenceTransformer model (cached after first call)."""
        if self._embedding_model is None:
            from sentence_transformers import SentenceTransformer

            self._embedding_model = SentenceTransformer(self.config.embedding_model)
        return self._embedding_model

    def _index_faiss(self, texts: list[str]) -> None:
        import numpy as np
        import faiss

        model = self._load_embedding_model()
        embeddings = model.encode(texts, normalize_embeddings=True)  # (N, 384)
        dim = embeddings.shape[1]

        self._faiss_index = faiss.IndexFlatIP(dim)
        if embeddings.shape[0] > 0:
            self._faiss_index.add(embeddings)

    @staticmethod
    def _faiss_score_to_similarity(score: float) -> float:
        """Convert raw FAISS inner‑product to a [0, 1] similarity.

        With L2‑normalised vectors the inner product equals cosine
        similarity which naturally lies in [-1, 1].  We clamp to [0, 1]
        because negative scores indicate no semantic overlap.
        """
        return max(0.0, score)

    def _search_faiss(self, query: str, top_k: int) -> list[tuple[int, float]]:
        if self._faiss_index is None or self._faiss_index.ntotal == 0:
            return []

        model = self._load_embedding_model()
        q_emb = model.encode([query], normalize_embeddings=True)  # (1, 384)

        k = min(top_k, self._faiss_index.ntotal)
        distances, indices = self._faiss_index.search(q_emb, k)

        results: list[tuple[int, float]] = []
        for pos in range(k):
            idx = indices[0][pos]
            if idx == -1:  # FAISS may return -1 when k > ntotal
                continue
            results.append((idx, self._faiss_score_to_similarity(float(distances[0][pos]))))
        return results

    # ------------------------------------------------------------------
    # TF‑IDF backend (sklearn)
    # ------------------------------------------------------------------

    def _index_tfidf(self, texts: list[str]) -> None:
        from sklearn.feature_extraction.text import TfidfVectorizer

        self._tfidf_vectorizer = TfidfVectorizer()
        if texts:
            self._tfidf_matrix = self._tfidf_vectorizer.fit_transform(texts)
        else:
            self._tfidf_matrix = None

    def _search_tfidf(self, query: str, top_k: int) -> list[tuple[int, float]]:
        from sklearn.metrics.pairwise import cosine_similarity

        if self._tfidf_matrix is None:
            return []

        q_vec = self._tfidf_vectorizer.transform([query])
        scores = cosine_similarity(q_vec, self._tfidf_matrix)[0]

        scored: list[tuple[int, float]] = [
            (i, float(scores[i])) for i in range(len(scores))
        ]
        scored.sort(key=lambda x: x[1], reverse=True)
        return scored[:top_k]

    # ------------------------------------------------------------------
    # Pure‑Python keyword backend
    # ------------------------------------------------------------------

    def _index_keyword(self, texts: list[str]) -> None:
        self._keyword_index = [set(_tokenize(t)) for t in texts]

    def _search_keyword(self, query: str, top_k: int) -> list[tuple[int, float]]:
        q_terms = _tokenize(query)
        if not q_terms:
            return []
        q_set = set(q_terms)

        scored: list[tuple[int, float]] = []
        for i, doc_set in enumerate(self._keyword_index):
            overlap = len(q_set & doc_set)
            score = overlap / len(q_set)
            scored.append((i, score))

        scored.sort(key=lambda x: x[1], reverse=True)
        return scored[:top_k]
