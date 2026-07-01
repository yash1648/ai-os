use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Event types — matching docs/08-event-bus.md
// ---------------------------------------------------------------------------

/// All event kinds the Kernel emits.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EventKind {
    ObjectiveCreated,
    PlanGenerated,
    PlanApproved,
    WorkspaceLocked,
    WorkerStarted,
    WorkerFinished,
    DiffGenerated,
    ReviewPassed,
    ReviewFailed,
    GuardianPassed,
    GuardianFailed,
    IntegrationStarted,
    MergeCompleted,
    ObjectiveCompleted,
    RollbackStarted,
    RollbackCompleted,
    HumanApprovalRequested,
    HumanApprovalGranted,
    HumanApprovalDenied,
    PermissionDenied,
    SchedulingThrottled,
    StateTransitioned,
    ConstitutionAmended,
}

/// The canonical event envelope — docs/08-event-bus.md §Event Schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub event_id: Uuid,
    #[serde(flatten)]
    pub kind: EventKind,
    pub timestamp: DateTime<Utc>,
    pub objective_id: Option<String>,
    pub plan_id: Option<String>,
    pub actor: Actor,
    pub payload: serde_json::Value,
    pub causation_id: Option<Uuid>,
    pub correlation_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Actor {
    #[serde(rename = "kind")]
    pub kind: ActorKind,
    #[serde(rename = "id")]
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Kernel,
    Worker,
    Reviewer,
    Guardian,
    Human,
    Scheduler,
}

impl Event {
    pub fn new(kind: EventKind, actor: Actor, payload: serde_json::Value) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            kind,
            timestamp: Utc::now(),
            objective_id: None,
            plan_id: None,
            actor,
            payload,
            causation_id: None,
            correlation_id: None,
        }
    }

    pub fn with_objective(mut self, id: &str) -> Self {
        self.objective_id = Some(id.to_string());
        self
    }

    pub fn with_plan(mut self, id: &str) -> Self {
        self.plan_id = Some(id.to_string());
        self
    }

    pub fn with_causation(mut self, id: Uuid) -> Self {
        self.causation_id = Some(id);
        self
    }

    pub fn with_correlation(mut self, id: Uuid) -> Self {
        self.correlation_id = Some(id);
        self
    }
}

// ---------------------------------------------------------------------------
// Event Bus — Stage 1 in-process implementation
// ---------------------------------------------------------------------------

/// Capacity of the broadcast channel (events are ephemeral in Stage 1 —
/// subscribers must keep up or miss events. In Stage 3+, a durable event
/// store (Redis Streams / NATS) replaces this.)
const BUS_CAPACITY: usize = 4096;

/// The in-process event bus. Components subscribe to receive events and
/// publish to notify all subscribers.
///
/// ## Ordering Guarantees
/// - Total ordering per objective (events with same `objective_id` appear in
///   publish order on all subscribers).
/// - No global ordering guarantee across objectives.
///
/// ## Delivery
/// - At-least-once to each subscriber. Slow subscribers that fall behind
///   the buffer capacity will miss events (`RecvError::Lagged`).
#[derive(Debug, Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Event>,
}

impl EventBus {
    /// Create a new event bus with the default channel capacity.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(BUS_CAPACITY);
        Self { tx }
    }

    /// Create a new event bus with a custom channel capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Publish an event to all subscribers.
    pub fn publish(&self, event: Event) {
        // If no subscribers are active, the send "fails" — that's fine.
        let _ = self.tx.send(event);
    }

    /// Subscribe to receive events. Returns a receiver that yields events
    /// published after this subscription was created.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    /// Returns the number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast::error::TryRecvError;

    #[tokio::test]
    async fn publish_and_receive() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        let event = Event::new(
            EventKind::ObjectiveCreated,
            Actor {
                kind: ActorKind::Kernel,
                id: "test".into(),
            },
            serde_json::json!({"objective_id": "obj-001"}),
        )
        .with_objective("obj-001");

        bus.publish(event);

        // Should receive it (allow a tiny delay for async dispatch)
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        match rx.try_recv() {
            Ok(e) => assert_eq!(e.objective_id, Some("obj-001".into())),
            Err(TryRecvError::Empty) => panic!("Expected event but channel was empty"),
            Err(e) => panic!("Unexpected error: {e}"),
        }
    }

    #[tokio::test]
    async fn multiple_subscribers() {
        let bus = EventBus::new();
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        bus.publish(Event::new(
            EventKind::ObjectiveCreated,
            Actor { kind: ActorKind::Kernel, id: "test".into() },
            serde_json::json!({}),
        ));

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_ok());
    }

    #[test]
    fn subscriber_count() {
        let bus = EventBus::new();
        assert_eq!(bus.subscriber_count(), 0);
        let _rx = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 1);
        let _rx2 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 2);
    }
}
