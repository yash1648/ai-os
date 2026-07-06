# Cross-Domain Example

> Ownership-model workflow: define two domains (kernel, docs), create an
> objective whose scope spans both, and observe domain-boundary behaviour.

## What it demonstrates

1. **Ownership model** — a YAML config (`domains.yaml`) defines three domains
   with path globs, owner identities, and owned interfaces.
2. **Cross-domain scope** — the example objective (`objective_cross.json`)
   references files in both `kernel/` and `docs/` domains.
3. **Guardian evaluation** — when ownership enforcement is enabled, the
   Architecture Guardian checks each diff against domain boundaries, interface
   compatibility, and ownership constraints.
4. **Dashboard audit log** — `GET /api/dashboard/audit-log` returns
   hash-chained entries that can be independently verified.

## Files

| File                | Purpose                                                |
| ------------------- | ------------------------------------------------------ |
| `domains.yaml`      | Three-domain ownership model config                    |
| `objective_cross.json` | Objective spanning kernel + docs domains           |
| `run.sh`            | Automation script                                      |
| `README.md`         | This file                                              |

## Run it

```bash
cd examples/cross-domain
bash run.sh
```

## Expected output

```
>>> Step 1: Starting AI-OS Kernel with ownership enforcement...
>>> Step 2: Waiting for /health endpoint...
>>> Step 3: Creating cross-domain objective...
  → Created objective: 550e8400-e29b-41d4-a716-446655440000
>>> Step 4: Listing objectives...
  → [DISCOVERED] Add runtime metrics to kernel and document the design
>>> Step 5: Domain config loaded...
  → 3 domain(s) registered: project-kernel, docs, schemas
>>> Step 6: Dashboard audit log...
  → 1 audit entry(ies) found
>>> Step 7: Stopping kernel...
>>> Done.
```

## Ownership in practice

With `ownership.enforce = true` in the config, the Guardian will verify that
any diff produced by this objective stays within its `allowed_domains`. If a
worker submitted a diff touching `docs/25-performance-benchmarks.md` without
the objective having `"docs"` in its allowed domains, the Guardian would
return a `RequiresHumanApproval` verdict with a `domain-boundary` violation.

This example runs in simulation mode — the objective is created and visible,
demonstrating the API surface before real workers produce diffs.
