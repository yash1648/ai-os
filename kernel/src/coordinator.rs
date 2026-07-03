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
use crate::execution_engine::{WorkerPool, WorkerStatus, WorkerResult, WorkerMetrics};

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
}

/// Factory for creating a Coordinator.
impl Coordinator {
    /// Create a new Coordinator with a fresh WorkerPool.
    pub fn new() -> Self {
        Self {
            worker_pool: WorkerPool::new(4), // Default max 4 concurrent workers
            dispatch_count: AtomicUsize::new(0),
            event_bus: None,
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
