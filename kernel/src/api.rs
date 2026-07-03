//! AI-OS Kernel HTTP API — axum-based REST server.
//!
//! Exposes the kernel's capabilities (scheduler, event bus, objectives,
//! state machine) over HTTP for the CLI, dashboard, and Python workers.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::sse::{Event, Sse},
    response::Json,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::config::KernelConfig;
use crate::event_bus::{Event as BusEvent, EventBus};
use crate::objective::Objective;
use crate::scheduler::Scheduler;
use crate::state_machine::{self, ObjectiveState, RetryPolicy};

// ---------------------------------------------------------------------------
// Application state
// ---------------------------------------------------------------------------

/// Shared application state accessible from all request handlers.
pub struct AppState {
    pub config: KernelConfig,
    pub scheduler: tokio::sync::Mutex<Scheduler>,
    pub event_bus: EventBus,
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

/// Error response body.
#[derive(Serialize)]
struct ApiError {
    success: bool,
    error: String,
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
        // Scheduler
        .route("/api/v1/scheduler/status", get(scheduler_status_handler))
        .route("/api/v1/scheduler/queue", get(scheduler_queue_handler))
        .route(
            "/api/v1/scheduler/dispatch",
            post(scheduler_dispatch_handler),
        )
        // State machine
        .route("/api/v1/validate", post(validate_transition_handler))
        // Events (SSE)
        .route("/api/v1/events", get(events_handler))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Handler: Health
// ---------------------------------------------------------------------------

async fn health_handler(
    State(_state): State<Arc<AppState>>,
) -> Json<ApiResponse<HealthResponse>> {
    Json(ApiResponse {
        success: true,
        data: HealthResponse {
            status: "ok".into(),
            version: "0.1.0".into(),
            uptime_seconds: 0,
        },
    })
}

// ---------------------------------------------------------------------------
// Handler: Objectives (Stage 1 — in-memory placeholder)
// ---------------------------------------------------------------------------

async fn create_objective_handler(
    State(_state): State<Arc<AppState>>,
    Json(req): Json<CreateObjectiveRequest>,
) -> impl IntoResponse {
    let _priority = match req.priority.to_lowercase().as_str() {
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

    // Stage 1: placeholder — generate an ID without persisting.
    let id = uuid::Uuid::new_v4().to_string();

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "success": true,
            "data": { "id": id },
        })),
    )
}

async fn list_objectives_handler() -> Json<ApiResponse<Vec<Objective>>> {
    // Stage 1: no persistent store yet — return empty list.
    Json(ApiResponse {
        success: true,
        data: vec![],
    })
}

async fn get_objective_handler(
    Path(id): Path<String>,
) -> impl IntoResponse {
    // Stage 1: no persistent store yet — return 404.
    (
        StatusCode::NOT_FOUND,
        Json(ApiError {
            success: false,
            error: format!("Objective '{id}' not found (in-memory store not available)"),
        }),
    )
}

async fn mark_ready_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let mut scheduler = state.scheduler.lock().await;
    scheduler.notify_objective_ready(&id, crate::scheduler::Priority::Medium, chrono::Utc::now(), 0);

    (
        StatusCode::OK,
        Json(ApiResponse {
            success: true,
            data: serde_json::json!({ "objective_id": id, "status": "ready" }),
        }),
    )
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
    let mut s = state.scheduler.lock().await;
    match s.try_dispatch() {
        Some(objective_id) => (
            StatusCode::OK,
            Json(ApiResponse {
                success: true,
                data: serde_json::json!({
                    "dispatched": objective_id,
                }),
            }),
        ),
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
