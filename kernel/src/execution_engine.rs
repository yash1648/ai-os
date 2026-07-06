// AI-OS Kernel — Execution Engine
//
// Stage 1: tokio async tasks (Rust)
// Stage 2: gRPC calls to Python workers
// Stage 3: pool of persistent worker processes

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
pub(crate) use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::event_bus::{Actor, ActorKind, Event, EventBus, EventKind};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Worker configuration
// ---------------------------------------------------------------------------

/// Configuration for worker execution behaviour.
///
/// Stage 1 uses simulated workers with configurable delay.
/// Stage 2+ will route to gRPC Python workers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerConfig {
    /// How long (in ms) the worker simulates work before returning.
    /// Set to 0 for instantaneous completion (useful in tests).
    pub simulation_delay_ms: u64,

    /// Optional list of objective IDs that should simulate failure.
    /// Workers for objectives NOT in this list will complete successfully.
    pub fail_objective_ids: Vec<String>,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            simulation_delay_ms: 0,
            fail_objective_ids: vec![],
        }
    }
}

/// Run a simulated worker task.
///
/// Sleeps for `config.simulation_delay_ms`, then returns a `WorkerResult`
/// with the configured status (Completed or Failed), the actual duration,
/// and starter metrics.
pub async fn run_simulated_worker(
    objective_id: &str,
    config: &WorkerConfig,
) -> WorkerResult {
    let start = std::time::Instant::now();

    if config.simulation_delay_ms > 0 {
        tokio::time::sleep(tokio::time::Duration::from_millis(config.simulation_delay_ms)).await;
    }

    let elapsed = start.elapsed().as_millis() as u64;

    let failed = config.fail_objective_ids.iter().any(|id| id == objective_id);
    let status = if failed {
        WorkerStatus::Failed("Simulated failure per WorkerConfig".into())
    } else {
        WorkerStatus::Completed
    };

    WorkerResult {
        objective_id: objective_id.to_string(),
        status,
        metrics: WorkerMetrics {
            duration_ms: elapsed,
            tokens_used: None,
            files_changed: 0,
        },
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum WorkerStatus {
    Running,
    Completed,
    Failed(String),
    Cancelled,
}

impl WorkerStatus {
    pub fn label(&self) -> &'static str {
        match self {
            WorkerStatus::Running => "running",
            WorkerStatus::Completed => "completed",
            WorkerStatus::Failed(_) => "failed",
            WorkerStatus::Cancelled => "cancelled",
        }
    }
}

pub struct WorkerHandle {
    pub objective_id: String,
    pub worker_id: String,
    pub status: WorkerStatus,
    pub(crate) handle: JoinHandle<WorkerResult>,
}

#[derive(Debug, Clone)]
pub struct WorkerResult {
    pub objective_id: String,
    pub status: WorkerStatus,
    pub metrics: WorkerMetrics,
}

#[derive(Debug, Clone, Default)]
pub struct WorkerMetrics {
    pub duration_ms: u64,
    pub tokens_used: Option<u64>,
    pub files_changed: usize,
}

// ---------------------------------------------------------------------------
// Worker pool
// ---------------------------------------------------------------------------

pub struct WorkerPool {
    max_concurrent: usize,
    active: HashMap<String, WorkerHandle>,
    _completed: Vec<WorkerResult>,
    total_spawned: AtomicUsize,
    event_bus: Option<Arc<EventBus>>,
}

impl WorkerPool {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            max_concurrent,
            active: HashMap::new(),
            _completed: Vec::new(),
            total_spawned: AtomicUsize::new(0),
            event_bus: None,
        }
    }

    pub fn with_event_bus(mut self, bus: Arc<EventBus>) -> Self {
        self.event_bus = Some(bus);
        self
    }

    pub fn can_accept(&self) -> bool {
        self.active.len() < self.max_concurrent
    }

    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    pub fn total_spawned(&self) -> usize {
        self.total_spawned.load(Ordering::SeqCst)
    }

    pub fn spawn<F>(&mut self, objective_id: &str, worker_fn: F) -> Option<String>
    where
        F: std::future::Future<Output = WorkerResult> + Send + 'static,
    {
        if !self.can_accept() {
            return None;
        }

        let worker_id = format!(
            "worker-{}-{}",
            self.total_spawned.fetch_add(1, Ordering::SeqCst),
            Uuid::new_v4().to_string().split_at(8).0
        );

        let wid = worker_id.clone();

        let handle = tokio::spawn(worker_fn);

        self.active.insert(
            objective_id.to_string(),
            WorkerHandle {
                objective_id: objective_id.to_string(),
                worker_id: wid,
                status: WorkerStatus::Running,
                handle,
            },
        );

        self.emit_event(EventKind::WorkerStarted, objective_id, Some(&worker_id));

        Some(worker_id)
    }

    /// Remove and return the JoinHandle for a running worker by objective_id.
    /// The caller takes ownership of the handle and must await it to collect
    /// the worker's completion.
    pub fn take_handle(&mut self, objective_id: &str) -> Option<JoinHandle<WorkerResult>> {
        self.active.remove(objective_id).map(|h| h.handle)
    }

    pub fn cancel(&mut self, objective_id: &str) -> bool {
        if let Some(handle) = self.active.remove(objective_id) {
            handle.handle.abort();
            self.emit_event(
                EventKind::WorkerFinished,
                objective_id,
                Some(&handle.worker_id),
            );
            true
        } else {
            false
        }
    }

    fn emit_event(&self, kind: EventKind, objective_id: &str, worker_id: Option<&str>) {
        if let Some(bus) = &self.event_bus {
            let event = Event::new(
                kind,
                Actor {
                    kind: ActorKind::Kernel,
                    id: "execution-engine".into(),
                },
                serde_json::json!({
                    "objective_id": objective_id,
                    "worker_id": worker_id,
                    "pool_status": {
                        "active": self.active.len(),
                        "max": self.max_concurrent,
                    }
                }),
            );
            bus.publish(event);
        }
    }
}
