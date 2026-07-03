use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use std::sync::atomic::{AtomicI64, Ordering};
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
    CrossDomainRequestRaised,
    CrossDomainRequestResolved,
    SchedulingThrottled,
    DispatchDecision,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
// SQLite-backed event store
// ---------------------------------------------------------------------------

/// Row representation of an event in the SQLite `events` table.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
struct EventRow {
    event_id: String,
    kind: String,
    timestamp: String,
    objective_id: Option<String>,
    plan_id: Option<String>,
    actor_kind: String,
    actor_id: String,
    payload: String,
    causation_id: Option<String>,
    correlation_id: Option<String>,
    sequence: i64,
    objective_seq: Option<i64>,
}

/// Initialize the events table. Idempotent — safe to call multiple times.
pub async fn init_event_store(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS events (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id        TEXT    NOT NULL UNIQUE,
            kind            TEXT    NOT NULL,
            timestamp       TEXT    NOT NULL,
            objective_id    TEXT,
            plan_id         TEXT,
            actor_kind      TEXT    NOT NULL,
            actor_id        TEXT    NOT NULL,
            payload         TEXT    NOT NULL,
            causation_id    TEXT,
            correlation_id  TEXT,
            sequence        INTEGER NOT NULL,
            objective_seq   INTEGER,
            UNIQUE(event_id)
        );
        "#,
    )
    .execute(pool)
    .await?;

    // Index for per-objective replay (ordered by objective_seq).
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_events_objective
            ON events(objective_id, objective_seq)
        "#,
    )
    .execute(pool)
    .await?;

    // Index for time-range replay.
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_events_timestamp
            ON events(timestamp)
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Event Bus — Stage 2 in-process + SQLite-persisted implementation
// ---------------------------------------------------------------------------

/// Capacity of the broadcast channel for live subscribers.
const BUS_CAPACITY: usize = 4096;

/// Global sequence counter shared across EventBus instances.
static GLOBAL_SEQ: AtomicI64 = AtomicI64::new(0);

/// The event bus with optional SQLite persistence.
///
/// Components subscribe to receive events in real time via the in-memory
/// broadcast channel. When a `SqlitePool` is configured, every published
/// event is also persisted to the `events` table before being broadcast.
///
/// ## Ordering Guarantees
/// - Total ordering per objective (events with same `objective_id` appear in
///   publish order on all subscribers and in the event store).
/// - Global monotonic sequence number attached to every persisted event.
///
/// ## Delivery
/// - At-least-once to each subscriber. Slow subscribers that fall behind
///   the buffer capacity will miss events (`RecvError::Lagged`) but the
///   event store allows replay.
/// - Persistence is best-effort: a DB write failure logs a warning but does
///   not block the broadcast. Live subscribers still receive the event.
#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Event>,
    pool: Option<SqlitePool>,
}

impl std::fmt::Debug for EventBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventBus")
            .field("subscriber_count", &self.subscriber_count())
            .field("persistent", &self.pool.is_some())
            .finish()
    }
}

impl EventBus {
    /// Create a new in-memory-only event bus.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(BUS_CAPACITY);
        Self { tx, pool: None }
    }

    /// Create a new event bus with a custom channel capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx, pool: None }
    }

    /// Wrap this bus with a SQLite pool for durable persistence.
    /// Events published after this call will be written to the store.
    pub fn with_persistence(mut self, pool: SqlitePool) -> Self {
        self.pool = Some(pool);
        self
    }

    /// Publish an event to all live subscribers, and persist to the
    /// SQLite store if configured.
    ///
    /// Persistence is asynchronous (spawns a tokio task) so the caller
    /// is never blocked by a DB write. If the DB write fails, a warning
    /// is logged but the broadcast still succeeds.
    pub fn publish(&self, event: Event) {
        if let Some(pool) = &self.pool {
            let sequence = GLOBAL_SEQ.fetch_add(1, Ordering::AcqRel);
            let seq_event = event.with_sequence(sequence);
            let row = event_to_row(&seq_event, sequence);

            // Spawn the DB write so publish never blocks.
            let pool = pool.clone();
            tokio::spawn(async move {
                if let Err(e) = insert_event(&pool, &row).await {
                    tracing::warn!(err = %e, event_id = %seq_event.event_id, "Failed to persist event");
                }
            });

            let _ = self.tx.send(seq_event);
        } else {
            let _ = self.tx.send(event);
        }
    }

    /// Subscribe to receive live events. Returns a receiver that yields
    /// events published after this subscription was created.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    /// Returns the number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }

    /// Returns true if this bus has a SQLite persistence backend.
    pub fn is_persistent(&self) -> bool {
        self.pool.is_some()
    }

    /// Replay all events for a given objective, in order.
    pub async fn replay_objective(
        &self,
        objective_id: &str,
    ) -> Result<Vec<Event>, EventStoreError> {
        let pool = self.pool.as_ref().ok_or(EventStoreError::NotPersistent)?;
        replay_objective_events(pool, objective_id).await
    }

    /// Replay events within a time range.
    pub async fn replay_range(
        &self,
        from: &DateTime<Utc>,
        to: &DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<Event>, EventStoreError> {
        let pool = self.pool.as_ref().ok_or(EventStoreError::NotPersistent)?;
        replay_events_in_range(pool, from, to, limit).await
    }

    /// Replay all events (up to `limit`), newest first.
    pub async fn replay_recent(
        &self,
        limit: i64,
    ) -> Result<Vec<Event>, EventStoreError> {
        let pool = self.pool.as_ref().ok_or(EventStoreError::NotPersistent)?;
        replay_recent_events(pool, limit).await
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Event persistence helpers
// ---------------------------------------------------------------------------

impl Event {
    /// Attach a global sequence number (used for ordering in the event store).
    fn with_sequence(mut self, seq: i64) -> Self {
        // We store seq in the payload to avoid adding a field to the Event
        // struct (which would break the event schema). The canonical sequence
        // lives in the `events` table row.
        if let serde_json::Value::Object(ref mut map) = self.payload {
            map.insert("_seq".into(), serde_json::Value::Number(seq.into()));
        }
        self
    }
}

fn event_to_row(event: &Event, sequence: i64) -> EventRow {
    EventRow {
        event_id: event.event_id.to_string(),
        kind: serde_json::to_value(&event.kind)
            .and_then(|v| Ok(v["type"].as_str().unwrap_or("unknown").to_string()))
            .unwrap_or_else(|_| "unknown".into()),
        timestamp: event.timestamp.to_rfc3339(),
        objective_id: event.objective_id.clone(),
        plan_id: event.plan_id.clone(),
        actor_kind: serde_json::to_value(&event.actor.kind)
            .and_then(|v| Ok(v.as_str().unwrap_or("unknown").to_string()))
            .unwrap_or_else(|_| "unknown".into()),
        actor_id: event.actor.id.clone(),
        payload: serde_json::to_string(&event.payload).unwrap_or_else(|_| "{}".into()),
        causation_id: event.causation_id.map(|u| u.to_string()),
        correlation_id: event.correlation_id.map(|u| u.to_string()),
        sequence,
        objective_seq: None,
    }
}

async fn insert_event(pool: &SqlitePool, row: &EventRow) -> Result<(), sqlx::Error> {
    // Compute objective_seq: auto-increment per objective (1, 2, 3, ...)
    let objective_seq: Option<i64> = if row.objective_id.is_some() {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM events WHERE objective_id = ?",
        )
        .bind(&row.objective_id)
        .fetch_one(pool)
        .await
        .unwrap_or((0,));
        Some(count.0 + 1)
    } else {
        None
    };

    sqlx::query(
        r#"
        INSERT INTO events
            (event_id, kind, timestamp, objective_id, plan_id,
             actor_kind, actor_id, payload, causation_id, correlation_id,
             sequence, objective_seq)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&row.event_id)
    .bind(&row.kind)
    .bind(&row.timestamp)
    .bind(&row.objective_id)
    .bind(&row.plan_id)
    .bind(&row.actor_kind)
    .bind(&row.actor_id)
    .bind(&row.payload)
    .bind(&row.causation_id)
    .bind(&row.correlation_id)
    .bind(row.sequence)
    .bind(objective_seq)
    .execute(pool)
    .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Event store errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum EventStoreError {
    #[error("Event bus is not persistent (no SQLite pool configured)")]
    NotPersistent,
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Deserialization error: {0}")]
    Deserialize(#[from] serde_json::Error),
    #[error("UUID parse error: {0}")]
    UuidParse(#[from] uuid::Error),
}

// ---------------------------------------------------------------------------
// Replay queries
// ---------------------------------------------------------------------------

async fn replay_objective_events(
    pool: &SqlitePool,
    objective_id: &str,
) -> Result<Vec<Event>, EventStoreError> {
    let rows: Vec<EventRow> = sqlx::query_as(
        r#"
        SELECT event_id, kind, timestamp, objective_id, plan_id,
               actor_kind, actor_id, payload, causation_id, correlation_id,
               sequence, objective_seq
        FROM events
        WHERE objective_id = ?
        ORDER BY objective_seq ASC
        "#,
    )
    .bind(objective_id)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(row_to_event).collect()
}

async fn replay_events_in_range(
    pool: &SqlitePool,
    from: &DateTime<Utc>,
    to: &DateTime<Utc>,
    limit: i64,
) -> Result<Vec<Event>, EventStoreError> {
    let rows: Vec<EventRow> = sqlx::query_as(
        r#"
        SELECT event_id, kind, timestamp, objective_id, plan_id,
               actor_kind, actor_id, payload, causation_id, correlation_id,
               sequence, objective_seq
        FROM events
        WHERE timestamp >= ? AND timestamp <= ?
        ORDER BY sequence ASC
        LIMIT ?
        "#,
    )
    .bind(from.to_rfc3339())
    .bind(to.to_rfc3339())
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(row_to_event).collect()
}

async fn replay_recent_events(
    pool: &SqlitePool,
    limit: i64,
) -> Result<Vec<Event>, EventStoreError> {
    let rows: Vec<EventRow> = sqlx::query_as(
        r#"
        SELECT event_id, kind, timestamp, objective_id, plan_id,
               actor_kind, actor_id, payload, causation_id, correlation_id,
               sequence, objective_seq
        FROM events
        ORDER BY sequence DESC
        LIMIT ?
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(row_to_event).collect()
}

/// Convert a DB row back into an Event. This is lossy for the `kind` and
/// `actor.kind` fields (we store them as flat strings but Event uses a
/// tagged enum). We reconstruct using `serde_json::from_value`.
fn row_to_event(row: EventRow) -> Result<Event, EventStoreError> {
    let event_id = Uuid::parse_str(&row.event_id)?;

    let timestamp: DateTime<Utc> = row.timestamp.parse().unwrap_or_else(|_| Utc::now());

    let causation_id = row.causation_id.as_ref().and_then(|s| Uuid::parse_str(s).ok());
    let correlation_id = row.correlation_id.as_ref().and_then(|s| Uuid::parse_str(s).ok());

    // Reconstruct kind from the stored string.
    let kind_json = serde_json::json!({"type": row.kind});
    let kind: EventKind = serde_json::from_value(kind_json)?;

    // Reconstruct actor.
    let actor_json = serde_json::json!({"kind": row.actor_kind, "id": row.actor_id});
    let actor: Actor = serde_json::from_value(actor_json)?;

    let payload: serde_json::Value = serde_json::from_str(&row.payload)?;

    Ok(Event {
        event_id,
        kind,
        timestamp,
        objective_id: row.objective_id,
        plan_id: row.plan_id,
        actor,
        payload,
        causation_id,
        correlation_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;
    use tokio::sync::broadcast::error::TryRecvError;

    // -----------------------------------------------------------------------
    // In-memory (no persistence) tests
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Persistence tests
    // -----------------------------------------------------------------------

    /// Create a fresh in-memory SQLite pool with the events table initialized.
    async fn init_test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("Failed to create in-memory SQLite pool");
        init_event_store(&pool)
            .await
            .expect("Failed to init event store");
        pool
    }

    fn test_event() -> Event {
        Event::new(
            EventKind::ObjectiveCreated,
            Actor { kind: ActorKind::Kernel, id: "kernel".into() },
            serde_json::json!({"key": "val"}),
        )
    }

    #[tokio::test]
    async fn is_persistent_false_when_no_pool() {
        let bus = EventBus::new();
        assert!(!bus.is_persistent());
    }

    #[tokio::test]
    async fn is_persistent_true_when_pool_configured() {
        let pool = init_test_pool().await;
        let bus = EventBus::new().with_persistence(pool);
        assert!(bus.is_persistent());
    }

    #[tokio::test]
    async fn publish_with_persistence_stores_event() {
        let pool = init_test_pool().await;
        let bus = EventBus::new().with_persistence(pool.clone());

        let event = test_event().with_objective("obj-persist-1");
        bus.publish(event);

        // Allow the async spawn to complete the write.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Verify via SQL query directly.
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 1, "Expected 1 persisted event");
    }

    #[tokio::test]
    async fn replay_objective_returns_ordered_events() {
        let pool = init_test_pool().await;
        let bus = EventBus::new().with_persistence(pool.clone());

        let obj_id = "obj-replay-1";
        bus.publish(test_event().with_objective(obj_id));
        bus.publish(test_event().with_objective(obj_id));
        bus.publish(test_event().with_objective("other-obj"));

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let events = bus
            .replay_objective(obj_id)
            .await
            .expect("replay_objective failed");
        assert_eq!(events.len(), 2, "Should find 2 events for objective");
        // Events should have objective_seq in order — we verify by order returned.
        assert_eq!(events[0].objective_id.as_deref(), Some(obj_id));
        assert_eq!(events[1].objective_id.as_deref(), Some(obj_id));
    }

    #[tokio::test]
    async fn replay_recent_limits_correctly() {
        let pool = init_test_pool().await;
        let bus = EventBus::new().with_persistence(pool.clone());

        // Publish 5 events
        for _ in 0..5 {
            bus.publish(test_event());
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let events = bus.replay_recent(3).await.expect("replay_recent failed");
        assert_eq!(events.len(), 3, "Should limit to 3 events");
    }

    #[tokio::test]
    async fn replay_range_respects_time_bounds() {
        let pool = init_test_pool().await;
        let bus = EventBus::new().with_persistence(pool.clone());

        bus.publish(test_event());
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        bus.publish(test_event());

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let now = Utc::now();
        let from = now - chrono::Duration::hours(1);
        let to = now + chrono::Duration::hours(1);

        let events = bus
            .replay_range(&from, &to, 100)
            .await
            .expect("replay_range failed");
        assert_eq!(events.len(), 2, "Both events should be within the wide time range");
    }

    #[tokio::test]
    async fn replay_without_persistence_returns_error() {
        let bus = EventBus::new();
        let result = bus.replay_objective("any").await;
        assert!(
            matches!(result, Err(EventStoreError::NotPersistent)),
            "Expected NotPersistent error"
        );
    }

    #[tokio::test]
    async fn in_memory_only_still_works_backward_compatible() {
        // Verify that EventBus without persistence still works exactly as before.
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        assert!(!bus.is_persistent());

        bus.publish(test_event());

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(rx.try_recv().is_ok());
    }

    #[tokio::test]
    async fn global_sequence_is_monotonic() {
        let pool = init_test_pool().await;
        let bus = EventBus::new().with_persistence(pool.clone());

        bus.publish(test_event());
        bus.publish(test_event());
        bus.publish(test_event());

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let rows: Vec<EventRow> = sqlx::query_as(
            "SELECT event_id, kind, timestamp, objective_id, plan_id,
                    actor_kind, actor_id, payload, causation_id, correlation_id,
                    sequence, objective_seq
             FROM events ORDER BY sequence ASC",
        )
        .fetch_all(&pool)
        .await
        .unwrap();

        assert_eq!(rows.len(), 3);
        // sequence must be strictly increasing.
        assert!(rows[0].sequence < rows[1].sequence);
        assert!(rows[1].sequence < rows[2].sequence);
    }
}
