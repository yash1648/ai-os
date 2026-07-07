use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::state_machine::{
    ObjectivePrimaryState, ObjectiveState, ObjectiveTerminalState,
};

/// A discrete unit of engineering work — the atomic unit the Kernel schedules
/// and tracks. (docs/01-philosophy-and-terminology.md, docs/20-json-schemas.md)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Objective {
    pub id: String,
    pub title: String,
    pub description: String,
    pub owner: String,
    pub parent_id: Option<String>,
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
    Minimal,
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
                id               TEXT PRIMARY KEY,
                title            TEXT NOT NULL,
                description      TEXT NOT NULL DEFAULT '',
                owner            TEXT NOT NULL,
                parent_id        TEXT,
                priority         TEXT NOT NULL DEFAULT 'medium',
                status           TEXT NOT NULL DEFAULT 'DISCOVERED',
                dependencies     TEXT NOT NULL DEFAULT '[]',
                success_criteria TEXT NOT NULL DEFAULT '[]',
                plan_id          TEXT,
                retry_count      INTEGER NOT NULL DEFAULT 0,
                tags             TEXT NOT NULL DEFAULT '[]',
                created_at       TEXT NOT NULL,
                updated_at       TEXT NOT NULL
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
                (id, title, description, owner, parent_id, priority, status,
                 dependencies, success_criteria, plan_id, retry_count, tags,
                 created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&obj.id)
        .bind(&obj.title)
        .bind(&obj.description)
        .bind(&obj.owner)
        .bind(&obj.parent_id)
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

    /// Number of objectives currently persisted.
    pub async fn count(&self) -> sqlx::Result<i64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM objectives")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }
}

// ── Internal row type for sqlx ────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct ObjectiveRow {
    id: String,
    title: String,
    description: String,
    owner: String,
    parent_id: Option<String>,
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
            description: self.description,
            owner: self.owner,
            parent_id: self.parent_id,
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

/// Insert a small set of realistic sample objectives so the dashboard is not
/// empty on first boot. Used when the store starts with zero objectives.
pub async fn seed_sample_objectives(store: &ObjectiveStore) -> sqlx::Result<()> {
    let now = chrono::Utc::now();
    let samples = [
        (
            "obj-kernel-api",
            "Expose Kernel HTTP API for objective lifecycle",
            "Build REST endpoints for create/list/get/transition of objectives.",
            "platform",
            Priority::Critical,
            ObjectiveState::Terminal(ObjectiveTerminalState::Done),
            vec!["api", "kernel"],
        ),
        (
            "obj-dashboard",
            "Real-time observability dashboard",
            "htmx dashboard with overview, timeline, objectives, audit, metrics tabs.",
            "platform",
            Priority::High,
            ObjectiveState::Primary(ObjectivePrimaryState::Ready),
            vec!["dashboard", "frontend"],
        ),
        (
            "obj-guardian",
            "Architecture Guardian constitutional enforcement",
            "Compile constitution into machine-checkable policies and enforce on apply.",
            "governance",
            Priority::High,
            ObjectiveState::Primary(ObjectivePrimaryState::Executing),
            vec!["guardian", "policies"],
        ),
        (
            "obj-pil",
            "Project Intelligence Layer indexing",
            "Code graph, dependency graph, and search index persistence.",
            "intelligence",
            Priority::Medium,
            ObjectiveState::Primary(ObjectivePrimaryState::Discovered),
            vec!["pil", "indexing"],
        ),
        (
            "obj-federation",
            "Cross-process worker federation",
            "Distribute worker pools across processes with consensus coordination.",
            "platform",
            Priority::Low,
            ObjectiveState::Terminal(ObjectiveTerminalState::Abandoned),
            vec!["federation", "stage-2"],
        ),
    ];

    for (idx, (id, title, desc, owner, priority, status, tags)) in samples.iter().enumerate() {
        let objective = Objective {
            id: id.to_string(),
            title: title.to_string(),
            description: desc.to_string(),
            owner: owner.to_string(),
            parent_id: None,
            priority: priority.clone(),
            status: status.clone(),
            dependencies: vec![],
            success_criteria: vec![
                "Unit tests pass".to_string(),
                "Reviewed by Guardian".to_string(),
            ],
            plan_id: None,
            retry_count: 0,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            created_at: now - chrono::Duration::minutes(30 * (idx as i64 + 1)),
            updated_at: now - chrono::Duration::minutes(5 * (idx as i64 + 1)),
        };
        store.insert(&objective).await?;
    }
    Ok(())
}

// ── Serialization helpers stored in state_machine.rs ──────────────────────
// (we add from_label there)

// ═════════════════════════════════════════════════════════════════════════
// Tests
// ═════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_machine::{ObjectivePrimaryState, ObjectiveTerminalState};

    /// Create a fresh in-memory SQLite pool with the objectives table initialized.
    async fn init_test_store() -> ObjectiveStore {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("Failed to create in-memory SQLite pool");
        ObjectiveStore::new(pool)
            .await
            .expect("Failed to init objectives table")
    }

    fn sample_objective(id: &str, status: ObjectiveState) -> Objective {
        Objective {
            id: id.to_string(),
            title: "Test Objective".into(),
            description: "A test".into(),
            owner: "test-user".into(),
            parent_id: None,
            priority: Priority::Medium,
            status,
            dependencies: vec![],
            success_criteria: vec!["pass".into()],
            plan_id: None,
            retry_count: 0,
            tags: vec!["test".into()],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    // ── Insert + Get ────────────────────────────────────────────────────

    #[tokio::test]
    async fn store_insert_and_get() {
        let store = init_test_store().await;
        let obj = sample_objective("obj-1", ObjectiveState::Primary(ObjectivePrimaryState::Discovered));
        store.insert(&obj).await.expect("insert failed");

        let fetched = store.get("obj-1").await.expect("get failed");
        assert!(fetched.is_some());
        assert_eq!(fetched.as_ref().unwrap().id, "obj-1");
        assert_eq!(fetched.as_ref().unwrap().title, "Test Objective");
        assert_eq!(fetched.as_ref().unwrap().status.label(), "DISCOVERED");
    }

    #[tokio::test]
    async fn store_get_nonexistent() {
        let store = init_test_store().await;
        let fetched = store.get("nonexistent").await.expect("get failed");
        assert!(fetched.is_none());
    }

    // ── Update status ───────────────────────────────────────────────────

    #[tokio::test]
    async fn store_update_status() {
        let store = init_test_store().await;
        let obj = sample_objective("obj-upd", ObjectiveState::Primary(ObjectivePrimaryState::Discovered));
        store.insert(&obj).await.expect("insert failed");

        let new_status = ObjectiveState::Primary(ObjectivePrimaryState::Ready);
        let updated = store.update_status("obj-upd", &new_status, 0).await.expect("update failed");
        assert!(updated, "update_status should return true when row affected");

        let fetched = store.get("obj-upd").await.expect("get failed").unwrap();
        assert_eq!(fetched.status.label(), "READY");
    }

    #[tokio::test]
    async fn store_update_status_nonexistent() {
        let store = init_test_store().await;
        let new_status = ObjectiveState::Primary(ObjectivePrimaryState::Ready);
        let updated = store.update_status("no-such-id", &new_status, 0).await.expect("update failed");
        assert!(!updated, "update_status on nonexistent id should return false");
    }

    #[tokio::test]
    async fn store_update_status_retry_count() {
        let store = init_test_store().await;
        let obj = sample_objective("obj-retry", ObjectiveState::Primary(ObjectivePrimaryState::Discovered));
        store.insert(&obj).await.expect("insert failed");

        store.update_status("obj-retry", &ObjectiveState::Primary(ObjectivePrimaryState::Ready), 0).await.unwrap();
        store.update_status("obj-retry", &ObjectiveState::Primary(ObjectivePrimaryState::Discovered), 4).await.unwrap();

        let fetched = store.get("obj-retry").await.expect("get failed").unwrap();
        assert_eq!(fetched.retry_count, 4);
    }

    // ── List ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn store_list_all() {
        let store = init_test_store().await;
        store.insert(&sample_objective("a", ObjectiveState::Primary(ObjectivePrimaryState::Discovered))).await.unwrap();
        store.insert(&sample_objective("b", ObjectiveState::Primary(ObjectivePrimaryState::Ready))).await.unwrap();
        store.insert(&sample_objective("c", ObjectiveState::Terminal(ObjectiveTerminalState::Done))).await.unwrap();

        let all = store.list(None).await.expect("list failed");
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn store_list_filtered_by_status() {
        let store = init_test_store().await;
        store.insert(&sample_objective("a", ObjectiveState::Primary(ObjectivePrimaryState::Discovered))).await.unwrap();
        store.insert(&sample_objective("b", ObjectiveState::Primary(ObjectivePrimaryState::Ready))).await.unwrap();
        store.insert(&sample_objective("c", ObjectiveState::Primary(ObjectivePrimaryState::Ready))).await.unwrap();

        let ready = store.list(Some("READY")).await.expect("list failed");
        assert_eq!(ready.len(), 2);
        for obj in &ready {
            assert_eq!(obj.status.label(), "READY");
        }
    }

    #[tokio::test]
    async fn store_list_empty() {
        let store = init_test_store().await;
        let all = store.list(None).await.expect("list failed");
        assert!(all.is_empty());
    }

    // ── List active ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn store_list_active_excludes_terminal() {
        let store = init_test_store().await;
        store.insert(&sample_objective("a", ObjectiveState::Primary(ObjectivePrimaryState::Discovered))).await.unwrap();
        store.insert(&sample_objective("b", ObjectiveState::Primary(ObjectivePrimaryState::Ready))).await.unwrap();
        store.insert(&sample_objective("c", ObjectiveState::Terminal(ObjectiveTerminalState::Done))).await.unwrap();
        store.insert(&sample_objective("d", ObjectiveState::Terminal(ObjectiveTerminalState::Abandoned))).await.unwrap();

        let active = store.list_active().await.expect("list_active failed");
        assert_eq!(active.len(), 2);
        for obj in &active {
            assert!(!obj.status.is_terminal(), "list_active returned a terminal objective");
        }
    }

    #[tokio::test]
    async fn store_list_active_all_terminal() {
        let store = init_test_store().await;
        store.insert(&sample_objective("done-1", ObjectiveState::Terminal(ObjectiveTerminalState::Done))).await.unwrap();
        store.insert(&sample_objective("abandoned-1", ObjectiveState::Terminal(ObjectiveTerminalState::Abandoned))).await.unwrap();

        let active = store.list_active().await.expect("list_active failed");
        assert!(active.is_empty());
    }

    // ── Duplicate insert ────────────────────────────────────────────────

    #[tokio::test]
    async fn store_insert_duplicate_fails() {
        let store = init_test_store().await;
        let obj = sample_objective("dup", ObjectiveState::Primary(ObjectivePrimaryState::Discovered));
        store.insert(&obj).await.expect("first insert failed");
        let result = store.insert(&obj).await;
        assert!(result.is_err(), "duplicate insert should fail (PK constraint)");
    }
}
