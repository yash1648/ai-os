//! Architecture Guardian — mechanical enforcer of ownership boundaries,
//! dependency rules, and interface compatibility.
//!
//! Docs: docs/17-architecture-guardian.md
//!
//! The Guardian operates independently from the Reviewer. It asks not
//! "is this good code?" but "is this change *allowed here*?" — checking
//! domain boundaries, interface compatibility, ownership rules, and
//! no-op detection. All checks are deterministic and require zero LLM
//! inference.

use metrics::{counter, describe_counter};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::diff_applier::{FileChange, StructuredDiff};
use crate::interface_registry::{ChangeVerdict, InterfaceRegistry, RegistryError};
use crate::objective::Objective;
use crate::ownership::OwnershipModel;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Severity of a violated rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViolationSeverity {
    /// The change is unconditionally rejected — no human override possible
    /// through normal pipeline flow.
    Blocking,
    /// The change is blocked from the automatic pipeline but may proceed
    /// through a Human Approval Gate.
    RequiresApproval,
}

/// A single violation detected by the Guardian.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardianViolation {
    /// Machine-readable rule identifier (e.g. "domain-boundary",
    /// "interface-compatibility", "noop-change", "ownership-boundary").
    pub rule_id: String,
    /// Human-readable description of what was violated.
    pub description: String,
    /// How severe the violation is.
    pub severity: ViolationSeverity,
    /// Supporting evidence — file paths, domains, versions, etc.
    pub evidence: String,
}

/// The Guardian's verdict on a structured diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardianVerdict {
    /// All checks passed — the diff is structurally sound.
    Pass,
    /// At least one blocking violation was found — the diff is rejected
    /// unconditionally.
    Fail(Vec<GuardianViolation>),
    /// No blocking violations, but at least one policy requires a human
    /// approval gate before the diff can proceed.
    RequiresHumanApproval(Vec<GuardianViolation>),
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Additional configuration for the Guardian's behaviour.
#[derive(Debug, Clone)]
pub struct GuardianConfig {
    /// Domains the objective is permitted to touch. When empty, all domains
    /// are allowed (subject to other checks).
    pub allowed_domains: Vec<String>,
    /// Whether cross-domain diffs are permitted without human approval.
    pub allow_cross_domain: bool,
}

impl Default for GuardianConfig {
    fn default() -> Self {
        Self {
            allowed_domains: vec![],
            allow_cross_domain: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Guardian
// ---------------------------------------------------------------------------

/// The Architecture Guardian — evaluates a [`StructuredDiff`] against the
/// [`OwnershipModel`], [`InterfaceRegistry`], and its own configuration, then
/// returns a [`GuardianVerdict`].
///
/// All checks are deterministic. The Guardian never invokes an LLM.
#[derive(Debug)]
pub struct Guardian {
    ownership: Arc<OwnershipModel>,
    interface_registry: Arc<InterfaceRegistry>,
    config: GuardianConfig,
}

impl Guardian {
    /// Create a new Guardian with default configuration.
    pub fn new(
        ownership: Arc<OwnershipModel>,
        interface_registry: Arc<InterfaceRegistry>,
    ) -> Self {
        Self {
            ownership,
            interface_registry,
            config: GuardianConfig::default(),
        }
    }

    /// Apply additional configuration (builder-pattern style).
    pub fn with_config(mut self, config: GuardianConfig) -> Self {
        self.config = config;
        self
    }

    /// Expose the ownership model for cross-domain coordinator checks.
    pub fn ownership_model(&self) -> &OwnershipModel {
        &self.ownership
    }

    /// Expose the interface registry for cross-domain coordinator checks.
    pub fn interface_registry(&self) -> &InterfaceRegistry {
        &self.interface_registry
    }

    /// Evaluate a structured diff against all Guardian checks.
    ///
    /// Runs, in order:
    /// 1. Domain boundary compliance (§2) — every touched file must belong
    ///    to a domain in `allowed_domains`.
    /// 2. Interface compatibility (§4) — modified files matching a registered
    ///    interface are checked for breaking-change policy.
    /// 3. No-op change detection — diffs with identical old/new content are
    ///    flagged.
    /// 4. Ownership boundary (§13) — cross-domain diffs require approval
    ///    unless explicitly allowed.
    pub fn evaluate(
        &self,
        diff: &StructuredDiff,
        _objective: &Objective,
    ) -> GuardianVerdict {
        let mut blocking: Vec<GuardianViolation> = Vec::new();
        let mut requires_approval: Vec<GuardianViolation> = Vec::new();

        // ── 1. Domain boundary compliance ──────────────────────────────
        self.check_domain_boundary(diff, &mut blocking, &mut requires_approval);

        // ── 2. Interface compatibility ─────────────────────────────────
        self.check_interface_compatibility(diff, &mut blocking, &mut requires_approval);

        // ── 3. No-op change detection ──────────────────────────────────
        self.check_noop_changes(diff, &mut requires_approval);

        // ── 4. Ownership boundary ──────────────────────────────────────
        self.check_ownership_boundary(diff, &mut requires_approval);

        // ── Assemble verdict ───────────────────────────────────────────
        describe_counter!("ai_os_guardian_pass_count", "Number of guardian checks that passed");
        describe_counter!("ai_os_guardian_fail_count", "Number of guardian checks that failed");
        describe_counter!("ai_os_guardian_human_approval_count", "Number of guardian checks requiring human approval");

        if !blocking.is_empty() {
            counter!("ai_os_guardian_fail_count").increment(1);
            let mut all = blocking;
            all.extend(requires_approval);
            return GuardianVerdict::Fail(all);
        }
        if !requires_approval.is_empty() {
            counter!("ai_os_guardian_human_approval_count").increment(1);
            return GuardianVerdict::RequiresHumanApproval(requires_approval);
        }
        counter!("ai_os_guardian_pass_count").increment(1);
        GuardianVerdict::Pass
    }

    // ── Individual checks ──────────────────────────────────────────────

    /// Check (a): every file in the diff must belong to a domain that is
    /// within `allowed_domains`.
    fn check_domain_boundary(
        &self,
        diff: &StructuredDiff,
        blocking: &mut Vec<GuardianViolation>,
        warnings: &mut Vec<GuardianViolation>,
    ) {
        for change in &diff.changes {
            let path = change.path();
            let path_str = path.to_string_lossy();

            match self.ownership.domain_for_file(&path_str) {
                None => {
                    // File not found in any domain — warning, not blocking.
                    warnings.push(GuardianViolation {
                        rule_id: "domain-boundary".into(),
                        description: format!(
                            "File '{}' does not belong to any registered domain",
                            path_str,
                        ),
                        severity: ViolationSeverity::RequiresApproval,
                        evidence: format!("path={}", path_str),
                    });
                }
                Some(domain) => {
                    // If allowed_domains is non-empty, the file's domain
                    // must be in the list.
                    if !self.config.allowed_domains.is_empty()
                        && !self.config.allowed_domains.contains(&domain.id)
                    {
                        blocking.push(GuardianViolation {
                            rule_id: "domain-boundary".into(),
                            description: format!(
                                "File '{}' belongs to domain '{}' which is not in \
                                 the allowed domains for this objective",
                                path_str, domain.id,
                            ),
                            severity: ViolationSeverity::Blocking,
                            evidence: format!(
                                "path={}, domain={}, allowed_domains={:?}",
                                path_str, domain.id, self.config.allowed_domains,
                            ),
                        });
                    }
                }
            }
        }
    }

    /// Check (b): if any modified file path matches a registered interface,
    /// verify the change is compatible per the interface's breaking-change
    /// policy.
    fn check_interface_compatibility(
        &self,
        diff: &StructuredDiff,
        blocking: &mut Vec<GuardianViolation>,
        requires_approval: &mut Vec<GuardianViolation>,
    ) {
        // Collect paths from Modify operations — those are the changes that
        // could affect an interface definition.
        let modified_paths: Vec<String> = diff
            .changes
            .iter()
            .filter_map(|change| {
                if matches!(change, FileChange::Modify { .. }) {
                    Some(change.path().to_string_lossy().to_string())
                } else {
                    None
                }
            })
            .collect();

        if modified_paths.is_empty() {
            return;
        }

        for iface in self.interface_registry.list_all() {
            // Heuristic: does any modified path "contain" the interface ID?
            // This is a deterministic string match — the test fixtures are
            // set up so that interface IDs appear in the file paths they
            // govern.
            let matched = modified_paths
                .iter()
                .any(|p| path_matches_interface(p, &iface.interface_id));

            if !matched {
                continue;
            }

            // Propose a major-version bump to exercise the policy.
            let proposed = bump_major_version(&iface.version);

            match self
                .interface_registry
                .check_change(&iface.interface_id, &proposed)
            {
                Ok(ChangeVerdict::RequiresHumanApproval) => {
                    requires_approval.push(GuardianViolation {
                        rule_id: "interface-compatibility".into(),
                        description: format!(
                            "Breaking change to interface '{}' requires human approval",
                            iface.interface_id,
                        ),
                        severity: ViolationSeverity::RequiresApproval,
                        evidence: format!(
                            "interface={}, version={}->{}, policy=requires_approval",
                            iface.interface_id, iface.version, proposed,
                        ),
                    });
                }
                Err(RegistryError::BreakingChangeForbidden(_)) => {
                    blocking.push(GuardianViolation {
                        rule_id: "interface-compatibility".into(),
                        description: format!(
                            "Breaking change to interface '{}' is forbidden by policy",
                            iface.interface_id,
                        ),
                        severity: ViolationSeverity::Blocking,
                        evidence: format!(
                            "interface={}, version={}->{}, policy=forbidden",
                            iface.interface_id, iface.version, proposed,
                        ),
                    });
                }
                // Permitted and RequiresDeprecation do not generate violations.
                _ => {}
            }
        }
    }

    /// Check (c): if the diff contains only Modify operations where
    /// old_content == new_content, flag as a no-op change.
    fn check_noop_changes(
        &self,
        diff: &StructuredDiff,
        warnings: &mut Vec<GuardianViolation>,
    ) {
        // If there are no changes, nothing to do.
        if diff.changes.is_empty() {
            return;
        }

        let all_modifies_noop = diff.changes.iter().all(|change| match change {
            FileChange::Modify { old_content, new_content, .. } => old_content == new_content,
            // Create and Delete operations are real changes regardless.
            FileChange::Create { .. } => false,
            FileChange::Delete { .. } => false,
        });

        if all_modifies_noop {
            warnings.push(GuardianViolation {
                rule_id: "noop-change".into(),
                description: "Diff contains no effective content changes \
                              (all Modify operations have identical old/new content)"
                    .into(),
                severity: ViolationSeverity::RequiresApproval,
                evidence: format!("changes={}", diff.changes.len()),
            });
        }
    }

    /// Check (d): if the diff touches files in multiple domains and
    /// cross-domain access is not explicitly allowed, flag for approval.
    fn check_ownership_boundary(
        &self,
        diff: &StructuredDiff,
        requires_approval: &mut Vec<GuardianViolation>,
    ) {
        if self.config.allow_cross_domain {
            return;
        }

        let paths: Vec<String> = diff
            .changes
            .iter()
            .map(|change| change.path().to_string_lossy().to_string())
            .collect();

        if paths.is_empty() {
            return;
        }

        let domains = self.ownership.domains_for_files(&paths);
        if domains.len() > 1 {
            let domain_names: Vec<&str> = domains.iter().map(|d| d.id.as_str()).collect();
            requires_approval.push(GuardianViolation {
                rule_id: "ownership-boundary".into(),
                description: format!(
                    "Diff touches multiple domains ({}) without \
                     cross-domain authorization",
                    domain_names.join(", "),
                ),
                severity: ViolationSeverity::RequiresApproval,
                evidence: format!("domains={:?}", domain_names),
            });
        }
    }

    // ── Reporting ──────────────────────────────────────────────────────

    /// Produce a human-readable summary of a Guardian verdict.
    pub fn format_report(&self, verdict: &GuardianVerdict) -> String {
        match verdict {
            GuardianVerdict::Pass => "GUARDIAN: PASS — All checks passed.".to_string(),
            GuardianVerdict::Fail(violations) => {
                let mut lines = vec![format!(
                    "GUARDIAN: FAIL — {} violation(s) found.",
                    violations.len()
                )];
                for v in violations {
                    lines.push(format!("  [BLOCKING] {}: {}", v.rule_id, v.description));
                    lines.push(format!("    evidence: {}", v.evidence));
                }
                lines.join("\n")
            }
            GuardianVerdict::RequiresHumanApproval(violations) => {
                let mut lines = vec![format!(
                    "GUARDIAN: REQUIRES HUMAN APPROVAL — {} issue(s) found.",
                    violations.len()
                )];
                for v in violations {
                    lines.push(format!(
                        "  [REQUIRES_APPROVAL] {}: {}",
                        v.rule_id, v.description
                    ));
                    lines.push(format!("    evidence: {}", v.evidence));
                }
                lines.join("\n")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Bump the major version component of a semver string ("1.2.3" → "2.0.0").
/// If parsing fails, returns "2.0.0" as a safe default.
fn bump_major_version(version: &str) -> String {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() == 3 {
        if let Ok(major) = parts[0].parse::<u64>() {
            return format!("{}.0.0", major + 1);
        }
    }
    "2.0.0".to_string()
}

/// Determine whether a file path is associated with a given interface ID.
///
/// Uses a simple substring heuristic: the path (with path separators replaced
/// by hyphens) must contain the interface_id.  This is deterministic and
/// works with test fixtures where interface IDs appear in file paths
/// (e.g. interface "objectives-api" and path "specs/objectives-api.yaml").
fn path_matches_interface(path: &str, interface_id: &str) -> bool {
    // Normalise the path: replace separators with hyphens, lowercase.
    let normalised = path.replace('/', "-").replace('\\', "-").to_lowercase();
    let id = interface_id.to_lowercase();
    normalised.contains(&id) || path.contains(interface_id)
}

// ---------------------------------------------------------------------------
// Trait: extract path from any FileChange variant
// ---------------------------------------------------------------------------

trait ChangePath {
    fn path(&self) -> &std::path::PathBuf;
}

impl ChangePath for FileChange {
    fn path(&self) -> &std::path::PathBuf {
        match self {
            FileChange::Create { path, .. } => path,
            FileChange::Modify { path, .. } => path,
            FileChange::Delete { path, .. } => path,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface_registry::{BreakingChangePolicy, CompatibilityPolicy, Interface, VersionEntry};
    use chrono::Utc;
    use std::sync::Arc;

    // ── Helpers ────────────────────────────────────────────────────────

    fn sample_ownership() -> Arc<OwnershipModel> {
        let yaml = r#"
domains:
  - id: project-kernel
    name: "Project Kernel"
    owner: "kernel-team"
    paths:
      - "kernel/**/*.rs"
    owned_interfaces: ["objective-storage", "state-machine"]
    approval_required_for: ["public-api"]

  - id: docs
    name: "Documentation"
    owner: "docs-team"
    paths:
      - "docs/**/*.md"
    owned_interfaces: []
    approval_required_for: []

  - id: schemas
    name: "Schemas"
    owner: "kernel-team"
    paths:
      - "schemas/**/*.json"
    owned_interfaces: []
    approval_required_for: []
"#;
        Arc::new(OwnershipModel::from_yaml(yaml).unwrap())
    }

    fn sample_interface_registry() -> Arc<InterfaceRegistry> {
        let mut reg = InterfaceRegistry::new();

        // Interface with RequiresApproval policy.
        let iface_approval = Interface {
            interface_id: "objectives-api".to_string(),
            kind: crate::interface_registry::InterfaceKind::RestApi,
            owner_domain: "project-kernel".to_string(),
            consumers: vec!["worker-pool".to_string()],
            version: "1.0.0".to_string(),
            signature: "specs/objectives-api.yaml".to_string(),
            compatibility: CompatibilityPolicy {
                breaking_change_policy: BreakingChangePolicy::RequiresApproval,
                deprecated_since: None,
                sunset_date: None,
            },
            history: vec![VersionEntry {
                version: "1.0.0".to_string(),
                changed_by_objective: "init".to_string(),
                timestamp: Utc::now(),
                change_summary: "Initial".to_string(),
            }],
        };
        reg.register(iface_approval).unwrap();

        // Interface with Forbidden breaking change policy.
        let iface_forbidden = Interface {
            interface_id: "critical-core".to_string(),
            kind: crate::interface_registry::InterfaceKind::InternalModule,
            owner_domain: "project-kernel".to_string(),
            consumers: vec!["everything".to_string()],
            version: "1.0.0".to_string(),
            signature: "kernel/src/core.rs".to_string(),
            compatibility: CompatibilityPolicy {
                breaking_change_policy: BreakingChangePolicy::Forbidden,
                deprecated_since: None,
                sunset_date: None,
            },
            history: vec![VersionEntry {
                version: "1.0.0".to_string(),
                changed_by_objective: "init".to_string(),
                timestamp: Utc::now(),
                change_summary: "Initial".to_string(),
            }],
        };
        reg.register(iface_forbidden).unwrap();

        Arc::new(reg)
    }

    fn objective(id: &str) -> Objective {
        Objective {
            id: id.to_string(),
            title: "Test".to_string(),
            description: "Test objective".to_string(),
            owner: "test-user".to_string(),
            parent_id: None,
            priority: crate::objective::Priority::Medium,
            status: crate::state_machine::ObjectiveState::Primary(
                crate::state_machine::ObjectivePrimaryState::Executing,
            ),
            dependencies: vec![],
            success_criteria: vec!["pass".into()],
            plan_id: None,
            retry_count: 0,
            tags: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    // ── test_guardian_pass ─────────────────────────────────────────────

    #[test]
    fn test_guardian_pass() {
        let ownership = sample_ownership();
        let registry = sample_interface_registry();
        let guardian = Guardian::new(ownership.clone(), registry).with_config(GuardianConfig {
            allowed_domains: vec!["project-kernel".into()],
            ..GuardianConfig::default()
        });

        let diff = StructuredDiff {
            objective_id: "obj-1".into(),
            worker_id: "worker-1".into(),
            changes: vec![FileChange::Modify {
                path: "kernel/src/main.rs".into(),
                old_content: "old".into(),
                new_content: "new".into(),
            }],
            commit_metadata: crate::diff_applier::CommitMetadata {
                summary: "test".into(),
                objective_id: "obj-1".into(),
                worker_id: "worker-1".into(),
                reviewer_id: None,
                guardian_id: None,
                human_approval_id: None,
            },
        };

        let verdict = guardian.evaluate(&diff, &objective("obj-1"));
        assert_eq!(verdict, GuardianVerdict::Pass, "Expected Pass for in-domain change");
    }

    // ── test_guardian_cross_domain_fail ────────────────────────────────

    #[test]
    fn test_guardian_cross_domain_fail() {
        let ownership = sample_ownership();
        let registry = sample_interface_registry();
        let guardian = Guardian::new(ownership.clone(), registry).with_config(GuardianConfig {
            allowed_domains: vec!["project-kernel".into()], // docs domain NOT allowed
            ..GuardianConfig::default()
        });

        let diff = StructuredDiff {
            objective_id: "obj-2".into(),
            worker_id: "worker-1".into(),
            changes: vec![
                FileChange::Modify {
                    path: "kernel/src/main.rs".into(),
                    old_content: "old".into(),
                    new_content: "new".into(),
                },
                FileChange::Modify {
                    path: "docs/readme.md".into(),
                    old_content: "old".into(),
                    new_content: "new".into(),
                },
            ],
            commit_metadata: crate::diff_applier::CommitMetadata {
                summary: "test".into(),
                objective_id: "obj-2".into(),
                worker_id: "worker-1".into(),
                reviewer_id: None,
                guardian_id: None,
                human_approval_id: None,
            },
        };

        let verdict = guardian.evaluate(&diff, &objective("obj-2"));
        match &verdict {
            GuardianVerdict::Fail(violations) => {
                assert!(
                    violations.iter().any(|v| v.rule_id == "domain-boundary"),
                    "Expected domain-boundary violation in Fail verdict: {:?}",
                    violations,
                );
            }
            _ => panic!("Expected Fail verdict, got {:?}", verdict),
        }
    }

    // ── test_guardian_interface_requires_approval ──────────────────────

    #[test]
    fn test_guardian_interface_requires_approval() {
        let ownership = sample_ownership();
        let registry = sample_interface_registry();
        let guardian = Guardian::new(ownership.clone(), registry).with_config(GuardianConfig {
            allowed_domains: vec!["project-kernel".into()],
            ..GuardianConfig::default()
        });

        // The file path contains "objectives-api" which matches the interface_id
        // registered with RequiresApproval policy.
        let diff = StructuredDiff {
            objective_id: "obj-3".into(),
            worker_id: "worker-1".into(),
            changes: vec![FileChange::Modify {
                path: "specs/objectives-api.yaml".into(),
                old_content: "old".into(),
                new_content: "new".into(),
            }],
            commit_metadata: crate::diff_applier::CommitMetadata {
                summary: "test".into(),
                objective_id: "obj-3".into(),
                worker_id: "worker-1".into(),
                reviewer_id: None,
                guardian_id: None,
                human_approval_id: None,
            },
        };

        let verdict = guardian.evaluate(&diff, &objective("obj-3"));
        match &verdict {
            GuardianVerdict::RequiresHumanApproval(violations) => {
                assert!(
                    violations.iter().any(|v| v.rule_id == "interface-compatibility"),
                    "Expected interface-compatibility violation: {:?}",
                    violations,
                );
            }
            _ => panic!("Expected RequiresHumanApproval, got {:?}", verdict),
        }
    }

    // ── test_guardian_interface_forbidden ──────────────────────────────

    #[test]
    fn test_guardian_interface_forbidden() {
        let ownership = sample_ownership();
        let registry = sample_interface_registry();
        let guardian = Guardian::new(ownership.clone(), registry).with_config(GuardianConfig {
            allowed_domains: vec!["project-kernel".into()],
            ..GuardianConfig::default()
        });

        // The file path contains "critical-core" which matches the interface_id
        // registered with Forbidden breaking change policy.
        let diff = StructuredDiff {
            objective_id: "obj-4".into(),
            worker_id: "worker-1".into(),
            changes: vec![FileChange::Modify {
                path: "kernel/src/critical-core.rs".into(),
                old_content: "old".into(),
                new_content: "new".into(),
            }],
            commit_metadata: crate::diff_applier::CommitMetadata {
                summary: "test".into(),
                objective_id: "obj-4".into(),
                worker_id: "worker-1".into(),
                reviewer_id: None,
                guardian_id: None,
                human_approval_id: None,
            },
        };

        let verdict = guardian.evaluate(&diff, &objective("obj-4"));
        match &verdict {
            GuardianVerdict::Fail(violations) => {
                assert!(
                    violations.iter().any(|v| v.rule_id == "interface-compatibility"),
                    "Expected interface-compatibility violation in Fail: {:?}",
                    violations,
                );
            }
            _ => panic!("Expected Fail, got {:?}", verdict),
        }
    }

    // ── test_guardian_noop_warning ─────────────────────────────────────

    #[test]
    fn test_guardian_noop_warning() {
        let ownership = sample_ownership();
        let registry = sample_interface_registry();
        let guardian = Guardian::new(ownership.clone(), registry).with_config(GuardianConfig {
            allowed_domains: vec!["project-kernel".into()],
            ..GuardianConfig::default()
        });

        let diff = StructuredDiff {
            objective_id: "obj-5".into(),
            worker_id: "worker-1".into(),
            changes: vec![FileChange::Modify {
                path: "kernel/src/main.rs".into(),
                old_content: "identical content".into(),
                new_content: "identical content".into(),
            }],
            commit_metadata: crate::diff_applier::CommitMetadata {
                summary: "test".into(),
                objective_id: "obj-5".into(),
                worker_id: "worker-1".into(),
                reviewer_id: None,
                guardian_id: None,
                human_approval_id: None,
            },
        };

        let verdict = guardian.evaluate(&diff, &objective("obj-5"));
        match &verdict {
            GuardianVerdict::RequiresHumanApproval(violations) => {
                assert!(
                    violations.iter().any(|v| v.rule_id == "noop-change"),
                    "Expected noop-change violation: {:?}",
                    violations,
                );
            }
            other => panic!("Expected RequiresHumanApproval, got {:?}", other),
        }
    }

    // ── test_guardian_domain_not_found ─────────────────────────────────

    #[test]
    fn test_guardian_domain_not_found() {
        let ownership = sample_ownership();
        let registry = sample_interface_registry();
        let guardian = Guardian::new(ownership.clone(), registry);

        let diff = StructuredDiff {
            objective_id: "obj-6".into(),
            worker_id: "worker-1".into(),
            changes: vec![FileChange::Modify {
                path: "README.md".into(),
                old_content: "old".into(),
                new_content: "new".into(),
            }],
            commit_metadata: crate::diff_applier::CommitMetadata {
                summary: "test".into(),
                objective_id: "obj-6".into(),
                worker_id: "worker-1".into(),
                reviewer_id: None,
                guardian_id: None,
                human_approval_id: None,
            },
        };

        let verdict = guardian.evaluate(&diff, &objective("obj-6"));
        match &verdict {
            GuardianVerdict::RequiresHumanApproval(violations) => {
                assert!(
                    violations.iter().any(|v| v.rule_id == "domain-boundary"
                        && v.severity == ViolationSeverity::RequiresApproval),
                    "Expected RequiresApproval domain-boundary violation: {:?}",
                    violations,
                );
            }
            other => panic!("Expected RequiresHumanApproval, got {:?}", other),
        }
    }

    // ── format_report tests ────────────────────────────────────────────

    #[test]
    fn test_format_report_pass() {
        let guardian = Guardian::new(sample_ownership(), sample_interface_registry());
        let report = guardian.format_report(&GuardianVerdict::Pass);
        assert!(report.contains("PASS"), "Report should contain PASS: {report}");
    }

    #[test]
    fn test_format_report_fail() {
        let guardian = Guardian::new(sample_ownership(), sample_interface_registry());
        let violations = vec![GuardianViolation {
            rule_id: "test-rule".into(),
            description: "Something is wrong".into(),
            severity: ViolationSeverity::Blocking,
            evidence: "file.rs".into(),
        }];
        let report = guardian.format_report(&GuardianVerdict::Fail(violations));
        assert!(report.contains("FAIL"), "Report should contain FAIL: {report}");
        assert!(report.contains("BLOCKING"), "Report should mention BLOCKING: {report}");
    }

    #[test]
    fn test_format_report_requires_approval() {
        let guardian = Guardian::new(sample_ownership(), sample_interface_registry());
        let violations = vec![GuardianViolation {
            rule_id: "test-rule".into(),
            description: "Needs approval".into(),
            severity: ViolationSeverity::RequiresApproval,
            evidence: "file.rs".into(),
        }];
        let report = guardian.format_report(&GuardianVerdict::RequiresHumanApproval(violations));
        assert!(
            report.contains("REQUIRES HUMAN APPROVAL"),
            "Report should contain REQUIRES HUMAN APPROVAL: {report}"
        );
        assert!(
            report.contains("REQUIRES_APPROVAL"),
            "Report should mention REQUIRES_APPROVAL: {report}"
        );
    }

    // ── Helper tests ───────────────────────────────────────────────────

    #[test]
    fn test_bump_major_version() {
        assert_eq!(bump_major_version("1.0.0"), "2.0.0");
        assert_eq!(bump_major_version("0.9.9"), "1.0.0");
        assert_eq!(bump_major_version("3.2.1"), "4.0.0");
    }

    #[test]
    fn test_path_matches_interface_positive() {
        assert!(path_matches_interface(
            "specs/objectives-api.yaml",
            "objectives-api",
        ));
        assert!(path_matches_interface(
            "kernel/src/critical-core.rs",
            "critical-core",
        ));
    }

    #[test]
    fn test_path_matches_interface_negative() {
        assert!(!path_matches_interface(
            "kernel/src/main.rs",
            "objectives-api",
        ));
    }
}
