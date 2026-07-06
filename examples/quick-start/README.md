# Quick-Start Example

> Minimal end-to-end AI-OS workflow: start the kernel, create an objective,
> inspect it, then shut down.

## What it demonstrates

1. **Kernel startup** — the `ai-os serve` command loads config from `config.toml`
   and starts the HTTP API on `127.0.0.1:8080`.
2. **Health check** — `GET /health` returns service status, version, and uptime.
3. **Objective creation** — `POST /api/v1/objectives` accepts a JSON body and
   returns a UUID for the newly created objective.
4. **Objective listing** — `GET /api/v1/objectives` returns all objectives
   with their state machine status.
5. **Dashboard metrics** — `GET /api/dashboard/metrics` returns aggregate
   counts (total, active, completed, failed, etc.).
6. **Clean shutdown** — the kernel is stopped gracefully via `SIGTERM`.

## Files

| File            | Purpose                                    |
| --------------- | ------------------------------------------ |
| `config.toml`   | Kernel configuration (SQLite, port 8080)   |
| `objective.json`| Example objective payload to POST          |
| `run.sh`        | Automation script                          |
| `README.md`     | This file                                  |

## Run it

```bash
cd examples/quick-start
bash run.sh
```

## Expected output

```
>>> Step 1: Starting AI-OS Kernel...
>>> Step 2: Waiting for /health endpoint...
>>> Step 3: Creating objective...
  → Created objective: 550e8400-e29b-41d4-a716-446655440000
>>> Step 4: Listing all objectives...
  → Found 1 objective(s)
  → [DISCOVERED] Example — refactor error handling
>>> Step 5: Dashboard metrics...
  → total_objectives: 1
  → active_objectives: 1
>>> Step 6: Stopping kernel...
>>> Done.
```

*Note: UUIDs and timestamps will differ on each run.*
