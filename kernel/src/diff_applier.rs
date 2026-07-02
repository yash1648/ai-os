//! Diff Applier — atomic file modification with backup and rollback.
//!
//! The only component with authority to mutate the working tree.
//! Applies a validated, reviewed, and approved diff atomically, and
//! creates a structured commit linking back to the objective, worker,
//! reviewer, and guardian decisions.
//!
//! See docs/03-project-kernel.md --- Diff Applier and Rollback Manager.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A single file-level change within a diff.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileChange {
    /// Create a new file with the given content.
    Create { path: PathBuf, content: String },
    /// Modify an existing file (full content replacement).
    Modify { path: PathBuf, old_content: String, new_content: String },
    /// Delete a file.
    Delete { path: PathBuf },
}

/// A validated, structured diff --- not a raw patch string, but a set of
/// typed file operations the Kernel can apply atomically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredDiff {
    /// Objective this diff belongs to.
    pub objective_id: String,

    /// Worker that produced this diff.
    pub worker_id: String,

    /// Individual file changes.
    pub changes: Vec<FileChange>,

    /// Structured commit message fields.
    pub commit_metadata: CommitMetadata,
}

/// Structured commit metadata linking the diff to the pipeline that produced it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitMetadata {
    pub summary: String,
    pub objective_id: String,
    pub worker_id: String,
    pub reviewer_id: Option<String>,
    pub guardian_id: Option<String>,
    pub human_approval_id: Option<String>,
}

/// Outcome of applying a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    Applied {
        commit_hash: String,
        files_changed: usize,
    },
    DryRun {
        files_changed: usize,
        would_create: Vec<PathBuf>,
        would_modify: Vec<PathBuf>,
        would_delete: Vec<PathBuf>,
    },
}

/// Errors from the diff applier.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DiffApplyError {
    #[error("File not found on disk for modification: {0}")]
    FileNotFound(PathBuf),

    #[error("File content mismatch: expected old_content does not match disk for {0}")]
    ContentMismatch(PathBuf),

    #[error("File already exists, cannot create: {0}")]
    AlreadyExists(PathBuf),

    #[error("Cannot delete non-existent file: {0}")]
    NotExist(PathBuf),

    #[error("Change touches path outside allowed scope: {0}")]
    OutsideScope(PathBuf),

    #[error("Backup failed for: {0}")]
    BackupFailed(PathBuf),

    #[error("Rollback failed: {0}")]
    RollbackFailed(String),

    #[error("I/O error: {0}")]
    Io(String),
}

impl From<std::io::Error> for DiffApplyError {
    fn from(e: std::io::Error) -> Self {
        DiffApplyError::Io(e.to_string())
    }
}

pub type DiffApplyResult<T> = Result<T, DiffApplyError>;

// ---------------------------------------------------------------------------
// Snapshot --- point-in-time file state for rollback
// ---------------------------------------------------------------------------

/// A snapshot of one file's content before modification.
#[derive(Debug, Clone, PartialEq)]
struct FileSnapshot {
    path: PathBuf,
    content: Option<String>, // None = file didn't exist (will be created)
}

/// A complete before-state of an apply operation, enabling atomic rollback.
#[derive(Debug, Default, PartialEq)]
pub struct WorkspaceSnapshot {
    files: Vec<FileSnapshot>,
}

impl WorkspaceSnapshot {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }
}

// ---------------------------------------------------------------------------
// Diff Applier
// ---------------------------------------------------------------------------

/// Applies structured diffs to a working tree with backup and rollback.
///
/// Stage 1 operates on the local filesystem (no git2 integration yet).
/// Stage 2+ will apply through git with proper atomic commits.
pub struct DiffApplier {
    /// Root directory for all file operations (project root).
    workspace_root: PathBuf,
}

impl DiffApplier {
    /// Create a new DiffApplier rooted at `workspace_root`.
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    /// Resolve a path relative to the workspace root.
    fn resolve(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workspace_root.join(path)
        }
    }

    /// Validate that all changes in the diff are within `allowed_paths`.
    pub fn validate_scope(
        &self,
        diff: &StructuredDiff,
        allowed_paths: &[PathBuf],
    ) -> DiffApplyResult<()> {
        if allowed_paths.is_empty() {
            return Ok(());
        }

        for change in &diff.changes {
            let target = path_change(change);
            let resolved = self.resolve(target);
            let in_scope = allowed_paths.iter().any(|allowed| {
                let allowed_resolved = self.resolve(allowed);
                resolved.starts_with(&allowed_resolved)
            });

            if !in_scope {
                return Err(DiffApplyError::OutsideScope(target.clone()));
            }
        }
        Ok(())
    }

    /// Dry-run: report what would happen without touching anything.
    pub fn dry_run(&self, diff: &StructuredDiff) -> DiffApplyResult<ApplyOutcome> {
        let mut would_create = Vec::new();
        let mut would_modify = Vec::new();
        let mut would_delete = Vec::new();

        for change in &diff.changes {
            let path = path_change(change);
            match change {
                FileChange::Create { .. } => would_create.push(path.clone()),
                FileChange::Modify { .. } => would_modify.push(path.clone()),
                FileChange::Delete { .. } => would_delete.push(path.clone()),
            }
        }

        Ok(ApplyOutcome::DryRun {
            files_changed: diff.changes.len(),
            would_create,
            would_modify,
            would_delete,
        })
    }

    /// Apply a structured diff to disk.
    ///
    /// 1. Validates all preconditions.
    /// 2. Takes a snapshot of every file that will change.
    /// 3. Applies all changes.
    /// 4. Returns the snapshot so callers can roll back on failure.
    pub fn apply(
        &self,
        diff: &StructuredDiff,
    ) -> DiffApplyResult<(ApplyOutcome, WorkspaceSnapshot)> {
        // Phase 1: Validate all preconditions
        for change in &diff.changes {
            match &change {
                FileChange::Create { path, .. } => {
                    let resolved = self.resolve(path);
                    if resolved.exists() {
                        return Err(DiffApplyError::AlreadyExists(path.clone()));
                    }
                }
                FileChange::Modify { path, old_content, .. } => {
                    let resolved = self.resolve(path);
                    if !resolved.exists() {
                        return Err(DiffApplyError::FileNotFound(path.clone()));
                    }
                    let on_disk = std::fs::read_to_string(&resolved).map_err(|e| {
                        DiffApplyError::Io(format!("Cannot read {}: {}", path.display(), e))
                    })?;
                    if on_disk != *old_content {
                        return Err(DiffApplyError::ContentMismatch(path.clone()));
                    }
                }
                FileChange::Delete { path } => {
                    let resolved = self.resolve(path);
                    if !resolved.exists() {
                        return Err(DiffApplyError::NotExist(path.clone()));
                    }
                }
            }
        }

        // Phase 2: Snapshot before changes
        let mut snapshot = WorkspaceSnapshot::default();

        for change in &diff.changes {
            let resolved = self.resolve(path_change(change));
            let content = if resolved.exists() {
                Some(std::fs::read_to_string(&resolved).map_err(|_| {
                    DiffApplyError::BackupFailed(path_change(change).clone())
                })?)
            } else {
                None
            };

            snapshot.files.push(FileSnapshot {
                path: path_change(change).clone(),
                content,
            });
        }

        // Phase 3: Apply changes
        let mut applied = 0usize;

        for change in &diff.changes {
            let resolved = self.resolve(path_change(change));
            match change {
                FileChange::Create { content, .. } | FileChange::Modify { new_content: content, .. } => {
                    if let Some(parent) = resolved.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(&resolved, content)?;
                    applied += 1;
                }
                FileChange::Delete { .. } => {
                    std::fs::remove_file(&resolved)?;
                    applied += 1;
                }
            }
        }

        let commit_hash = "0000000000000000000000000000000000000000".to_string();
        Ok((
            ApplyOutcome::Applied {
                commit_hash,
                files_changed: applied,
            },
            snapshot,
        ))
    }

    /// Roll back a previously taken snapshot, restoring every file to
    /// its pre-apply state.
    pub fn rollback(&self, snapshot: WorkspaceSnapshot) -> DiffApplyResult<()> {
        for snap in snapshot.files.into_iter().rev() {
            let resolved = self.resolve(&snap.path);
            match snap.content {
                Some(content) => {
                    if let Some(parent) = resolved.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(&resolved, &content).map_err(|e| {
                        DiffApplyError::RollbackFailed(format!("restore {}: {}", snap.path.display(), e))
                    })?;
                }
                None => {
                    if resolved.exists() {
                        std::fs::remove_file(&resolved).map_err(|e| {
                            DiffApplyError::RollbackFailed(format!("remove {}: {}", snap.path.display(), e))
                        })?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Build a commit message from structured metadata.
    pub fn format_commit_message(&self, meta: &CommitMetadata) -> String {
        let mut lines = Vec::new();

        lines.push(meta.summary.clone());
        lines.push(String::new());

        lines.push(format!("objective-id: {}", meta.objective_id));
        lines.push(format!("worker-id: {}", meta.worker_id));

        if let Some(ref r) = meta.reviewer_id {
            lines.push(format!("reviewer-id: {r}"));
        }
        if let Some(ref g) = meta.guardian_id {
            lines.push(format!("guardian-id: {g}"));
        }
        if let Some(ref h) = meta.human_approval_id {
            lines.push(format!("human-approval-id: {h}"));
        }

        lines.join("\n")
    }
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

/// Extract the path from any FileChange variant.
fn path_change(change: &FileChange) -> &PathBuf {
    path(change)
}

fn path(change: &FileChange) -> &PathBuf {
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
    use super::*;

    fn test_diff(objective_id: &str) -> StructuredDiff {
        StructuredDiff {
            objective_id: objective_id.to_string(),
            worker_id: "worker-1".to_string(),
            changes: vec![],
            commit_metadata: CommitMetadata {
                summary: "feat: test change".to_string(),
                objective_id: objective_id.to_string(),
                worker_id: "worker-1".to_string(),
                reviewer_id: None,
                guardian_id: None,
                human_approval_id: None,
            },
        }
    }

    #[test]
    fn dry_run_empty() {
        let applier = DiffApplier::new(PathBuf::from("/tmp"));
        let diff = test_diff("obj-1");
        let result = applier.dry_run(&diff).unwrap();
        assert_eq!(
            result,
            ApplyOutcome::DryRun {
                files_changed: 0,
                would_create: vec![],
                would_modify: vec![],
                would_delete: vec![],
            }
        );
    }

    #[test]
    fn dry_run_with_changes() {
        use std::path::PathBuf;
        let applier = DiffApplier::new(PathBuf::from("/tmp"));
        let diff = StructuredDiff {
            changes: vec![
                FileChange::Create {
                    path: PathBuf::from("new_file.rs"),
                    content: "fn hello() {}".to_string(),
                },
                FileChange::Modify {
                    path: PathBuf::from("existing.rs"),
                    old_content: "old".to_string(),
                    new_content: "new".to_string(),
                },
            ],
            ..test_diff("obj-2")
        };
        let result = applier.dry_run(&diff).unwrap();
        match result {
            ApplyOutcome::DryRun {
                files_changed,
                would_create,
                would_modify,
                would_delete,
            } => {
                assert_eq!(files_changed, 2);
                assert_eq!(would_create, vec![PathBuf::from("new_file.rs")]);
                assert_eq!(would_modify, vec![PathBuf::from("existing.rs")]);
                assert!(would_delete.is_empty());
            }
            _ => panic!("Expected DryRun"),
        }
    }

    #[test]
    fn validate_scope_allows_within() {
        let applier = DiffApplier::new(PathBuf::from("/workspace"));
        let diff = StructuredDiff {
            changes: vec![FileChange::Create {
                path: PathBuf::from("src/lib.rs"),
                content: "pub fn f() {}".to_string(),
            }],
            ..test_diff("obj-3")
        };
        let allowed = vec![PathBuf::from("src")];
        assert!(applier.validate_scope(&diff, &allowed).is_ok());
    }

    #[test]
    fn validate_scope_rejects_outside() {
        let applier = DiffApplier::new(PathBuf::from("/workspace"));
        let diff = StructuredDiff {
            changes: vec![FileChange::Modify {
                path: PathBuf::from("src/lib.rs"),
                old_content: "".to_string(),
                new_content: "".to_string(),
            }],
            ..test_diff("obj-4")
        };
        let allowed = vec![PathBuf::from("tests")];
        assert_eq!(
            applier.validate_scope(&diff, &allowed),
            Err(DiffApplyError::OutsideScope(PathBuf::from("src/lib.rs")))
        );
    }

    #[test]
    fn create_and_rollback_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let applier = DiffApplier::new(root.clone());

        let diff = StructuredDiff {
            changes: vec![FileChange::Create {
                path: PathBuf::from("hello.txt"),
                content: "Hello, world!".to_string(),
            }],
            ..test_diff("obj-5")
        };

        let (outcome, snapshot) = applier.apply(&diff).unwrap();
        match outcome {
            ApplyOutcome::Applied { files_changed, .. } => assert_eq!(files_changed, 1),
            _ => panic!("Expected Applied"),
        }

        // File exists
        let file_path = root.join("hello.txt");
        assert!(file_path.exists());
        assert_eq!(
            std::fs::read_to_string(&file_path).unwrap(),
            "Hello, world!"
        );

        // Rollback
        applier.rollback(snapshot).unwrap();
        assert!(!file_path.exists());
    }

    #[test]
    fn modify_and_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let file_path = root.join("data.txt");
        let original = "original content".to_string();
        std::fs::write(&file_path, &original).unwrap();

        let applier = DiffApplier::new(root.clone());
        let diff = StructuredDiff {
            changes: vec![FileChange::Modify {
                path: PathBuf::from("data.txt"),
                old_content: original.clone(),
                new_content: "modified content".to_string(),
            }],
            ..test_diff("obj-6")
        };

        let (outcome, snapshot) = applier.apply(&diff).unwrap();
        match outcome {
            ApplyOutcome::Applied { files_changed, .. } => assert_eq!(files_changed, 1),
            _ => panic!("Expected Applied"),
        }

        assert_eq!(
            std::fs::read_to_string(&file_path).unwrap(),
            "modified content"
        );

        // Rollback
        applier.rollback(snapshot).unwrap();
        assert_eq!(
            std::fs::read_to_string(&file_path).unwrap(),
            original
        );
    }

    #[test]
    fn content_mismatch_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let file_path = root.join("data.txt");
        std::fs::write(&file_path, "actual content").unwrap();

        let applier = DiffApplier::new(root);
        let diff = StructuredDiff {
            changes: vec![FileChange::Modify {
                path: PathBuf::from("data.txt"),
                old_content: "wrong old content".to_string(),
                new_content: "new content".to_string(),
            }],
            ..test_diff("obj-7")
        };

        assert_eq!(
            applier.apply(&diff),
            Err(DiffApplyError::ContentMismatch(PathBuf::from("data.txt")))
        );
    }

    #[test]
    fn commit_message_format() {
        let applier = DiffApplier::new(PathBuf::from("."));
        let meta = CommitMetadata {
            summary: "feat: add widget".to_string(),
            objective_id: "obj-42".to_string(),
            worker_id: "worker-3".to_string(),
            reviewer_id: Some("reviewer-1".to_string()),
            guardian_id: None,
            human_approval_id: Some("human-5".to_string()),
        };
        let msg = applier.format_commit_message(&meta);
        assert!(msg.contains("feat: add widget"));
        assert!(msg.contains("objective-id: obj-42"));
        assert!(msg.contains("worker-id: worker-3"));
        assert!(msg.contains("reviewer-id: reviewer-1"));
        assert!(msg.contains("human-approval-id: human-5"));
    }
}
