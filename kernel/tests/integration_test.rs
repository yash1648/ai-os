//! Integration tests for the AI-OS Kernel.
//!
//! These tests exercise multiple modules together, simulating the flow
//! from state machine transitions through manifest creation to diff
//! application.

use ai_os_kernel::diff_applier::{
    self, CommitMetadata, DiffApplier, FileChange, StructuredDiff,
};
use ai_os_kernel::state_machine::{
    self, ObjectiveState, ObjectivePrimaryState, ObjectiveFailureState,
    ObjectiveTerminalState, RetryPolicy};
use std::path::PathBuf;

// ── State machine + diff applier integration ─────────────────────────────

#[test]
fn objective_lifecycle_with_diff() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let applier = DiffApplier::new(root.clone());

    // Phase 1: State machine transitions through the full happy path
    let policy = RetryPolicy::default();
    let mut state = ObjectiveState::Primary(ObjectivePrimaryState::Discovered);

    let expected = [
        ObjectivePrimaryState::Discovered,
        ObjectivePrimaryState::Planned,
        ObjectivePrimaryState::Ready,
        ObjectivePrimaryState::Executing,
        ObjectivePrimaryState::Review,
        ObjectivePrimaryState::Integration,
        ObjectivePrimaryState::Done,
    ];

    for &next in &expected[1..] {
        let target = ObjectiveState::Primary(next);
        state = state_machine::transition(state, target, &policy, 0).unwrap();
    }

    assert_eq!(
        state,
        ObjectiveState::Primary(ObjectivePrimaryState::Done)
    );

    // Phase 2: Create a file (simulates a worker producing output)
    let diff = StructuredDiff {
        objective_id: "obj-lifecycle-1".to_string(),
        worker_id: "worker-fake-1".to_string(),
        changes: vec![FileChange::Create {
            path: PathBuf::from("output.txt"),
            content: "Objective completed successfully.".to_string(),
        }],
        commit_metadata: CommitMetadata {
            summary: "feat: complete objective".to_string(),
            objective_id: "obj-lifecycle-1".to_string(),
            worker_id: "worker-fake-1".to_string(),
            reviewer_id: None,
            guardian_id: None,
            human_approval_id: None,
        },
    };

    let (outcome, snapshot) = applier.apply(&diff).unwrap();
    match outcome {
        diff_applier::ApplyOutcome::Applied { files_changed, .. } => {
            assert_eq!(files_changed, 1);
        }
        _ => panic!("Expected Applied"),
    }

    assert!(root.join("output.txt").exists());
    assert_eq!(
        std::fs::read_to_string(root.join("output.txt")).unwrap(),
        "Objective completed successfully."
    );

    // Phase 3: Rollback and verify state machine handles it
    applier.rollback(snapshot).unwrap();
    assert!(!root.join("output.txt").exists());

    // After rollback, the objective can retry from Rollback -> Ready
    let rollback_state = ObjectiveState::Failure(ObjectiveFailureState::Rollback);
    let re_ready = state_machine::transition(
        rollback_state,
        ObjectiveState::Primary(ObjectivePrimaryState::Ready),
        &policy,
        1,
    ).unwrap();
    assert_eq!(
        re_ready,
        ObjectiveState::Primary(ObjectivePrimaryState::Ready)
    );
}

// ── Error recovery: retry then abandon ─────────────────────────────────

#[test]
fn test_retry_exhaustion_triggers_abandonment() {
    let policy = RetryPolicy { max_retries: 2 };

    // Exhausted retries (count == max_retries) should be denied
    let r3 = state_machine::transition(
        ObjectiveState::Failure(ObjectiveFailureState::ExecutionFailure),
        ObjectiveState::Primary(ObjectivePrimaryState::Ready),
        &policy,
        2,
    );
    assert_eq!(r3, Err(state_machine::TransitionError::RetryLimitExhausted));

    // After exhaustion, abandon is always allowed
    let abandon = state_machine::transition(
        ObjectiveState::Failure(ObjectiveFailureState::ExecutionFailure),
        ObjectiveState::Terminal(ObjectiveTerminalState::Abandoned),
        &policy,
        2,
    );
    assert!(abandon.is_ok());
}

// ── Diff scope validation ────────────────────────────────────────────────

#[test]
fn test_diff_outside_scope_rejected() {
    let applier = DiffApplier::new(PathBuf::from("/workspace"));
    let diff = StructuredDiff {
        objective_id: "obj-scope-1".to_string(),
        worker_id: "worker-1".to_string(),
        changes: vec![FileChange::Modify {
            path: PathBuf::from("src/lib.rs"),
            old_content: String::new(),
            new_content: String::new(),
        }],
        commit_metadata: CommitMetadata {
            summary: String::new(),
            objective_id: "obj-scope-1".to_string(),
            worker_id: "worker-1".to_string(),
            reviewer_id: None,
            guardian_id: None,
            human_approval_id: None,
        },
    };

    let allowed = vec![PathBuf::from("tests")];
    assert_eq!(
        applier.validate_scope(&diff, &allowed),
        Err(diff_applier::DiffApplyError::OutsideScope(
            PathBuf::from("src/lib.rs")
        ))
    );
}

// ── Manifest + Diff roundtrip ──────────────────────────────────────────

#[test]
fn test_manifest_and_diff_integration() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let applier = DiffApplier::new(root.clone());

    // Create a manifest file to "modify"
    std::fs::write(root.join("manifest.json"), r#"{"version": "0.1.0"}"#).unwrap();

    let diff = StructuredDiff {
        objective_id: "obj-manifest-1".to_string(),
        worker_id: "worker-2".to_string(),
        changes: vec![
            FileChange::Modify {
                path: PathBuf::from("manifest.json"),
                old_content: r#"{"version": "0.1.0"}"#.to_string(),
                new_content: r#"{"version": "1.0.0", "stage": "review"}"#.to_string(),
            },
            FileChange::Create {
                path: PathBuf::from("review_notes.md"),
                content: "# Review Notes\n- All checks passed.".to_string(),
            },
        ],
        commit_metadata: CommitMetadata {
            summary: "chore: update manifest version".to_string(),
            objective_id: "obj-manifest-1".to_string(),
            worker_id: "worker-2".to_string(),
            reviewer_id: None,
            guardian_id: None,
            human_approval_id: None,
        },
    };

    let (outcome, snapshot) = applier.apply(&diff).unwrap();
    match outcome {
        diff_applier::ApplyOutcome::Applied { files_changed, .. } => {
            assert_eq!(files_changed, 2);
        }
        _ => panic!("Expected Applied"),
    }

    // Verify both changes are present
    let updated = std::fs::read_to_string(root.join("manifest.json")).unwrap();
    assert!(updated.contains("1.0.0"));
    assert!(root.join("review_notes.md").exists());

    // Rollback
    applier.rollback(snapshot).unwrap();
    let rolled_back = std::fs::read_to_string(root.join("manifest.json")).unwrap();
    assert!(rolled_back.contains("0.1.0"));
    assert!(!root.join("review_notes.md").exists());
}

// ── Commit message format ─────────────────────────────────────────────

#[test]
fn test_commit_message_contains_all_ids() {
    let applier = DiffApplier::new(PathBuf::from("."));
    let meta = CommitMetadata {
        summary: "fix: resolve timeout issue".to_string(),
        objective_id: "obj-42".to_string(),
        worker_id: "worker-7".to_string(),
        reviewer_id: Some("reviewer-3".to_string()),
        guardian_id: Some("guardian-1".to_string()),
        human_approval_id: Some("human-approval-2".to_string()),
    };
    let msg = applier.format_commit_message(&meta);

    assert!(msg.starts_with("fix: resolve timeout issue"));
    assert!(msg.contains("objective-id: obj-42"));
    assert!(msg.contains("worker-id: worker-7"));
    assert!(msg.contains("reviewer-id: reviewer-3"));
    assert!(msg.contains("guardian-id: guardian-1"));
    assert!(msg.contains("human-approval-id: human-approval-2"));
}
