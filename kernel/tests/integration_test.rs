//! Integration tests for the AI-OS Kernel.
//!
//! These tests exercise multiple modules together, simulating the flow
//! from state machine transitions through manifest creation to diff
//! application, and Stage 2 subsystems (ownership, permissions,
//! interface registry).

use ai_os_kernel::diff_applier::{
    self, CommitMetadata, DiffApplier, FileChange, StructuredDiff,
};
use ai_os_kernel::event_bus::{Actor, ActorKind};
use ai_os_kernel::interface_registry::{
    BreakingChangePolicy, ChangeVerdict, CompatibilityPolicy,
    Interface, InterfaceKind, InterfaceRegistry,
};
use ai_os_kernel::manifest::{
    ExecutionManifest, ManifestEnvironment, ManifestStage,
};
use ai_os_kernel::ownership::OwnershipModel;
use ai_os_kernel::permission::{Action, PermissionEngine};
use ai_os_kernel::state_machine::{
    self, ObjectiveState, ObjectivePrimaryState, ObjectiveFailureState,
    ObjectiveTerminalState, RetryPolicy};
use std::path::PathBuf;
use std::sync::Arc;

// ── State machine + diff applier integration ─────────────────────────────

#[test]
fn objective_lifecycle_with_diff() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let applier = DiffApplier::new(root.clone());

    // Phase 1: State machine transitions through the full happy path
    let policy = RetryPolicy::default();
    let mut state = ObjectiveState::Primary(ObjectivePrimaryState::Discovered);

    let expected: Vec<ObjectiveState> = vec![
        ObjectiveState::Primary(ObjectivePrimaryState::Discovered),
        ObjectiveState::Primary(ObjectivePrimaryState::Planned),
        ObjectiveState::Primary(ObjectivePrimaryState::Ready),
        ObjectiveState::Primary(ObjectivePrimaryState::Executing),
        ObjectiveState::Primary(ObjectivePrimaryState::Review),
        ObjectiveState::Primary(ObjectivePrimaryState::Integration),
        ObjectiveState::Terminal(ObjectiveTerminalState::Done),
    ];

    for target in &expected[1..] {
        state = state_machine::transition(state, *target, &policy, 0).unwrap();
    }

    assert_eq!(
        state,
        ObjectiveState::Terminal(ObjectiveTerminalState::Done)
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

// ═════════════════════════════════════════════════════════════════════════
// Stage 2 — Ownership Model integration
// ═════════════════════════════════════════════════════════════════════════

fn test_ownership_yaml() -> &'static str {
    r#"
domains:
  - id: kernel
    name: "Kernel"
    owner: "kernel-team"
    paths:
      - "kernel/**/*.rs"
      - "kernel/**/*.toml"
  - id: docs
    name: "Documentation"
    owner: "docs-team"
    paths:
      - "docs/**/*.md"
  - id: schemas
    name: "JSON Schemas"
    owner: "kernel-team"
    paths:
      - "schemas/**/*.json"
"#
}

#[test]
fn stage2_ownership_resolves_multiple_domains_correctly() {
    let model = OwnershipModel::from_yaml(test_ownership_yaml()).unwrap();

    // Kernel domain
    let domain = model.domain_for_file("kernel/src/main.rs");
    assert!(domain.is_some());
    assert_eq!(domain.unwrap().id, "kernel");

    // Docs domain
    let domain = model.domain_for_file("docs/13-ownership-model.md");
    assert!(domain.is_some());
    assert_eq!(domain.unwrap().id, "docs");

    // Schemas domain
    let domain = model.domain_for_file("schemas/manifest.json");
    assert!(domain.is_some());
    assert_eq!(domain.unwrap().id, "schemas");

    // Unmatched file returns None
    assert!(model.domain_for_file("README.md").is_none());
    assert!(model.domain_for_file("src/lib.rs").is_none());
}

#[test]
fn stage2_ownership_dedup_domain_for_files() {
    let model = OwnershipModel::from_yaml(test_ownership_yaml()).unwrap();
    let paths: Vec<String> = vec![
        "kernel/src/main.rs".into(),
        "docs/01-philosophy.md".into(),
        "kernel/src/lib.rs".into(),
        "schemas/event.json".into(),
    ];
    let domains = model.domains_for_files(&paths);
    assert_eq!(domains.len(), 3);
    assert!(domains.iter().any(|d| d.id == "kernel"));
    assert!(domains.iter().any(|d| d.id == "docs"));
    assert!(domains.iter().any(|d| d.id == "schemas"));
}

#[test]
fn stage2_ownership_validates_config_strictly() {
    // Empty domain list
    let bad = r#"domains: []"#;
    assert!(OwnershipModel::from_yaml(bad).is_err());

    // Domain without paths
    let bad = r#"
domains:
  - id: empty
    name: "Empty"
    owner: "nobody"
    paths: []
"#;
    assert!(OwnershipModel::from_yaml(bad).is_err());

    // Duplicate patterns
    let bad = r#"
domains:
  - id: kernel
    name: "Kernel"
    owner: "team"
    paths: ["kernel/**/*.rs"]
  - id: also-kernel
    name: "Also Kernel"
    owner: "team"
    paths: ["kernel/**/*.rs"]
"#;
    assert!(OwnershipModel::from_yaml(bad).is_err());
}

// ═════════════════════════════════════════════════════════════════════════
// Stage 2 — Permission Engine integration
// ═════════════════════════════════════════════════════════════════════════

fn test_engine() -> PermissionEngine {
    let model = Arc::new(OwnershipModel::from_yaml(test_ownership_yaml()).unwrap());
    PermissionEngine::new(model)
}

fn test_manifest_with_domains(domains: Vec<String>) -> ExecutionManifest {
    ExecutionManifest {
        manifest_id: "test-manifest-int".into(),
        objective_id: "obj-int-001".into(),
        stage: ManifestStage::Execution,
        title: "Integration Test".into(),
        description: None,
        groups: vec![],
        environment: ManifestEnvironment {
            language: None,
            framework: None,
            sdk: None,
            interface_registry: vec![],
        },
        dependencies: vec![],
        allowed_domains: domains,
        worker_type: Some("coder".into()),
        schema_version: "1.0".into(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

#[test]
fn stage2_permission_kernel_full_access() {
    let engine = test_engine();

    // Kernel can commit
    assert!(engine.check(
        &Actor { kind: ActorKind::Kernel, id: "kernel".into() },
        &Action::CreateCommit,
        "",
        None,
    ).is_allowed());

    // Kernel can branch
    assert!(engine.check(
        &Actor { kind: ActorKind::Kernel, id: "kernel".into() },
        &Action::CreateBranch,
        "",
        None,
    ).is_allowed());
}

#[test]
fn stage2_permission_worker_read_is_allowed() {
    let engine = test_engine();
    assert!(engine.check(
        &Actor { kind: ActorKind::Worker, id: "w-1".into() },
        &Action::Read,
        "docs/any-file.md",
        None,
    ).is_allowed());
}

#[test]
fn stage2_permission_worker_write_scoped_by_ownership() {
    let engine = test_engine();
    let manifest = test_manifest_with_domains(vec!["kernel".into()]);

    // Worker can write to owned files
    let result = engine.check_worker_write("w-1", "kernel/src/main.rs", &manifest);
    assert!(result.is_allowed(), "Worker should write to owned kernel file");

    // Worker cannot write to unowned files
    let result = engine.check_worker_write("w-1", "docs/README.md", &manifest);
    assert!(!result.is_allowed(), "Worker should be denied write to unowned docs file");
}

#[test]
fn stage2_permission_workers_cannot_create_commits() {
    let engine = test_engine();
    let result = engine.check(
        &Actor { kind: ActorKind::Worker, id: "w-1".into() },
        &Action::CreateCommit,
        "",
        None,
    );
    assert!(!result.is_allowed());
}

#[test]
fn stage2_permission_breaking_change_requires_human() {
    let engine = test_engine();
    let result = engine.check(
        &Actor { kind: ActorKind::Worker, id: "w-1".into() },
        &Action::ProposeBreakingChange,
        "objectives-api",
        None,
    );
    assert!(!result.is_allowed(), "Workers cannot propose breaking changes");

    let result = engine.check(
        &Actor { kind: ActorKind::Human, id: "alice@corp.com".into() },
        &Action::ProposeBreakingChange,
        "objectives-api",
        None,
    );
    assert!(result.is_allowed(), "Humans can propose breaking changes");
}

// ═════════════════════════════════════════════════════════════════════════
// Stage 2 — Interface Registry integration
// ═════════════════════════════════════════════════════════════════════════

fn sample_registry() -> InterfaceRegistry {
    let mut reg = InterfaceRegistry::new();

    let objectives_api = Interface {
        interface_id: "objectives-api".into(),
        kind: InterfaceKind::RestApi,
        owner_domain: "kernel".into(),
        consumers: vec!["cli".into(), "worker-pool".into()],
        version: "1.0.0".into(),
        signature: "GET /api/v1/objectives".into(),
        compatibility: CompatibilityPolicy {
            breaking_change_policy: BreakingChangePolicy::RequiresApproval,
            deprecated_since: None,
            sunset_date: None,
        },
        history: vec![],
    };

    let event_schema = Interface {
        interface_id: "objective-events".into(),
        kind: InterfaceKind::EventSchema,
        owner_domain: "kernel".into(),
        consumers: vec!["planner".into(), "scheduler".into()],
        version: "0.5.0".into(),
        signature: "Event { id, kind, timestamp, actor, payload }".into(),
        compatibility: CompatibilityPolicy {
            breaking_change_policy: BreakingChangePolicy::Forbidden,
            deprecated_since: None,
            sunset_date: None,
        },
        history: vec![],
    };

    reg.register(objectives_api).unwrap();
    reg.register(event_schema).unwrap();
    reg
}

#[test]
fn stage2_registry_blast_radius() {
    let reg = sample_registry();

    let radius = reg.blast_radius("objectives-api");
    assert_eq!(radius.owner_domain, "kernel");
    assert_eq!(radius.consumer_count, 2);
    assert!(radius.consumers.contains(&"cli".to_string()));

    // Non-existent interface
    let radius = reg.blast_radius("nonexistent");
    assert_eq!(radius.consumer_count, 0);
}

#[test]
fn stage2_registry_compatibility_enforcement() {
    let reg = sample_registry();

    // Non-breaking change is permitted
    let verdict = reg.check_change("objectives-api", "1.1.0").unwrap();
    assert_eq!(verdict, ChangeVerdict::Permitted);

    // Breaking change on requires_approval interface → needs human gate
    let verdict = reg.check_change("objectives-api", "2.0.0").unwrap();
    assert_eq!(verdict, ChangeVerdict::RequiresHumanApproval);

    // Breaking change on forbidden interface → rejected
    let result = reg.check_change("objective-events", "1.0.0");
    assert!(result.is_err());
}

#[test]
fn stage2_registry_list_by_domain() {
    let reg = sample_registry();
    let kernel_interfaces = reg.list_by_domain("kernel");
    assert_eq!(kernel_interfaces.len(), 2);

    let other = reg.list_by_domain("docs");
    assert!(other.is_empty());
}

// ═════════════════════════════════════════════════════════════════════════
// Stage 2 — Cross-domain flow (ownership + permissions + registry)
// ═════════════════════════════════════════════════════════════════════════

#[test]
fn stage2_cross_domain_write_requires_explicit_scope() {
    let engine = test_engine();

    // Worker with domain_scope = ["kernel"] trying to write to docs
    let manifest = test_manifest_with_domains(vec!["kernel".into()]);

    // Writing to kernel files: allowed
    assert!(engine.check_worker_write("w-1", "kernel/src/main.rs", &manifest).is_allowed());

    // Cross-domain request to a docs file: needs manifest scope
    let result = engine.check(
        &Actor { kind: ActorKind::Worker, id: "w-1".into() },
        &Action::RequestCrossDomainChange,
        "docs/architecture.md",
        Some(&manifest),
    );
    assert!(result.is_allowed(), "Worker with manifest scoped to kernel can make cross-domain request to docs file");
}

#[test]
fn stage2_cross_domain_no_manifest_denied() {
    let engine = test_engine();
    let result = engine.check(
        &Actor { kind: ActorKind::Worker, id: "w-1".into() },
        &Action::RequestCrossDomainChange,
        "docs",
        None,
    );
    assert!(!result.is_allowed());
}

#[test]
fn stage2_full_workflow_ownership_permits_worker_operations() {
    // Simulates: objective created → ownership resolved → permission checked
    // → manifest built with domain scope → worker produces diff

    let model = OwnershipModel::from_yaml(test_ownership_yaml()).unwrap();
    let engine = PermissionEngine::new(Arc::new(model));

    // A worker operating in the "kernel" domain
    let manifest = test_manifest_with_domains(vec!["kernel".into()]);

    // The worker wants to propose a write to a kernel source file
    let result = engine.check_worker_write("worker-001", "kernel/src/main.rs", &manifest);
    assert!(result.is_allowed(), "Worker should be permitted to write to kernel file");

    // The same worker should be denied writing to docs
    let result = engine.check_worker_write("worker-001", "docs/architecture.md", &manifest);
    assert!(!result.is_allowed(), "Worker should be denied writing outside domain scope");
}

// ═════════════════════════════════════════════════════════════════════════
// API — Objective Lifecycle (via HTTP Router)
// ═════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod api_objective_lifecycle {
    use ai_os_kernel::api::{self, AppState};
    use ai_os_kernel::config::{KernelConfig, SchedulerConfig};
    use ai_os_kernel::coordinator::Coordinator;
    use ai_os_kernel::event_bus::{Actor, ActorKind, Event as BusEvent, EventBus, EventKind};
    use ai_os_kernel::objective::ObjectiveStore;
    use ai_os_kernel::scheduler::Scheduler;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use serde_json::Value;
    use std::sync::Arc;
    use tower::ServiceExt;

    async fn test_app_state() -> (Arc<AppState>, Arc<ObjectiveStore>) {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("Failed to create in-memory SQLite pool");

        let store = Arc::new(
            ObjectiveStore::new(pool.clone())
                .await
                .expect("Failed to create ObjectiveStore"),
        );

        // Initialize the events table so replay methods work.
        ai_os_kernel::event_bus::init_event_store(&pool)
            .await
            .expect("Failed to init event store");

        let scheduler = Arc::new(tokio::sync::Mutex::new(
            Scheduler::new(SchedulerConfig::default()),
        ));
        let event_bus = EventBus::new().with_persistence(pool.clone());
        let config = KernelConfig::default();

        let coordinator = Coordinator::new()
            .with_objective_store(store.clone())
            .with_scheduler(scheduler.clone());

        let state = Arc::new(AppState {
            objective_store: store.clone(),
            scheduler,
            coordinator: tokio::sync::Mutex::new(coordinator),
            event_bus,
            config,
            started_at: chrono::Utc::now(),
            pool: pool.clone(),
            metrics_handle: metrics_exporter_prometheus::PrometheusBuilder::new()
                .build_recorder()
                .handle(),
        });

        (state, store)
    }

    async fn body_json(body: Body) -> Value {
        let bytes = body.collect().await.expect("Failed to collect body").to_bytes();
        serde_json::from_slice(&bytes).expect("Failed to parse JSON body")
    }

    #[tokio::test]
    async fn objective_crud_lifecycle() {
        let (state, _store) = test_app_state().await;
        let app = api::router(state);

        let create_body = serde_json::json!({
            "title": "Test objective",
            "description": "Created in integration test",
            "owner": "sisyphus",
            "priority": "high",
            "dependencies": [],
            "success_criteria": ["all tests pass"],
            "tags": ["test", "integration"]
        });

        let req = Request::post("/api/v1/objectives")
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_string(&create_body).unwrap()))
            .unwrap();

        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let json: Value = body_json(res.into_body()).await;
        assert!(json["success"].as_bool().unwrap());
        let obj_id = json["data"]["id"].as_str().unwrap().to_string();
        assert!(!obj_id.is_empty());

        let req = Request::get("/api/v1/objectives")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let req = Request::get(format!("/api/v1/objectives/{obj_id}"))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let json: Value = body_json(res.into_body()).await;
        assert!(json["success"].as_bool().unwrap());
        assert_eq!(json["data"]["title"], "Test objective");
        assert_eq!(json["data"]["status"]["Primary"], "Discovered");

        let req = Request::get("/api/v1/objectives/nonexistent-id")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn objective_rejects_invalid_priority() {
        let (state, _) = test_app_state().await;
        let app = api::router(state);

        let create_body = serde_json::json!({
            "title": "Bad priority",
            "description": "Should be rejected",
            "owner": "sisyphus",
            "priority": "ultra-high",
            "dependencies": [],
            "success_criteria": [],
            "tags": []
        });

        let req = Request::post("/api/v1/objectives")
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_string(&create_body).unwrap()))
            .unwrap();

        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let json: Value = body_json(res.into_body()).await;
        assert!(!json["success"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn objective_delete_abandons() {
        let (state, store) = test_app_state().await;
        let app = api::router(state);

        let create_body = serde_json::json!({
            "title": "To abandon",
            "description": "Will be abandoned via DELETE",
            "owner": "sisyphus",
            "priority": "low",
            "dependencies": [],
            "success_criteria": [],
            "tags": []
        });
        let req = Request::post("/api/v1/objectives")
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_string(&create_body).unwrap()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let json: Value = body_json(res.into_body()).await;
        let obj_id = json["data"]["id"].as_str().unwrap().to_string();

        let req = Request::delete(format!("/api/v1/objectives/{obj_id}"))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let json: Value = body_json(res.into_body()).await;
        assert!(json["success"].as_bool().unwrap());
        assert_eq!(json["data"]["status"], "abandoned");

        let obj = store.get(&obj_id).await.unwrap().unwrap();
        assert_eq!(obj.status.label(), "ABANDONED");
    }

    #[tokio::test]
    async fn objective_delete_nonexistent_returns_404() {
        let (state, _) = test_app_state().await;
        let app = api::router(state);

        let req = Request::delete("/api/v1/objectivities/missing-id")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn health_endpoint() {
        let (state, _) = test_app_state().await;
        let app = api::router(state);

        let req = Request::get("/api/v1/health")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let json: Value = body_json(res.into_body()).await;
        assert!(json["success"].as_bool().unwrap());
        assert_eq!(json["data"]["status"], "ok");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Stage 3 — Event Timeline API
    // ═══════════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn event_objective_timeline_returns_ordered_events() {
        let (state, _store) = test_app_state().await;
        let event_bus = state.event_bus.clone();
        let app = api::router(state);

        let obj_id = uuid::Uuid::new_v4().to_string();

        // Publish two events for this objective so the timeline has data
        event_bus.publish(
            BusEvent::new(EventKind::ObjectiveCreated, Actor { kind: ActorKind::Kernel, id: "test".into() }, serde_json::json!({"title": "timeline test"}))
                .with_objective(&obj_id),
        );
        event_bus.publish(
            BusEvent::new(EventKind::WorkerStarted, Actor { kind: ActorKind::Worker, id: "w1".into() }, serde_json::json!({}))
                .with_objective(&obj_id),
        );

        // Allow the async DB writes to complete
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Query objective timeline
        let req = Request::get(format!("/api/v1/events/objective/{obj_id}"))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let json: Value = body_json(res.into_body()).await;
        assert!(json["success"].as_bool().unwrap());
        let events = json["data"].as_array().unwrap();
        assert!(!events.is_empty(), "Objective timeline should have events");
        assert_eq!(events[0]["objective_id"], obj_id);
    }

    #[tokio::test]
    async fn event_recent_returns_events_with_limit() {
        let (state, _store) = test_app_state().await;
        let app = api::router(state);

        let req = Request::get("/api/v1/events/recent?limit=5")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let json: Value = body_json(res.into_body()).await;
        assert!(json["success"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn event_recent_defaults_to_reasonable_limit() {
        let (state, _store) = test_app_state().await;
        let app = api::router(state);

        let req = Request::get("/api/v1/events/recent")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn event_timeline_with_time_range() {
        let (state, _store) = test_app_state().await;
        let app = api::router(state);
        let create_body = serde_json::json!({
            "title": "Range test",
            "description": "Test timeline range endpoint",
            "owner": "sisyphus",
            "priority": "medium",
            "dependencies": [],
            "success_criteria": ["works"],
            "tags": []
        });
        let req = Request::post("/api/v1/objectives")
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_string(&create_body).unwrap()))
            .unwrap();
        let _ = app.clone().oneshot(req).await.unwrap();

        let now = chrono::Utc::now();
        let from = (now - chrono::Duration::hours(1)).to_rfc3339();
        let to = (now + chrono::Duration::hours(1)).to_rfc3339();

        let req = Request::get(format!(
            "/api/v1/events/timeline?from={from}&to={to}&limit=10"
        ))
        .body(Body::empty())
        .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let json: Value = body_json(res.into_body()).await;
        assert!(json["success"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn objective_timeline_for_nonexistent_objective_returns_empty() {
        let (state, _store) = test_app_state().await;
        let app = api::router(state);

        let req = Request::get("/api/v1/events/objective/nonexistent-id")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let json: Value = body_json(res.into_body()).await;
        assert!(json["success"].as_bool().unwrap());
        let events = json["data"].as_array().unwrap();
        assert!(events.is_empty(), "Non-existent objective should have no events");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // E2E — Full objective lifecycle (the core loop)
    // ═══════════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn e2e_full_lifecycle_create_through_done() {
        let (state, store) = test_app_state().await;
        let app = api::router(state);

        // ── Step 1: Create objective ────────────────────────────────
        let create_body = serde_json::json!({
            "title": "E2E lifecycle test",
            "description": "Tests the full create → dispatch → done path",
            "owner": "sisyphus",
            "priority": "high",
            "dependencies": [],
            "success_criteria": ["it works"],
            "tags": ["e2e"]
        });

        let req = Request::post("/api/v1/objectives")
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_string(&create_body).unwrap()))
            .unwrap();

        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let json: Value = body_json(res.into_body()).await;
        assert!(json["success"].as_bool().unwrap());
        let obj_id = json["data"]["id"].as_str().unwrap().to_string();
        assert!(!obj_id.is_empty());

        // Verify initial state
        let obj = store.get(&obj_id).await.unwrap().unwrap();
        assert_eq!(obj.status.label(), "DISCOVERED");

        // ── Step 2: Mark ready ─────────────────────────────────────
        let req = Request::post(format!("/api/v1/objectives/{obj_id}/ready"))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let json: Value = body_json(res.into_body()).await;
        assert!(json["success"].as_bool().unwrap());

        // Verify READY state persisted
        let obj = store.get(&obj_id).await.unwrap().unwrap();
        assert_eq!(obj.status.label(), "READY");

        // ── Step 3: Dispatch ────────────────────────────────────────
        let req = Request::post("/api/v1/scheduler/dispatch")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let json: Value = body_json(res.into_body()).await;
        assert!(json["success"].as_bool().unwrap());
        assert!(json["data"]["dispatched"].as_str().is_some(), "Expected a dispatched objective");

        // ── Step 4: Wait for completion (background transitions) ────
        // The coordinator's dispatch_and_monitor spawns a tokio task:
        //   Executing → Review → Integration → Done
        // Poll until Done (with timeout).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut final_label;
        loop {
            let obj = store.get(&obj_id).await.unwrap().unwrap();
            final_label = obj.status.label().to_string();
            if final_label == "DONE" {
                break;
            }
            if std::time::Instant::now() > deadline {
                panic!(
                    "Timed out waiting for objective to reach DONE. Last status: {final_label}"
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        // ── Step 5: Verify final state ─────────────────────────────
        assert_eq!(final_label, "DONE", "Objective should have completed the full lifecycle");
        let obj = store.get(&obj_id).await.unwrap().unwrap();
        assert_eq!(obj.status.label(), "DONE");
    }

    #[tokio::test]
    async fn e2e_worker_failure_goes_to_abandoned() {
        let (state, store) = test_app_state().await;
        let app = api::router(state.clone());

        let create_body = serde_json::json!({
            "title": "E2E failure test",
            "description": "Tests the worker failure → abandoned path",
            "owner": "sisyphus",
            "priority": "high",
            "dependencies": [],
            "success_criteria": ["it fails"],
            "tags": ["e2e", "failure"]
        });

        let req = Request::post("/api/v1/objectives")
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_string(&create_body).unwrap()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let json: Value = body_json(res.into_body()).await;
        assert!(json["success"].as_bool().unwrap());
        let obj_id = json["data"]["id"].as_str().unwrap().to_string();

        {
            let mut coord = state.coordinator.lock().await;
            coord.set_fail_objectives(vec![obj_id.clone()]);
        }

        let req = Request::post(format!("/api/v1/objectives/{obj_id}/ready"))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let req = Request::post("/api/v1/scheduler/dispatch")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut final_label;
        loop {
            let obj = store.get(&obj_id).await.unwrap().unwrap();
            final_label = obj.status.label().to_string();
            if final_label == "ABANDONED" {
                break;
            }
            if std::time::Instant::now() > deadline {
                panic!(
                    "Timed out waiting for objective to reach ABANDONED. Last status: {final_label}"
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        assert_eq!(final_label, "ABANDONED");
    }
}
