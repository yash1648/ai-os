"""
Planner CLI — decompose business objectives into execution plans.

Usage:
    python -m planner.main --objective "Add multi-tenant support to the billing service"
    python -m planner.main --objective "..." --model gpt-4o --context-file context.json
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from planner.decomposer import GoalDecomposer
from planner.llm import OpenAiLlmClient


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="AI-OS Goal Decomposer — decompose a business objective into an execution plan."
    )
    parser.add_argument(
        "--objective",
        type=str,
        required=True,
        help="Free-form business objective (e.g. 'Add user authentication').",
    )
    parser.add_argument(
        "--model",
        type=str,
        default=None,
        help="LLM model identifier (default: $PLANNER_MODEL or gpt-4o-mini).",
    )
    parser.add_argument(
        "--context-file",
        type=str,
        default=None,
        help="Path to a JSON file with additional context (ADR paths, domains, etc.).",
    )
    parser.add_argument(
        "--output",
        type=str,
        default=None,
        help="Path to write the execution plan JSON. Defaults to stdout.",
    )
    parser.add_argument(
        "--mock",
        action="store_true",
        default=False,
        help="Use mock LLM client (deterministic, no API key needed).",
    )

    args = parser.parse_args(argv)

    # Load context if provided.
    context: dict = {}
    if args.context_file:
        ctx_path = Path(args.context_file)
        if not ctx_path.exists():
            print(f"Error: context file not found: {ctx_path}", file=sys.stderr)
            return 1
        try:
            context = json.loads(ctx_path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as e:
            print(f"Error: invalid JSON in context file: {e}", file=sys.stderr)
            return 1

    # Build the LLM client.
    if args.mock:
        from planner.llm import MockLlmClient

        llm = MockLlmClient()
    else:
        try:
            llm = OpenAiLlmClient(model=args.model)
        except ValueError as e:
            print(f"Error: {e}", file=sys.stderr)
            return 1

    # Decompose.
    decomposer = GoalDecomposer(llm_client=llm)
    try:
        plan = decomposer.decompose(args.objective, context=context)
    except ValueError as e:
        print(f"Decomposition failed: {e}", file=sys.stderr)
        return 1

    # Output.
    output_json = plan.model_dump_json(indent=2)
    if args.output:
        Path(args.output).write_text(output_json, encoding="utf-8")
        print(f"Plan written to {args.output}")
    else:
        print(output_json)

    return 0


if __name__ == "__main__":
    sys.exit(main())
