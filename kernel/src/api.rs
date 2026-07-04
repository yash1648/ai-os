//! AI-OS Kernel HTTP API — axum-based REST server.
//!
//! Exposes the kernel's capabilities (scheduler, event bus, objectives,
//! state machine) over HTTP for the CLI, dashboard, and Python workers.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, Sse},
    response::Json,
    response::IntoResponse,
    routing::{delete, get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::config::KernelConfig;
use crate::coordinator::Coordinator;
use crate::event_bus::{Event as BusEvent, EventBus};
use crate::objective::{Objective, ObjectiveStore};
use crate::scheduler::Scheduler;
use crate::dashboard;
use crate::state_machine::{self, ObjectivePrimaryState, ObjectiveState, RetryPolicy};

// ---------------------------------------------------------------------------
// Application state
// ---------------------------------------------------------------------------

/// Shared application state accessible from all request handlers.
pub struct AppState {
    pub config: KernelConfig,
    pub scheduler: Arc<tokio::sync::Mutex<Scheduler>>,
    pub coordinator: tokio::sync::Mutex<Coordinator>,
    pub event_bus: EventBus,
    pub objective_store: Arc<ObjectiveStore>,
    pub started_at: chrono::DateTime<chrono::Utc>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Generic API response wrapper.
#[derive(Serialize)]
struct ApiResponse<T: Serialize> {
    success: bool,
    data: T,
}

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    version: String,
    uptime_seconds: u64,
}

#[derive(Serialize)]
struct SchedulerStatusResponse {
    active_count: usize,
    queue_length: usize,
    total_dispatched: usize,
    throttle_count: usize,
    max_concurrent: usize,
    can_dispatch: bool,
}

#[derive(Serialize)]
struct QueueEntryResponse {
    objective_id: String,
    priority: String,
    retry_count: u32,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct CreateObjectiveRequest {
    title: String,
    description: String,
    owner: String,
    priority: String,
    dependencies: Vec<String>,
    success_criteria: Vec<String>,
    tags: Vec<String>,
}

#[allow(dead_code)]
#[derive(Serialize)]
struct CreateObjectiveResponse {
    id: String,
}

#[derive(Deserialize)]
struct ValidateTransitionRequest {
    from: String,
    to: String,
}

#[derive(Serialize)]
struct ValidateTransitionResponse {
    allowed: bool,
    message: String,
}

#[derive(Deserialize)]
struct TransitionRequest {
    status: String,
}

#[derive(Deserialize)]
struct RecentEventsParams {
    limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct TimelineParams {
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Serialize)]
pub struct TimelineEvent {
    pub event_id: String,
    pub kind: String,
    pub timestamp: String,
    pub objective_id: Option<String>,
    pub plan_id: Option<String>,
    pub actor: serde_json::Value,
    pub payload: serde_json::Value,
    pub sequence: i64,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Build the axum Router with all API routes.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        // Health
        .route("/health", get(health_handler))
        .route("/api/v1/health", get(health_handler))
        // Objectives
        .route("/api/v1/objectives", post(create_objective_handler))
        .route("/api/v1/objectives", get(list_objectives_handler))
        .route(
            "/api/v1/objectives/{id}",
            get(get_objective_handler),
        )
        .route(
            "/api/v1/objectives/{id}/ready",
            post(mark_ready_handler),
        )
        .route(
            "/api/v1/objectives/{id}/transition",
            post(transition_handler),
        )
        .route(
            "/api/v1/objectives/{id}",
            delete(delete_objective_handler),
        )
        // Scheduler
        .route("/api/v1/scheduler/status", get(scheduler_status_handler))
        .route("/api/v1/scheduler/queue", get(scheduler_queue_handler))
        .route(
            "/api/v1/scheduler/dispatch",
            post(scheduler_dispatch_handler),
        )
        // State machine
        .route("/api/v1/validate", post(validate_transition_handler))
        // Events — SSE stream
        .route("/api/v1/events", get(events_handler))
        // Events — persistent timeline (Stage 3 dashboard backend)
        .route(
            "/api/v1/events/objective/{id}",
            get(event_objective_handler),
        )
        .route("/api/v1/events/recent", get(event_recent_handler))
        .route("/api/v1/events/timeline", get(event_timeline_handler))
        // Dashboard — timeline, objectives, metrics, audit log
        .route("/api/dashboard/timeline", get(dashboard::timeline_handler))
        .route("/api/dashboard/objectives", get(dashboard::objectives_handler))
        .route("/api/dashboard/metrics", get(dashboard::metrics_handler))
        .route("/api/dashboard/audit-log", get(dashboard::audit_log_handler))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Handler: Health
// ---------------------------------------------------------------------------

async fn health_handler(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<HealthResponse>> {
    let uptime = (chrono::Utc::now() - state.started_at)
        .num_seconds()
        .max(0) as u64;
    Json(ApiResponse {
        success: true,
        data: HealthResponse {
            status: "ok".into(),
            version: "0.1.0".into(),
            uptime_seconds: uptime,
        },
    })
}

// ---------------------------------------------------------------------------
// Handler: Objectives
// ---------------------------------------------------------------------------

async fn create_objective_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateObjectiveRequest>,
) -> impl IntoResponse {
    let priority = match req.priority.to_lowercase().as_str() {
        "minimal" => crate::objective::Priority::Minimal,
        "low" => crate::objective::Priority::Low,
        "medium" => crate::objective::Priority::Medium,
        "high" => crate::objective::Priority::High,
        "critical" => crate::objective::Priority::Critical,
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "success": false,
                    "error": format!(
                        "Invalid priority '{other}'. Expected: minimal, low, medium, high, critical"
                    ),
                })),
            );
        }
    };

    let objective = Objective {
        id: uuid::Uuid::new_v4().to_string(),
        title: req.title,
        description: req.description,
        owner: req.owner,
        parent_id: None,
        priority,
        status: ObjectiveState::from_label("DISCOVERED"),
        dependencies: req.dependencies,
        success_criteria: req.success_criteria,
        plan_id: None,
        retry_count: 0,
        tags: req.tags,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    match state.objective_store.insert(&objective).await {
        Ok(_) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "success": true,
                "data": { "id": objective.id },
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "success": false,
                "error": format!("Failed to persist objective: {e}"),
            })),
        ),
    }
}

async fn list_objectives_handler(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<Objective>>> {
    match state.objective_store.list(None).await {
        Ok(objectives) => Json(ApiResponse {
            success: true,
            data: objectives,
        }),
        Err(_) => Json(ApiResponse {
            success: true,
            data: vec![],
        }),
    }
}

async fn get_objective_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.objective_store.get(&id).await {
        Ok(Some(objective)) => {
            (StatusCode::OK, Json(serde_json::json!({
                "success": true,
                "data": objective,
            })))
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "success": false,
                "error": format!("Objective '{id}' not found"),
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "success": false,
                "error": format!("Error fetching objective '{id}': {e}"),
            })),
        ),
    }
}

async fn mark_ready_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.objective_store.get(&id).await {
        Ok(Some(obj)) => {
            if let Err(e) = state
                .objective_store
                .update_status(&id, &ObjectiveState::from_label("READY"), obj.retry_count)
                .await
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "success": false,
                        "error": format!("Failed to persist status: {e}"),
                    })),
                );
            }

            let mut scheduler = state.scheduler.lock().await;
            scheduler.notify_objective_ready(&id, crate::scheduler::Priority::Medium, chrono::Utc::now(), obj.retry_count);

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "success": true,
                    "data": { "objective_id": id, "status": "ready" },
                })),
            )
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "success": false,
                "error": format!("Objective '{id}' not found"),
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "success": false,
                "error": format!("Error fetching objective '{id}': {e}"),
            })),
        ),
    }
}

// ---------------------------------------------------------------------------
// Handler: Generic state transition
// ---------------------------------------------------------------------------

async fn transition_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<TransitionRequest>,
) -> impl IntoResponse {
    let target = ObjectiveState::from_label(&req.status);
    let policy = RetryPolicy::default();

    match state.objective_store.get(&id).await {
        Ok(Some(obj)) => {
            let current = obj.status;
            match state_machine::transition(current, target, &policy, obj.retry_count) {
                Ok(new_state) => {
                    if let Err(e) = state
                        .objective_store
                        .update_status(&id, &new_state, obj.retry_count)
                        .await
                    {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "success": false,
                                "error": format!("Failed to persist transition: {e}"),
                            })),
                        );
                    }

                    // If transitioning to READY, also notify the scheduler
                    if req.status.to_uppercase() == "READY" {
                        let mut scheduler = state.scheduler.lock().await;
                        scheduler.notify_objective_ready(
                            &id,
                            crate::scheduler::Priority::Medium,
                            chrono::Utc::now(),
                            obj.retry_count,
                        );
                    }

                    (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "success": true,
                            "data": {
                                "objective_id": id,
                                "previous": current.label(),
                                "status": new_state.label(),
                            },
                        })),
                    )
                }
                Err(e) => (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "success": false,
                        "error": format!("Transition denied: {e}"),
                    })),
                ),
            }
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "success": false,
                "error": format!("Objective '{id}' not found"),
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "success": false,
                "error": format!("Error fetching objective '{id}': {e}"),
            })),
        ),
    }
}

// ---------------------------------------------------------------------------
// Handler: Delete / Abandon objective
// ---------------------------------------------------------------------------

async fn delete_objective_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.objective_store.get(&id).await {
        Ok(Some(obj)) => {
            let abandoned = ObjectiveState::from_label("ABANDONED");
            if let Err(e) = state
                .objective_store
                .update_status(&id, &abandoned, obj.retry_count)
                .await
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "success": false,
                        "error": format!("Failed to abandon objective '{id}': {e}"),
                    })),
                );
            }

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "success": true,
                    "data": { "objective_id": id, "status": "abandoned" },
                })),
            )
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "success": false,
                "error": format!("Objective '{id}' not found"),
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "success": false,
                "error": format!("Error fetching objective '{id}': {e}"),
            })),
        ),
    }
}

// ---------------------------------------------------------------------------
// Handler: Scheduler
// ---------------------------------------------------------------------------

async fn scheduler_status_handler(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<SchedulerStatusResponse>> {
    let s = state.scheduler.lock().await;
    Json(ApiResponse {
        success: true,
        data: SchedulerStatusResponse {
            active_count: s.active_count(),
            queue_length: s.queue_len(),
            total_dispatched: s.total_dispatched() as usize,
            throttle_count: s.throttle_count() as usize,
            max_concurrent: s.max_concurrent(),
            can_dispatch: s.can_dispatch(),
        },
    })
}

async fn scheduler_queue_handler(
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<Vec<QueueEntryResponse>>> {
    let s = state.scheduler.lock().await;
    let entries: Vec<QueueEntryResponse> = s
        .peek_queue()
        .into_iter()
        .map(|e| QueueEntryResponse {
            objective_id: e.objective_id.clone(),
            priority: format!("{:?}", e.priority),
            retry_count: e.retry_count,
        })
        .collect();
    Json(ApiResponse {
        success: true,
        data: entries,
    })
}

async fn scheduler_dispatch_handler(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let objective_id = {
        let mut s = state.scheduler.lock().await;
        s.try_dispatch()
    };
    match objective_id {
        Some(id) => {
            let executing = ObjectiveState::Primary(ObjectivePrimaryState::Executing);
            match state.objective_store.update_status(&id, &executing, 0).await {
                Ok(_) => {
                    // Spawn the worker via coordinator (runs async, monitors completion)
                    let mut c = state.coordinator.lock().await;
                    let worker_id = c.dispatch_and_monitor(&id).await;
                    drop(c);

                    (
                        StatusCode::OK,
                        Json(ApiResponse {
                            success: true,
                            data: serde_json::json!({
                                "dispatched": id,
                                "worker_id": worker_id,
                            }),
                        }),
                    )
                }
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        success: false,
                        data: serde_json::json!({
                            "error": format!("Failed to persist dispatched status for '{id}': {e}"),
                        }),
                    }),
                ),
            }
        }
        None => (
            StatusCode::OK,
            Json(ApiResponse {
                success: true,
                data: serde_json::json!({
                    "dispatched": null,
                    "reason": "queue_empty_or_at_capacity",
                }),
            }),
        ),
    }
}

// ---------------------------------------------------------------------------
// Handler: State machine validation
// ---------------------------------------------------------------------------

async fn validate_transition_handler(
    Json(req): Json<ValidateTransitionRequest>,
) -> Json<ApiResponse<ValidateTransitionResponse>> {
    let from = ObjectiveState::from_label(&req.from);
    let to = ObjectiveState::from_label(&req.to);
    let policy = RetryPolicy::default();

    match state_machine::transition(from, to, &policy, 0) {
        Ok(state) => Json(ApiResponse {
            success: true,
            data: ValidateTransitionResponse {
                allowed: true,
                message: format!("{} → {} [allowed]", req.from, state.label()),
            },
        }),
        Err(e) => Json(ApiResponse {
            success: true,
            data: ValidateTransitionResponse {
                allowed: false,
                message: format!("{} → {} [denied]: {e}", req.from, req.to),
            },
        }),
    }
}

// ---------------------------------------------------------------------------
// Handler: Events — objective timeline (persistent backend for dashboard)
// ---------------------------------------------------------------------------

/// Replay all events for a specific objective, ordered by sequence.
async fn event_objective_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.event_bus.replay_objective(&id).await {
        Ok(events) => {
            let data: Vec<TimelineEvent> = events
                .into_iter()
                .map(|e| TimelineEvent {
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
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"success": false, "error": format!("{e}")})),
        ),
    }
}

/// Replay recent events (newest first), with optional limit.
async fn event_recent_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<RecentEventsParams>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(50).min(500);
    match state.event_bus.replay_recent(limit).await {
        Ok(events) => {
            let data: Vec<TimelineEvent> = events
                .into_iter()
                .map(|e| TimelineEvent {
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
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"success": false, "error": format!("{e}")})),
        ),
    }
}

/// Replay events in an optional time range, with optional limit.
async fn event_timeline_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TimelineParams>,
) -> impl IntoResponse {
    // SAFETY: these constants are guaranteed-valid ISO 8601 strings.
    let unix_epoch: chrono::DateTime<chrono::Utc> = "1970-01-01T00:00:00Z".parse().unwrap();
    let default_from = unix_epoch;
    let default_to = chrono::Utc::now();
    let from = params
        .from
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok().map(|dt| dt.with_timezone(&chrono::Utc)))
        .unwrap_or(default_from);
    let to = params
        .to
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok().map(|dt| dt.with_timezone(&chrono::Utc)))
        .unwrap_or(default_to);
    let limit = params.limit.unwrap_or(100).min(1000);

    match state.event_bus.replay_range(&from, &to, limit).await {
        Ok(events) => {
            let data: Vec<TimelineEvent> = events
                .into_iter()
                .map(|e| TimelineEvent {
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
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"success": false, "error": format!("{e}")})),
        ),
    }
}

// ---------------------------------------------------------------------------
// Handler: Server-Sent Events stream
// ---------------------------------------------------------------------------

async fn events_handler(
    State(state): State<Arc<AppState>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let rx: broadcast::Receiver<BusEvent> = state.event_bus.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(event) => {
            let payload = serde_json::to_string(&event).unwrap_or_default();
            let kind_label = serde_json::to_string(&event.kind).unwrap_or_else(|_| "\"unknown\"".into());
            Some(Ok(Event::default()
                .event(kind_label)
                .data(payload)))
        }
        Err(_) => None, // skip lagged events silently
    });

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keep-alive"),
    )
}
