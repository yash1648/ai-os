"""
Planner CLI — Stub for Stage 2.

Usage:
    python -m planner.main --objective "Add user authentication"
"""

import argparse
import sys


def main() -> None:
    parser = argparse.ArgumentParser(description="AI-OS Planner stub")
    parser.add_argument("--objective", type=str, help="Objective description")
    args = parser.parse_args()

    if args.objective:
        print(f"Planner stub: received objective '{args.objective}'")
        print("Stage 2 will decompose this into an Execution Manifest.")
    else:
        parser.print_help()

    sys.exit(0)


if __name__ == "__main__":
    main()
