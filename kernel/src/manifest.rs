//! Execution Manifest — PIL (Platform Integration Layer)
//!
//! Defines the deterministic operation sheet for transforming an Objective into
//! a runnable workload, independently validated by the Kernel via schema (*/schemas/manifest.json*).
//!
//! Stage 1: PIL stub only — full decomposition comes from the Planner module in Stage 2.
//! This stub verifies structural validity, ensuring compatibility with future planner runs.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use validator::Validate;

/// Execution Stage — docs/03-execution-manifest.md §3
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ManifestStage {
    /// Discovery — interface registry, directory walk
    Discovery,
    /// Analysis — flexible decomposition via LLM planner
    Analysis,
    /// Planning — deterministic step list construction
    Planning,
    /// Execution — worker pool activation
    Execution,
    /// Review — diff scope validation
    Review,
    /// Merge — internal git plumbing
    Merge,
    /// Completed — clean success terminal
    Completed,
    /// Failed rollback — atomic revert
    Failed,
}

/// Step Type — Pilot Program level manifestation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ManifestStepType {
    /// Nested manifest (sub-objective)
    NestedManifest,
    /// Programmatic call into named module
    ModuleCall,
    /// CLI command invocation (relative to workspace root)
    Command,
    /// Worker call (remote actor)
    WorkerCall,
    /// Functional gate (validation predicate)
    Gate,
    /// Test assertion (post-conditions)
    Test,
}

/// Executable step — atomic transformation directive
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ManifestStep {
    /// Unique step identifier
    pub id: String,

    /// Step type discrimination
    pub step_type: ManifestStepType,

    /// Manifest or worker module name
    pub target: Option<String>,

    /// Command, module function name, or condition predicate
    pub operation: Option<String>,

    /// Configuration fields or positional arguments
    pub config: Value,

    /// Human-readable step summary
    pub summary: String,
}

/// Execution Manifest — the PIL artifact linking Objective → deterministic steps
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Validate)]
pub struct ExecutionManifest {
    /// Manifest identifier (ULID or uuid)
    pub manifest_id: String,

    /// Parent objective identifier
    pub objective_id: String,

    /// Current manifest stage
    pub stage: ManifestStage,

    /// Human-readable objective summary
    pub title: String,

    /// Optionally provided: human description
    pub description: Option<String>,

    /// Decomposition into steps — one manifest per group
    pub groups: Vec<ManifestGroup>,

    /// Environment snapshot
    pub environment: ManifestEnvironment,

    /// Dependency closure
    pub dependencies: Vec<String>,

    /// Schema compliance validation hash
    pub schema_version: String,

    /// Creation timestamp (RFC-3339)
    pub created_at: DateTime<Utc>,
    
    /// Last update timestamp (RFC-3339)
    pub updated_at: DateTime<Utc>,
}

/// Execution group — units of parallelism/modularity
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ManifestGroup {
    /// Group identifier
    pub id: String,

    /// Group title
    pub title: String,

    /// Description (human)
    pub description: Option<String>,
    
    /// Ordered steps in this group
    pub steps: Vec<ManifestStep>,
    
    /// Intra-group dependency closure ([step_id])
    pub dependencies: Vec<String>,
}

/// Environment snapshot — machine/language/framework state
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ManifestEnvironment {
    /// Programming language (rust, python, typescript, etc)
    pub language: Option<String>,

    /// Framework moniker
    pub framework: Option<String>,

    /// SDK version constraints
    pub sdk: Option<ManifestSdk>,
    
    /// Interface registry snapshot — docs/02-interface-registry.md
    pub interface_registry: Vec<InterfaceSnapshot>
}

/// SDK snapshot — version, feature flags
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ManifestSdk {
    pub name: String,
    pub version: String,
    pub features: Vec<String>
}

/// Interface Snapshot — PIL record (stubbed)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InterfaceSnapshot {
    pub name: String,
    pub version: String,
    pub location: String
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::schema_for;

    #[test]
    fn test_manifest_schema_matches() {
        let value = schema_for!(ExecutionManifest);
        let generated_schema = serde_json::to_string_pretty(&value).unwrap();
        
        // Coarse check (dev): verify generated schema is structurally sound
        // TODO: integrate jsonschema crate for proper validation against schemas/manifest.json
        assert!(generated_schema.contains("ExecutionManifest"),
            "Generated schema doesn't declare root title");
        assert!(generated_schema.len() > 200,
            "Schema suspiciously small")
    }

    #[test]
    fn test_stub_manifest() {
        let step = ManifestStep {
            id: "step-1".into(),
            step_type: ManifestStepType::Command,
            target   : None,
            operation: Some("sleep 5".into()),
            config   : serde_json::json!({"timeout_s": 10}),
            summary  : "Delay for effect".into()
        };

        let env = ManifestEnvironment {
            language: Some("bash".into()),
            framework: None,
            sdk: None,
            interface_registry: vec![]
        };

        let manifest = ExecutionManifest {
            manifest_id: "pl-kernel-1".into(),
            objective_id: "obj-0".into(),
            stage: ManifestStage::Discovery,
            title: "PIL stub objective".into(),
            description: None,
            groups: vec![ManifestGroup {
                id: "group-0".into(),
                title: "Discovery phase".into(),
                description: None,
                steps: vec![step],
                dependencies: vec![]
            }],
            environment: env,
            dependencies: vec![],
            schema_version: "https://ai-os.dev/schema/manifest/1.0".into(),
            created_at: Utc::now(),
            updated_at: Utc::now()
        };

        assert_eq!(manifest.title, "PIL stub objective");
        assert_eq!(manifest.groups.len(), 1);
    }
}