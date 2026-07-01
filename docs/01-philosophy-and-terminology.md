# AI-OS Documentation — 01. Philosophy & Terminology

## Philosophy

AI-OS borrows its central metaphor from operating system design. In a traditional OS:

- The **kernel** owns memory, scheduling, and hardware access.
- **Processes** run in isolated, bounded contexts and cannot directly touch shared resources.
- **System calls** are the only sanctioned way for a process to request privileged action.
- The **filesystem** is the durable, structured source of truth.

AI-OS maps this directly onto software engineering automation:

| OS Concept | AI-OS Equivalent |
|---|---|
| Kernel | Project Kernel |
| Process | Stateless Worker |
| System call | Execution Manifest submission |
| Scheduler | Kernel Scheduler |
| Permissions / ACLs | Ownership Model + Permission Engine |
| Filesystem | Git repository + Project Intelligence Layer |
| Interrupt / signal | Event Bus event |
| Crash recovery | Rollback |
| man pages | Project Constitution + ADRs |

This mapping is not decorative. It is a design discipline: whenever a new capability is proposed for AI-OS, the first question is "what is the OS analogy, and does it preserve the kernel's authority?" If a proposed feature would let a worker bypass the Kernel — for example, direct filesystem writes, direct Git commits, or unmediated tool access to production systems — it is rejected on principle, regardless of convenience.

## Terminology Reference

**Objective** — A discrete unit of engineering work with a title, owner, priority, dependencies, and measurable success criteria. Objectives are the atomic unit the Kernel schedules and tracks.

**Execution Plan** — An immutable, ordered set of objectives and sub-objectives produced by the Goal Decomposer from a business-level goal. Once approved, a plan is not silently mutated; changes require a new planning cycle.

**Execution Manifest** — The complete, self-contained packet of context (constitution excerpts, relevant ADRs, code snapshot, interface contracts, allowed files, success criteria, forbidden actions, output schema) handed to a worker for a single objective.

**Worker** — A stateless process (typically LLM-backed) that receives exactly one Execution Manifest, produces exactly one diff and execution report, and then terminates. Workers hold no memory across objectives.

**Diff** — The only artifact a worker is permitted to produce that affects the repository. Diffs are unified-format patches, never direct file writes.

**Project Intelligence Layer (PIL)** — The system of record for everything workers need to know about the repository that isn't in the diff itself: symbol graph, dependency graph, interface registry, ownership map, ADR index, constitution, and semantic search index.

**Project Constitution** — The immutable (within a release cycle) set of architectural and process rules governing the project: language versions, architecture style, coverage thresholds, approval requirements, and similar non-negotiables.

**Architecture Decision Record (ADR)** — A structured record of a significant design decision, its context, the alternatives considered, the rationale, and its consequences.

**Reviewer** — The automated pipeline stage that checks a worker's diff for correctness, style, testing adequacy, performance, and maintainability.

**Architecture Guardian** — The automated pipeline stage that checks a diff against the constitution, domain boundaries, forbidden dependencies, and interface compatibility rules. The Guardian has veto power independent of the Reviewer.

**Ownership Domain** — A partition of the repository (by directory, module, or declared boundary) assigned to exactly one owning team or worker specialization. Cross-domain edits are requests, not direct writes.

**Event** — An immutable record emitted onto the Event Bus whenever a meaningful state transition occurs (e.g., `ObjectiveCreated`, `DiffGenerated`, `ReviewFailed`).

**Rollback** — A Kernel-owned operation that reverts a repository (and associated state) to a prior known-good point when a downstream failure or human rejection occurs.

**Human Approval Gate** — A mandatory checkpoint requiring a human decision before the Kernel will proceed, triggered by policy for specific classes of change (schema migrations, breaking interface changes, dependency additions, constitutional violations, production deploys).
