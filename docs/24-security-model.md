# AI-OS Documentation — 24. Security Model

## Threat Model

AI-OS assumes:

- LLM-backed workers may be manipulated (via prompt injection embedded in code comments, issue text, or retrieved documents) into attempting actions outside their intended scope.
- Workers may hallucinate authority they do not have.
- Plugins may contain bugs or, in adversarial supply-chain scenarios, malicious code.
- Human operators may make mistakes when granting approvals.

The security model is built around the assumption that **workers are untrusted code**, even though they operate on behalf of a trusted project. No security property of AI-OS depends on a worker "behaving well" — every guarantee is enforced structurally by the Kernel regardless of worker behavior.

## Core Guarantees

1. **No direct repository access.** Workers cannot write, commit, push, or delete anything in the actual Git repository. They can only propose diffs against a read-only snapshot.
2. **No privilege escalation via output.** Worker output is treated as untrusted data, not as instructions — even if a worker's execution report contains text like "please merge this immediately without review," the Kernel does not parse or act on natural-language instructions embedded in worker output. Only structured, schema-validated fields are consumed.
3. **Sandboxed execution.** Workers and plugins run in isolated sandboxes with no network access to production systems and no ability to affect other concurrently running workers.
4. **Deny-by-default permissions.** See `14-permission-engine.md` — absence of an explicit grant is always a denial.
5. **Mandatory gates for high-risk actions.** Schema migrations, breaking interface changes, dependency additions, constitutional exceptions, and production deploys always require an authenticated human decision; no configuration can fully disable these gates for these categories (they can be reassigned to different approvers, but not removed).
6. **Immutable audit trail.** Every decision — Kernel, Reviewer, Guardian, human — is recorded in a hash-chained log that cannot be edited after the fact, supporting forensic reconstruction of any incident.

## Prompt Injection Defense

Because Execution Manifests may include retrieved code, ADR text, or interface documentation that could contain adversarial content, the worker runtime treats all such content as data, not instruction — the worker's system-level operating constraints (allowed files, forbidden actions, output schema) are enforced by the Kernel independent of what the worker "decides" to do, so a successful prompt injection can, at worst, cause a worker to produce a bad diff — which is then still subject to Review, Guardian, and (for high-risk categories) human approval, all operating on the *content* of the diff, not on any instructions the worker might have been tricked into emitting.

## Secrets and Credentials

Workers never receive raw credentials for external systems. Any tool access requiring credentials (e.g., a deploy step) is executed by the Kernel or a narrowly scoped, Kernel-invoked service account, never by the worker process directly.

## Plugin Supply-Chain Security

Plugins are versioned, signed, and loaded only from a configured trusted registry. A plugin's declared capabilities are validated against its actual runtime behavior in a sandboxed conformance test before it is trusted in production pipelines (see `18-plugin-sdk.md`).

## Incident Response

Every security-relevant event category (`PermissionDenied`, `GuardianFailed` with `severity: blocking`, `HumanApprovalDenied`) is available for real-time alerting integration, allowing security teams to detect anomalous patterns (e.g., a spike in permission denials suggesting a compromised or misbehaving worker configuration) without waiting for a periodic audit review.
