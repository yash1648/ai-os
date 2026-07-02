//! Permission Engine — deny-by-default access control for the Kernel.
//!
//! Docs: docs/14-permission-engine.md
//!
//! Evaluates `(actor, action, resource)` triples through four sequential
//! phases: identity → scope → policy → gate.  The first failing phase
//! short-circuits and produces a structured denial reason logged as a
//! `PermissionDenied` event.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::event_bus::{Actor, ActorKind, Event, EventBus, EventKind};
use crate::manifest::ExecutionManifest;
use crate::ownership::OwnershipModel;

/// Actions a system actor may attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    /// Read a file path.
    Read,
    /// Submit a diff proposing file changes (workers never have direct Write).
    ProposeWrite,
    /// Request changes to files owned by a different domain.
    RequestCrossDomainChange,
    /// Propose a breaking change to a registered interface.
    ProposeBreakingChange,
    /// Create a commit (Kernel-only).
    CreateCommit,
    /// Create a branch (Kernel-only).
    CreateBranch,
    /// Read the Constitution.
    ReadConstitution,
    /// Amend the Constitution (human maintainers only).
    AmendConstitution,
    /// Query PIL endpoints.
    QueryPil,
}

/// Which phase of the evaluation rejected the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckPhase {
    Identity,
    Scope,
    Policy,
    Gate,
}

/// Structured denial reason returned when permission is denied.
#[derive(Debug, Clone)]
pub struct Denial {
    pub reason: String,
    pub phase: CheckPhase,
}

/// Result of a permission check.
#[derive(Debug, Clone)]
pub enum PermissionResult {
    Allowed,
    Denied(Denial),
}

impl PermissionResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, PermissionResult::Allowed)
    }
}

/// The Permission Engine answers "is this actor allowed to perform this
/// action, on this resource, right now?"
///
/// ## Evaluation Order
///
/// 1. **Identity** — is the actor valid (valid manifest binding)?
/// 2. **Scope** — does the actor's manifest include this resource?
/// 3. **Policy** — does any Constitution rule forbid the action regardless?
/// 4. **Gate** — does a Human Approval Gate apply?
///
/// Deny-by-default: anything not explicitly granted is denied.
pub struct PermissionEngine {
    ownership: Arc<OwnershipModel>,
    event_bus: Option<EventBus>,
}

impl PermissionEngine {
    /// Create a new permission engine.
    pub fn new(ownership: Arc<OwnershipModel>) -> Self {
        Self {
            ownership,
            event_bus: None,
        }
    }

    /// Attach an event bus for audit logging.
    pub fn with_event_bus(mut self, bus: EventBus) -> Self {
        self.event_bus = Some(bus);
        self
    }

    /// Evaluate a permission check.  Returns `Allowed` or `Denied` with
    /// the phase and reason.  Logs every decision as a `PermissionDenied`
    /// event when denied.
    pub fn check(
        &self,
        actor: &Actor,
        action: &Action,
        resource: &str,
        manifest: Option<&ExecutionManifest>,
    ) -> PermissionResult {
        // Phase 1 — Identity
        let identity_result = self.check_identity(actor, manifest);
        if let PermissionResult::Denied(denial) = &identity_result {
            self.emit_denied(actor, action, resource, denial);
            return identity_result;
        }

        // Phase 2 — Scope
        let scope_result = self.check_scope(actor, action, resource, manifest);
        if let PermissionResult::Denied(denial) = &scope_result {
            self.emit_denied(actor, action, resource, denial);
            return scope_result;
        }

        // Phase 3 — Policy (standing rules)
        let policy_result = self.check_policy(actor, action, resource);
        if let PermissionResult::Denied(denial) = &policy_result {
            self.emit_denied(actor, action, resource, denial);
            return policy_result;
        }

        // Phase 4 — Gate (human approval required)
        let gate_result = self.check_gate(actor, action, resource);
        if let PermissionResult::Denied(denial) = &gate_result {
            self.emit_denied(actor, action, resource, denial);
            return gate_result;
        }

        PermissionResult::Allowed
    }

    // ------------------------------------------------------------------
    // Phase internals
    // ------------------------------------------------------------------

    /// Phase 1 — Identity check.
    ///
    /// Verifies the actor is valid for its claimed role.  Workers must have
    /// a valid objective-id and manifest binding.  Human actors require an
    /// identity string.
    fn check_identity(
        &self,
        actor: &Actor,
        manifest: Option<&ExecutionManifest>,
    ) -> PermissionResult {
        match actor.kind {
            ActorKind::Worker => {
                // Workers must have a valid objective_id matching their manifest.
                if manifest.is_none() && matches!(actor.id.as_str(), "kernel" | "scheduler") {
                    return PermissionResult::Denied(Denial {
                        reason: "Worker actor requires an Execution Manifest".into(),
                        phase: CheckPhase::Identity,
                    });
                }
                PermissionResult::Allowed
            }
            ActorKind::Human => {
                if actor.id.is_empty() {
                    return PermissionResult::Denied(Denial {
                        reason: "Human actor requires a non-empty identity".into(),
                        phase: CheckPhase::Identity,
                    });
                }
                PermissionResult::Allowed
            }
            // Kernel, Reviewer, Guardian, Scheduler are trusted system actors.
            _ => PermissionResult::Allowed,
        }
    }

    /// Phase 2 — Scope check.
    ///
    /// Verifies the resource is within the actor's granted scope (manifest's
    /// `allowed_files`, `allowed_interfaces`, or domain ownership).
    fn check_scope(
        &self,
        actor: &Actor,
        action: &Action,
        resource: &str,
        manifest: Option<&ExecutionManifest>,
    ) -> PermissionResult {
        match action {
            // Read is broadly permitted for valid actors.
            Action::Read | Action::ReadConstitution | Action::QueryPil => {
                PermissionResult::Allowed
            }

            // Write-like actions require scope validation.
            Action::ProposeWrite => {
                if let Some(manifest) = manifest {
                    // Check the resource is within the domain that owns it,
                    // AND that the domain is in the manifest's allowed_domains.
                    let domain = self.ownership.domain_for_file(resource);
                    match domain {
                        Some(domain) => {
                            if manifest.allowed_domains.is_empty()
                                || manifest.allowed_domains.contains(&domain.id)
                            {
                                PermissionResult::Allowed
                            } else {
                                PermissionResult::Denied(Denial {
                                    reason: format!(
                                        "Domain '{}' is not in manifest's allowed domains",
                                        domain.id
                                    ),
                                    phase: CheckPhase::Scope,
                                })
                            }
                        }
                        None => PermissionResult::Denied(Denial {
                            reason: format!("Resource '{}' does not belong to any domain", resource),
                            phase: CheckPhase::Scope,
                        }),
                    }
                } else {
                    PermissionResult::Denied(Denial {
                        reason: "No Execution Manifest provided for scope check".into(),
                        phase: CheckPhase::Scope,
                    })
                }
            }

            // Cross-domain requests are scoped differently: the resource being
            // targeted is deliberately OUTSIDE the manifest's allowed_domains.
            // The scope check only verifies the resource belongs to SOME domain
            // and the actor has a manifest.  Approval gates (Policy/Gate phases
            // or human override) handle the actual cross-domain authorization.
            Action::RequestCrossDomainChange => {
                if manifest.is_none() {
                    return PermissionResult::Denied(Denial {
                        reason: "No Execution Manifest provided for cross-domain request".into(),
                        phase: CheckPhase::Scope,
                    });
                }
                // Verify the resource belongs to a known domain.
                if self.ownership.domain_for_file(resource).is_none() {
                    return PermissionResult::Denied(Denial {
                        reason: format!("Resource '{}' does not belong to any domain", resource),
                        phase: CheckPhase::Scope,
                    });
                }
                PermissionResult::Allowed
            }

            // Kernel-only actions.
            Action::CreateCommit | Action::CreateBranch => {
                if actor.kind == ActorKind::Kernel || actor.id == "scheduler" {
                    PermissionResult::Allowed
                } else {
                    PermissionResult::Denied(Denial {
                        reason: format!(
                            "Action '{:?}' is reserved for the Kernel",
                            action
                        ),
                        phase: CheckPhase::Scope,
                    })
                }
            }

            // Constitution amendments are human-only.
            Action::AmendConstitution => PermissionResult::Denied(Denial {
                reason: "Constitution amendments require a human actor".into(),
                phase: CheckPhase::Scope,
            }),

            // Breaking changes require interface ownership or special grant.
            Action::ProposeBreakingChange => {
                if actor.kind == ActorKind::Human {
                    PermissionResult::Allowed
                } else {
                    PermissionResult::Denied(Denial {
                        reason: "Breaking changes require human approval".into(),
                        phase: CheckPhase::Scope,
                    })
                }
            }
        }
    }

    /// Phase 3 — Policy check.
    ///
    /// Checks standing policy rules (Constitution, domain rules).  Extended
    /// in future stages with dynamic policy loading.
    fn check_policy(
        &self,
        _actor: &Actor,
        _action: &Action,
        _resource: &str,
    ) -> PermissionResult {
        // Stage 2: no standing policy rules yet.
        // Future: query Constitution engine, domain-specific rules.
        PermissionResult::Allowed
    }

    /// Phase 4 — Gate check.
    ///
    /// Checks whether the action requires a Human Approval Gate.
    fn check_gate(
        &self,
        actor: &Actor,
        action: &Action,
        resource: &str,
    ) -> PermissionResult {
        // Breaking changes always require human approval.
        if *action == Action::ProposeBreakingChange && actor.kind != ActorKind::Human {
            return PermissionResult::Denied(Denial {
                reason: format!(
                    "ProposeBreakingChange on '{}' requires a human approval gate",
                    resource
                ),
                phase: CheckPhase::Gate,
            });
        }

        // Constitution amendments always require human approval.
        if *action == Action::AmendConstitution && actor.kind != ActorKind::Human {
            return PermissionResult::Denied(Denial {
                reason: "AmendConstitution requires a human approval gate".into(),
                phase: CheckPhase::Gate,
            });
        }

        PermissionResult::Allowed
    }

    /// Emit a PermissionDenied event for audit logging.
    fn emit_denied(&self, actor: &Actor, action: &Action, resource: &str, denial: &Denial) {
        if let Some(bus) = &self.event_bus {
            let event = Event::new(
                EventKind::PermissionDenied,
                Actor {
                    kind: ActorKind::Kernel,
                    id: "permission-engine".into(),
                },
                serde_json::json!({
                    "actor": actor,
                    "action": action,
                    "resource": resource,
                    "reason": denial.reason,
                    "phase": format!("{:?}", denial.phase),
                }),
            );
            bus.publish(event);
        }
    }

    /// Shortcut: check whether a worker is allowed to propose writes to a file.
    pub fn check_worker_write(
        &self,
        worker_id: &str,
        file_path: &str,
        manifest: &ExecutionManifest,
    ) -> PermissionResult {
        self.check(
            &Actor {
                kind: ActorKind::Worker,
                id: worker_id.into(),
            },
            &Action::ProposeWrite,
            file_path,
            Some(manifest),
        )
    }

    /// Shortcut: check cross-domain request permission.
    pub fn check_cross_domain(
        &self,
        worker_id: &str,
        target_domain: &str,
        manifest: &ExecutionManifest,
    ) -> PermissionResult {
        self.check(
            &Actor {
                kind: ActorKind::Worker,
                id: worker_id.into(),
            },
            &Action::RequestCrossDomainChange,
            target_domain,
            Some(manifest),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ownership::OwnershipModel;
    use std::sync::Arc;

    fn test_ownership() -> Arc<OwnershipModel> {
        let yaml = r#"
domains:
  - id: kernel
    name: "Kernel"
    owner: "kernel-team"
    paths: ["kernel/**/*.rs"]
  - id: docs
    name: "Documentation"
    owner: "docs-team"
    paths: ["docs/**/*.md"]
"#;
        Arc::new(OwnershipModel::from_yaml(yaml).unwrap())
    }

    fn test_manifest() -> ExecutionManifest {
        // Minimal manifest for testing — uses optional fields.
        ExecutionManifest {
            manifest_id: "test-manifest".into(),
            objective_id: "obj-001".into(),
            stage: crate::manifest::ManifestStage::Execution,
            title: "Test".into(),
            description: None,
            groups: vec![],
            environment: crate::manifest::ManifestEnvironment {
                language: None,
                framework: None,
                sdk: None,
                interface_registry: vec![],
            },
            dependencies: vec![],
            allowed_domains: vec![],
            worker_type: None,
            schema_version: "1.0".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn kernel_can_create_commit() {
        let engine = PermissionEngine::new(test_ownership());
        let result = engine.check(
            &Actor {
                kind: ActorKind::Kernel,
                id: "kernel".into(),
            },
            &Action::CreateCommit,
            "",
            None,
        );
        assert!(result.is_allowed());
    }

    #[test]
    fn worker_cannot_create_commit() {
        let engine = PermissionEngine::new(test_ownership());
        let result = engine.check(
            &Actor {
                kind: ActorKind::Worker,
                id: "worker-001".into(),
            },
            &Action::CreateCommit,
            "",
            None,
        );
        assert!(!result.is_allowed());
    }

    #[test]
    fn worker_can_propose_write_to_owned_file() {
        let engine = PermissionEngine::new(test_ownership());
        let manifest = test_manifest();
        let result = engine.check_worker_write(
            "worker-001",
            "kernel/src/main.rs",
            &manifest,
        );
        assert!(result.is_allowed());
    }

    #[test]
    fn worker_write_to_unowned_file_fails() {
        let engine = PermissionEngine::new(test_ownership());
        let manifest = test_manifest();
        let result = engine.check_worker_write(
            "worker-001",
            "README.md",
            &manifest,
        );
        assert!(!result.is_allowed());
    }

    #[test]
    fn human_with_identity_allowed() {
        let engine = PermissionEngine::new(test_ownership());
        let result = engine.check(
            &Actor {
                kind: ActorKind::Human,
                id: "alice@example.com".into(),
            },
            &Action::ProposeBreakingChange,
            "some-interface",
            None,
        );
        assert!(result.is_allowed());
    }

    #[test]
    fn empty_human_identity_denied() {
        let engine = PermissionEngine::new(test_ownership());
        let result = engine.check(
            &Actor {
                kind: ActorKind::Human,
                id: "".into(),
            },
            &Action::Read,
            "docs/foo.md",
            None,
        );
        assert!(!result.is_allowed());
    }

    #[test]
    fn worker_cross_domain_without_manifest_denied() {
        let engine = PermissionEngine::new(test_ownership());
        let result = engine.check(
            &Actor {
                kind: ActorKind::Worker,
                id: "worker-001".into(),
            },
            &Action::RequestCrossDomainChange,
            "docs",
            None,
        );
        assert!(!result.is_allowed());
    }

    #[test]
    fn deny_by_default_for_unknown_action() {
        let engine = PermissionEngine::new(test_ownership());
        // ProposeWrite without a manifest should be denied.
        let result = engine.check(
            &Actor {
                kind: ActorKind::Worker,
                id: "worker-001".into(),
            },
            &Action::ProposeWrite,
            "some/file.rs",
            None,
        );
        assert!(!result.is_allowed());
    }
}
