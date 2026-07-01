# AI-OS Documentation — 00. Vision

## Purpose

AI-OS is a deterministic operating-system-inspired runtime for AI-assisted software engineering. It exists to solve a structural problem in current agentic coding systems: language models are powerful reasoners but unreliable executors. AI-OS resolves this by separating **reasoning** (performed by LLM-based workers) from **execution authority** (owned exclusively by a deterministic Kernel).

## Core Thesis

> LLMs think. The Kernel decides.

No AI worker in AI-OS is ever granted direct write access to a repository, a build system, or a deployment target. Every privileged action — writing a diff, transitioning an objective's state, merging code, rolling back a change — is mediated by the Kernel, which applies fixed, auditable, non-probabilistic rules.

## Why This Matters

Autonomous coding agents today typically fail in one of five ways:

1. **Context collapse** — the agent loses track of architectural intent as the codebase grows.
2. **Silent architecture drift** — small, individually reasonable changes accumulate into an incoherent system.
3. **Unauditable decisions** — nobody can reconstruct why a change was made or by what authority.
4. **Unsafe autonomy** — agents perform irreversible actions (schema migrations, dependency changes, deployments) without human checkpoints.
5. **Coordination failure at scale** — multiple agents editing a shared repository produce conflicting, overlapping, or logically inconsistent changes.

AI-OS treats each of these as an infrastructure problem, not a prompting problem. The system is designed the way an operating system kernel is designed: a small, trusted, deterministic core; a strict permission model; and clearly bounded process (worker) execution.

## Guiding Values

- **Determinism over improvisation.** The same objective, given the same repository state and constitution, should produce the same class of outcome — reviewed, validated, and reproducible.
- **Statelessness of intelligence.** Workers do not accumulate hidden memory. All persistent knowledge lives in the Project Intelligence Layer, not in a model's context window.
- **Governance as infrastructure, not policy documents.** Rules about architecture, ownership, and approval are enforced mechanically by the Kernel and Guardian, not merely written down and hoped for.
- **Human oversight is a first-class citizen.** High-risk actions always route through an approval gate; AI-OS never treats human review as an optional feature.
- **Replaceability of workers.** Any specific LLM or agent implementation is a interchangeable component. The system's integrity does not depend on any one model's competence.

## Long-Term Vision

AI-OS aims to become to AI-assisted engineering what a kernel is to an operating system: the invisible, trusted layer that makes many different programs (workers) coexist safely on shared resources (a codebase), without ever needing to trust any individual program completely.

Successive stages of the roadmap move AI-OS from a single-process prototype toward a distributed, plugin-extensible engineering platform capable of coordinating many specialized AI workers across large, multi-domain repositories — without sacrificing architectural integrity, auditability, or human control.
