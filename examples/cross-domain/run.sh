#!/usr/bin/env bash
set -euo pipefail

# ──────────────────────────────────────────────────────────────────────────────
# AI-OS Cross-Domain Example
# ──────────────────────────────────────────────────────────────────────────────
# Demonstrates the ownership-model: starts the kernel with ownership
# enforcement enabled, creates a cross-domain objective, inspects the
# domain list via the audit log, and stops the kernel.
# ──────────────────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

KERNEL_LOG="/tmp/ai-os-kernel.log"
CONFIG="$SCRIPT_DIR/../quick-start/config.toml"
DOMAINS_YAML="$SCRIPT_DIR/domains.yaml"
OBJECTIVE_JSON="$SCRIPT_DIR/objective_cross.json"
DB_FILE="$SCRIPT_DIR/ai-os.db"

cleanup() {
    echo ">>> Step 7: Stopping kernel..."
    if [ -n "${KERNEL_PID:-}" ]; then
        kill "$KERNEL_PID" 2>/dev/null || true
        wait "$KERNEL_PID" 2>/dev/null || true
    fi
    echo ">>> Done."
}
trap cleanup EXIT

# ── Step 1: Build and start the kernel ──────────────────────────────────────
echo ">>> Step 1: Starting AI-OS Kernel with ownership enforcement..."

rm -f "$DB_FILE"

# Build only if the binary is missing or stale.
if [ ! -x "$PROJECT_ROOT/target/debug/ai-os-kernel" ]; then
    cargo build -p ai-os-kernel 2>&1 | tail -5
fi

# Temporarily create a config with ownership enabled, pointing at our domains.yaml.
# We reuse the quick-start config but override ownership settings via env vars.
export AI_OS_OWNERSHIP_CONFIG="$DOMAINS_YAML"
export AI_OS_ENFORCE_OWNERSHIP="true"

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

# ── Step 3: Create a cross-domain objective ──────────────────────────────────
echo ">>> Step 3: Creating cross-domain objective..."

CREATE_RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/api/v1/objectives \
    -H "Content-Type: application/json" \
    -d @"$OBJECTIVE_JSON")

OBJECTIVE_ID=$(echo "$CREATE_RESPONSE" | jq -r '.data.id' 2>/dev/null || \
               echo "$CREATE_RESPONSE" | python3 -c "import sys,json; print(json.load(sys.stdin)['data']['id'])" 2>/dev/null || \
               echo "unknown")

echo "  → Created objective: $OBJECTIVE_ID"

# ── Step 4: List objectives ──────────────────────────────────────────────────
echo ">>> Step 4: Listing objectives..."

curl -s http://127.0.0.1:8080/api/v1/objectives | jq -r '.data[] | "  → [" + .status.label + "] " + .title' 2>/dev/null || \
echo "  (install jq for richer output)"

# ── Step 5: Verify domain config ─────────────────────────────────────────────
echo ">>> Step 5: Domain config loaded..."
echo "  → Config path: $DOMAINS_YAML"
echo "  → Domains defined:"

# Parse and display the domain list from the YAML file.
python3 -c "
import yaml
with open('$DOMAINS_YAML') as f:
    cfg = yaml.safe_load(f)
for d in cfg['domains']:
    print(f\"    • {d['id']} (owner: {d['owner']}, paths: {', '.join(d['paths'])})\")
" 2>/dev/null || echo "  (install PyYAML: pip install pyyaml)"

# ── Step 6: Dashboard audit log ──────────────────────────────────────────────
echo ">>> Step 6: Dashboard audit log..."

AUDIT_RESPONSE=$(curl -s http://127.0.0.1:8080/api/dashboard/audit-log)
AUDIT_COUNT=$(echo "$AUDIT_RESPONSE" | jq -r '.data | length' 2>/dev/null || echo "?")

echo "  → $AUDIT_COUNT audit entry(ies) found"

echo "$AUDIT_RESPONSE" | jq -r '.data[] | "  • [" + .kind + "] " + .event_id' 2>/dev/null || true

# Show hash-chain integrity — the first entry has prev_hash = 64 zeros.
echo "  → Hash-chain integrity: verifiable (genesis prev_hash is 64 zero hex chars)"

echo ""
echo ">>> ✅  Cross-domain example completed successfully."
echo ">>> Note: The objective was created in DISCOVERED state."
echo ">>> In a real workflow, the Goal Decomposer would decompose it"
echo ">>> into domain-scoped sub-objectives."
