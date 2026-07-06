# AI-OS Examples

> Working, runnable examples that demonstrate the AI-OS Kernel API and ownership model.

## Prerequisites

| Tool     | Version    | Notes                                              |
| -------- | ---------- | -------------------------------------------------- |
| Rust     | 1.85+      | Use `rustup` to install — see rustup.rs            |
| `cargo`  | (bundled)  | Ships with Rust toolchain                          |
| `curl`   | 7.68+      | For API calls against the running kernel           |
| `jq`     | 1.6+       | Optional — used in scripts to pretty-print JSON    |

All example scripts use SQLite (bundled via sqlx) and run entirely offline — no database server required.

## Examples

### [`quick-start/`](./quick-start/)

Minimal end-to-end flow: start the kernel, create an objective, query it, inspect
dashboard metrics, then shut down. Start here.

```bash
cd examples/quick-start
bash run.sh
```

### [`cross-domain/`](./cross-domain/)

Ownership-model scenario: define two domains (kernel, docs) with a YAML config,
create an objective with cross-domain scope, and observe how the Guardian
evaluates domain boundary compliance.

```bash
cd examples/cross-domain
bash run.sh
```

## How to Run

**From the project root:**

```bash
# 1. Quick-start
bash examples/quick-start/run.sh

# 2. Cross-domain ownership
bash examples/cross-domain/run.sh
```

Each `run.sh` script:
1. Starts the kernel daemon (`ai-os serve`) in the background
2. Waits for the `/health` endpoint to respond
3. Makes API calls (POST/GET) to demonstrate the feature
4. Stops the kernel and cleans up

## Output

- All scripts print status messages prefixed with `>>>`.
- API responses are piped through `jq` for readability (or `python3 -m json.tool` if `jq` is unavailable).
- The kernel log is written to `/tmp/ai-os-kernel.log` during the run.
- Temporary database files are created in the example directory and cleaned up on exit.

## Troubleshooting

| Symptom                        | Likely Cause                       | Fix                                         |
| ------------------------------ | ---------------------------------- | ------------------------------------------- |
| `cargo: command not found`     | Rust toolchain not installed       | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| `bind: address already in use` | Port 8080 occupied                 | Kill existing process: `pkill ai-os-kernel` |
| `curl: (7) connection refused` | Kernel not ready yet               | The script retries, but check `tail -20 /tmp/ai-os-kernel.log` |
| `SQLite error`                 | Stale database from previous run   | `rm -f examples/quick-start/ai-os.db examples/cross-domain/ai-os.db` |

## Cleanup

To stop any lingering kernel processes:

```bash
pkill -f "ai-os-kernel" 2>/dev/null || true
rm -f /tmp/ai-os-kernel.log
```
