// AI-OS Kernel — Execution Engine
//
// Stage 1: tokio async tasks (Rust)
// Stage 2: gRPC calls to Python workers
// Stage 3: pool of persistent worker processes

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::event_bus::{Actor, ActorKind, Event, EventBus, EventKind};

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
    pub(crate) handle: JoinHandle<()>,
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

        let _obj_id = objective_id.to_string();
        let wid = worker_id.clone();

        let handle = tokio::spawn(async move {
            let _result = worker_fn.await;
        });

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
