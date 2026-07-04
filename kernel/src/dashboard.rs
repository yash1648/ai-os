//! Dashboard backend — timeline, audit log with hash-chain, metrics, and
//! objective status aggregation for the kernel dashboard UI.
//!
//! ## Audit hash-chain
//! Each audit entry includes the SHA-256 of `(prev_hash || event_id || kind || timestamp)`.
//! The genesis entry uses a 64-char zero hex string as its `prev_hash`.
//! Consumers can independently verify the chain by recomputing hashes.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::api::AppState;
use crate::event_bus::Event;

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AuditLogQuery {
    pub since: Option<String>,
    pub limit: Option<i64>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// A single hash-chained audit entry.
#[derive(Serialize)]
pub struct AuditEntry {
    pub event_id: String,
    pub hash: String,
    pub prev_hash: String,
    pub timestamp: String,
    pub kind: String,
    pub objective_id: Option<String>,
}

/// Aggregated metrics for the dashboard.
#[derive(Serialize)]
pub struct DashboardMetrics {
    pub total_objectives: usize,
    pub active_objectives: usize,
    pub completed_objectives: usize,
    pub failed_objectives: usize,
    pub abandoned_objectives: usize,
    pub avg_retry_count: f64,
    pub objectives_by_status: Vec<StatusCount>,
}

#[derive(Serialize)]
pub struct StatusCount {
    pub status: String,
    pub count: usize,
}

/// High-level objective status summary.
#[derive(Serialize)]
pub struct ObjectivesSummary {
    pub total: usize,
    pub items: Vec<ObjectiveSummaryItem>,
}

#[derive(Serialize)]
pub struct ObjectiveSummaryItem {
    pub id: String,
    pub title: String,
    pub status: String,
    pub priority: String,
    pub retry_count: u32,
    pub created_at: String,
    pub updated_at: String,
}

// ---------------------------------------------------------------------------
// Hash-chain utilities
// ---------------------------------------------------------------------------

const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn compute_entry_hash(prev_hash: &str, event_id: &str, kind: &str, timestamp: &str) -> String {
    let input = format!("{prev_hash}{event_id}{kind}{timestamp}");
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex_encode(&hasher.finalize())
}

/// Build a hash-chain of audit entries from a sequence of events.
/// The first entry's `prev_hash` is the genesis (64 zero hex chars).
fn build_audit_chain(events: &[Event]) -> Vec<AuditEntry> {
    let mut chain = Vec::with_capacity(events.len());
    let mut prev_hash = GENESIS_HASH.to_string();

    for event in events {
        let kind_str = format!("{:?}", event.kind);
        let ts = event.timestamp.to_rfc3339();
        let hash = compute_entry_hash(&prev_hash, &event.event_id.to_string(), &kind_str, &ts);

        chain.push(AuditEntry {
            event_id: event.event_id.to_string(),
            hash: hash.clone(),
            prev_hash: prev_hash.clone(),
            timestamp: ts,
            kind: kind_str,
            objective_id: event.objective_id.clone(),
        });

        prev_hash = hash;
    }

    chain
}

// ---------------------------------------------------------------------------
// Metrics aggregation
// ---------------------------------------------------------------------------

fn compute_metrics(objectives: &[crate::objective::Objective]) -> DashboardMetrics {
    let total = objectives.len();
    let mut active = 0usize;
    let mut completed = 0usize;
    let mut failed = 0usize;
    let mut abandoned = 0usize;
    let mut total_retries = 0u64;
    let mut status_map: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();

    for obj in objectives {
        total_retries += obj.retry_count as u64;
        let label = obj.status.label().to_string();
        *status_map.entry(label.clone()).or_insert(0) += 1;

        match obj.status {
            crate::state_machine::ObjectiveState::Terminal(
                crate::state_machine::ObjectiveTerminalState::Done,
            ) => completed += 1,
            crate::state_machine::ObjectiveState::Terminal(
                crate::state_machine::ObjectiveTerminalState::Abandoned,
            ) => abandoned += 1,
            crate::state_machine::ObjectiveState::Failure(_) => failed += 1,
            _ => active += 1,
        }
    }

    let avg_retry = if total > 0 {
        total_retries as f64 / total as f64
    } else {
        0.0
    };

    let objectives_by_status: Vec<StatusCount> = status_map
        .into_iter()
        .map(|(status, count)| StatusCount { status, count })
        .collect();

    DashboardMetrics {
        total_objectives: total,
        active_objectives: active,
        completed_objectives: completed,
        failed_objectives: failed,
        abandoned_objectives: abandoned,
        avg_retry_count: avg_retry,
        objectives_by_status,
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /api/dashboard/timeline` — recent events from the event store.
pub async fn timeline_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<crate::api::TimelineParams>,
) -> impl axum::response::IntoResponse {
    let limit = params.limit.unwrap_or(50).min(500);
    match state.event_bus.replay_recent(limit).await {
        Ok(events) => {
            let data: Vec<crate::api::TimelineEvent> = events
                .into_iter()
                .map(|e| crate::api::TimelineEvent {
                    event_id: e.event_id.to_string(),
                    kind: format!("{:?}", e.kind),
                    timestamp: e.timestamp.to_rfc3339(),
                    objective_id: e.objective_id,
                    plan_id: e.plan_id,
                    actor: serde_json::json!({"kind": e.actor.kind, "id": e.actor.id}),
                    payload: e.payload,
                    sequence: 0,
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!({"success": true, "data": data})))
        }
        Err(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"success": true, "data": []})),
        ),
    }
}

/// `GET /api/dashboard/objectives` — objective status aggregation.
pub async fn objectives_handler(
    State(state): State<Arc<AppState>>,
) -> impl axum::response::IntoResponse {
    match state.objective_store.list(None).await {
        Ok(objectives) => {
            let items: Vec<ObjectiveSummaryItem> = objectives
                .into_iter()
                .map(|o| ObjectiveSummaryItem {
                    id: o.id,
                    title: o.title,
                    status: o.status.label().to_string(),
                    priority: format!("{:?}", o.priority),
                    retry_count: o.retry_count,
                    created_at: o.created_at.to_rfc3339(),
                    updated_at: o.updated_at.to_rfc3339(),
                })
                .collect();
            let total = items.len();
            (StatusCode::OK, Json(serde_json::json!({
                "success": true,
                "data": ObjectivesSummary { total, items },
            })))
        }
        Err(_) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "data": ObjectivesSummary { total: 0, items: Vec::new() },
            })),
        ),
    }
}

/// `GET /api/dashboard/metrics` — avg duration, retry counts, outcome rates.
pub async fn metrics_handler(
    State(state): State<Arc<AppState>>,
) -> impl axum::response::IntoResponse {
    match state.objective_store.list(None).await {
        Ok(objectives) => {
            let metrics = compute_metrics(&objectives);
            (StatusCode::OK, Json(serde_json::json!({"success": true, "data": metrics})))
        }
        Err(_) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "success": true,
                "data": DashboardMetrics {
                    total_objectives: 0,
                    active_objectives: 0,
                    completed_objectives: 0,
                    failed_objectives: 0,
                    abandoned_objectives: 0,
                    avg_retry_count: 0.0,
                    objectives_by_status: vec![],
                },
            })),
        ),
    }
}

/// `GET /api/dashboard/audit-log` — hash-chained audit entries, optionally
/// filtered by `since` (ISO 8601) timestamp.
pub async fn audit_log_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<AuditLogQuery>,
) -> impl axum::response::IntoResponse {
    let limit = params.limit.unwrap_or(100).min(1000);

    let events = if let Some(since_str) = &params.since {
        let since = match chrono::DateTime::parse_from_rfc3339(since_str) {
            Ok(dt) => dt.with_timezone(&chrono::Utc),
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "success": false,
                        "error": format!("Invalid ISO 8601 timestamp: '{since_str}'"),
                    })),
                );
            }
        };
        let now = chrono::Utc::now();
        match state.event_bus.replay_range(&since, &now, limit).await {
            Ok(ev) => ev,
            Err(_) => vec![],
        }
    } else {
        match state.event_bus.replay_recent(limit).await {
            Ok(ev) => ev,
            Err(_) => vec![],
        }
    };

    // Build audit chain in chronological order (replay_recent returns newest first)
    let mut chain_events: Vec<Event> = events;
    chain_events.reverse();
    let chain = build_audit_chain(&chain_events);

    (StatusCode::OK, Json(serde_json::json!({"success": true, "data": chain})))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_encode_empty() {
        assert_eq!(hex_encode(b""), "");
    }

    #[test]
    fn hex_encode_known() {
        // SHA-256 of empty string
        let hash = Sha256::digest(b"");
        assert_eq!(hex_encode(&hash), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn genesis_hash_is_64_zeros() {
        assert_eq!(GENESIS_HASH.len(), 64);
        assert!(GENESIS_HASH.chars().all(|c| c == '0'));
    }

    #[test]
    fn compute_entry_hash_is_deterministic() {
        let h1 = compute_entry_hash(GENESIS_HASH, "evt-1", "ObjectiveCreated", "2025-01-01T00:00:00Z");
        let h2 = compute_entry_hash(GENESIS_HASH, "evt-1", "ObjectiveCreated", "2025-01-01T00:00:00Z");
        assert_eq!(h1, h2);
    }

    #[test]
    fn compute_entry_hash_differs_on_input() {
        let h1 = compute_entry_hash(GENESIS_HASH, "evt-1", "ObjectiveCreated", "2025-01-01T00:00:00Z");
        let h2 = compute_entry_hash(GENESIS_HASH, "evt-2", "ObjectiveCreated", "2025-01-01T00:00:00Z");
        assert_ne!(h1, h2);
    }

    #[test]
    fn build_audit_chain_chaining() {
        let events = vec![
            Event::new(
                crate::event_bus::EventKind::ObjectiveCreated,
                crate::event_bus::Actor { kind: crate::event_bus::ActorKind::Kernel, id: "sys".into() },
                serde_json::json!({"msg": "first"}),
            )
            .with_objective("obj-1"),
            Event::new(
                crate::event_bus::EventKind::PlanGenerated,
                crate::event_bus::Actor { kind: crate::event_bus::ActorKind::Worker, id: "w-1".into() },
                serde_json::json!({"msg": "second"}),
            )
            .with_objective("obj-1"),
        ];

        let chain = build_audit_chain(&events);
        assert_eq!(chain.len(), 2);

        // First entry's prev_hash should be genesis
        assert_eq!(chain[0].prev_hash, GENESIS_HASH);

        // Second entry's prev_hash should be first entry's hash
        assert_eq!(chain[1].prev_hash, chain[0].hash);

        // Both hashes should be 64 hex chars
        for entry in &chain {
            assert_eq!(entry.hash.len(), 64);
            assert_eq!(entry.prev_hash.len(), 64);
        }
    }

    #[test]
    fn build_audit_chain_empty() {
        let chain = build_audit_chain(&[]);
        assert!(chain.is_empty());
    }

    #[test]
    fn compute_metrics_empty() {
        let metrics = compute_metrics(&[]);
        assert_eq!(metrics.total_objectives, 0);
        assert_eq!(metrics.active_objectives, 0);
        assert_eq!(metrics.completed_objectives, 0);
        assert_eq!(metrics.avg_retry_count, 0.0);
        assert!(metrics.objectives_by_status.is_empty());
    }

    #[test]
    fn compute_metrics_mixed() {
        use crate::objective::{Objective, Priority};
        use crate::state_machine::*;

        let objectives = vec![
            Objective {
                id: "o1".into(), title: "A".into(), description: "".into(), owner: "u".into(),
                parent_id: None, priority: Priority::Medium,
                status: ObjectiveState::Primary(ObjectivePrimaryState::Executing),
                dependencies: vec![], success_criteria: vec!["pass".into()],
                plan_id: None, retry_count: 2, tags: vec![],
                created_at: chrono::Utc::now(), updated_at: chrono::Utc::now(),
            },
            Objective {
                id: "o2".into(), title: "B".into(), description: "".into(), owner: "u".into(),
                parent_id: None, priority: Priority::High,
                status: ObjectiveState::Terminal(ObjectiveTerminalState::Done),
                dependencies: vec![], success_criteria: vec!["pass".into()],
                plan_id: None, retry_count: 0, tags: vec![],
                created_at: chrono::Utc::now(), updated_at: chrono::Utc::now(),
            },
        ];

        let metrics = compute_metrics(&objectives);
        assert_eq!(metrics.total_objectives, 2);
        assert_eq!(metrics.active_objectives, 1);
        assert_eq!(metrics.completed_objectives, 1);
        assert_eq!(metrics.failed_objectives, 0);
        assert_eq!(metrics.abandoned_objectives, 0);
        assert!((metrics.avg_retry_count - 1.0).abs() < f64::EPSILON);
    }
}
