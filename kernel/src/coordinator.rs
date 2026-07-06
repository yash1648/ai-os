// AI-OS Kernel — Coordinator
//
// Bridges the Scheduler and ExecutionEngine with a clean 2-phase event lifecycle:
//   Scheduler emits DispatchDecision → Coordinator receives → ExecutionEngine spawns worker
//   ExecutionEngine emits WorkerStarted/WorkerFinished → Coordinator passes through
//
// This prevents semantic collision where both Scheduler and ExecutionEngine
// emitted WorkerStarted for different stages.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use metrics::{counter, gauge, histogram, describe_counter, describe_gauge, describe_histogram};

use crate::diff_applier::{CommitMetadata, FileChange, StructuredDiff};
use crate::event_bus::{Actor, ActorKind, Event, EventBus, EventKind};
use crate::execution_engine::{run_simulated_worker, WorkerConfig, WorkerPool};
use crate::guardian::{Guardian, GuardianVerdict};
use crate::interface_registry::{ChangeVerdict, InterfaceRegistry};
use crate::objective::ObjectiveStore;
use crate::ownership::OwnershipModel;
use crate::review::{ReviewVerdict, Reviewer};
use crate::scheduler::Scheduler;
use crate::state_machine::{self, ObjectiveState, RetryPolicy};

// ---------------------------------------------------------------------------
// Cross-domain check types
// ---------------------------------------------------------------------------

/// Verdict from a cross-domain compatibility check performed by the Coordinator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrossDomainVerdict {
    /// Diff touches files in only one domain (or none) — no cross-domain concern.
    SingleDomain,
    /// Diff touches multiple domains and all owned interfaces are compatible.
    CompatibleCrossDomain,
    /// Diff touches multiple domains with at least one breaking interface change
    /// that requires human approval.  Carries the list of affected interface IDs.
    RequiresHumanApproval(Vec<String>),
}

// ---------------------------------------------------------------------------
// Coordinator — orchestrates scheduler dispatch → execution engine
// ---------------------------------------------------------------------------

/// Coordinates the flow from dispatch decision to worker execution.
///
/// Listens for DispatchDecision events from the Scheduler, forwards to
/// the ExecutionEngine for actual worker spawn, and manages the lifecycle.
/// Optionally runs Review Pipeline and Architecture Guardian checks
/// in the REVIEW and INTEGRATION phases.
pub struct Coordinator {
    /// The execution engine pool for spawning workers.
    pub worker_pool: WorkerPool,
    worker_config: WorkerConfig,
    /// Total dispatches processed by this coordinator.
    dispatch_count: AtomicUsize,
    /// Optional event bus for publishing lifecycle events.
    event_bus: Option<Arc<EventBus>>,
    /// Reference to the objective store for status transitions on worker completion.
    objective_store: Option<Arc<ObjectiveStore>>,
    /// Reference to the scheduler for freeing dispatch slots on worker completion.
    scheduler: Option<Arc<tokio::sync::Mutex<Scheduler>>>,
    /// Optional Review Pipeline — runs during the REVIEW state.
    reviewer: Option<Arc<Reviewer>>,
    /// Optional Architecture Guardian — runs during the INTEGRATION state.
    guardian: Option<Arc<Guardian>>,
}

/// Factory for creating a Coordinator.
impl Coordinator {
    /// Create a new Coordinator with a fresh WorkerPool.
    pub fn new() -> Self {
        Self {
            worker_pool: WorkerPool::new(4), // Default max 4 concurrent workers
            worker_config: WorkerConfig::default(),
            dispatch_count: AtomicUsize::new(0),
            event_bus: None,
            objective_store: None,
            scheduler: None,
            reviewer: None,
            guardian: None,
        }
    }

    /// Set the maximum concurrent workers in the pool.
    pub fn with_max_concurrent(mut self, max: usize) -> Self {
        self.worker_pool = WorkerPool::new(max);
        self
    }

    pub fn with_worker_config(mut self, config: WorkerConfig) -> Self {
        self.worker_config = config;
        self
    }

    /// Update the list of objective IDs that should simulate failure.
    /// Useful in tests where the objective ID is not known at construction time.
    pub fn set_fail_objectives(&mut self, ids: Vec<String>) {
        self.worker_config.fail_objective_ids = ids;
    }

    /// Attach an event bus for lifecycle event emission.
    pub fn with_event_bus(mut self, bus: Arc<EventBus>) -> Self {
        self.worker_pool = self.worker_pool.with_event_bus(bus.clone());
        self.event_bus = Some(bus);
        self
    }

    /// Attach an objective store reference for auto-transitioning on completion.
    pub fn with_objective_store(mut self, store: Arc<ObjectiveStore>) -> Self {
        self.objective_store = Some(store);
        self
    }

    /// Attach a scheduler reference for freeing dispatch slots on completion.
    pub fn with_scheduler(mut self, sched: Arc<tokio::sync::Mutex<Scheduler>>) -> Self {
        self.scheduler = Some(sched);
        self
    }

    /// Attach a Review Pipeline — runs during the REVIEW state after worker completion.
    pub fn with_reviewer(mut self, reviewer: Reviewer) -> Self {
        self.reviewer = Some(Arc::new(reviewer));
        self
    }

    /// Attach an Architecture Guardian — runs during the INTEGRATION state after review passes.
    pub fn with_guardian(mut self, guardian: Guardian) -> Self {
        self.guardian = Some(Arc::new(guardian));
        self
    }

    /// Process a dispatch decision — spawn a worker if capacity allows.
    pub fn dispatch(&mut self, objective_id: &str) -> Option<String> {
        if !self.worker_pool.can_accept() {
            self.emit_pool_full(objective_id);
            return None;
        }

        describe_counter!("ai_os_coordinator_dispatch_count", "Total number of objectives dispatched through the coordinator");
        describe_gauge!("ai_os_coordinator_active_count", "Current number of active workers");
        counter!("ai_os_coordinator_dispatch_count").increment(1);

        let objective_id_owned = objective_id.to_string();
        let config = self.worker_config.clone();

        let worker_id = self.worker_pool.spawn(&objective_id_owned.clone(), async move {
            run_simulated_worker(&objective_id_owned, &config).await
        });

        worker_id
    }

    /// Query the number of active workers.
    pub fn active_count(&self) -> usize {
        let count = self.worker_pool.active_count();
        gauge!("ai_os_coordinator_active_count").set(count as f64);
        count
    }

    /// Query total dispatches processed.
    pub fn dispatch_count(&self) -> usize {
        self.dispatch_count.load(Ordering::SeqCst)
    }

    /// Dispatch an objective and monitor its worker for completion.
    ///
    /// Spawns the worker via the execution engine and sets up a background
    /// task that transitions the objective through the state machine on
    /// completion:
    ///
    ///   EXECUTING → REVIEW (Review Pipeline check)
    ///           → INTEGRATION (Architecture Guardian check)
    ///           → DONE
    ///
    /// If the Review Pipeline finds blocking issues, the objective transitions
    /// REVIEW → REVIEW_FAILURE → ABANDONED.
    /// If the Architecture Guardian finds blocking violations, the objective
    /// transitions INTEGRATION → INTEGRATION_FAILURE → ABANDONED.
    ///
    /// When no reviewer/guardian is configured, the flow behaves as before
    /// (backward compatible).
    pub async fn dispatch_and_monitor(
        &mut self,
        objective_id: &str,
    ) -> Option<String> {
        if !self.worker_pool.can_accept() {
            self.emit_pool_full(objective_id);
            return None;
        }

        self.dispatch_count.fetch_add(1, Ordering::SeqCst);
        let objective_id_owned = objective_id.to_string();
        let config = self.worker_config.clone();
        let worker_id_for_sim = objective_id_owned.clone();

        let worker_id = self.worker_pool.spawn(&objective_id_owned, async move {
            run_simulated_worker(&worker_id_for_sim, &config).await
        })?;

        let wid = worker_id.clone();

        // Take the JoinHandle to monitor completion
        let handle = self.worker_pool.take_handle(objective_id);
        let store = self.objective_store.clone();
        let sched = self.scheduler.clone();
        let reviewer = self.reviewer.clone();
        let guardian = self.guardian.clone();
        let event_bus = self.event_bus.clone();

        if let Some(handle) = handle {
            let worker_start = Instant::now();
            tokio::spawn(async move {
                let id = &objective_id_owned;
                let policy = RetryPolicy::default();

                let result = handle.await
                    .unwrap_or_else(|join_err| crate::execution_engine::WorkerResult {
                        objective_id: id.clone(),
                        status: crate::execution_engine::WorkerStatus::Failed(
                            format!("worker task panicked: {join_err}"),
                        ),
                        metrics: crate::execution_engine::WorkerMetrics::default(),
                    });

                match result.status {
                    crate::execution_engine::WorkerStatus::Completed => {
                        let executing = ObjectiveState::from_label("EXECUTING");
                        let review_st = ObjectiveState::from_label("REVIEW");
                        let integration = ObjectiveState::from_label("INTEGRATION");
                        let done_st =
                            ObjectiveState::Terminal(state_machine::ObjectiveTerminalState::Done);

                        // ── Phase 1: EXECUTING → REVIEW ─────────────────
                        Self::apply_transition(&store, id, executing, review_st, &policy, 0).await;

                        // ── Phase 2: Review Pipeline check ──────────────
                        let review_ok = match &reviewer {
                            Some(r) => {
                                Self::run_review_check(r, &store, &event_bus, id, &wid).await
                            }
                            None => true,
                        };

                        if !review_ok {
                            let review_failure =
                                ObjectiveState::from_label("REVIEW_FAILURE");
                            let abandoned = ObjectiveState::Terminal(
                                state_machine::ObjectiveTerminalState::Abandoned,
                            );
                            Self::apply_transition(&store, id, review_st, review_failure, &policy, 0).await;
                            Self::apply_transition(&store, id, review_failure, abandoned, &policy, 0).await;
                            Self::free_scheduler_slot(&sched, id).await;
                            return;
                        }

                        // ── Phase 3: REVIEW → INTEGRATION ──────────────
                        Self::apply_transition(&store, id, review_st, integration, &policy, 0).await;

                        // ── Phase 4: Architecture Guardian check ────────
                        let guardian_ok = match &guardian {
                            Some(g) => {
                                Self::run_guardian_check(g, &store, &event_bus, id, &wid).await
                            }
                            None => true,
                        };

                        if !guardian_ok {
                            let integration_failure =
                                ObjectiveState::from_label("INTEGRATION_FAILURE");
                            let abandoned = ObjectiveState::Terminal(
                                state_machine::ObjectiveTerminalState::Abandoned,
                            );
                            Self::apply_transition(&store, id, integration, integration_failure, &policy, 0).await;
                            Self::apply_transition(&store, id, integration_failure, abandoned, &policy, 0).await;
                            Self::free_scheduler_slot(&sched, id).await;
                            return;
                        }

                        // ── Phase 4b: Cross-domain compatibility check ─────
                        let diff_for_cd = Self::placeholder_diff(id, &wid);
                        let cd_verdict = match &guardian {
                            Some(g) => Self::check_cross_domain(
                                g.ownership_model(),
                                g.interface_registry(),
                                &diff_for_cd,
                            ),
                            None => CrossDomainVerdict::SingleDomain,
                        };

                        match &cd_verdict {
                            CrossDomainVerdict::RequiresHumanApproval(interfaces) => {
                                Self::emit_event(
                                    &event_bus,
                                    EventKind::CrossDomainRequestRaised,
                                    "coordinator",
                                    serde_json::json!({
                                        "objective_id": id,
                                        "worker_id": wid,
                                        "compatible": false,
                                        "interfaces": interfaces,
                                    }),
                                );
                                Self::emit_event(
                                    &event_bus,
                                    EventKind::HumanApprovalRequested,
                                    "coordinator",
                                    serde_json::json!({
                                        "objective_id": id,
                                        "worker_id": wid,
                                        "reason": "cross_domain_breaking_change",
                                        "interfaces": interfaces,
                                    }),
                                );
                                // Stay in INTEGRATION — do not transition to Done
                                Self::free_scheduler_slot(&sched, id).await;
                                return;
                            }
                            CrossDomainVerdict::CompatibleCrossDomain => {
                                Self::emit_event(
                                    &event_bus,
                                    EventKind::CrossDomainRequestRaised,
                                    "coordinator",
                                    serde_json::json!({
                                        "objective_id": id,
                                        "worker_id": wid,
                                        "compatible": true,
                                    }),
                                );
                                // Fall through to Integration → Done below
                            }
                            CrossDomainVerdict::SingleDomain => {
                                // No cross-domain concern — fall through to Done
                            }
                        }

                        // ── Phase 5: INTEGRATION → DONE ────────────────
                        Self::apply_transition(&store, id, integration, done_st, &policy, 0).await;
                        Self::emit_event(
                            &event_bus,
                            EventKind::ObjectiveCompleted,
                            "coordinator",
                            serde_json::json!({
                                "objective_id": id,
                                "worker_id": wid,
                            }),
                        );
                    }
                    crate::execution_engine::WorkerStatus::Failed(_) => {
                        let executing = ObjectiveState::from_label("EXECUTING");
                        let exec_failure = ObjectiveState::from_label("EXECUTION_FAILURE");
                        let abandoned =
                            ObjectiveState::Terminal(state_machine::ObjectiveTerminalState::Abandoned);

                        if state_machine::transition(executing, exec_failure, &policy, 0).is_ok() {
                            if let Some(ref store) = store {
                                let _ = store.update_status(id, &exec_failure, 0).await;
                            }
                        }

                        if state_machine::transition(exec_failure, abandoned, &policy, 0).is_ok() {
                            if let Some(ref store) = store {
                                let _ = store.update_status(id, &abandoned, 0).await;
                            }
                        }
                    }
                    _ => {}
                }

                describe_histogram!("ai_os_coordinator_worker_duration_seconds", "Duration of worker execution in seconds");
                let elapsed = worker_start.elapsed();
                histogram!("ai_os_coordinator_worker_duration_seconds").record(elapsed.as_secs_f64());

                // Free the scheduler slot
                Self::free_scheduler_slot(&sched, id).await;
            });
        }

        Some(worker_id)
    }

    // ── Static helpers (callable from tokio::spawn without &self) ────────

    /// Transition the objective state and persist to the store.
    async fn apply_transition(
        store: &Option<Arc<ObjectiveStore>>,
        objective_id: &str,
        current: ObjectiveState,
        target: ObjectiveState,
        policy: &RetryPolicy,
        retry_count: u32,
    ) {
        if state_machine::transition(current, target, policy, retry_count).is_ok() {
            if let Some(store) = store {
                let _ = store.update_status(objective_id, &target, retry_count).await;
            }
        }
    }

    /// Emit an event to the event bus if configured.
    fn emit_event(
        event_bus: &Option<Arc<EventBus>>,
        kind: EventKind,
        actor_id: &str,
        payload: serde_json::Value,
    ) {
        if let Some(bus) = event_bus {
            let event = Event::new(
                kind,
                Actor {
                    kind: ActorKind::Kernel,
                    id: actor_id.into(),
                },
                payload,
            );
            bus.publish(event);
        }
    }

    /// Notify the scheduler that a worker slot is free.
    async fn free_scheduler_slot(
        sched: &Option<Arc<tokio::sync::Mutex<Scheduler>>>,
        objective_id: &str,
    ) {
        if let Some(sched) = sched {
            let mut s = sched.lock().await;
            s.notify_worker_finished(objective_id);
        }
    }

    /// Run the Review Pipeline against a placeholder diff.
    ///
    /// Returns `true` if the review passes (no blocking findings), `false`
    /// if the review rejects the diff with blocking issues.
    async fn run_review_check(
        reviewer: &Reviewer,
        store: &Option<Arc<ObjectiveStore>>,
        event_bus: &Option<Arc<EventBus>>,
        objective_id: &str,
        worker_id: &str,
    ) -> bool {
        // Build a deterministic diff for review (simulated workers produce
        // no real changes, so the diff is empty — which passes all checks).
        let diff = Self::placeholder_diff(objective_id, worker_id);

        // Fetch the objective if a store is available.
        let objective = match store {
            Some(s) => s.get(objective_id).await.ok().flatten()
                .unwrap_or_else(|| default_objective(objective_id)),
            None => default_objective(objective_id),
        };

        let verdict = reviewer.review(&diff, &objective, None);

        match verdict {
            ReviewVerdict::Fail(findings) => {
                Self::emit_event(
                    event_bus,
                    EventKind::ReviewFailed,
                    "coordinator",
                    serde_json::json!({
                        "objective_id": objective_id,
                        "worker_id": worker_id,
                        "findings": findings,
                    }),
                );
                false
            }
            ReviewVerdict::Pass(findings) => {
                Self::emit_event(
                    event_bus,
                    EventKind::ReviewPassed,
                    "coordinator",
                    serde_json::json!({
                        "objective_id": objective_id,
                        "worker_id": worker_id,
                        "warnings": findings,
                    }),
                );
                true
            }
        }
    }

    /// Run the Architecture Guardian against a placeholder diff.
    ///
    /// Returns `true` if the guardian passes or requires human approval
    /// (non-blocking), `false` if the guardian rejects the diff.
    async fn run_guardian_check(
        guardian: &Guardian,
        store: &Option<Arc<ObjectiveStore>>,
        event_bus: &Option<Arc<EventBus>>,
        objective_id: &str,
        worker_id: &str,
    ) -> bool {
        let diff = Self::placeholder_diff(objective_id, worker_id);

        let objective = match store {
            Some(s) => s.get(objective_id).await.ok().flatten()
                .unwrap_or_else(|| default_objective(objective_id)),
            None => default_objective(objective_id),
        };

        let verdict = guardian.evaluate(&diff, &objective);

        match verdict {
            GuardianVerdict::Fail(violations) => {
                Self::emit_event(
                    event_bus,
                    EventKind::GuardianFailed,
                    "coordinator",
                    serde_json::json!({
                        "objective_id": objective_id,
                        "worker_id": worker_id,
                        "violations": violations,
                    }),
                );
                false
            }
            GuardianVerdict::RequiresHumanApproval(violations) => {
                Self::emit_event(
                    event_bus,
                    EventKind::HumanApprovalRequested,
                    "coordinator",
                    serde_json::json!({
                        "objective_id": objective_id,
                        "worker_id": worker_id,
                        "violations": violations,
                    }),
                );
                true // non-blocking — proceeds to INTEGRATION
            }
            GuardianVerdict::Pass => {
                Self::emit_event(
                    event_bus,
                    EventKind::GuardianPassed,
                    "coordinator",
                    serde_json::json!({
                        "objective_id": objective_id,
                        "worker_id": worker_id,
                    }),
                );
                true
            }
        }
    }

    /// Check whether a structured diff crosses domain boundaries and whether
    /// the affected owned interfaces have compatible changes.
    ///
    /// Returns:
    /// - `SingleDomain` when the diff touches at most one domain.
    /// - `CompatibleCrossDomain` when multiple domains are touched but all
    ///   owned interfaces are compatible or have no breaking policy.
    /// - `RequiresHumanApproval(interfaces)` when a breaking change is detected
    ///   on an interface owned by one of the affected domains.
    fn check_cross_domain(
        ownership: &OwnershipModel,
        interface_registry: &InterfaceRegistry,
        diff: &StructuredDiff,
    ) -> CrossDomainVerdict {
        use crate::diff_applier::FileChange;

        // 1. Extract file paths from all diff changes.
        let paths: Vec<String> = diff
            .changes
            .iter()
            .map(|change| {
                let path = match change {
                    FileChange::Create { path, .. } => path,
                    FileChange::Modify { path, .. } => path,
                    FileChange::Delete { path, .. } => path,
                };
                path.to_string_lossy().to_string()
            })
            .collect();

        if paths.is_empty() {
            return CrossDomainVerdict::SingleDomain;
        }

        // 2. Resolve the distinct domains touched by these paths.
        let domains = ownership.domains_for_files(&paths);

        // 3. If at most one domain is touched there is no cross-domain concern.
        if domains.len() <= 1 {
            return CrossDomainVerdict::SingleDomain;
        }

        // 4. For each domain, check every interface it owns for a breaking
        //    change.  A major-version bump is proposed to exercise the policy.
        let mut breaking_interfaces: Vec<String> = Vec::new();

        for domain in &domains {
            for iface_id in &domain.owned_interfaces {
                if let Some(iface) = interface_registry.get(iface_id) {
                    let proposed = bump_major_version(&iface.version);
                    match interface_registry.check_change(iface_id, &proposed) {
                        Ok(ChangeVerdict::RequiresHumanApproval) => {
                            breaking_interfaces.push(iface_id.clone());
                        }
                        // Permitted, RequiresDeprecation, and errors are not
                        // considered blocking for cross-domain routing.
                        Ok(ChangeVerdict::RequiresDeprecation) => {}
                        Ok(ChangeVerdict::Permitted) => {}
                        Err(_) => {}
                    }
                }
            }
        }

        if breaking_interfaces.is_empty() {
            CrossDomainVerdict::CompatibleCrossDomain
        } else {
            CrossDomainVerdict::RequiresHumanApproval(breaking_interfaces)
        }
    }

    /// Build a minimal placeholder diff for simulated workers.
    fn placeholder_diff(objective_id: &str, worker_id: &str) -> StructuredDiff {
        StructuredDiff {
            objective_id: objective_id.to_string(),
            worker_id: worker_id.to_string(),
            changes: vec![],
            commit_metadata: CommitMetadata {
                summary: "simulated worker — no real changes".into(),
                objective_id: objective_id.to_string(),
                worker_id: worker_id.to_string(),
                reviewer_id: None,
                guardian_id: None,
                human_approval_id: None,
            },
        }
    }

    fn emit_pool_full(&self, objective_id: &str) {
        Self::emit_event(
            &self.event_bus,
            EventKind::SchedulingThrottled,
            "coordinator",
            serde_json::json!({
                "objective_id": objective_id,
                "reason": "worker_pool_full",
                "active_workers": self.worker_pool.active_count(),
                "max_concurrent": self.worker_pool.max_concurrent(),
            }),
        );
    }
}

/// Default objective for when the store is unavailable.
fn default_objective(objective_id: &str) -> crate::objective::Objective {
    use chrono::Utc;
    use crate::objective::{Objective, Priority};
    Objective {
        id: objective_id.into(),
        title: String::new(),
        description: String::new(),
        owner: "coordinator".into(),
        parent_id: None,
        priority: Priority::Medium,
        status: ObjectiveState::from_label("REVIEW"),
        dependencies: vec![],
        success_criteria: vec![],
        plan_id: None,
        retry_count: 0,
        tags: vec![],
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

/// Bump the major version component of a semver string ("1.2.3" → "2.0.0").
/// Used by `check_cross_domain` to exercise interface policies.
fn bump_major_version(version: &str) -> String {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() == 3 {
        if let Ok(major) = parts[0].parse::<u64>() {
            return format!("{}.0.0", major + 1);
        }
    }
    "2.0.0".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff_applier::FileChange;
    use crate::interface_registry::{BreakingChangePolicy, CompatibilityPolicy, Interface, InterfaceKind, VersionEntry};
    use crate::interface_registry::InterfaceRegistry;
    use crate::ownership::OwnershipModel;
    use chrono::Utc;

    // ── Existing tests ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_coordinator_dispatches_to_pool() {
        let mut coord = Coordinator::new().with_max_concurrent(2);
        let wid = coord.dispatch("obj1");
        assert!(wid.is_some());
        assert_eq!(coord.active_count(), 1);
    }

    #[tokio::test]
    async fn test_coordinator_respects_capacity() {
        let mut coord = Coordinator::new().with_max_concurrent(1);
        let _ = coord.dispatch("obj1");
        let wid = coord.dispatch("obj2");
        // Pool is full — second dispatch should fail
        assert!(wid.is_none());
    }

    // ── Builder method tests ────────────────────────────────────────────

    #[test]
    fn test_with_reviewer() {
        let reviewer = Reviewer::new();
        let coord = Coordinator::new().with_reviewer(reviewer);
        assert!(coord.reviewer.is_some());
        assert!(coord.guardian.is_none());
    }

    #[test]
    fn test_with_both_reviewer_and_guardian() {
        let reviewer = Reviewer::new();
        let ownership = sample_ownership();
        let registry = sample_interface_registry();
        let guardian = Guardian::new(ownership, registry);
        let coord = Coordinator::new()
            .with_reviewer(reviewer)
            .with_guardian(guardian);
        assert!(coord.reviewer.is_some());
        assert!(coord.guardian.is_some());
    }

    // ── placeholder_diff tests ──────────────────────────────────────────

    #[test]
    fn test_placeholder_diff_structure() {
        let diff = Coordinator::placeholder_diff("obj-42", "worker-99");
        assert_eq!(diff.objective_id, "obj-42");
        assert_eq!(diff.worker_id, "worker-99");
        assert!(diff.changes.is_empty(), "placeholder diff should have zero changes");
        assert_eq!(diff.commit_metadata.summary, "simulated worker — no real changes");
        assert_eq!(diff.commit_metadata.objective_id, "obj-42");
        assert_eq!(diff.commit_metadata.worker_id, "worker-99");
        assert!(diff.commit_metadata.reviewer_id.is_none());
        assert!(diff.commit_metadata.guardian_id.is_none());
    }

    // ── dispatch_and_monitor with reviewer + guardian configured ────────
    //
    // With simulated workers (no real diff), the review and guardian checks
    // trivially pass on the empty diff, so the flow completes normally.

    #[tokio::test]
    async fn test_dispatch_with_reviewer_and_guardian_passes() {
        let reviewer = Reviewer::new();
        let ownership = sample_ownership();
        let registry = sample_interface_registry();
        let guardian = Guardian::new(ownership, registry);

        let mut coord = Coordinator::new()
            .with_max_concurrent(4)
            .with_reviewer(reviewer)
            .with_guardian(guardian);

        // Use dispatch_and_monitor (not dispatch) to exercise the full
        // EXECUTING → REVIEW → INTEGRATION → DONE flow with gate checks.
        let wid = coord.dispatch_and_monitor("obj-review-pass").await;
        assert!(wid.is_some(), "should dispatch successfully");

        // Give the background monitoring task time to finish.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // After take_handle the worker is no longer in the active pool.
        assert_eq!(coord.active_count(), 0);
    }

    #[tokio::test]
    async fn test_dispatch_with_review_and_guardian_manual() {
        // Validate that the review and guardian checks trivially pass
        // by calling the static helper methods directly.
        let reviewer = Reviewer::new();
        let ownership = sample_ownership();
        let registry = sample_interface_registry();
        let guardian = Guardian::new(ownership, registry);

        let event_bus = Arc::new(EventBus::new());
        let store: Option<Arc<ObjectiveStore>> = None;

        let passed = Coordinator::run_review_check(
            &reviewer,
            &store,
            &Some(event_bus.clone()),
            "obj-manual",
            "worker-manual",
        )
        .await;
        assert!(passed, "review should pass on empty diff");

        let passed = Coordinator::run_guardian_check(
            &guardian,
            &store,
            &Some(event_bus.clone()),
            "obj-manual",
            "worker-manual",
        )
        .await;
        assert!(passed, "guardian should pass on empty diff");
    }

    // ── Helper: create a test OwnershipModel from inline YAML ───────────
    //
    // Mirrors the pattern from guardian.rs tests so Guardian can be
    // constructed in coordinator tests.

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
"#;
        Arc::new(OwnershipModel::from_yaml(yaml).unwrap())
    }

    fn sample_interface_registry() -> Arc<InterfaceRegistry> {
        let mut reg = InterfaceRegistry::new();

        let iface_approval = crate::interface_registry::Interface {
            interface_id: "objectives-api".to_string(),
            kind: crate::interface_registry::InterfaceKind::RestApi,
            owner_domain: "project-kernel".to_string(),
            consumers: vec!["worker-pool".to_string()],
            version: "1.0.0".to_string(),
            signature: "specs/objectives-api.yaml".to_string(),
            compatibility: crate::interface_registry::CompatibilityPolicy {
                breaking_change_policy:
                    crate::interface_registry::BreakingChangePolicy::RequiresApproval,
                deprecated_since: None,
                sunset_date: None,
            },
            history: vec![],
        };
        reg.register(iface_approval).unwrap();

        Arc::new(reg)
    }

    // ── Cross-domain check tests ─────────────────────────────────────

    #[test]
    fn test_cross_domain_single_domain_noop() {
        let ownership = sample_ownership();
        let registry = sample_interface_registry();

        // Single-domain diff: only kernel files.
        let diff = StructuredDiff {
            objective_id: "obj-cd-1".into(),
            worker_id: "worker-1".into(),
            changes: vec![FileChange::Modify {
                path: "kernel/src/main.rs".into(),
                old_content: "old".into(),
                new_content: "new".into(),
            }],
            commit_metadata: CommitMetadata {
                summary: "test".into(),
                objective_id: "obj-cd-1".into(),
                worker_id: "worker-1".into(),
                reviewer_id: None,
                guardian_id: None,
                human_approval_id: None,
            },
        };

        let verdict = Coordinator::check_cross_domain(&ownership, &registry, &diff);
        assert_eq!(verdict, CrossDomainVerdict::SingleDomain);
    }

    #[test]
    fn test_cross_domain_compatible_interfaces() {
        let ownership = sample_ownership();
        let mut reg = InterfaceRegistry::new();

        // Register interfaces with Permitted policy for project-kernel owned interfaces.
        let iface_state_machine = Interface {
            interface_id: "state-machine".to_string(),
            kind: InterfaceKind::InternalModule,
            owner_domain: "project-kernel".to_string(),
            consumers: vec![],
            version: "1.0.0".to_string(),
            signature: "kernel/src/state_machine.rs".to_string(),
            compatibility: CompatibilityPolicy {
                breaking_change_policy: BreakingChangePolicy::AllowedWithDeprecation,
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
        reg.register(iface_state_machine).unwrap();

        let iface_storage = Interface {
            interface_id: "objective-storage".to_string(),
            kind: InterfaceKind::InternalModule,
            owner_domain: "project-kernel".to_string(),
            consumers: vec!["worker-pool".to_string()],
            version: "1.0.0".to_string(),
            signature: "kernel/src/storage.rs".to_string(),
            compatibility: CompatibilityPolicy {
                breaking_change_policy: BreakingChangePolicy::AllowedWithDeprecation,
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
        reg.register(iface_storage).unwrap();

        let registry = Arc::new(reg);

        // Cross-domain diff: kernel + docs files, but owned interfaces have
        // compatible policies (Permitted and AllowedWithDeprecation) so the verdict is compatible.
        let diff = StructuredDiff {
            objective_id: "obj-cd-2".into(),
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
            commit_metadata: CommitMetadata {
                summary: "test".into(),
                objective_id: "obj-cd-2".into(),
                worker_id: "worker-1".into(),
                reviewer_id: None,
                guardian_id: None,
                human_approval_id: None,
            },
        };

        let verdict = Coordinator::check_cross_domain(&ownership, &registry, &diff);
        assert_eq!(verdict, CrossDomainVerdict::CompatibleCrossDomain);
    }

    #[test]
    fn test_cross_domain_breaking_change() {
        let ownership = sample_ownership();
        let mut reg = InterfaceRegistry::new();

        // Register project-kernel's owned interface with RequiresApproval policy.
        let iface_storage = Interface {
            interface_id: "objective-storage".to_string(),
            kind: InterfaceKind::InternalModule,
            owner_domain: "project-kernel".to_string(),
            consumers: vec!["worker-pool".to_string()],
            version: "1.0.0".to_string(),
            signature: "kernel/src/storage.rs".to_string(),
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
        reg.register(iface_storage).unwrap();

        let registry = Arc::new(reg);

        // Cross-domain diff: kernel + docs files.  project-kernel owns
        // "objective-storage" which has RequiresApproval policy,
        // so the verdict is RequiresHumanApproval.
        let diff = StructuredDiff {
            objective_id: "obj-cd-3".into(),
            worker_id: "worker-1".into(),
            changes: vec![
                FileChange::Modify {
                    path: "kernel/src/storage.rs".into(),
                    old_content: "old".into(),
                    new_content: "new".into(),
                },
                FileChange::Modify {
                    path: "docs/architecture.md".into(),
                    old_content: "old".into(),
                    new_content: "new".into(),
                },
            ],
            commit_metadata: CommitMetadata {
                summary: "test".into(),
                objective_id: "obj-cd-3".into(),
                worker_id: "worker-1".into(),
                reviewer_id: None,
                guardian_id: None,
                human_approval_id: None,
            },
        };

        let verdict = Coordinator::check_cross_domain(&ownership, &registry, &diff);
        assert_eq!(
            verdict,
            CrossDomainVerdict::RequiresHumanApproval(vec!["objective-storage".into()])
        );
    }

    #[test]
    fn test_cross_domain_empty_diff() {
        let ownership = sample_ownership();
        let registry = sample_interface_registry();

        // Empty diff — no files touched, should be SingleDomain.
        let diff = StructuredDiff {
            objective_id: "obj-cd-4".into(),
            worker_id: "worker-1".into(),
            changes: vec![],
            commit_metadata: CommitMetadata {
                summary: "test".into(),
                objective_id: "obj-cd-4".into(),
                worker_id: "worker-1".into(),
                reviewer_id: None,
                guardian_id: None,
                human_approval_id: None,
            },
        };

        let verdict = Coordinator::check_cross_domain(&ownership, &registry, &diff);
        assert_eq!(verdict, CrossDomainVerdict::SingleDomain);
    }
}
