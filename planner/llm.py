"""
LLM client abstraction for the Goal Decomposer.

Provides a thin interface around LLM calls that is mockable at test time.
The default implementation (``OpenAiLlmClient``) uses OpenAI-compatible
endpoints; a ``MockLlmClient`` is provided for unit tests.
"""

from __future__ import annotations

import json
import os
from abc import ABC, abstractmethod
from typing import Any


# ── LLM Response ──────────────────────────────────────────────────────────────


class LlmResponse:
    """Structured wrapper around an LLM completion.

    .. attribute:: content
        The text response from the model.
    .. attribute:: parsed
        Optional parsed JSON object (if ``request_json`` was set).
    .. attribute:: model
        Model identifier that produced the response.
    .. attribute:: usage
        Optional dict of token usage (``prompt_tokens``, ``completion_tokens``).
    """

    def __init__(
        self,
        content: str,
        parsed: Any = None,
        model: str = "",
        usage: dict[str, int] | None = None,
    ) -> None:
        self.content = content
        self.parsed = parsed
        self.model = model
        self.usage = usage or {}


# ── Abstract client ───────────────────────────────────────────────────────────


class BaseLlmClient(ABC):
    """Abstract LLM client that the GoalDecomposer uses for all LLM calls."""

    @abstractmethod
    def complete(
        self,
        system_prompt: str,
        user_prompt: str,
        *,
        request_json: bool = False,
        temperature: float = 0.3,
        max_tokens: int = 2048,
    ) -> LlmResponse:
        """Send a completion request and return the response.

        Args:
            system_prompt: System-level instructions.
            user_prompt: The user message / objective.
            request_json: If true, the response is expected to be valid JSON
                and ``parsed`` will be populated.
            temperature: Sampling temperature (lower = more deterministic).
            max_tokens: Maximum output tokens.

        Returns:
            An ``LlmResponse`` with the model output.
        """
        ...


# ── OpenAI-compatible client ──────────────────────────────────────────────────


class OpenAiLlmClient(BaseLlmClient):
    """Default LLM client using OpenAI-compatible API endpoints.

    Reads the ``OPENAI_API_KEY`` and ``OPENAI_BASE_URL`` environment variables.
    Falls back to ``OPENROUTER_API_KEY`` if ``OPENAI_API_KEY`` is not set and
    sets the base URL to OpenRouter.
    """

    def __init__(
        self,
        model: str | None = None,
        api_key: str | None = None,
        base_url: str | None = None,
    ) -> None:
        self.model = model or os.getenv("PLANNER_MODEL", "gpt-4o-mini")

        self.api_key = api_key or os.getenv("OPENAI_API_KEY") or os.getenv("OPENROUTER_API_KEY") or ""
        self.base_url = base_url or os.getenv(
            "OPENAI_BASE_URL",
            "https://api.openai.com/v1" if os.getenv("OPENAI_API_KEY") else "https://openrouter.ai/api/v1",
        )

        if not self.api_key:
            raise ValueError(
                "No API key found. Set OPENAI_API_KEY, OPENROUTER_API_KEY, "
                "or pass api_key=... to OpenAiLlmClient()."
            )

    def complete(
        self,
        system_prompt: str,
        user_prompt: str,
        *,
        request_json: bool = False,
        temperature: float = 0.3,
        max_tokens: int = 2048,
    ) -> LlmResponse:
        import httpx

        messages = [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_prompt},
        ]

        body: dict[str, Any] = {
            "model": self.model,
            "messages": messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
        }
        if request_json:
            body["response_format"] = {"type": "json_object"}

        resp = httpx.post(
            f"{self.base_url.rstrip('/')}/chat/completions",
            headers={
                "Authorization": f"Bearer {self.api_key}",
                "Content-Type": "application/json",
            },
            json=body,
            timeout=120.0,
        )
        resp.raise_for_status()
        data = resp.json()

        choice = data["choices"][0]
        content = choice["message"]["content"] or ""

        parsed = None
        if request_json:
            try:
                parsed = json.loads(content)
            except json.JSONDecodeError:
                pass  # caller can fall back to raw content

        return LlmResponse(
            content=content,
            parsed=parsed,
            model=data.get("model", self.model),
            usage=data.get("usage"),
        )


# ── Mock client (for testing) ─────────────────────────────────────────────────


class MockLlmClient(BaseLlmClient):
    """Deterministic mock LLM client for unit tests.

    Returns pre-configured responses keyed by partial matches of the
    ``user_prompt``.  Falls back to a default canned response if no
    match is found.
    """

    def __init__(self, responses: dict[str, str] | None = None) -> None:
        self.responses: dict[str, str] = responses or {}
        self.call_log: list[tuple[str, str]] = []  # (system_prompt, user_prompt)

    def complete(
        self,
        system_prompt: str,
        user_prompt: str,
        *,
        request_json: bool = False,
        temperature: float = 0.3,
        max_tokens: int = 2048,
    ) -> LlmResponse:
        self.call_log.append((system_prompt, user_prompt))

        content = self._match_response(user_prompt)

        parsed = None
        if request_json:
            try:
                parsed = json.loads(content)
            except json.JSONDecodeError:
                parsed = {"_raw": content}

        return LlmResponse(content=content, parsed=parsed, model="mock-model")

    def _match_response(self, user_prompt: str) -> str:
        """Return the best matching canned response — prefers longest key match for specificity."""
        candidates: list[tuple[str, str]] = []
        prompt_lower = user_prompt.lower()
        for key, value in self.responses.items():
            if key.lower() in prompt_lower:
                candidates.append((key, value))
        if candidates:
            # Longest key = most specific match
            candidates.sort(key=lambda x: len(x[0]), reverse=True)
            return candidates[0][1]
        # Default: minimal valid response
        return json.dumps({
            "rationale": "Mock decomposition.",
            "objectives": [
                {
                    "title": "Mock Objective",
                    "owning_domain": "kernel",
                    "description": "Auto-generated by mock LLM.",
                }
            ],
        })

    def add_response(self, key: str, response: str) -> None:
        """Register an additional canned response (useful in test setup)."""
        self.responses[key] = response
