//! AI-OS Plugin SDK — core trait definitions, types, and governance stubs.
//!
//! This crate defines the [`Plugin`] trait that all AI-OS plugins must
//! implement, along with supporting types for results, manifests, and
//! governance (approval workflows and compliance export).

pub mod example_plugin;
pub mod governance;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// StructuredDiff
// ---------------------------------------------------------------------------

/// A structured representation of a code diff that plugins can inspect to
/// evaluate changes against language-specific, framework-specific, or
/// guardian-rule constraints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredDiff {
    /// Files changed in this diff.
    pub files: Vec<DiffFile>,
    /// Optional metadata about the diff (e.g., source branch, author).
    pub metadata: HashMap<String, String>,
}

/// A single file within a [`StructuredDiff`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffFile {
    /// Relative path of the changed file.
    pub path: String,
    /// Language inferred from the file extension, if available.
    pub language: Option<String>,
    /// Number of added lines.
    pub lines_added: usize,
    /// Number of removed lines.
    pub lines_removed: usize,
    /// Raw diff text (unified format).
    pub diff: String,
}

// ---------------------------------------------------------------------------
// Plugin trait and associated types
// ---------------------------------------------------------------------------

/// The primary trait that all AI-OS plugins must implement.
///
/// Plugins receive a structured diff along with contextual metadata and
/// return a [`PluginResult`] that encodes a verdict, zero or more findings,
/// and a confidence score.
pub trait Plugin: Send + Sync {
    /// Human-readable name of this plugin (e.g. `"rust-validator"`).
    fn name(&self) -> &str;

    /// Evaluate a structured diff and return a plugin result.
    fn evaluate(&self, diff: &StructuredDiff, context: &PluginContext) -> PluginResult;
}

// ---------------------------------------------------------------------------
// PluginContext
// ---------------------------------------------------------------------------

/// Contextual information provided to a plugin at evaluation time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginContext {
    /// The objective under which this evaluation is running.
    pub objective_id: String,
    /// Optional domain override (e.g., `"safety"`, `"quality"`).
    pub domain: Option<String>,
    /// Arbitrary configuration key-value pairs.
    pub config: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// PluginResult and PluginVerdict
// ---------------------------------------------------------------------------

/// The result of a single plugin evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginResult {
    /// The overall verdict (pass, fail, or requires approval).
    pub verdict: PluginVerdict,
    /// Individual findings or issues discovered during evaluation.
    pub findings: Vec<Finding>,
    /// Confidence in the result, from 0.0 to 1.0.
    pub confidence: f64,
}

/// The verdict of a plugin evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PluginVerdict {
    /// The diff passes all checks.
    Pass,
    /// The diff fails with a reason.
    Fail(String),
    /// The diff requires human approval with a rationale.
    RequiresApproval(String),
}

// ---------------------------------------------------------------------------
// Finding
// ---------------------------------------------------------------------------

/// A single finding or issue discovered by a plugin during evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    /// Category label (e.g., `"style"`, `"security"`, `"performance"`).
    pub category: String,
    /// Severity level (e.g., `"error"`, `"warning"`, `"info"`).
    pub severity: String,
    /// Human-readable description of the finding.
    pub description: String,
}

// ---------------------------------------------------------------------------
// PluginKind
// ---------------------------------------------------------------------------

/// The kind or category of a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PluginKind {
    /// Language-specific analyzer (e.g., Rust, Python, TypeScript).
    Language,
    /// Framework-specific analyzer (e.g., Axum, React, Django).
    Framework,
    /// Guardian rule that enforces project-specific policies.
    GuardianRule,
}

// ---------------------------------------------------------------------------
// PluginManifest
// ---------------------------------------------------------------------------

/// Metadata describing a plugin's identity, kind, and target scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Unique identifier for the plugin (e.g., `"rust-analyzer-v1"`).
    pub plugin_id: String,
    /// The kind of plugin.
    pub kind: PluginKind,
    /// File globs or language targets this plugin applies to
    /// (e.g., `["*.rs"]`, `["*.py", "*.pyi"]`).
    pub targets: Vec<String>,
    /// Semantic version of the plugin.
    pub version: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that the trait can be implemented as a concrete struct.
    #[test]
    fn plugin_trait_is_object_safe() {
        struct DummyPlugin;

        impl Plugin for DummyPlugin {
            fn name(&self) -> &str {
                "dummy"
            }

            fn evaluate(
                &self,
                _diff: &StructuredDiff,
                _context: &PluginContext,
            ) -> PluginResult {
                PluginResult {
                    verdict: PluginVerdict::Pass,
                    findings: vec![],
                    confidence: 1.0,
                }
            }
        }

        let plugin: Box<dyn Plugin> = Box::new(DummyPlugin);
        assert_eq!(plugin.name(), "dummy");
    }

    #[test]
    fn plugin_result_serialization_roundtrip() {
        let result = PluginResult {
            verdict: PluginVerdict::Fail("test failure".into()),
            findings: vec![Finding {
                category: "test".into(),
                severity: "error".into(),
                description: "Something went wrong".into(),
            }],
            confidence: 0.5,
        };

        let json = serde_json::to_string(&result).expect("serialize");
        let deserialized: PluginResult = serde_json::from_str(&json).expect("deserialize");

        assert!(matches!(deserialized.verdict, PluginVerdict::Fail(_)));
        assert_eq!(deserialized.findings.len(), 1);
        assert!((deserialized.confidence - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn plugin_kind_variants() {
        assert!(matches!(PluginKind::Language, PluginKind::Language));
        assert!(matches!(PluginKind::Framework, PluginKind::Framework));
        assert!(matches!(PluginKind::GuardianRule, PluginKind::GuardianRule));
    }

    #[test]
    fn structured_diff_defaults() {
        let diff = StructuredDiff {
            files: vec![],
            metadata: HashMap::new(),
        };
        assert!(diff.files.is_empty());
        assert!(diff.metadata.is_empty());
    }
}
