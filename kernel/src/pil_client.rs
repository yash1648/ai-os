//! HTTP client for the PIL (Python Intelligence Layer) sidecar.
//!
//! The PIL sidecar runs on a separate port (default 8082) and provides
//! context-aware intelligence: ADR queries, constitution validation,
//! symbol resolution, and semantic search across the workspace.

use std::time::Duration;

use serde::Deserialize;

use crate::config::PilConfig;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Generic PIL API response wrapper.
#[derive(Debug, Deserialize)]
pub struct PilResponse<T> {
    pub success: bool,
    pub data: T,
}

/// ADR search result item.
#[derive(Debug, Deserialize)]
pub struct AdrSearchResult {
    pub id: String,
    pub title: String,
    pub status: String,
    pub date: Option<String>,
    pub tags: Vec<String>,
    pub content: String,
}

/// Constitution section with embedded rules.
#[derive(Debug, Deserialize)]
pub struct ConstitutionSection {
    pub title: String,
    pub content: String,
    pub rules: Vec<String>,
}

/// Symbol definition returned by the PIL symbol graph.
#[derive(Debug, Deserialize)]
pub struct SymbolDef {
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub line: u32,
    pub column: u32,
}

/// Semantic search result item.
#[derive(Debug, Deserialize)]
pub struct SemanticSearchResult {
    pub title: String,
    pub content: String,
    pub file_path: String,
    pub score: f64,
}

/// Health check response from PIL.
#[derive(Debug, Deserialize)]
pub struct PilHealth {
    pub status: String,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// HTTP client for the PIL sidecar.
///
/// All requests have a configurable timeout. Network errors and non-2xx
/// responses are surfaced as [`PilClientError`].
#[derive(Debug, Clone)]
pub struct PilClient {
    client: reqwest::Client,
    base_url: String,
}

/// Errors that can occur during PIL API calls.
#[derive(Debug, thiserror::Error)]
pub enum PilClientError {
    #[error("PIL request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("PIL returned error for {endpoint}: {message}")]
    Api { endpoint: String, message: String },

    #[error("PIL response was missing expected data at {endpoint}")]
    MissingData { endpoint: String },
}

impl PilClient {
    /// Create a new PIL client from configuration.
    pub fn new(config: &PilConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .expect("reqwest Client::builder() call should not fail with default settings");

        let base_url = config.url.trim_end_matches('/').to_string();

        Self { client, base_url }
    }

    /// Check PIL health endpoint.
    pub async fn health(&self) -> Result<PilHealth, PilClientError> {
        let resp = self
            .client
            .get(format!("{}/api/v1/health", self.base_url))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(PilClientError::Api {
                endpoint: "health".into(),
                message: format!("HTTP {}", resp.status()),
            });
        }

        let body: PilResponse<PilHealth> = resp.json().await?;
        Ok(body.data)
    }

    /// Search ADRs by a text query.
    pub async fn search_adr(
        &self,
        query: &str,
        status: Option<&str>,
    ) -> Result<Vec<AdrSearchResult>, PilClientError> {
        let req = self
            .client
            .get(format!("{}/api/v1/adr/search", self.base_url));

        let mut params: Vec<(&str, String)> = Vec::new();
        params.push(("q", query.to_string()));
        if let Some(s) = status {
            params.push(("status", s.to_string()));
        }

        let resp = req.query(&params).send().await?;

        if !resp.status().is_success() {
            return Err(PilClientError::Api {
                endpoint: "adr/search".into(),
                message: format!("HTTP {}", resp.status()),
            });
        }

        let body: PilResponse<Vec<AdrSearchResult>> = resp.json().await?;
        Ok(body.data)
    }

    /// Validate an action or plan against the constitution.
    pub async fn validate_constitution(
        &self,
        action: &str,
    ) -> Result<Vec<ConstitutionSection>, PilClientError> {
        let resp = self
            .client
            .get(format!("{}/api/v1/constitution/validate", self.base_url))
            .query(&[("action", action)])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(PilClientError::Api {
                endpoint: "constitution/validate".into(),
                message: format!("HTTP {}", resp.status()),
            });
        }

        let body: PilResponse<Vec<ConstitutionSection>> = resp.json().await?;
        Ok(body.data)
    }

    /// Resolve a symbol by name.
    pub async fn resolve_symbol(
        &self,
        name: &str,
        kind: Option<&str>,
    ) -> Result<Vec<SymbolDef>, PilClientError> {
        let req = self
            .client
            .get(format!("{}/api/v1/symbol/resolve", self.base_url));

        let mut params: Vec<(&str, String)> = Vec::new();
        params.push(("name", name.to_string()));
        if let Some(k) = kind {
            params.push(("kind", k.to_string()));
        }

        let resp = req.query(&params).send().await?;

        if !resp.status().is_success() {
            return Err(PilClientError::Api {
                endpoint: "symbol/resolve".into(),
                message: format!("HTTP {}", resp.status()),
            });
        }

        let body: PilResponse<Vec<SymbolDef>> = resp.json().await?;
        Ok(body.data)
    }

    /// Semantic search across workspace documents.
    pub async fn search_semantic(
        &self,
        query: &str,
        top_k: Option<u32>,
    ) -> Result<Vec<SemanticSearchResult>, PilClientError> {
        let req = self
            .client
            .get(format!("{}/api/v1/search/semantic", self.base_url));

        let mut params: Vec<(&str, String)> = Vec::new();
        params.push(("q", query.to_string()));
        if let Some(k) = top_k {
            params.push(("top_k", k.to_string()));
        }

        let resp = req.query(&params).send().await?;

        if !resp.status().is_success() {
            return Err(PilClientError::Api {
                endpoint: "search/semantic".into(),
                message: format!("HTTP {}", resp.status()),
            });
        }

        let body: PilResponse<Vec<SemanticSearchResult>> = resp.json().await?;
        Ok(body.data)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_constructs_from_config() {
        let config = PilConfig {
            url: "http://localhost:9999".into(),
            timeout_secs: 5,
        };
        let client = PilClient::new(&config);
        // Base URL should have trailing slash trimmed.
        assert_eq!(client.base_url, "http://localhost:9999");
    }

    #[test]
    fn client_defaults_to_pil_port() {
        let config = PilConfig::default();
        assert_eq!(config.url, "http://127.0.0.1:8082");
        assert_eq!(config.timeout_secs, 30);
    }

    #[test]
    fn deserialize_adr_search_result() {
        let json = r#"{
            "id": "ADR-001",
            "title": "Use SQLite for local storage",
            "status": "accepted",
            "date": "2025-01-15",
            "tags": ["database", "storage"],
            "content": "We will use SQLite..."
        }"#;
        let result: AdrSearchResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.id, "ADR-001");
        assert_eq!(result.status, "accepted");
    }
}
