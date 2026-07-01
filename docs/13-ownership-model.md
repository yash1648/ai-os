# AI-OS Documentation — 13. Ownership Model Specification

## Purpose

The Ownership Model partitions the repository into domains, each with exactly one owner, so that concurrent work can proceed safely and so that "who is allowed to change this?" is always a mechanically answerable question rather than a social convention.

## Domain Definition

A domain is a named partition of the repository, typically aligned with directory structure or declared module boundaries:

```yaml
domain_id: string
name: string
owner: string              # team or worker-specialization identifier
paths: [string]            # glob patterns defining domain membership
owned_interfaces: [string] # interface_ids this domain is authoritative for
approval_required_for: [string]  # change categories needing domain-owner sign-off
```

Every file in the repository must belong to exactly one domain; the Ownership Model validator flags any unassigned or multiply-assigned path as a configuration error requiring resolution before it can affect scheduling.

## Rules

1. A worker's `allowed_files` (computed by the Permission Engine) can never include files outside the domain(s) explicitly granted by its Execution Manifest.
2. An objective that requires changes across multiple domains is either (a) decomposed by the Goal Decomposer into per-domain sub-objectives with an explicit coordination dependency, or (b) explicitly scoped as a multi-domain objective requiring sign-off from all affected domain owners before admission.
3. A worker cannot request a cross-domain edit directly; it can only emit a **cross-domain request** — a structured proposal routed to the owning domain, which either becomes a new objective owned by that domain or is rejected.

## Cross-Domain Request Flow

```
Worker (Domain A) → detects need to change Domain B
   → emits CrossDomainRequestRaised event
   → Kernel routes to Domain B's objective queue
   → Domain B owner (worker or human) evaluates
   → Accepted: new objective created, scheduled normally
   → Rejected: original objective proceeds without the change, or is blocked pending resolution
```

This mirrors an OS-level IPC (inter-process communication) pattern: no process writes directly into another process's memory space; it sends a message and the receiving process decides how to respond.

## Domain Granularity

Domains should be fine-grained enough to enable meaningful parallelism (Stage 2+ of the roadmap depends on this) but coarse-grained enough that ownership boundaries track real architectural seams (e.g., "billing service" rather than "billing service, file 3 of 40"). Guidance:

- Align domains with deployable units or bounded contexts where possible.
- Avoid domains smaller than a single cohesive module; excessive fragmentation increases cross-domain request overhead without a corresponding safety benefit.
- Revisit domain boundaries as an explicit architectural decision (recorded as an ADR) rather than letting them drift informally.

## Enforcement Point

Ownership is enforced by the Permission Engine at manifest-construction time (computing `allowed_files`) and re-validated by the Kernel's Diff Applier at commit time (rejecting any diff that, despite manifest scoping, somehow touches a path outside the granted domain — defense in depth against manifest construction bugs).
