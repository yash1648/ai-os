# Security Rules

Security baselines, input validation requirements, and forbidden patterns.

## Input Validation

- **Rule:** All API request bodies must be validated through serde deserialization with strict mode. Disallow unknown fields on all request structs `#[serde(deny_unknown_fields)]`.

- **Rule:** File paths received from external requests must be sanitized to prevent directory traversal. Reject paths containing `..` segments.

- **Rule:** Objective IDs and worker IDs must match the regex `^[a-zA-Z0-9_-]+$`. Reject any input containing shell metacharacters.

## Forbidden Patterns

- **Rule:** Never use `std::process::Command` with shell expansion. All subprocess invocations must use the `exec` array form (no `sh -c` wrapper).

- **Rule:** Never log secrets, API keys, or tokens. Use the `Secret` wrapper type from the `secrecy` crate for any sensitive string field.

- **Rule:** Never construct SQL queries via string concatenation. All SQL must use parameterized queries through sqlx's `?` or `$1` bind syntax.

## Audit

- **Rule:** All state transitions that affect objectives must be recorded as events on the EventBus and persisted in the audit log table.

- **Rule:** The audit log must use a SHA-256 hash chain where each entry includes the hash of the preceding entry, enabling tamper detection.
