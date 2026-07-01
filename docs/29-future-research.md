# AI-OS Documentation — 29. Future Research

This document tracks open research questions that are out of scope for the current roadmap (`22-development-roadmap.md`) but are considered important to AI-OS's long-term evolution.

## Multi-Repository Orchestration

How should the Kernel model objectives that span multiple, separately governed repositories (e.g., a change requiring coordinated updates to a client SDK and a server API in different repos)? This likely requires an extension of the Ownership Model and Interface Registry beyond a single-repository scope, and raises open questions about cross-repository transactional guarantees (can a rollback in one repository be safely coordinated with a rollback in another?).

## Adaptive Manifest Scoping

Can the Project Intelligence Layer learn, over time, better heuristics for what "minimal sufficient context" means for a given objective type, reducing both token cost and irrelevant-context-induced errors, without compromising the auditability of what context a worker had access to?

## Formal Verification of Guardian Rules

For safety-critical projects, could a subset of Constitution rules be expressed in a form amenable to formal verification against the Guardian's implementation, providing a stronger guarantee than test-suite coverage that a given rule cannot be silently bypassed by a Guardian implementation bug?

## Worker Model Diversity and Ensemble Review

Would routing the same objective to multiple differently-trained worker models and having the Reviewer/Guardian evaluate divergence between their outputs provide a useful signal for objective difficulty or risk, independent of any single model's self-reported confidence?

## Human Approval Fatigue

As objective throughput scales, how should the system avoid degrading human approval quality due to volume (rubber-stamping)? Potential directions include risk-based sampling of gated approvals for deeper human review, and better summarization of *why* a gate was triggered to reduce reviewer cognitive load without reducing rigor.

## Constitutional Evolution at Scale

For large organizations with many AI-OS-managed repositories, how should organization-wide policy packs (Stage 5) interact with per-repository Constitution customization without creating an unmanageable combinatorial space of effective rule sets?

## Economic Modeling of Objective Cost

Can the Kernel incorporate real-time cost/token-budget awareness into scheduling decisions (e.g., deprioritizing low-priority objectives during periods of high LLM API cost), and what governance is needed to ensure such optimization never silently trades off correctness or safety for cost?

## Status

These are open questions, not commitments. Proposals to formally begin research or prototyping in any of these areas should follow the standard RFC process (`28-rfc-process.md`).
