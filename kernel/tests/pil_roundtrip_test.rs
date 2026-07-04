//! Integration test: kernel ↔ PIL sidecar round-trip.
//!
//! Spins up a minimal axum-based mock of the PIL sidecar and exercises the
//! full PilClient against real HTTP responses, verifying deserialization.

use std::sync::mpsc;
use std::sync::OnceLock;

use ai_os_kernel::config::PilConfig;
use ai_os_kernel::pil_client::PilClient;
use axum::extract::Query;
use axum::response::Json;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use serde_json::json;

// ---------------------------------------------------------------------------
// Mock PIL server
// ---------------------------------------------------------------------------

static MOCK_PORT: OnceLock<u16> = OnceLock::new();

/// Start a mock PIL sidecar on a random available port, returning the port.
fn start_mock_pil() -> u16 {
    if let Some(port) = MOCK_PORT.get() {
        return *port;
    }

    let router: Router = Router::new()
        .route("/api/v1/health", get(health_handler))
        .route("/api/v1/adr/search", get(adr_search_handler))
        .route("/api/v1/constitution/validate", get(constitution_validate_handler))
        .route("/api/v1/symbol/resolve", get(symbol_resolve_handler))
        .route("/api/v1/search/semantic", get(semantic_search_handler));

    // Spawn a tokio runtime in a thread, bind entirely within it.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime for mock");
        rt.block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("failed to bind mock PIL");
            let port = listener.local_addr().unwrap().port();
            tx.send(port).expect("failed to send port");
            axum::serve(listener, router)
                .await
                .expect("mock PIL server failed");
        });
    });

    let port = rx.recv().expect("failed to receive port from mock server");
    let _ = MOCK_PORT.set(port);
    port
}

// ---------------------------------------------------------------------------
// Mock handlers
// ---------------------------------------------------------------------------

async fn health_handler() -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "data": { "status": "healthy" }
    }))
}

#[derive(Deserialize)]
struct AdrSearchParams {
    q: Option<String>,
    status: Option<String>,
}

async fn adr_search_handler(Query(_params): Query<AdrSearchParams>) -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "data": [{
            "id": "ADR-001",
            "title": "Use SQLite for local storage",
            "status": "accepted",
            "date": "2025-01-15",
            "tags": ["database", "storage"],
            "content": "We will use SQLite as the local storage backend because..."
        }]
    }))
}

#[derive(Deserialize)]
struct ConstitutionValidateParams {
    action: Option<String>,
}

async fn constitution_validate_handler(
    Query(_params): Query<ConstitutionValidateParams>,
) -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "data": [{
            "title": "Open Source First",
            "content": "All components must be open-source.",
            "rules": ["Use MIT or Apache 2.0 license"]
        }]
    }))
}

#[derive(Deserialize)]
struct SymbolResolveParams {
    name: Option<String>,
    kind: Option<String>,
}

async fn symbol_resolve_handler(
    Query(_params): Query<SymbolResolveParams>,
) -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "data": [{
            "name": "sqlite_setup",
            "kind": "function",
            "file_path": "kernel/src/db.rs",
            "line": 42,
            "column": 1
        }]
    }))
}

#[derive(Deserialize)]
struct SemanticSearchParams {
    q: Option<String>,
    top_k: Option<u32>,
}

async fn semantic_search_handler(
    Query(_params): Query<SemanticSearchParams>,
) -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "data": [{
            "title": "ADR-001: Use SQLite",
            "content": "We will use SQLite for local storage.",
            "file_path": "adr/001-use-sqlite.md",
            "score": 0.92
        }]
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

fn pil_config() -> PilConfig {
    let port = start_mock_pil();
    PilConfig {
        url: format!("http://127.0.0.1:{}", port),
        timeout_secs: 5,
    }
}

#[tokio::test]
async fn roundtrip_health() {
    let config = pil_config();
    let client = PilClient::new(&config);

    let health = client.health().await.expect("health should succeed");
    assert_eq!(health.status, "healthy");
}

#[tokio::test]
async fn roundtrip_adr_search() {
    let config = pil_config();
    let client = PilClient::new(&config);

    let results = client
        .search_adr("sqlite", Some("accepted"))
        .await
        .expect("ADR search should succeed");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "ADR-001");
    assert_eq!(results[0].status, "accepted");
    assert!(results[0].tags.contains(&"database".to_string()));
}

#[tokio::test]
async fn roundtrip_constitution_validate() {
    let config = pil_config();
    let client = PilClient::new(&config);

    let sections = client
        .validate_constitution("add sqlite dependency")
        .await
        .expect("constitution validate should succeed");

    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].title, "Open Source First");
}

#[tokio::test]
async fn roundtrip_symbol_resolve() {
    let config = pil_config();
    let client = PilClient::new(&config);

    let symbols = client
        .resolve_symbol("sqlite_setup", Some("function"))
        .await
        .expect("symbol resolve should succeed");

    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "sqlite_setup");
    assert_eq!(symbols[0].kind, "function");
}

#[tokio::test]
async fn roundtrip_semantic_search() {
    let config = pil_config();
    let client = PilClient::new(&config);

    let results = client
        .search_semantic("sqlite storage", Some(5))
        .await
        .expect("semantic search should succeed");

    assert_eq!(results.len(), 1);
    assert!(results[0].score > 0.9);
}
