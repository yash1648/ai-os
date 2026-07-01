# AI-OS Documentation — 10. Interface Registry Specification

## Purpose

The Interface Registry tracks every declared contract in the system — public APIs, internal module boundaries, event schemas, and database schemas exposed to more than one domain — so that the impact of any proposed change can be assessed mechanically before a diff is generated, not discovered after the fact via a broken build.

## Data Model

```yaml
interface_id: string
kind: enum(rest_api, internal_module, event_schema, db_schema, cli, sdk)
owner_domain: string
consumers: [string]          # domains or services depending on this interface
version: semver
signature: string            # canonical representation (OpenAPI fragment, type signature, schema)
compatibility:
  breaking_change_policy: enum(forbidden, requires_approval, allowed_with_deprecation)
  deprecated_since: string | null
  sunset_date: date | null
history: [ {version, changed_by_objective, timestamp, change_summary} ]
```

## Registration

Interfaces are registered in two ways:

1. **Declarative** — explicitly authored in the `interfaces/` directory of the repository, for intentionally designed contracts (public APIs, cross-domain event schemas).
2. **Derived** — automatically inferred by the Project Intelligence Layer from code (e.g., exported module signatures), for internal boundaries not otherwise formally declared.

## Blast-Radius Analysis

Before a worker manifest is constructed for any objective touching a registered interface, the Kernel queries the registry for all `consumers`. This consumer list is:

- Included in the manifest's `interfaces` section so the worker is aware of downstream impact.
- Used by the Architecture Guardian to determine whether a proposed diff constitutes a breaking change relative to the interface's `compatibility.breaking_change_policy`.

## Compatibility Enforcement

| Policy | Guardian Behavior |
|---|---|
| `forbidden` | Any diff that changes the interface's signature in a backward-incompatible way is auto-rejected. |
| `requires_approval` | A backward-incompatible change routes to a mandatory Human Approval Gate. |
| `allowed_with_deprecation` | A backward-incompatible change is permitted only if it introduces the change alongside a deprecation path for the prior version, verified structurally (both versions coexist and the old path is marked deprecated). |

## Versioning

Interfaces follow semantic versioning. A `MAJOR` bump is only permitted through an approved breaking change; `MINOR` and `PATCH` bumps are used for additive and non-functional changes respectively, and can proceed through the normal Review/Guardian pipeline without a mandatory human gate (unless another policy applies).

## Relationship to Ownership

An interface's `owner_domain` is authoritative for approving changes to its signature. A worker operating in a different domain that wishes to change another domain's interface cannot do so directly — this is treated as a cross-domain request (see `13-ownership-model.md`) and requires either delegation or explicit multi-domain objective scoping in the Execution Plan.
