//! Ownership Model — partitions the repository into domains with single owners.
//!
//! Docs: docs/13-ownership-model.md
//!
//! A domain is a named partition of the repository identified by glob patterns.
//! Every file in the repository must belong to exactly one domain.

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A named partition of the repository with a single designated owner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Domain {
    /// Canonical identifier (e.g. "project-kernel", "goal-decomposer").
    pub id: String,

    /// Human-readable name.
    pub name: String,

    /// Team or worker-specialization identifier.
    pub owner: String,

    /// Glob patterns defining which files belong to this domain.
    pub paths: Vec<String>,

    /// Interface IDs this domain is authoritative for.
    #[serde(default)]
    pub owned_interfaces: Vec<String>,

    /// Change categories needing domain-owner sign-off.
    #[serde(default)]
    pub approval_required_for: Vec<String>,
}

/// YAML config file format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainConfig {
    pub domains: Vec<Domain>,
}

/// Resolves file paths to owning domains and validates domain coverage.
///
/// Constructed from a YAML config file at startup. All permission checks
/// and manifest-scoping decisions flow through this model.
#[derive(Debug)]
pub struct OwnershipModel {
    domains: Vec<Domain>,
    /// Pre-compiled glob sets indexed by domain index.
    glob_sets: Vec<GlobSet>,
}

impl OwnershipModel {
    /// Load from a YAML config file on disk.
    pub fn from_config(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let yaml = std::fs::read_to_string(path)?;
        Self::from_yaml(&yaml)
    }

    /// Parse from a YAML string.
    pub fn from_yaml(yaml: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let config: DomainConfig = serde_yaml::from_str(yaml)?;
        let mut glob_sets = Vec::with_capacity(config.domains.len());

        for domain in &config.domains {
            let mut builder = GlobSetBuilder::new();
            for pattern in &domain.paths {
                let glob = Glob::new(pattern)?;
                builder.add(glob);
            }
            glob_sets.push(builder.build()?);
        }

        let model = Self {
            domains: config.domains,
            glob_sets,
        };
        model.validate().map_err(|errors| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                errors.join("; "),
            )) as Box<dyn std::error::Error>
        })?;
        Ok(model)
    }

    /// Return a reference to all registered domains.
    pub fn domains(&self) -> &[Domain] {
        &self.domains
    }

    /// Find the domain that owns a given file path.
    ///
    /// Returns `None` if the path does not match any domain.
    pub fn domain_for_file(&self, path: &str) -> Option<&Domain> {
        for (i, gs) in self.glob_sets.iter().enumerate() {
            if gs.is_match(path) {
                return Some(&self.domains[i]);
            }
        }
        None
    }

    /// Find all distinct domains covering a set of file paths.
    pub fn domains_for_files(&self, paths: &[String]) -> Vec<&Domain> {
        let mut result: Vec<&Domain> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for path in paths {
            if let Some(domain) = self.domain_for_file(path) {
                if seen.insert(&domain.id) {
                    result.push(domain);
                }
            }
        }
        result
    }

    /// Validate domain config for internal consistency.
    ///
    /// Checks:
    /// - At least one domain is defined.
    /// - No two domains share an identical path pattern (exact string match).
    /// - Every domain has at least one path pattern.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors: Vec<String> = Vec::new();

        if self.domains.is_empty() {
            errors.push("No domains defined".to_string());
            return Err(errors);
        }

        for domain in &self.domains {
            if domain.paths.is_empty() {
                errors.push(format!("Domain '{}' has no path patterns", domain.id));
            }
        }

        // Check for duplicate path patterns across domains.
        let mut seen_patterns: std::collections::HashMap<&str, &str> =
            std::collections::HashMap::new();
        for domain in &self.domains {
            for pattern in &domain.paths {
                if let Some(existing) = seen_patterns.get(pattern.as_str()) {
                    errors.push(format!(
                        "Duplicate path pattern '{}' in domains '{}' and '{}'",
                        pattern, domain.id, existing
                    ));
                } else {
                    seen_patterns.insert(pattern, &domain.id);
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Number of registered domains.
    pub fn len(&self) -> usize {
        self.domains.len()
    }

    /// Whether no domains are registered.
    pub fn is_empty(&self) -> bool {
        self.domains.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_yaml() -> &'static str {
        r#"
domains:
  - id: project-kernel
    name: "Project Kernel"
    owner: "kernel-team"
    paths:
      - "kernel/**/*.rs"
      - "kernel/Cargo.toml"
    owned_interfaces: ["objective-storage", "state-machine"]
    approval_required_for: ["public-api"]

  - id: goal-decomposer
    name: "Goal Decomposer"
    owner: "planner-team"
    paths:
      - "planner/**/*.py"
      - "planner/pyproject.toml"
    owned_interfaces: ["execution-plan"]

  - id: project-config
    name: "Project Configuration"
    owner: "kernel-team"
    paths:
      - "schemas/**/*.json"
      - "docs/**/*.md"
      - "adr/**/*.md"
"#
    }

    #[test]
    fn load_from_yaml() {
        let model = OwnershipModel::from_yaml(sample_yaml()).unwrap();
        assert_eq!(model.len(), 3);
    }

    #[test]
    fn domain_for_file_resolves_kernel_rs() {
        let model = OwnershipModel::from_yaml(sample_yaml()).unwrap();
        let domain = model.domain_for_file("kernel/src/ownership.rs").unwrap();
        assert_eq!(domain.id, "project-kernel");
    }

    #[test]
    fn domain_for_file_resolves_planner_py() {
        let model = OwnershipModel::from_yaml(sample_yaml()).unwrap();
        let domain = model.domain_for_file("planner/main.py").unwrap();
        assert_eq!(domain.id, "goal-decomposer");
    }

    #[test]
    fn domain_for_file_resolves_schema_json() {
        let model = OwnershipModel::from_yaml(sample_yaml()).unwrap();
        let domain = model.domain_for_file("schemas/objective.json").unwrap();
        assert_eq!(domain.id, "project-config");
    }

    #[test]
    fn domain_for_file_returns_none_for_unmatched() {
        let model = OwnershipModel::from_yaml(sample_yaml()).unwrap();
        assert!(model.domain_for_file("README.md").is_none());
        assert!(model.domain_for_file("node_modules/foo.js").is_none());
    }

    #[test]
    fn domains_for_files_deduplicates() {
        let model = OwnershipModel::from_yaml(sample_yaml()).unwrap();
        let files = vec![
            "kernel/src/main.rs".into(),
            "kernel/src/state_machine.rs".into(),
        ];
        let domains = model.domains_for_files(&files);
        assert_eq!(domains.len(), 1);
        assert_eq!(domains[0].id, "project-kernel");
    }

    #[test]
    fn domains_for_files_multiple_domains() {
        let model = OwnershipModel::from_yaml(sample_yaml()).unwrap();
        let files = vec![
            "kernel/src/main.rs".into(),
            "planner/main.py".into(),
        ];
        let domains = model.domains_for_files(&files);
        assert_eq!(domains.len(), 2);
    }

    #[test]
    fn validate_passes_for_valid_config() {
        let model = OwnershipModel::from_yaml(sample_yaml()).unwrap();
        assert!(model.validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_domains() {
        // from_yaml now validates internally — empty domain list rejects at parse time.
        let result = OwnershipModel::from_yaml("domains: []");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("No domains defined"), "got: {err}");
    }

    #[test]
    fn validate_rejects_duplicate_patterns() {
        let yaml = r#"
domains:
  - id: kernel
    name: "Kernel"
    owner: "team-a"
    paths: ["src/**/*.rs"]
  - id: other
    name: "Other"
    owner: "team-b"
    paths: ["src/**/*.rs"]
"#;
        let result = OwnershipModel::from_yaml(yaml);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Duplicate path pattern"), "got: {err}");
    }

    #[test]
    fn validate_rejects_domain_without_paths() {
        let yaml = r#"
domains:
  - id: empty
    name: "Empty"
    owner: "nobody"
    paths: []
"#;
        let result = OwnershipModel::from_yaml(yaml);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("has no path patterns"), "got: {err}");
    }
}
