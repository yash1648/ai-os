// AI-OS Kernel — Scheduler
//
// The Scheduler decides which READY objectives to dispatch to workers,
// subject to dependency ordering, priority, and configured concurrency
// limits. (docs/15-scheduler.md)
//
// Stage 1: single-worker FIFO queue with priority ordering.
// Stage 2+: lock management, per-domain concurrency, event-driven dispatch.

use chrono::{DateTime, Utc};
use metrics::{counter, gauge, describe_counter, describe_gauge};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

use crate::config::SchedulerConfig;
use crate::event_bus::{Actor, ActorKind, Event, EventBus, EventKind};

// ---------------------------------------------------------------------------
// Entry — an objective waiting to be scheduled
// ---------------------------------------------------------------------------

/// An entry in the scheduler's ready queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerEntry {
    /// Objective identifier.
    pub objective_id: String,
    /// Objective priority — higher-priority entries dispatch first.
    pub priority: Priority,
    /// When this objective became READY (used as age tiebreak).
    pub ready_at: DateTime<Utc>,
    /// Number of retry attempts so far.
    pub retry_count: u32,
}

/// Scheduling priority — mirrors `objective::Priority` without a direct dep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Minimal = 0,
    Low = 1,
    Medium = 2,
    High = 3,
    Critical = 4,
}

impl From<crate::objective::Priority> for Priority {
    fn from(p: crate::objective::Priority) -> Self {
        match p {
            crate::objective::Priority::Minimal => Self::Minimal,
            crate::objective::Priority::Low => Self::Low,
            crate::objective::Priority::Medium => Self::Medium,
            crate::objective::Priority::High => Self::High,
            crate::objective::Priority::Critical => Self::Critical,
        }
    }
}

impl PartialOrd for SchedulerEntry {
    /// Entries are ordered by (priority DESC, ready_at ASC) — highest priority
    /// first, oldest-ready first as tiebreak.
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SchedulerEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reverse priority order (higher priority = earlier dispatch)
        other
            .priority
            .cmp(&self.priority)
            .then_with(|| self.ready_at.cmp(&other.ready_at))
    }
}

impl PartialEq for SchedulerEntry {
    fn eq(&self, other: &Self) -> bool {
        self.objective_id == other.objective_id
    }
}

impl Eq for SchedulerEntry {}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

/// The Kernel Scheduler — decides which objective runs next.
///
/// Stage 1 implements a FIFO queue with priority ordering. The kernel calls
/// `notify_objective_ready()` when an objective becomes READY, then calls
/// `try_dispatch()` to obtain the next objective to execute. When the worker
/// finishes, the kernel calls `notify_worker_finished()` to free the slot.
///
/// ## Ordering (docs/15-scheduler.md §Scheduling Algorithm)
///
/// 1. Eligible objectives (dependencies DONE) are enqueued via
///    `notify_objective_ready`.
/// 2. The queue is ordered by priority (Critical → Minimal), then by age
///    (oldest-ready first) as tiebreak.
/// 3. `try_dispatch` dequeues up to `max_concurrent_objectives`.
///
/// ## Concurrency Growth (docs/15-scheduler.md §Concurrency Growth Path)
///
/// - **Stage 1**: single worker — simple FIFO queue (this implementation).
/// - **Stage 2**: multiple domain-specialized workers gated by lock manager.
/// - **Stage 3+**: event-driven dispatch triggered by bus events.
#[derive(Debug)]
pub struct Scheduler {
    /// Configuration — concurrency limits, retry bounds.
    config: SchedulerConfig,
    /// Optional event bus for emitting scheduling events.
    event_bus: Option<EventBus>,
    /// Queue of ready-to-dispatch entries, ordered by priority then age.
    ready_queue: VecDeque<SchedulerEntry>,
    /// Number of objectives currently dispatched and executing.
    active_count: usize,
    /// Dispatched count — used for observable metrics.
    total_dispatched: u64,
    /// Throttling counter — number of times dispatch was skipped due to limits.
    throttle_count: u64,
}

impl Scheduler {
    /// Create a new scheduler from configuration.
    pub fn new(config: SchedulerConfig) -> Self {
        Self {
            config,
            event_bus: None,
            ready_queue: VecDeque::new(),
            active_count: 0,
            total_dispatched: 0,
            throttle_count: 0,
        }
    }

    /// Attach an event bus for emitting scheduling events.
    pub fn with_event_bus(mut self, bus: EventBus) -> Self {
        self.event_bus = Some(bus);
        self
    }

    /// Maximum concurrent objectives allowed.
    pub fn max_concurrent(&self) -> usize {
        self.config.max_concurrent_objectives
    }

    /// Maximum retries before abandoning.
    pub fn max_retries(&self) -> u32 {
        self.config.max_retries
    }

    /// Number of entries waiting in the ready queue.
    pub fn queue_len(&self) -> usize {
        let len = self.ready_queue.len();
        gauge!("ai_os_scheduler_queue_len").set(len as f64);
        len
    }

    /// Number of objectives currently executing.
    pub fn active_count(&self) -> usize {
        self.active_count
    }

    /// Total objectives dispatched since creation.
    pub fn total_dispatched(&self) -> u64 {
        self.total_dispatched
    }

    /// Number of times dispatch was throttled.
    pub fn throttle_count(&self) -> u64 {
        self.throttle_count
    }

    /// Whether any slot is available for dispatch.
    pub fn can_dispatch(&self) -> bool {
        self.active_count < self.config.max_concurrent_objectives
    }

    /// Whether the ready queue is empty.
    pub fn is_queue_empty(&self) -> bool {
        self.ready_queue.is_empty()
    }

    /// Notify the scheduler that an objective has transitioned to READY
    /// and should be considered for dispatch.
    ///
    /// The objective is inserted into the ready queue in priority order.
    /// Returns the position in queue (0 = next to dispatch).
    pub fn notify_objective_ready(
        &mut self,
        objective_id: &str,
        priority: Priority,
        ready_at: DateTime<Utc>,
        retry_count: u32,
    ) -> usize {
        let entry = SchedulerEntry {
            objective_id: objective_id.to_string(),
            priority,
            ready_at,
            retry_count,
        };

        // Insert in priority order: find the first position where the
        // new entry should go (lower priority or same-but-younger).
        // When an equal-priority, equal-age entry already exists, we insert
        // AFTER it (FIFO within same priority level).  To achieve that, we
        // tell binary_search that the probe is Less than the target, which
        // makes the search continue rightward and return Err(position after
        // the last equal entry).
        let pos = self.ready_queue.binary_search_by(|e| {
            let ord = e.cmp(&entry);
            if ord.is_eq() {
                std::cmp::Ordering::Less
            } else {
                ord
            }
        }).unwrap_or_else(|i| i);

        self.ready_queue.insert(pos, entry);
        pos
    }

    /// Attempt to dispatch the next eligible objective.
    ///
    /// Returns `Some(objective_id)` if an objective was dispatched,
    /// or `None` if the queue is empty or concurrency limit is reached.
    ///
    /// When the limit is reached, emits a `SchedulingThrottled` event
    /// if an event bus is configured.
    pub fn try_dispatch(&mut self) -> Option<String> {
        if !self.can_dispatch() {
            self.throttle_count += 1;
            describe_counter!("ai_os_scheduler_throttle_count", "Number of times dispatch was throttled due to concurrency limits");
            counter!("ai_os_scheduler_throttle_count").increment(1);
            self.emit_event(
                EventKind::SchedulingThrottled,
                serde_json::json!({
                    "active_count": self.active_count,
                    "max_concurrent": self.config.max_concurrent_objectives,
                    "queue_len": self.ready_queue.len(),
                    "reason": "max_concurrent_reached",
                }),
            );
            return None;
        }

        let entry = self.ready_queue.pop_front()?;

        self.active_count += 1;
        self.total_dispatched += 1;

        describe_counter!("ai_os_scheduler_dispatch_count", "Total number of objectives dispatched");
        describe_gauge!("ai_os_scheduler_queue_len", "Current number of objectives waiting in the ready queue");
        counter!("ai_os_scheduler_dispatch_count").increment(1);

        self.emit_event(
            EventKind::DispatchDecision,
            serde_json::json!({
                "objective_id": &entry.objective_id,
                "priority": serde_json::to_value(&entry.priority).unwrap_or_default(),
                "retry_count": entry.retry_count,
                "active_count": self.active_count,
                "queue_remaining": self.ready_queue.len(),
            }),
        );

        tracing::info!(
            objective = %entry.objective_id,
            priority = ?entry.priority,
            active = self.active_count,
            remaining = self.ready_queue.len(),
            "Dispatched objective"
        );

        Some(entry.objective_id)
    }

    /// Notify the scheduler that a worker has finished executing an objective,
    /// freeing a dispatch slot.
    pub fn notify_worker_finished(&mut self, objective_id: &str) {
        if self.active_count == 0 {
            tracing::warn!(
                "notify_worker_finished called for {objective_id} but active_count is 0"
            );
            return;
        }

        self.active_count = self.active_count.saturating_sub(1);

        tracing::info!(
            objective = %objective_id,
            active = self.active_count,
            "Worker finished, slot freed"
        );
    }

    /// Return all entries currently in the ready queue (for inspection).
    pub fn peek_queue(&self) -> Vec<&SchedulerEntry> {
        self.ready_queue.iter().collect()
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn emit_event(&self, kind: EventKind, payload: serde_json::Value) {
        if let Some(ref bus) = self.event_bus {
            let event = Event::new(
                kind,
                Actor {
                    kind: ActorKind::Scheduler,
                    id: "scheduler".into(),
                },
                payload,
            );
            bus.publish(event);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    fn scheduler_config() -> SchedulerConfig {
        SchedulerConfig {
            // Use a generous limit so ordering tests don't hit concurrency caps.
            max_concurrent_objectives: 10,
            max_retries: 3,
        }
    }

    fn entry(
        id: &str,
        priority: Priority,
        ready_at: DateTime<Utc>,
        retry_count: u32,
    ) -> SchedulerEntry {
        SchedulerEntry {
            objective_id: id.to_string(),
            priority,
            ready_at,
            retry_count,
        }
    }

    // ── Queue ordering ──────────────────────────────────────────────────

    #[test]
    fn queue_orders_by_priority() {
        let mut s = Scheduler::new(scheduler_config());
        let now = Utc::now();

        s.notify_objective_ready("low", Priority::Low, now, 0);
        s.notify_objective_ready("high", Priority::High, now, 0);
        s.notify_objective_ready("critical", Priority::Critical, now, 0);

        // Should dispatch Critical → High → Low
        assert_eq!(s.try_dispatch().as_deref(), Some("critical"));
        assert_eq!(s.try_dispatch().as_deref(), Some("high"));
        assert_eq!(s.try_dispatch().as_deref(), Some("low"));
        assert_eq!(s.try_dispatch(), None);
    }

    #[test]
    fn queue_uses_age_as_tiebreaker() {
        let mut s = Scheduler::new(scheduler_config());
        let now = Utc::now();

        // Same priority, different ages — older should go first
        s.notify_objective_ready("young", Priority::Medium, now, 0);
        s.notify_objective_ready("old", Priority::Medium, now - Duration::from_secs(60), 0);

        assert_eq!(s.try_dispatch().as_deref(), Some("old"));
        assert_eq!(s.try_dispatch().as_deref(), Some("young"));
    }

    #[test]
    fn queue_mixed_priority_and_age() {
        let mut s = Scheduler::new(scheduler_config());
        let now = Utc::now();

        // High priority younger vs Medium priority older
        s.notify_objective_ready("medium_old", Priority::Medium, now - Duration::from_secs(120), 0);
        s.notify_objective_ready("high_young", Priority::High, now, 0);

        // Priority wins over age
        assert_eq!(s.try_dispatch().as_deref(), Some("high_young"));
        assert_eq!(s.try_dispatch().as_deref(), Some("medium_old"));
    }

    // ── Concurrency limits ──────────────────────────────────────────────

    #[test]
    fn respects_max_concurrent() {
        let cfg = SchedulerConfig {
            max_concurrent_objectives: 2,
            max_retries: 3,
        };
        let mut s = Scheduler::new(cfg);
        let now = Utc::now();

        s.notify_objective_ready("a", Priority::High, now, 0);
        s.notify_objective_ready("b", Priority::High, now, 0);
        s.notify_objective_ready("c", Priority::High, now, 0);

        // Two should dispatch, third should be throttled
        assert_eq!(s.active_count(), 0);
        assert_eq!(s.try_dispatch().as_deref(), Some("a"));
        assert_eq!(s.active_count(), 1);
        assert_eq!(s.try_dispatch().as_deref(), Some("b"));
        assert_eq!(s.active_count(), 2);
        assert_eq!(s.try_dispatch(), None);
        assert_eq!(s.throttle_count(), 1);

        // After one finishes, next can dispatch
        s.notify_worker_finished("a");
        assert_eq!(s.active_count(), 1);
        assert_eq!(s.try_dispatch().as_deref(), Some("c"));
    }

    #[test]
    fn dispatch_none_when_queue_empty() {
        let mut s = Scheduler::new(scheduler_config());
        assert_eq!(s.try_dispatch(), None);
    }

    // ── Worker finish tracking ──────────────────────────────────────────

    #[test]
    fn worker_finished_decrements_active_count() {
        let mut s = Scheduler::new(scheduler_config());

        s.active_count = 2;
        s.notify_worker_finished("obj-1");
        assert_eq!(s.active_count, 1);
    }

    #[test]
    fn worker_finished_idempotent_at_zero() {
        let mut s = Scheduler::new(scheduler_config());
        s.notify_worker_finished("nonexistent");
        assert_eq!(s.active_count, 0);
    }

    // ── Priority ordering (ord) ─────────────────────────────────────────

    #[test]
    fn entry_ordering() {
        let now = Utc::now();

        // Higher priority should sort first
        let high = entry("h", Priority::High, now, 0);
        let low = entry("l", Priority::Low, now, 0);
        assert!(high.cmp(&low) == std::cmp::Ordering::Less);

        // Same priority, older should sort first
        let old = entry("o", Priority::Medium, now - Duration::from_secs(10), 0);
        let young = entry("y", Priority::Medium, now, 0);
        assert!(old.cmp(&young) == std::cmp::Ordering::Less);
    }

    // ── Priority conversion ─────────────────────────────────────────────

    #[test]
    fn priority_from_objective() {
        use crate::objective::Priority as ObjPri;

        assert_eq!(Priority::from(ObjPri::Minimal), Priority::Minimal);
        assert_eq!(Priority::from(ObjPri::Low), Priority::Low);
        assert_eq!(Priority::from(ObjPri::Medium), Priority::Medium);
        assert_eq!(Priority::from(ObjPri::High), Priority::High);
        assert_eq!(Priority::from(ObjPri::Critical), Priority::Critical);
    }

    // ── Query methods ───────────────────────────────────────────────────

    #[test]
    fn query_methods() {
        let s = Scheduler::new(scheduler_config());
        assert_eq!(s.max_concurrent(), 10);
        assert_eq!(s.max_retries(), 3);
        assert_eq!(s.queue_len(), 0);
        assert_eq!(s.active_count(), 0);
        assert!(s.is_queue_empty());
        assert!(s.can_dispatch());
    }

    #[test]
    fn total_dispatched_increments() {
        let mut s = Scheduler::new(scheduler_config());
        let now = Utc::now();

        s.notify_objective_ready("a", Priority::Medium, now, 0);
        s.notify_objective_ready("b", Priority::Medium, now, 0);

        assert_eq!(s.total_dispatched(), 0);
        s.try_dispatch();
        assert_eq!(s.total_dispatched(), 1);
        s.try_dispatch();
        assert_eq!(s.total_dispatched(), 2);
        s.try_dispatch(); // throttled
        assert_eq!(s.total_dispatched(), 2); // no change
    }

    // ── peek_queue ──────────────────────────────────────────────────────

    #[test]
    fn peek_returns_ordered_entries() {
        let mut s = Scheduler::new(scheduler_config());
        let now = Utc::now();

        s.notify_objective_ready("b", Priority::Low, now, 0);
        s.notify_objective_ready("a", Priority::High, now, 0);

        let entries = s.peek_queue();
        assert_eq!(entries.len(), 2);
        // Should be ordered: High first, then Low
        assert_eq!(entries[0].objective_id, "a");
        assert_eq!(entries[1].objective_id, "b");
    }

    // ── SchedulerEntry derives ─────────────────────────────────────────────

    #[test]
    fn scheduler_entry_debug_and_clone() {
        let now = Utc::now();
        let e = entry("test", Priority::High, now, 0);
        let _ = format!("{e:?}"); // Debug
        let _ = e.clone(); // Clone
    }

    // ── Event emission (integration with EventBus) ──────────────────────

    #[tokio::test]
    async fn emits_worker_started_on_dispatch() {
        let bus = crate::event_bus::EventBus::new();
        let mut rx = bus.subscribe();

        let mut s = Scheduler::new(scheduler_config()).with_event_bus(bus);
        let now = Utc::now();

        s.notify_objective_ready("obj-1", Priority::Critical, now, 0);
        let dispatched = s.try_dispatch();
        assert_eq!(dispatched.as_deref(), Some("obj-1"));

        // Allow async event dispatch
        tokio::time::sleep(Duration::from_millis(10)).await;

        match rx.try_recv() {
            Ok(event) => {
                assert!(matches!(event.kind, EventKind::DispatchDecision));
                assert_eq!(
                    event.payload.get("objective_id").and_then(|v| v.as_str()),
                    Some("obj-1")
                );
                assert_eq!(event.actor.kind, ActorKind::Scheduler);
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
                panic!("Expected WorkerStarted event but channel was empty");
            }
            Err(e) => panic!("Unexpected error: {e}"),
        }
    }

    #[tokio::test]
    async fn emits_throttled_on_limit() {
        let bus = crate::event_bus::EventBus::new();
        let mut rx = bus.subscribe();

        let cfg = SchedulerConfig {
            max_concurrent_objectives: 1,
            max_retries: 3,
        };
        let mut s = Scheduler::new(cfg).with_event_bus(bus);
        let now = Utc::now();

        s.notify_objective_ready("a", Priority::High, now, 0);
        s.notify_objective_ready("b", Priority::High, now, 0);

        s.try_dispatch(); // dispatches 'a'
        s.try_dispatch(); // throttled for 'b'

        tokio::time::sleep(Duration::from_millis(10)).await;

        // First event should be DispatchDecision, second is SchedulingThrottled
        let ev1 = rx.try_recv().expect("Expected DispatchDecision");
        assert!(matches!(ev1.kind, EventKind::DispatchDecision));

        let ev2 = rx.try_recv().expect("Expected SchedulingThrottled");
        assert!(matches!(ev2.kind, EventKind::SchedulingThrottled));
        assert_eq!(
            ev2.payload.get("reason").and_then(|v| v.as_str()),
            Some("max_concurrent_reached")
        );
    }

    #[test]
    fn no_event_without_bus() {
        // Without an event bus attached, dispatch should still work silently.
        let mut s = Scheduler::new(scheduler_config());
        let now = Utc::now();

        s.notify_objective_ready("obj-1", Priority::Medium, now, 0);
        assert_eq!(s.try_dispatch().as_deref(), Some("obj-1"));
    }
}
