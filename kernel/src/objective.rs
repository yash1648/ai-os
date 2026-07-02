use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::state_machine::{ObjectiveState, ObjectiveTerminalState};

/// A discrete unit of engineering work — the atomic unit the Kernel schedules
/// and tracks. (docs/01-philosophy-and-terminology.md, docs/20-json-schemas.md)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Objective {
    pub id: String,
    pub title: String,
    pub owner: String,
    pub priority: Priority,
    pub status: ObjectiveState,
    pub dependencies: Vec<String>,
    pub success_criteria: Vec<String>,
    pub plan_id: Option<String>,
    pub retry_count: u32,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Medium,
    High,
    Critical,
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

/// Repository for Objective persistence.
///
/// Stage 1 uses SQLite via sqlx. The interface is a plain struct of methods
/// so that in later stages a trait can be extracted for PostgreSQL.
#[derive(Debug, Clone)]
pub struct ObjectiveStore {
    pool: SqlitePool,
}

impl ObjectiveStore {
    /// Create the objectives table if it does not exist.
    pub async fn new(pool: SqlitePool) -> sqlx::Result<Self> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS objectives (
                id              TEXT PRIMARY KEY,
                title           TEXT NOT NULL,
                owner           TEXT NOT NULL,
                priority        TEXT NOT NULL DEFAULT 'medium',
                status          TEXT NOT NULL DEFAULT 'DISCOVERED',
                dependencies    TEXT NOT NULL DEFAULT '[]',
                success_criteria TEXT NOT NULL DEFAULT '[]',
                plan_id         TEXT,
                retry_count     INTEGER NOT NULL DEFAULT 0,
                tags            TEXT NOT NULL DEFAULT '[]',
                created_at      TEXT NOT NULL,
                updated_at      TEXT NOT NULL
            )
            "#,
        )
        .execute(&pool)
        .await?;

        Ok(Self { pool })
    }

    /// Insert a new objective.
    pub async fn insert(&self, obj: &Objective) -> sqlx::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO objectives
                (id, title, owner, priority, status, dependencies,
                 success_criteria, plan_id, retry_count, tags, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&obj.id)
        .bind(&obj.title)
        .bind(&obj.owner)
        .bind(serde_json::to_string(&obj.priority).unwrap_or_default())
        .bind(obj.status.label())
        .bind(serde_json::to_string(&obj.dependencies).unwrap_or_default())
        .bind(serde_json::to_string(&obj.success_criteria).unwrap_or_default())
        .bind(&obj.plan_id)
        .bind(obj.retry_count)
        .bind(serde_json::to_string(&obj.tags).unwrap_or_default())
        .bind(obj.created_at.to_rfc3339())
        .bind(obj.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Retrieve an objective by ID.
    pub async fn get(&self, id: &str) -> sqlx::Result<Option<Objective>> {
        let row = sqlx::query_as::<_, ObjectiveRow>(
            "SELECT * FROM objectives WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(ObjectiveRow::into_objective))
    }

    /// Update an objective's status and retry count in a single transaction.
    pub async fn update_status(
        &self,
        id: &str,
        new_status: &ObjectiveState,
        retry_count: u32,
    ) -> sqlx::Result<bool> {
        let rows = sqlx::query(
            r#"
            UPDATE objectives
            SET status = ?, retry_count = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(new_status.label())
        .bind(retry_count)
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .execute(&self.pool)
        .await?
        .rows_affected();

        Ok(rows > 0)
    }

    /// List all objectives, optionally filtered by status.
    pub async fn list(&self, status_filter: Option<&str>) -> sqlx::Result<Vec<Objective>> {
        let rows = if let Some(status) = status_filter {
            sqlx::query_as::<_, ObjectiveRow>(
                "SELECT * FROM objectives WHERE status = ? ORDER BY created_at DESC",
            )
            .bind(status)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, ObjectiveRow>(
                "SELECT * FROM objectives ORDER BY created_at DESC",
            )
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows.into_iter().map(ObjectiveRow::into_objective).collect())
    }

    /// List all non-terminal objectives.
    pub async fn list_active(&self) -> sqlx::Result<Vec<Objective>> {
        let rows = sqlx::query_as::<_, ObjectiveRow>(
            "SELECT * FROM objectives WHERE status NOT IN ('DONE', 'ABANDONED') ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(ObjectiveRow::into_objective).collect())
    }
}

// ── Internal row type for sqlx ────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct ObjectiveRow {
    id: String,
    title: String,
    owner: String,
    priority: String,
    status: String,
    dependencies: String,
    success_criteria: String,
    plan_id: Option<String>,
    retry_count: u32,
    tags: String,
    created_at: String,
    updated_at: String,
}

impl ObjectiveRow {
    fn into_objective(self) -> Objective {
        // Parse stored JSON arrays; fall back to empty on malformed data
        let parse_json_array = |s: &str| -> Vec<String> {
            serde_json::from_str(s).unwrap_or_default()
        };

        let de = |s: &str| -> Priority {
            serde_json::from_str(s).unwrap_or(Priority::Medium)
        };

        let parse_state = |s: &str| -> Option<ObjectiveState> {
            Some(match s {
                "DISCOVERED" => ObjectiveState::from_label(s),
                "PLANNED" => ObjectiveState::from_label(s),
                "READY" => ObjectiveState::from_label(s),
                "EXECUTING" => ObjectiveState::from_label(s),
                "REVIEW" => ObjectiveState::from_label(s),
                "INTEGRATION" => ObjectiveState::from_label(s),
                "DONE" => ObjectiveState::from_label(s),
                "PLANNING_FAILURE" => ObjectiveState::from_label(s),
                "PERMISSION_FAILURE" => ObjectiveState::from_label(s),
                "EXECUTION_FAILURE" => ObjectiveState::from_label(s),
                "REVIEW_FAILURE" => ObjectiveState::from_label(s),
                "INTEGRATION_FAILURE" => ObjectiveState::from_label(s),
                "HUMAN_REJECTED" => ObjectiveState::from_label(s),
                "ROLLBACK" => ObjectiveState::from_label(s),
                "ABANDONED" => ObjectiveState::from_label(s),
                _ => return None,
            })
        };

        Objective {
            id: self.id,
            title: self.title,
            owner: self.owner,
            priority: de(&self.priority),
            status: parse_state(&self.status)
                .unwrap_or(ObjectiveState::Terminal(ObjectiveTerminalState::Abandoned)),
            dependencies: parse_json_array(&self.dependencies),
            success_criteria: parse_json_array(&self.success_criteria),
            plan_id: self.plan_id,
            retry_count: self.retry_count,
            tags: parse_json_array(&self.tags),
            created_at: DateTime::parse_from_rfc3339(&self.created_at)
                .map(|d| d.to_utc())
                .unwrap_or_else(|_| Utc::now()),
            updated_at: DateTime::parse_from_rfc3339(&self.updated_at)
                .map(|d| d.to_utc())
                .unwrap_or_else(|_| Utc::now()),
        }
    }
}

// ── Serialization helpers stored in state_machine.rs ──────────────────────
// (we add from_label there)
