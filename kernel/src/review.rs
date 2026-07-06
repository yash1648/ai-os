//! Review Pipeline — deterministic quality checks on worker diffs.
//!
//! All checks are purely structural and require no LLM calls. The Reviewer
//! checks a [`StructuredDiff`] against an [`Objective`] and (optionally) an
//! [`ExecutionManifest`], producing a [`ReviewVerdict`] that the Kernel uses
//! to decide whether to advance the objective to the Integration stage or
//! transition to [`ReviewFailure`].
//!
//! See docs/16-review-pipeline.md

use std::path::Path;

use metrics::{counter, describe_counter};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::diff_applier::{FileChange, StructuredDiff};
use crate::manifest::ExecutionManifest;
use crate::objective::Objective;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The outcome of a review.
///
/// `Pass` includes non-blocking findings (warnings / info) that did not prevent
/// progression. `Fail` contains only the blocking findings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewVerdict {
    /// All checks passed — no blocking findings. Non-blocking findings
    /// (warnings, info) are included for visibility.
    Pass(Vec<ReviewFinding>),
    /// One or more blocking findings were detected — the diff is rejected.
    Fail(Vec<ReviewFinding>),
}

/// A single finding produced by a review check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFinding {
    /// The category of the finding.
    pub category: ReviewCategory,
    /// How severe the finding is.
    pub severity: ReviewSeverity,
    /// Human-readable description of the issue.
    pub description: String,
    /// Location in the diff or workspace (typically a file path).
    pub location: String,
}

/// Category of a review finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewCategory {
    /// Logic errors, incorrectness, bugs.
    Correctness,
    /// Coding style, formatting, conventions.
    Style,
    /// Missing or insufficient test coverage.
    Testing,
    /// Performance issues (N+1 queries, unbounded loops, etc.).
    Performance,
    /// Maintainability concerns (clarity, docs, consistency).
    Maintainability,
}

/// Severity of a review finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSeverity {
    /// Must be resolved before the diff can be applied.
    Blocking,
    /// Should be addressed but does not block progression.
    Warning,
    /// Informational observation — no action required.
    Info,
}

/// Confidence level for a review finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewConfidence {
    Low,
    Medium,
    High,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during a review.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ReviewError {
    /// Failed to serialize the diff for size measurement.
    #[error("Failed to serialize diff for size check: {0}")]
    SerializationFailed(String),
}

// ---------------------------------------------------------------------------
// Critical path prefixes — deleting files under these paths is blocking.
// ---------------------------------------------------------------------------

const CRITICAL_PATH_PREFIXES: &[&str] = &["kernel/src", "docs/"];

// ---------------------------------------------------------------------------
// Reviewer
// ---------------------------------------------------------------------------

/// Deterministic reviewer for worker diffs.
///
/// Performs all checks without any LLM calls — purely structural validation
/// of file paths, diff size, content integrity, and scope compliance.
///
/// # Examples
///
/// ```
/// use ai_os_kernel::review::Reviewer;
///
/// let reviewer = Reviewer::new();
/// assert_eq!(reviewer.max_diff_size_bytes, 1_000_000);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reviewer {
    /// Maximum allowed serialized diff size in bytes (default: 1 MB).
    pub max_diff_size_bytes: u64,
    /// File extensions that are forbidden from being introduced by workers.
    #[serde(default = "default_forbidden_extensions")]
    pub forbidden_extensions: Vec<String>,
}

fn default_forbidden_extensions() -> Vec<String> {
    vec![
        ".exe".into(),
        ".dll".into(),
        ".so".into(),
        ".dylib".into(),
        ".bin".into(),
    ]
}

impl Default for Reviewer {
    fn default() -> Self {
        Self::new()
    }
}

impl Reviewer {
    /// Create a new `Reviewer` with default configuration:
    ///
    /// | Parameter | Default |
    /// |---|---|
    /// | `max_diff_size_bytes` | 1,000,000 (1 MB) |
    /// | `forbidden_extensions` | `.exe`, `.dll`, `.so`, `.dylib`, `.bin` |
    pub fn new() -> Self {
        Self {
            max_diff_size_bytes: 1_000_000,
            forbidden_extensions: default_forbidden_extensions(),
        }
    }

    /// Create a `Reviewer` with explicit configuration.
    pub fn with_config(max_diff_size: u64, forbidden_ext: Vec<String>) -> Self {
        Self {
            max_diff_size_bytes: max_diff_size,
            forbidden_extensions: forbidden_ext,
        }
    }

    /// Run **all** deterministic review checks against `diff`.
    ///
    /// When `manifest` is provided and has non-empty `allowed_domains`, the
    /// file-scope check verifies every changed path lies within those domains.
    ///
    /// Checks performed (in order):
    ///
    /// 1. **File scope** — every file path must be within `manifest.allowed_domains`
    ///    (skipped when the manifest is absent or has no domains).
    /// 2. **Binary file** — no forbidden extensions.
    /// 3. **Diff size** — serialized JSON must not exceed `max_diff_size_bytes`.
    /// 4. **Empty diff** — warns if there are zero changes.
    /// 5. **Delete** — warns about destructive operations; blocking on
    ///    critical paths (`kernel/src`, `docs/`).
    /// 6. **Content integrity** — `Modify` old_content is non-empty and
    ///    plausibly complete.
    pub fn review(
        &self,
        diff: &StructuredDiff,
        _objective: &Objective,
        manifest: Option<&ExecutionManifest>,
    ) -> ReviewVerdict {
        let mut findings: Vec<ReviewFinding> = Vec::new();

        // ── (a) File scope check ───────────────────────────────────────
        if let Some(mf) = manifest {
            if !mf.allowed_domains.is_empty() {
                for change in &diff.changes {
                    let path = file_change_path(change);
                    let path_str = path.to_string_lossy();
                    let in_scope = mf
                        .allowed_domains
                        .iter()
                        .any(|domain| path_str.starts_with(domain));
                    if !in_scope {
                        findings.push(ReviewFinding {
                            category: ReviewCategory::Correctness,
                            severity: ReviewSeverity::Blocking,
                            description: format!(
                                "File '{}' is outside allowed domains ({:?})",
                                path_str, mf.allowed_domains,
                            ),
                            location: path_str.to_string(),
                        });
                    }
                }
            }
        }

        // ── (b) Binary / forbidden extension check ─────────────────────
        for change in &diff.changes {
            let path = file_change_path(change);
            if let Some(ext) = path.extension() {
                let ext_str = format!(".{}", ext.to_string_lossy());
                if self.forbidden_extensions.contains(&ext_str) {
                    findings.push(ReviewFinding {
                        category: ReviewCategory::Correctness,
                        severity: ReviewSeverity::Blocking,
                        description: format!(
                            "File '{}' has a forbidden extension '{}'",
                            path.display(),
                            ext_str,
                        ),
                        location: path.to_string_lossy().to_string(),
                    });
                }
            }
        }

        // ── (c) Diff size check ───────────────────────────────────────
        let diff_size = match serde_json::to_vec(diff) {
            Ok(bytes) => bytes.len() as u64,
            Err(e) => {
                findings.push(ReviewFinding {
                    category: ReviewCategory::Correctness,
                    severity: ReviewSeverity::Warning,
                    description: format!("Cannot serialize diff for size check: {e}"),
                    location: "diff".into(),
                });
                0
            }
        };

        if diff_size > self.max_diff_size_bytes {
            findings.push(ReviewFinding {
                category: ReviewCategory::Performance,
                severity: ReviewSeverity::Blocking,
                description: format!(
                    "Diff size ({} bytes) exceeds maximum allowed ({} bytes)",
                    diff_size, self.max_diff_size_bytes,
                ),
                location: "diff".into(),
            });
        }

        // ── (d) Empty diff check ──────────────────────────────────────
        if diff.changes.is_empty() {
            findings.push(ReviewFinding {
                category: ReviewCategory::Correctness,
                severity: ReviewSeverity::Warning,
                description: "Diff contains zero file changes — nothing to review".into(),
                location: "--all".into(),
            });
        }

        // ── (e) Delete check ──────────────────────────────────────────
        for change in &diff.changes {
            if let FileChange::Delete { path } = change {
                let path_str = path.to_string_lossy();
                let is_critical = CRITICAL_PATH_PREFIXES
                    .iter()
                    .any(|prefix| path_str.starts_with(*prefix));

                if is_critical {
                    findings.push(ReviewFinding {
                        category: ReviewCategory::Correctness,
                        severity: ReviewSeverity::Blocking,
                        description: format!(
                            "Destructive operation on critical path: '{}'",
                            path_str,
                        ),
                        location: path_str.to_string(),
                    });
                } else {
                    findings.push(ReviewFinding {
                        category: ReviewCategory::Maintainability,
                        severity: ReviewSeverity::Warning,
                        description: format!(
                            "Destructive operation: '{}' will be deleted",
                            path_str,
                        ),
                        location: path_str.to_string(),
                    });
                }
            }
        }

        // ── (f) Content integrity check ───────────────────────────────
        for change in &diff.changes {
            if let FileChange::Modify {
                path,
                old_content,
                new_content: _,
            } = change
            {
                let path_str = path.to_string_lossy();

                if old_content.is_empty() {
                    findings.push(ReviewFinding {
                        category: ReviewCategory::Correctness,
                        severity: ReviewSeverity::Warning,
                        description: format!(
                            "Modify for '{}' has empty old_content — content verification \
                             will be skipped at apply time",
                            path_str,
                        ),
                        location: path_str.to_string(),
                    });
                } else if old_content.len() < 10 {
                    findings.push(ReviewFinding {
                        category: ReviewCategory::Correctness,
                        severity: ReviewSeverity::Warning,
                        description: format!(
                            "Modify for '{}' has very short old_content ({} chars) — \
                             may be truncated",
                            path_str,
                            old_content.len(),
                        ),
                        location: path_str.to_string(),
                    });
                }
            }
        }

        // ── Final verdict ─────────────────────────────────────────────
        let blocking: Vec<ReviewFinding> = findings
            .iter()
            .filter(|f| f.severity == ReviewSeverity::Blocking)
            .cloned()
            .collect();

        describe_counter!("ai_os_review_pass_count", "Number of reviews that passed");
        describe_counter!("ai_os_review_fail_count", "Number of reviews that failed");

        if blocking.is_empty() {
            counter!("ai_os_review_pass_count").increment(1);
            ReviewVerdict::Pass(findings)
        } else {
            counter!("ai_os_review_fail_count").increment(1);
            ReviewVerdict::Fail(blocking)
        }
    }

    /// Format a [`ReviewVerdict`] as a human-readable string.
    ///
    /// Shows the overall verdict, the number of findings per severity, and a
    /// bullet-point list of every finding.
    pub fn format_verdict(&self, verdict: &ReviewVerdict) -> String {
        let (label, findings) = match verdict {
            ReviewVerdict::Pass(f) => ("PASS", f.as_slice()),
            ReviewVerdict::Fail(f) => ("FAIL", f.as_slice()),
        };

        let blocking_count = findings
            .iter()
            .filter(|f| f.severity == ReviewSeverity::Blocking)
            .count();
        let warning_count = findings
            .iter()
            .filter(|f| f.severity == ReviewSeverity::Warning)
            .count();
        let info_count = findings
            .iter()
            .filter(|f| f.severity == ReviewSeverity::Info)
            .count();

        let mut out = String::new();
        out.push_str(&format!(
            "═══ Review Verdict: {label} ═══\n\
             Blocking: {blocking_count}  Warning: {warning_count}  Info: {info_count}\n",
        ));

        if !findings.is_empty() {
            out.push('\n');
            for (i, finding) in findings.iter().enumerate() {
                out.push_str(&format!(
                    "{i}. [{:?}] [{:?}] {}\n   Location: {}\n",
                    finding.severity, finding.category, finding.description, finding.location,
                ));
            }
        }

        out
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Extract the underlying [`Path`] from any [`FileChange`] variant.
fn file_change_path(change: &FileChange) -> &Path {
    match change {
        FileChange::Create { path, .. } => path,
        FileChange::Modify { path, .. } => path,
        FileChange::Delete { path, .. } => path,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::Utc;
    use super::*;
    use crate::diff_applier::CommitMetadata;
    use crate::manifest::{ExecutionManifest, ManifestEnvironment, ManifestStage};
    use crate::state_machine::ObjectiveState;

    // ── Helpers ──────────────────────────────────────────────────────────

    fn dummy_manifest(allowed_domains: Vec<String>) -> ExecutionManifest {
        ExecutionManifest {
            manifest_id: "m-1".into(),
            objective_id: "obj-1".into(),
            stage: ManifestStage::Review,
            title: "review test".into(),
            description: None,
            groups: vec![],
            environment: ManifestEnvironment {
                language: None,
                framework: None,
                sdk: None,
                interface_registry: vec![],
            },
            dependencies: vec![],
            allowed_domains,
            worker_type: None,
            schema_version: "1.0".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn dummy_objective() -> Objective {
        Objective {
            id: "obj-1".into(),
            title: "test".into(),
            description: "desc".into(),
            owner: "worker-1".into(),
            parent_id: None,
            priority: crate::objective::Priority::Medium,
            status: ObjectiveState::from_label("REVIEW"),
            dependencies: vec![],
            success_criteria: vec!["pass".into()],
            plan_id: Some("m-1".into()),
            retry_count: 0,
            tags: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn dummy_diff(changes: Vec<FileChange>) -> StructuredDiff {
        StructuredDiff {
            objective_id: "obj-1".into(),
            worker_id: "worker-1".into(),
            changes,
            commit_metadata: CommitMetadata {
                summary: "test diff".into(),
                objective_id: "obj-1".into(),
                worker_id: "worker-1".into(),
                reviewer_id: None,
                guardian_id: None,
                human_approval_id: None,
            },
        }
    }

    #[allow(dead_code)]
    fn has_severity(findings: &[ReviewFinding], target: ReviewSeverity) -> bool {
        findings.iter().any(|f| f.severity == target)
    }

    #[allow(dead_code)]
    fn count_severity(findings: &[ReviewFinding], target: ReviewSeverity) -> usize {
        findings.iter().filter(|f| f.severity == target).count()
    }

    // ── test_review_pass ─────────────────────────────────────────────────

    #[test]
    fn test_review_pass() {
        let reviewer = Reviewer::new();
        let manifest = dummy_manifest(vec!["kernel/src".into()]);
        let objective = dummy_objective();
        let diff = dummy_diff(vec![FileChange::Create {
            path: PathBuf::from("kernel/src/feature.rs"),
            content: "fn new_feature() {}".to_string(),
        }]);

        let verdict = reviewer.review(&diff, &objective, Some(&manifest));
        assert!(
            matches!(verdict, ReviewVerdict::Pass(_)),
            "Expected Pass, got {:?}",
            verdict
        );
    }

    // ── test_review_forbidden_file ───────────────────────────────────────

    #[test]
    fn test_review_forbidden_file() {
        let reviewer = Reviewer::new();
        let manifest = dummy_manifest(vec!["kernel/src".into()]);
        let objective = dummy_objective();
        let diff = dummy_diff(vec![FileChange::Create {
            path: PathBuf::from("node_modules/malicious.js"),
            content: "evil".to_string(),
        }]);

        let verdict = reviewer.review(&diff, &objective, Some(&manifest));
        match verdict {
            ReviewVerdict::Fail(findings) => {
                assert!(
                    findings.iter().any(|f| {
                        f.category == ReviewCategory::Correctness
                            && f.severity == ReviewSeverity::Blocking
                            && f.description.contains("outside allowed domains")
                    }),
                    "Expected 'outside allowed domains' finding, got: {:#?}",
                    findings
                );
            }
            _ => panic!("Expected Fail verdict for forbidden file"),
        }
    }

    // ── test_review_binary_file ──────────────────────────────────────────

    #[test]
    fn test_review_binary_file() {
        let reviewer = Reviewer::new();
        let objective = dummy_objective();
        let diff = dummy_diff(vec![FileChange::Create {
            path: PathBuf::from("payload.exe"),
            content: vec![0u8; 64].into_iter().map(|b| b as char).collect(),
        }]);

        let verdict = reviewer.review(&diff, &objective, None);
        match verdict {
            ReviewVerdict::Fail(findings) => {
                assert!(
                    findings.iter().any(|f| {
                        f.category == ReviewCategory::Correctness
                            && f.severity == ReviewSeverity::Blocking
                            && f.description.contains("forbidden extension")
                    }),
                    "Expected 'forbidden extension' finding, got: {:#?}",
                    findings
                );
            }
            _ => panic!("Expected Fail verdict for .exe file"),
        }
    }

    // ── test_review_empty_diff ───────────────────────────────────────────

    #[test]
    fn test_review_empty_diff() {
        let reviewer = Reviewer::new();
        let objective = dummy_objective();
        let diff = dummy_diff(vec![]);

        let verdict = reviewer.review(&diff, &objective, None);
        match verdict {
            ReviewVerdict::Pass(findings) => {
                assert!(
                    findings.iter().any(|f| {
                        f.severity == ReviewSeverity::Warning
                            && f.description.contains("zero file changes")
                    }),
                    "Expected Warning about empty diff, got: {:#?}",
                    findings,
                );
            }
            other => panic!("Expected Pass (with warning) for empty diff, got: {:?}", other),
        }
    }

    // ── test_review_delete_critical ──────────────────────────────────────

    #[test]
    fn test_review_delete_critical() {
        let reviewer = Reviewer::new();
        let objective = dummy_objective();
        let diff = dummy_diff(vec![FileChange::Delete {
            path: PathBuf::from("kernel/src/core.rs"),
        }]);

        let verdict = reviewer.review(&diff, &objective, None);
        match verdict {
            ReviewVerdict::Fail(findings) => {
                assert!(
                    findings.iter().any(|f| {
                        f.severity == ReviewSeverity::Blocking
                            && f.description.contains("critical path")
                    }),
                    "Expected Blocking for critical path delete, got: {:#?}",
                    findings,
                );
            }
            other => panic!("Expected Fail for critical path delete, got: {:?}", other),
        }
    }

    // ── test_review_delete_benign ────────────────────────────────────────

    #[test]
    fn test_review_delete_benign() {
        let reviewer = Reviewer::new();
        let objective = dummy_objective();
        let diff = dummy_diff(vec![FileChange::Delete {
            path: PathBuf::from("temp/scratch.txt"),
        }]);

        let verdict = reviewer.review(&diff, &objective, None);
        match verdict {
            ReviewVerdict::Pass(findings) => {
                assert!(
                    findings.iter().any(|f| {
                        f.severity == ReviewSeverity::Warning
                            && f.description.contains("will be deleted")
                    }),
                    "Expected Warning for benign delete, got: {:#?}",
                    findings,
                );
            }
            other => panic!("Expected Pass (with warning) for benign delete, got: {:?}", other),
        }
    }

    // ── test_review_diff_too_large ───────────────────────────────────────

    #[test]
    fn test_review_diff_too_large() {
        // Use a tiny max size so any diff triggers the limit.
        let reviewer = Reviewer {
            max_diff_size_bytes: 1, // 1 byte limit
            forbidden_extensions: vec![],
        };
        let objective = dummy_objective();
        let diff = dummy_diff(vec![FileChange::Create {
            path: PathBuf::from("src/file.rs"),
            content: "fn f() {}".to_string(),
        }]);

        let verdict = reviewer.review(&diff, &objective, None);
        match verdict {
            ReviewVerdict::Fail(findings) => {
                assert!(
                    findings.iter().any(|f| {
                        f.severity == ReviewSeverity::Blocking
                            && f.description.contains("exceeds maximum")
                    }),
                    "Expected Blocking for oversized diff, got: {:#?}",
                    findings,
                );
            }
            other => panic!("Expected Fail for oversized diff, got: {:?}", other),
        }
    }

    // ── format_verdict smoke ─────────────────────────────────────────────

    #[test]
    fn test_format_verdict_pass() {
        let reviewer = Reviewer::new();
        let v = ReviewVerdict::Pass(vec![ReviewFinding {
            category: ReviewCategory::Style,
            severity: ReviewSeverity::Warning,
            description: "trailing whitespace".into(),
            location: "src/file.rs".into(),
        }]);
        let s = reviewer.format_verdict(&v);
        assert!(s.contains("PASS"));
        assert!(s.contains("trailing whitespace"));
    }

    #[test]
    fn test_format_verdict_fail() {
        let reviewer = Reviewer::new();
        let v = ReviewVerdict::Fail(vec![ReviewFinding {
            category: ReviewCategory::Correctness,
            severity: ReviewSeverity::Blocking,
            description: "forbidden extension".into(),
            location: "bad.exe".into(),
        }]);
        let s = reviewer.format_verdict(&v);
        assert!(s.contains("FAIL"));
        assert!(s.contains("forbidden extension"));
    }

    // ── Edge cases ───────────────────────────────────────────────────────

    #[test]
    fn test_review_empty_diff_skips_scope_check() {
        // Ensure an empty diff does not produce false-positive scope
        // violations (the loop over changes is simply empty).
        let reviewer = Reviewer::new();
        let manifest = dummy_manifest(vec!["kernel/src".into()]);
        let objective = dummy_objective();
        let diff = dummy_diff(vec![]);

        let verdict = reviewer.review(&diff, &objective, Some(&manifest));
        match verdict {
            ReviewVerdict::Pass(findings) => {
                assert!(
                    findings
                        .iter()
                        .any(|f| f.description.contains("zero file changes")),
                    "Expected Warning about empty diff"
                );
            }
            other => panic!("Expected Pass for empty diff, got: {:?}", other),
        }
    }

    #[test]
    fn test_review_manifest_without_domains_skips_scope() {
        // When allowed_domains is empty the scope check is skipped,
        // so a file outside any domain should not be blocked by scope.
        let reviewer = Reviewer::new();
        let manifest = dummy_manifest(vec![]); // empty = skip
        let objective = dummy_objective();
        let diff = dummy_diff(vec![FileChange::Create {
            path: PathBuf::from("anywhere/at/all.rs"),
            content: "ok".into(),
        }]);

        let verdict = reviewer.review(&diff, &objective, Some(&manifest));
        assert!(
            matches!(verdict, ReviewVerdict::Pass(_)),
            "Expected Pass when allowed_domains is empty, got: {:?}",
            verdict
        );
    }

    #[test]
    fn test_review_content_integrity_truncated() {
        let reviewer = Reviewer::new();
        let objective = dummy_objective();
        let diff = dummy_diff(vec![FileChange::Modify {
            path: PathBuf::from("src/main.rs"),
            old_content: "short".to_string(), // < 10 chars → warning
            new_content: "longer content here".into(),
        }]);

        let verdict = reviewer.review(&diff, &objective, None);
        match verdict {
            ReviewVerdict::Pass(findings) => {
                assert!(
                    findings
                        .iter()
                        .any(|f| f.description.contains("very short old_content")),
                    "Expected Warning about truncated old_content, got: {:#?}",
                    findings,
                );
            }
            other => panic!("Expected Pass for content integrity warning, got: {:?}", other),
        }
    }

    #[test]
    fn test_review_forbidden_extensions_all_blocked() {
        for ext in &[".exe", ".dll", ".so", ".dylib", ".bin"] {
            let reviewer = Reviewer::new();
            let objective = dummy_objective();
            let path = PathBuf::from(format!("payload{ext}"));
            let diff = dummy_diff(vec![FileChange::Create {
                path,
                content: "data".into(),
            }]);

            let verdict = reviewer.review(&diff, &objective, None);
            assert!(
                matches!(verdict, ReviewVerdict::Fail(_)),
                "Expected Fail for {ext} extension"
            );
        }
    }
}
