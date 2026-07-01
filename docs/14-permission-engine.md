# AI-OS Documentation — 14. Permission Engine Specification

## Purpose

The Permission Engine is the Kernel subsystem that answers, deterministically, one question repeatedly: "is this actor allowed to perform this action, on this resource, right now?" It is deny-by-default: absence of an explicit grant is a denial, never an implicit allow.

## Actors

- **Worker** (identified by objective ID + worker instance ID)
- **Reviewer** / **Guardian** (system actors with fixed, narrow privileges — read diffs, write verdicts)
- **Human** (identified by authenticated identity, used for approval gates and overrides)
- **Scheduler** (system actor requesting locks on behalf of the Kernel)

## Resources and Actions

| Resource | Actions |
|---|---|
| File path | read, propose_write |
| Domain | request_cross_domain_change |
| Interface | propose_breaking_change |
| Git ref | create_commit (Kernel-only), create_branch (Kernel-only) |
| Constitution | read_section (workers), amend (human maintainers only) |
| PIL query endpoints | query (rate-limited per worker) |

Note that "write" to a file path is never actually granted to a worker as a direct capability — workers only ever have `propose_write` via diff submission; the Kernel's Diff Applier is the sole holder of actual write capability.

## Evaluation Model

For each requested action, the engine evaluates, in order:

1. **Identity check** — is the actor who it claims to be (valid objective/manifest binding, valid signed worker instance token)?
2. **Scope check** — does the actor's Execution Manifest include this resource in its granted scope (`allowed_files`, `allowed_interfaces`, etc.)?
3. **Policy check** — does an applicable policy (Constitution rule, domain rule, interface compatibility policy) forbid the action regardless of scope?
4. **Gate check** — does the action require a Human Approval Gate before proceeding, even if otherwise permitted?

The first failing check short-circuits evaluation and produces a structured denial reason, logged as a `PermissionDenied` event.

## Configuration

Permissions are derived, not hand-authored per worker. They are computed from the composition of:

- The Ownership Model (domain → paths, interfaces)
- The Execution Manifest (objective-scoped subset of the above)
- The Constitution (standing policy rules)
- Active human approval-gate policies

This composition happens fresh for every objective, meaning a change to the Ownership Model or Constitution takes effect for all subsequently dispatched objectives without requiring any change to worker implementations.

## Auditability

Every permission evaluation — allowed or denied — is logged with the full input (actor, resource, action, applicable policies evaluated) and the resulting decision. This log is a critical input to security review and incident post-mortems, and is retained independently of the general audit log for compliance purposes.
