# AI-OS Documentation — 27. Contributor Guide

## Getting Started

1. Read `00-vision.md` and `01-philosophy-and-terminology.md` before proposing any change — most rejected contributions stem from proposals that would let a component bypass the Kernel's authority, which is a hard architectural boundary, not a style preference.
2. Review `21-repository-layout.md` to understand where a given change belongs.
3. Set up a local Stage 1 deployment (`26-deployment-guide.md`) to validate changes end-to-end before submitting.

## Contribution Categories

- **Kernel/core changes** — require the highest scrutiny; must include unit tests, contract tests, and (where behavior-affecting) updated fault-injection tests per `23-testing-strategy.md`.
- **Plugin contributions** — follow the Plugin SDK (`18-plugin-sdk.md`); must include a conformance test suite proving the plugin's declared capabilities match its actual behavior.
- **Documentation changes** — encouraged for any discovered drift between docs and behavior; treated as first-class contributions, not administrative overhead.
- **Constitution/policy changes to the AI-OS project itself** — require an RFC (`28-rfc-process.md`), since AI-OS governs its own development using its own Constitution (see `21-repository-layout.md`, "Ownership Domain Alignment").

## Pull Request Checklist

- [ ] Relevant `docs/` pages updated to reflect behavior changes
- [ ] Unit and contract tests added/updated
- [ ] For Kernel changes: fault-injection test added for any new failure path
- [ ] For interface changes: Interface Registry entry updated, compatibility policy respected
- [ ] Passes self-hosted Guardian evaluation (AI-OS's own CI runs your change through AI-OS's own pipeline)

## Code Review Expectations

Human code review remains mandatory for all contributions to AI-OS itself, regardless of whether the change was authored by a human or drafted with AI assistance — AI-OS applying its own Human Approval Gate philosophy to its own codebase's most sensitive category (Kernel changes).

## Communication Channels

Design discussions before an RFC is formalized should happen in the open (issue tracker or discussion forum) so that context is captured before, not only after, a decision is made — mirroring the "context over conclusions" principle underlying the ADR system (`12-adr-system.md`).

## Style and Tooling

Follow the language-specific style configuration under each domain's plugin conventions (`18-plugin-sdk.md`); style disagreements not resolvable by existing lint configuration should be raised as a proposed Constitution amendment, not litigated ad hoc in individual PRs.
