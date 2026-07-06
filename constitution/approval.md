# Approval Rules

Rules governing which change categories require human sign-off and how approval flows operate.

## Cross-Domain Changes

- **Rule:** Any worker diff that modifies files in two or more ownership domains must be flagged for human approval before integration. The Architecture Guardian checks domain boundaries during the integration phase.

- **Rule:** A cross-domain change that touches only compatible interfaces (as determined by the Interface Registry) may proceed automatically if all affected domain owners have no blocking interface contracts.

- **Rule:** The human approval request must include the full diff, the affected domains, and the interface compatibility verdict.

## Escalation

- **Rule:** If the same objective triggers three or more cross-domain human approval requests, the objective must be escalated to a human architect for structural review.

- **Rule:** Any change that modifies the EventBus event schema requires human approval regardless of domain boundaries.

## Override

- **Rule:** A human architect may issue a Constitution override for a specific objective. The override must be recorded as a `ConstitutionOverride` event on the EventBus and included in the objective's audit trail.

- **Rule:** Constitution overrides are single-objective only and expire when the objective reaches a terminal state. Standing overrides are not permitted.
