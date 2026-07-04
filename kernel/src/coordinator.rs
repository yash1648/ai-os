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

use crate::event_bus::{Actor, ActorKind, Event, EventBus, EventKind};
use crate::execution_engine::{WorkerMetrics, WorkerPool, WorkerResult, WorkerStatus};
use crate::objective::ObjectiveStore;
use crate::scheduler::Scheduler;
use crate::state_machine::{self, ObjectiveState, RetryPolicy};

// ---------------------------------------------------------------------------
// Coordinator — orchestrates scheduler dispatch → execution engine
// ---------------------------------------------------------------------------

/// Coordinates the flow from dispatch decision to worker execution.
///
/// Listens for DispatchDecision events from the Scheduler, forwards to
/// the ExecutionEngine for actual worker spawn, and manages the lifecycle.
pub struct Coordinator {
    /// The execution engine pool for spawning workers.
    pub worker_pool: WorkerPool,
    /// Total dispatches processed by this coordinator.
    dispatch_count: AtomicUsize,
    /// Optional event bus for publishing lifecycle events.
    event_bus: Option<Arc<EventBus>>,
    /// Reference to the objective store for status transitions on worker completion.
    objective_store: Option<Arc<ObjectiveStore>>,
    /// Reference to the scheduler for freeing dispatch slots on worker completion.
    scheduler: Option<Arc<tokio::sync::Mutex<Scheduler>>>,
}

/// Factory for creating a Coordinator.
impl Coordinator {
    /// Create a new Coordinator with a fresh WorkerPool.
    pub fn new() -> Self {
        Self {
            worker_pool: WorkerPool::new(4), // Default max 4 concurrent workers
            dispatch_count: AtomicUsize::new(0),
            event_bus: None,
            objective_store: None,
            scheduler: None,
        }
    }

    /// Set the maximum concurrent workers in the pool.
    pub fn with_max_concurrent(mut self, max: usize) -> Self {
        self.worker_pool = WorkerPool::new(max);
        self
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

/// Process a dispatch decision — spawn a worker if capacity allows.
    ///
    /// # Arguments
    /// * `objective_id` — The objective chosen by the scheduler for dispatch.
    pub fn dispatch(&mut self, objective_id: &str) -> Option<String> {
        if !self.worker_pool.can_accept() {
            self.emit_pool_full(objective_id);
            return None;
        }

        self.dispatch_count.fetch_add(1, Ordering::SeqCst);

        // Clone the objective_id for the async closure
        let objective_id_owned = objective_id.to_string();
        
// Forward to execution engine to actually spawn the worker
        let worker_id = self.worker_pool.spawn(&objective_id_owned.clone(), async move {
            // Stage 1: placeholder worker — just completes immediately
            // Stage 2: call out to Python worker via gRPC
            
            WorkerResult {
                objective_id: objective_id_owned.clone(),
                status: WorkerStatus::Completed,
                metrics: WorkerMetrics::default(),
            }
        });

        worker_id
    }

    /// Query the number of active workers.
    pub fn active_count(&self) -> usize {
        self.worker_pool.active_count()
    }

    /// Query total dispatches processed.
    pub fn dispatch_count(&self) -> usize {
        self.dispatch_count.load(Ordering::SeqCst)
    }

    /// Dispatch an objective and monitor its worker for completion.
    ///
    /// Spawns the worker via the execution engine and sets up a background
    /// task that transitions the objective through the state machine on
    /// completion: EXECUTING -> REVIEW -> INTEGRATION -> DONE.
    /// On success, frees the scheduler slot via notify_worker_finished.
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

        // Spawn the worker — Stage 1 stub completes immediately
        let worker_id_out = objective_id_owned.clone();
        let worker_id = self.worker_pool.spawn(&objective_id_owned, async move {
            // Stage 1: placeholder worker — just completes immediately
            // Stage 2: call out to Python worker via gRPC
            WorkerResult {
                objective_id: worker_id_out,
                status: WorkerStatus::Completed,
                metrics: WorkerMetrics::default(),
            }
        })?;

        // Take the JoinHandle to monitor completion
        let handle = self.worker_pool.take_handle(objective_id);
        let store = self.objective_store.clone();
        let sched = self.scheduler.clone();

        if let Some(handle) = handle {
            tokio::spawn(async move {
                let _ = handle.await;

                // Worker completed — advance through the state machine.
                // Stage 1 uses the full primary path; a real review pipeline
                // would interject here.
                let id = &objective_id_owned;
                let policy = RetryPolicy::default();

                let transitions: &[(ObjectiveState, ObjectiveState)] = &[
                    (ObjectiveState::from_label("EXECUTING"),
                     ObjectiveState::from_label("REVIEW")),
                    (ObjectiveState::from_label("REVIEW"),
                     ObjectiveState::from_label("INTEGRATION")),
                    (ObjectiveState::from_label("INTEGRATION"),
                     ObjectiveState::Terminal(state_machine::ObjectiveTerminalState::Done)),
                ];

                for (current, target) in transitions {
                    if state_machine::transition(*current, *target, &policy, 0).is_ok() {
                        if let Some(ref store) = store {
                            let _ = store.update_status(id, target, 0).await;
                        }
                    }
                }

                // Free the scheduler slot
                if let Some(ref sched) = sched {
                    let mut s = sched.lock().await;
                    s.notify_worker_finished(id);
                }
            });
        }

        Some(worker_id)
    }

    fn emit_pool_full(&self, objective_id: &str) {
        if let Some(bus) = &self.event_bus {
            let event = Event::new(
                EventKind::SchedulingThrottled,
                Actor {
                    kind: ActorKind::Kernel,
                    id: "coordinator".into(),
                },
                serde_json::json!({
                    "objective_id": objective_id,
                    "reason": "worker_pool_full",
                    "active_workers": self.worker_pool.active_count(),
                    "max_concurrent": self.worker_pool.max_concurrent(),
                }),
            );
            bus.publish(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
