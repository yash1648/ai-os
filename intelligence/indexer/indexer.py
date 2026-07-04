"""Repository indexer implementation.

Provides a snapshot-style indexer that scans a directory tree, respects
ignore patterns, and produces typed :class:`IndexedFile` records.
"""
from __future__ import annotations

import fnmatch
from datetime import datetime
from pathlib import Path

from intelligence.indexer.models import IndexedFile, IndexerConfig


class Indexer:
    """Scan a directory tree and produce a searchable index of files."""

    def __init__(self, config: IndexerConfig) -> None:
        self._config = config
        self._files: list[IndexedFile] = []

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    @property
    def files(self) -> list[IndexedFile]:
        """Return the current indexed files (sorted by path)."""
        return self._files

    def index(self) -> list[IndexedFile]:
        """Scan *root_dir* recursively and return all files sorted by path.

        Files matching any pattern in :attr:`IndexerConfig.ignore_patterns`
        are excluded.  The internal cache is updated and returned.
        """
        root = self._config.root_dir.resolve()
        ignore = self._config.ignore_patterns
        indexed: list[IndexedFile] = []

        for path in root.rglob("*"):
            if not path.is_file():
                continue
            rel = path.relative_to(root)
            if self._is_ignored(rel, ignore):
                continue
            stat = path.stat()
            indexed.append(
                IndexedFile(
                    name=path.name,
                    path=str(path),
                    rel_path=str(rel),
                    ext=path.suffix.lower(),
                    size=stat.st_size,
                    modified_at=datetime.fromtimestamp(stat.st_mtime),
                )
            )

        indexed.sort(key=lambda f: f.path)
        self._files = indexed
        return self._files

    def list_by_extension(self, ext: str) -> list[IndexedFile]:
        """Return files whose extension matches *ext* (case-insensitive)."""
        ext_norm = ext.lower() if not ext.startswith(".") else ext.lower()
        return [f for f in self._files if f.ext == ext_norm]

    def find_by_path_prefix(self, prefix: str) -> list[IndexedFile]:
        """Return files whose relative or absolute path starts with *prefix*."""
        return [f for f in self._files if f.rel_path.startswith(prefix) or f.path.startswith(prefix)]

    def count(self) -> int:
        """Total number of indexed files."""
        return len(self._files)

    # ------------------------------------------------------------------
    # Helpers
    # ------------------------------------------------------------------

    @staticmethod
    def _is_ignored(rel_path: Path, patterns: list[str]) -> bool:
        """Check whether *rel_path* matches any glob pattern in *patterns*."""
        rel_str = str(rel_path)
        for pattern in patterns:
            if fnmatch.fnmatch(rel_str, pattern):
                return True
            if fnmatch.fnmatch(rel_str, pattern.lstrip("**/")):
                return True
        return False
