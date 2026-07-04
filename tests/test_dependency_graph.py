"""Tests for the dependency graph engine — import extraction and graph queries."""

from __future__ import annotations

import tempfile
from pathlib import Path

import pytest

from intelligence.dependency_graph.engine import DependencyGraph, _extract_import_targets
from intelligence.dependency_graph.models import DependencyGraphConfig, DependencyEdge


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def temp_project() -> Path:
    """Create a temporary directory with a small multi-language project tree."""
    with tempfile.TemporaryDirectory() as td:
        root = Path(td)

        # ── Python ────────────────────────────────────────────────────────
        (root / "mod_a.py").write_text(
            "import os\nimport sys\nfrom pathlib import Path\n"
        )
        (root / "mod_b.py").write_text(
            "from mod_a import something\nimport json\n"
        )

        # ── Rust ──────────────────────────────────────────────────────────
        rust_src = root / "src"
        rust_src.mkdir()
        (rust_src / "lib.rs").write_text(
            "use std::collections::HashMap;\n"
            "use crate::event_bus::{Event, Bus};\n"
        )
        (rust_src / "event_bus.rs").write_text(
            "pub struct Event;\npub struct Bus;\n"
        )

        # ── TypeScript ────────────────────────────────────────────────────
        (root / "index.ts").write_text(
            "import { readFile } from 'fs';\n"
            "import path from 'path-browserify';\n"
        )
        yield root


# ---------------------------------------------------------------------------
# _extract_import_targets
# ---------------------------------------------------------------------------


class TestExtractImportTargets:
    def test_python_import(self) -> None:
        edges = _extract_import_targets(
            "python", "import os\nimport sys\n", Path("/tmp/dummy.py")
        )
        targets = {e.target for e in edges}
        assert "os" in targets
        assert "sys" in targets

    def test_python_from_import(self) -> None:
        edges = _extract_import_targets(
            "python",
            "from pathlib import Path\nfrom collections.abc import Iterator\n",
            Path("/tmp/dummy.py"),
        )
        targets = {e.target for e in edges}
        assert "pathlib" in targets
        assert "collections.abc" in targets

    def test_rust_use(self) -> None:
        edges = _extract_import_targets(
            "rust",
            "use std::collections::HashMap;\n",
            Path("/tmp/dummy.rs"),
        )
        assert len(edges) == 1
        assert edges[0].target == "std::collections::HashMap"

    def test_rust_use_group(self) -> None:
        edges = _extract_import_targets(
            "rust",
            "use crate::event_bus::{Event, Bus};\n",
            Path("/tmp/dummy.rs"),
        )
        assert len(edges) == 2
        targets = {e.target for e in edges}
        assert "crate::event_bus::Event" in targets
        assert "crate::event_bus::Bus" in targets

    def test_typescript_import(self) -> None:
        edges = _extract_import_targets(
            "typescript",
            "import { readFile } from 'fs';\nimport path from 'path-browserify';\n",
            Path("/tmp/dummy.ts"),
        )
        targets = {e.target for e in edges}
        assert "fs" in targets
        assert "path-browserify" in targets

    def test_javascript_require(self) -> None:
        edges = _extract_import_targets(
            "javascript",
            "const fs = require('fs');\nconst http = require('http');\n",
            Path("/tmp/dummy.js"),
        )
        targets = {e.target for e in edges}
        assert "fs" in targets
        assert "http" in targets

    def test_unknown_language_returns_empty(self) -> None:
        edges = _extract_import_targets(
            "ruby", "require 'net/http'", Path("/tmp/dummy.rb")
        )
        assert edges == []

    def test_empty_content_returns_empty(self) -> None:
        edges = _extract_import_targets("python", "", Path("/tmp/dummy.py"))
        assert edges == []


# ---------------------------------------------------------------------------
# DependencyGraph — end-to-end
# ---------------------------------------------------------------------------


class TestDependencyGraph:
    def test_index_files_returns_edge_count(self, temp_project: Path) -> None:
        config = DependencyGraphConfig(root_dir=temp_project)
        graph = DependencyGraph(config)
        total = graph.index_files()
        assert total > 0

    def test_stats_after_index(self, temp_project: Path) -> None:
        config = DependencyGraphConfig(root_dir=temp_project)
        graph = DependencyGraph(config)
        graph.index_files()
        stats = graph.stats()
        assert stats.node_count > 0
        assert stats.edge_count > 0
        assert "python" in stats.languages
        assert "rust" in stats.languages

    def test_resolve_finds_dependents(self, temp_project: Path) -> None:
        config = DependencyGraphConfig(root_dir=temp_project)
        graph = DependencyGraph(config)
        graph.index_files()
        resolved = graph.resolve("os")
        assert len(resolved.dependents) >= 1
        assert any("os" in e.target for e in resolved.dependents)

    def test_resolve_returns_empty_for_unknown(self, temp_project: Path) -> None:
        config = DependencyGraphConfig(root_dir=temp_project)
        graph = DependencyGraph(config)
        graph.index_files()
        resolved = graph.resolve("zzz_nonexistent_module_zzz")
        assert len(resolved.dependents) == 0
        assert len(resolved.dependencies) == 0

    def test_dependents_of_substring_match(self, temp_project: Path) -> None:
        config = DependencyGraphConfig(root_dir=temp_project)
        graph = DependencyGraph(config)
        graph.index_files()
        edges = graph.dependents_of("event_bus")
        # Both lib.rs and event_bus.rs should show up via imports
        assert len(edges) >= 1

    def test_all_edges_returns_all(self, temp_project: Path) -> None:
        config = DependencyGraphConfig(root_dir=temp_project)
        graph = DependencyGraph(config)
        graph.index_files()
        edges = graph.all_edges()
        assert len(edges) == graph.stats().edge_count

    def test_index_single_file(self, temp_project: Path) -> None:
        config = DependencyGraphConfig(root_dir=temp_project)
        graph = DependencyGraph(config)
        py_file = temp_project / "mod_a.py"
        edges = graph._index_file(py_file)
        assert len(edges) >= 2  # import os, import sys

    def test_index_non_existent_file(self, temp_project: Path) -> None:
        config = DependencyGraphConfig(root_dir=temp_project)
        graph = DependencyGraph(config)
        edges = graph._index_file(temp_project / "nonexistent.py")
        assert edges == []

    def test_index_unrecognised_extension(self, temp_project: Path) -> None:
        config = DependencyGraphConfig(root_dir=temp_project)
        graph = DependencyGraph(config)
        f = temp_project / "data.json"
        f.write_text('{"hello": "world"}')
        edges = graph._index_file(f)
        assert edges == []

    def test_double_index_is_idempotent(self, temp_project: Path) -> None:
        """Indexing the same project twice should produce the same edge count."""
        config = DependencyGraphConfig(root_dir=temp_project)
        graph = DependencyGraph(config)
        first = graph.index_files()
        second = graph.index_files()
        assert first == second


# ---------------------------------------------------------------------------
# DependencyEdge model validation
# ---------------------------------------------------------------------------


class TestDependencyEdgeModel:
    def test_minimal_creation(self) -> None:
        e = DependencyEdge(
            source_file="/a/b.py", source_line=5, target="os", kind="import"
        )
        assert e.source_file == "/a/b.py"
        assert e.source_line == 5
        assert e.target == "os"
        assert e.kind == "import"

    def test_default_kind(self) -> None:
        e = DependencyEdge(
            source_file="/a/b.py", source_line=1, target="os"
        )
        assert e.kind == "import"

    def test_source_line_must_be_positive(self) -> None:
        with pytest.raises(Exception):
            DependencyEdge(
                source_file="/a/b.py", source_line=0, target="os"
            )
