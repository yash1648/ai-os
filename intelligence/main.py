"""
PIL (Project Intelligence Layer) — FastAPI sidecar server.

Provides the intelligence layer for AI-OS: code indexing, ADR reasoning,
constitution validation, symbol graph queries, and semantic search.

Usage:
    python -m intelligence.main [--port PORT] [--root /path/to/project]
"""

from __future__ import annotations

import argparse
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from pathlib import Path

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware

from intelligence.api.routes import router as api_router
from intelligence.api.state import PilState


@asynccontextmanager
async def _lifespan(app: FastAPI) -> AsyncIterator[None]:
    """FastAPI lifespan context: initialise PIL state on startup."""
    project_root = app.state._project_root
    print(f"PIL server starting... root={project_root}", flush=True)
    app.state.pil_state = PilState(project_root)
    yield


def create_app(project_root: str | Path | None = None) -> FastAPI:
    """Create and configure the FastAPI application instance.

    Parameters
    ----------
    project_root:
        Root directory for engine data. Defaults to the current working
        directory when ``None``.
    """
    app = FastAPI(
        title="Project Intelligence Layer",
        version="0.1.0",
        description=(
            "AI-OS Intelligence sidecar for indexing, ADR reasoning, "
            "constitution validation, symbol graph queries, and semantic search"
        ),
        lifespan=_lifespan,
    )
    app.state._project_root = project_root

    app.add_middleware(
        CORSMiddleware,
        allow_origins=["*"],
        allow_credentials=True,
        allow_methods=["*"],
        allow_headers=["*"],
    )

    app.include_router(api_router)

    @app.get("/health")
    async def health() -> dict[str, str]:
        return {"status": "ok", "version": "0.1.0"}

    return app


def main() -> None:
    """Parse CLI arguments and start the uvicorn server."""
    parser = argparse.ArgumentParser(
        description="PIL (Project Intelligence Layer) server"
    )
    parser.add_argument(
        "--port",
        type=int,
        default=8082,
        help="Port to listen on (default: 8082)",
    )
    parser.add_argument(
        "--root",
        type=str,
        default=None,
        help="Project root directory (default: current working directory)",
    )
    args = parser.parse_args()

    import uvicorn

    app = create_app(project_root=args.root)
    uvicorn.run(app, host="0.0.0.0", port=args.port)


if __name__ == "__main__":
    main()
