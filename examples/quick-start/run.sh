#!/usr/bin/env bash
set -euo pipefail

# ──────────────────────────────────────────────────────────────────────────────
# AI-OS Quick-Start Example
# ──────────────────────────────────────────────────────────────────────────────
# Starts the kernel, creates an objective, lists objectives, fetches dashboard
# metrics, and stops the kernel.
# ──────────────────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

KERNEL_LOG="/tmp/ai-os-kernel.log"
CONFIG="$SCRIPT_DIR/config.toml"
OBJECTIVE_JSON="$SCRIPT_DIR/objective.json"
DB_FILE="$SCRIPT_DIR/ai-os.db"

cleanup() {
    echo ">>> Step 6: Stopping kernel..."
    if [ -n "${KERNEL_PID:-}" ]; then
        kill "$KERNEL_PID" 2>/dev/null || true
        wait "$KERNEL_PID" 2>/dev/null || true
    fi
    echo ">>> Done."
}
trap cleanup EXIT

# ── Step 1: Build and start the kernel ──────────────────────────────────────
echo ">>> Step 1: Starting AI-OS Kernel..."

# Remove any stale database from a previous run.
rm -f "$DB_FILE"

cargo build -p ai-os-kernel 2>&1 | tail -5

# Start the kernel in the background.
"$PROJECT_ROOT/target/debug/ai-os-kernel" serve -c "$CONFIG" >"$KERNEL_LOG" 2>&1 &
KERNEL_PID=$!

# ── Step 2: Wait for /health endpoint ────────────────────────────────────────
echo ">>> Step 2: Waiting for /health endpoint..."

for i in $(seq 1 30); do
    if curl -s --fail http://127.0.0.1:8080/health >/dev/null 2>&1; then
        echo "  → Kernel is healthy (attempt $i)"
        break
    fi
    if [ "$i" -eq 30 ]; then
        echo "  ERROR: Kernel did not start within 30 attempts."
        tail -30 "$KERNEL_LOG"
        exit 1
    fi
    sleep 1
done

# ── Step 3: Create an objective ──────────────────────────────────────────────
echo ">>> Step 3: Creating objective..."

CREATE_RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/api/v1/objectives \
    -H "Content-Type: application/json" \
    -d @"$OBJECTIVE_JSON")

OBJECTIVE_ID=$(echo "$CREATE_RESPONSE" | jq -r '.data.id' 2>/dev/null || \
               echo "$CREATE_RESPONSE" | python3 -c "import sys,json; print(json.load(sys.stdin)['data']['id'])" 2>/dev/null || \
               echo "unknown")

echo "  → Created objective: $OBJECTIVE_ID"

# ── Step 4: List all objectives ──────────────────────────────────────────────
echo ">>> Step 4: Listing all objectives..."

LIST_RESPONSE=$(curl -s http://127.0.0.1:8080/api/v1/objectives)
OBJECTIVE_COUNT=$(echo "$LIST_RESPONSE" | jq -r '.data | length' 2>/dev/null || echo "?")

echo "  → Found $OBJECTIVE_COUNT objective(s)"

# Pretty-print each objective's title and status.
echo "$LIST_RESPONSE" | jq -r '.data[] | "  → [" + .status.label + "] " + .title' 2>/dev/null || \
echo "  (install jq for richer output)"

# ── Step 5: Dashboard metrics ────────────────────────────────────────────────
echo ">>> Step 5: Dashboard metrics..."

curl -s http://127.0.0.1:8080/api/dashboard/metrics | jq '.data' 2>/dev/null || \
curl -s http://127.0.0.1:8080/api/dashboard/metrics

echo ""
echo ">>> ✅  Quick-start example completed successfully."
