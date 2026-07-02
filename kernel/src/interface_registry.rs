//! Interface Registry — tracks every declared contract in the system.
//!
//! Docs: docs/10-interface-registry.md
//!
//! Interfaces are typed contracts (REST API, event schema, module boundary,
//! etc.) with an owner domain, consumer list, semver versioning, and a
//! breaking-change policy.  The Registry provides blast-radius analysis and
//! compatibility enforcement for the Architecture Guardian.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Kinds of interfaces the registry tracks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterfaceKind {
    RestApi,
    InternalModule,
    EventSchema,
    DbSchema,
    Cli,
    Sdk,
}

/// Breaking-change policy for an interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BreakingChangePolicy {
    /// Any incompatible signature change is auto-rejected.
    Forbidden,
    /// Incompatible changes route to a mandatory Human Approval Gate.
    RequiresApproval,
    /// Permitted only with a coexisting deprecation path.
    AllowedWithDeprecation,
}

/// A compatibility policy block attached to an interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatibilityPolicy {
    pub breaking_change_policy: BreakingChangePolicy,
    #[serde(default)]
    pub deprecated_since: Option<String>,
    #[serde(default)]
    pub sunset_date: Option<DateTime<Utc>>,
}

/// One entry in the version history log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionEntry {
    pub version: String,
    pub changed_by_objective: String,
    pub timestamp: DateTime<Utc>,
    pub change_summary: String,
}

/// A registered interface.
///
/// Matches the YAML model from docs/10-interface-registry.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interface {
    pub interface_id: String,
    pub kind: InterfaceKind,
    /// The domain that owns this interface (authoritative for signature changes).
    pub owner_domain: String,
    /// Domains or services that depend on this interface.
    #[serde(default)]
    pub consumers: Vec<String>,
    /// Semver version string (e.g. "1.2.3").
    pub version: String,
    /// Canonical representation — OpenAPI fragment, type signature, schema, etc.
    pub signature: String,
    /// Compatibility policy for this interface.
    pub compatibility: CompatibilityPolicy,
    /// Version history log.
    #[serde(default)]
    pub history: Vec<VersionEntry>,
}

/// The Interface Registry.
///
/// Provides registration, lookup, blast-radius, and compatibility checks.
/// In Stage 2 the registry is in-memory; a persisted backend is added in
/// Stage 3.
#[derive(Debug, Clone)]
pub struct InterfaceRegistry {
    interfaces: HashMap<String, Interface>,
}

impl InterfaceRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            interfaces: HashMap::new(),
        }
    }

    /// Register an interface.  Returns an error if the `interface_id`
    /// already exists (use `update` to modify).
    pub fn register(&mut self, interface: Interface) -> Result<(), RegistryError> {
        let id = interface.interface_id.clone();
        if self.interfaces.contains_key(&id) {
            return Err(RegistryError::AlreadyRegistered(id));
        }
        self.interfaces.insert(id.clone(), interface);
        Ok(())
    }

    /// Update an existing interface.  Returns an error if not found.
    pub fn update(&mut self, interface: Interface) -> Result<(), RegistryError> {
        let id = interface.interface_id.clone();
        if !self.interfaces.contains_key(&id) {
            return Err(RegistryError::NotFound(id));
        }
        self.interfaces.insert(id, interface);
        Ok(())
    }

    /// Look up an interface by ID.
    pub fn get(&self, interface_id: &str) -> Option<&Interface> {
        self.interfaces.get(interface_id)
    }

    /// List all interface IDs belonging to a domain.
    pub fn list_by_domain(&self, domain_id: &str) -> Vec<&Interface> {
        self.interfaces
            .values()
            .filter(|iface| iface.owner_domain == domain_id)
            .collect()
    }

    /// List all registered interfaces.
    pub fn list_all(&self) -> Vec<&Interface> {
        self.interfaces.values().collect()
    }

    /// Return the set of domains that consume a given interface.
    /// This is the blast-radius set for a change to that interface.
    pub fn consumers_of(&self, interface_id: &str) -> Vec<String> {
        self.interfaces
            .get(interface_id)
            .map(|iface| iface.consumers.clone())
            .unwrap_or_default()
    }

    /// Check whether a version change constitutes a breaking change
    /// according to semver conventions (major bump = breaking).
    pub fn is_breaking_change(current_version: &str, proposed_version: &str) -> bool {
        let parse_version = |v: &str| -> Option<Vec<u64>> {
            let parts: Vec<&str> = v.split('.').collect();
            if parts.len() != 3 {
                return None;
            }
            Some(vec![
                parts[0].parse().ok()?,
                parts[1].parse().ok()?,
                parts[2].parse().ok()?,
            ])
        };

        let current = match parse_version(current_version) {
            Some(v) => v,
            None => return false, // can't parse — assume non-breaking
        };
        let proposed = match parse_version(proposed_version) {
            Some(v) => v,
            None => return false,
        };

        // Major bump (1.x.y → 2.0.0) is breaking.
        proposed[0] > current[0]
    }

    /// Check whether a proposed change to an interface is compatible
    /// per its policy.
    ///
    /// Returns `Ok` if the change is allowed (possibly subject to human
    /// approval), or `Err` with a reason if rejected outright.
    pub fn check_change(
        &self,
        interface_id: &str,
        proposed_version: &str,
    ) -> Result<ChangeVerdict, RegistryError> {
        let iface = self
            .interfaces
            .get(interface_id)
            .ok_or_else(|| RegistryError::NotFound(interface_id.to_string()))?;

        let is_breaking = Self::is_breaking_change(&iface.version, proposed_version);

        if !is_breaking {
            // Non-breaking changes are always permitted through the normal pipeline.
            return Ok(ChangeVerdict::Permitted);
        }

        match iface.compatibility.breaking_change_policy {
            BreakingChangePolicy::Forbidden => Err(RegistryError::BreakingChangeForbidden(
                interface_id.to_string(),
            )),
            BreakingChangePolicy::RequiresApproval => Ok(ChangeVerdict::RequiresHumanApproval),
            BreakingChangePolicy::AllowedWithDeprecation => Ok(ChangeVerdict::RequiresDeprecation),
        }
    }

    /// Compute the full blast radius for a given interface: all consumer
    /// domains that would be impacted by a change.
    pub fn blast_radius(&self, interface_id: &str) -> BlastRadius {
        let iface = match self.interfaces.get(interface_id) {
            Some(i) => i,
            None => {
                return BlastRadius {
                    interface_id: interface_id.to_string(),
                    owner_domain: String::new(),
                    consumers: vec![],
                    consumer_count: 0,
                };
            }
        };

        let consumers = iface.consumers.clone();
        let count = consumers.len();

        BlastRadius {
            interface_id: interface_id.to_string(),
            owner_domain: iface.owner_domain.clone(),
            consumers,
            consumer_count: count,
        }
    }
}

impl Default for InterfaceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of a compatibility check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeVerdict {
    /// Change is permitted through the normal pipeline.
    Permitted,
    /// Breaking change requires a Human Approval Gate.
    RequiresHumanApproval,
    /// Breaking change requires a coexisting deprecation path.
    RequiresDeprecation,
}

/// Blast-radius report for a proposed interface change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlastRadius {
    pub interface_id: String,
    pub owner_domain: String,
    pub consumers: Vec<String>,
    pub consumer_count: usize,
}

/// Errors from registry operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    NotFound(String),
    AlreadyRegistered(String),
    BreakingChangeForbidden(String),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::NotFound(id) => write!(f, "interface '{}' not found", id),
            RegistryError::AlreadyRegistered(id) => {
                write!(f, "interface '{}' is already registered", id)
            }
            RegistryError::BreakingChangeForbidden(id) => {
                write!(
                    f,
                    "breaking change to '{}' is forbidden by policy",
                    id
                )
            }
        }
    }
}

impl std::error::Error for RegistryError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_interface(id: &str) -> Interface {
        Interface {
            interface_id: id.to_string(),
            kind: InterfaceKind::RestApi,
            owner_domain: "kernel".to_string(),
            consumers: vec!["worker-pool".to_string(), "cli".to_string()],
            version: "1.0.0".to_string(),
            signature: "GET /api/v1/objectives".to_string(),
            compatibility: CompatibilityPolicy {
                breaking_change_policy: BreakingChangePolicy::RequiresApproval,
                deprecated_since: None,
                sunset_date: None,
            },
            history: vec![VersionEntry {
                version: "1.0.0".to_string(),
                changed_by_objective: "init".to_string(),
                timestamp: Utc::now(),
                change_summary: "Initial version".to_string(),
            }],
        }
    }

    #[test]
    fn register_and_lookup() {
        let mut reg = InterfaceRegistry::new();
        let iface = sample_interface("objectives-api");
        reg.register(iface).unwrap();
        assert!(reg.get("objectives-api").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn double_register_fails() {
        let mut reg = InterfaceRegistry::new();
        let iface = sample_interface("objectives-api");
        reg.register(iface).unwrap();
        let dup = sample_interface("objectives-api");
        let result = reg.register(dup);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            RegistryError::AlreadyRegistered("objectives-api".to_string())
        );
    }

    #[test]
    fn update_nonexistent_fails() {
        let mut reg = InterfaceRegistry::new();
        let iface = sample_interface("objectives-api");
        let result = reg.update(iface);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            RegistryError::NotFound("objectives-api".to_string())
        );
    }

    #[test]
    fn consumers_of_returns_consumer_list() {
        let mut reg = InterfaceRegistry::new();
        let iface = sample_interface("objectives-api");
        reg.register(iface).unwrap();
        let consumers = reg.consumers_of("objectives-api");
        assert_eq!(consumers, vec!["worker-pool", "cli"]);
    }

    #[test]
    fn consumers_of_nonexistent_returns_empty() {
        let reg = InterfaceRegistry::new();
        let consumers = reg.consumers_of("nonexistent");
        assert!(consumers.is_empty());
    }

    #[test]
    fn list_by_domain() {
        let mut reg = InterfaceRegistry::new();
        reg.register(sample_interface("objectives-api")).unwrap();
        let mut iface2 = sample_interface("planner-api");
        iface2.owner_domain = "planner".to_string();
        reg.register(iface2).unwrap();
        let kernel_interfaces = reg.list_by_domain("kernel");
        assert_eq!(kernel_interfaces.len(), 1);
        assert_eq!(kernel_interfaces[0].interface_id, "objectives-api");
    }

    #[test]
    fn major_bump_is_breaking() {
        assert!(InterfaceRegistry::is_breaking_change("1.0.0", "2.0.0"));
        assert!(InterfaceRegistry::is_breaking_change("1.9.9", "2.0.0"));
        assert!(!InterfaceRegistry::is_breaking_change("1.0.0", "1.1.0"));
        assert!(!InterfaceRegistry::is_breaking_change("1.0.0", "1.0.1"));
    }

    #[test]
    fn breaking_change_forbidden_is_rejected() {
        let mut reg = InterfaceRegistry::new();
        let mut iface = sample_interface("critical-api");
        iface.compatibility.breaking_change_policy = BreakingChangePolicy::Forbidden;
        reg.register(iface).unwrap();
        let result = reg.check_change("critical-api", "2.0.0");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            RegistryError::BreakingChangeForbidden("critical-api".to_string())
        );
    }

    #[test]
    fn breaking_change_requires_approval() {
        let mut reg = InterfaceRegistry::new();
        reg.register(sample_interface("objectives-api")).unwrap();
        let result = reg.check_change("objectives-api", "2.0.0").unwrap();
        assert_eq!(result, ChangeVerdict::RequiresHumanApproval);
    }

    #[test]
    fn non_breaking_change_is_permitted() {
        let mut reg = InterfaceRegistry::new();
        reg.register(sample_interface("objectives-api")).unwrap();
        let result = reg.check_change("objectives-api", "1.1.0").unwrap();
        assert_eq!(result, ChangeVerdict::Permitted);
    }

    #[test]
    fn blast_radius_reports_consumers() {
        let mut reg = InterfaceRegistry::new();
        reg.register(sample_interface("objectives-api")).unwrap();
        let radius = reg.blast_radius("objectives-api");
        assert_eq!(radius.consumer_count, 2);
        assert!(radius.consumers.contains(&"worker-pool".to_string()));
        assert!(radius.consumers.contains(&"cli".to_string()));
        assert_eq!(radius.owner_domain, "kernel");
    }

    #[test]
    fn blast_radius_nonexistent_returns_empty() {
        let reg = InterfaceRegistry::new();
        let radius = reg.blast_radius("nonexistent");
        assert_eq!(radius.consumer_count, 0);
        assert!(radius.consumers.is_empty());
    }
}
