//! AI-OS Worker Runtime — stateless execution unit
//!
//! Accepts an Execution Manifest, performs work described by each step,
//! and produces a structured `WorkerOutput` with diffs, file changes, and
//! execution metadata. See `docs/06-worker-runtime.md` for the full spec.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

// ---------------------------------------------------------------------------
// WorkerOutput types — matches schemas/worker-output.json
// ---------------------------------------------------------------------------

/// Terminal status of a worker execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerStatus {
    Success,
    Failure,
    Timeout,
}

/// File operation type for a change recorded in the output.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileOperation {
    Create,
    Modify,
    Delete,
}

/// A single file entry in the worker output — path, operation, optional content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub operation: FileOperation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// Structured output produced by a worker after executing a manifest.
/// Conforms to `schemas/worker-output.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerOutput {
    pub worker_id: String,
    pub objective_id: String,
    pub status: WorkerStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    pub files_changed: Vec<FileEntry>,
    pub metadata: Value,
    pub completed_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Manifest types — minimal subset of kernel/src/manifest.rs ExecutionManifest
// ---------------------------------------------------------------------------

/// Step type discrimination — mirrors `kernel::manifest::ManifestStepType`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestStepType {
    NestedManifest,
    ModuleCall,
    Command,
    WorkerCall,
    Gate,
    Test,
    /// Calls an OpenAI-compatible LLM endpoint. The API key and base URL are
    /// read from the `AI_OS_LLM_API_KEY` / `AI_OS_LLM_BASE_URL` environment
    /// variables and never embedded in the manifest or worker output.
    Llm,
}

/// An atomic transformation directive within a manifest group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestStep {
    pub id: String,
    pub step_type: ManifestStepType,
    pub target: Option<String>,
    pub operation: Option<String>,
    pub config: Value,
    pub summary: String,
}

/// A group of steps — units of parallelism or modularity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestGroup {
    pub id: String,
    pub title: String,
    pub steps: Vec<ManifestStep>,
}

/// Execution Manifest — the plan a worker receives and executes.
/// This is a minimal subset of `kernel::manifest::ExecutionManifest` containing
/// the fields a stateless worker needs to do its job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerManifest {
    pub manifest_id: String,
    pub objective_id: String,
    pub title: String,
    #[serde(default)]
    pub groups: Vec<ManifestGroup>,
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    pub worker_type: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Execution logic
// ---------------------------------------------------------------------------

/// Execute all steps in a manifest and produce a `WorkerOutput`.
///
/// - Command steps: run via `tokio::process::Command` with optional timeout.
/// - WorkerCall steps: produce file changes from `config.changes`.
/// - Gate/Test steps: evaluated as pass/fail predicates (stub: always pass).
/// - Other step types: skipped with a metadata note.
pub async fn execute_manifest(manifest: &WorkerManifest) -> WorkerOutput {
    let worker_id = format!("ai-worker-{}", uuid::Uuid::new_v4());
    let start = std::time::Instant::now();

    let mut all_files_changed: Vec<FileEntry> = Vec::new();
    let mut step_results: Vec<Value> = Vec::new();
    let mut overall_status = WorkerStatus::Success;

    for group in &manifest.groups {
        for step in &group.steps {
            // Any step type may declare file changes in its config
            if let Some(changes) = parse_file_changes(&step.config) {
                all_files_changed.extend(changes);
            }

            match step.step_type {
                ManifestStepType::Command => {
                    let cmd = match step.operation.as_deref() {
                        Some(c) if !c.is_empty() => c,
                        _ => {
                            step_results.push(serde_json::json!({
                                "step_id": step.id,
                                "status": "skipped",
                                "reason": "no command in operation field",
                            }));
                            continue;
                        }
                    };

                    let timeout_s = step
                        .config
                        .get("timeout_s")
                        .and_then(|v| v.as_u64());

                    match run_shell_command(cmd, timeout_s).await {
                        Ok(output) => {
                            step_results.push(serde_json::json!({
                                "step_id": step.id,
                                "command": cmd,
                                "status": "ok",
                                "stdout": output,
                            }));
                        }
                        Err(e) => {
                            let err_msg = format!("{:#}", e);
                            let is_timeout = err_msg.contains("timed out");
                            if is_timeout {
                                overall_status = WorkerStatus::Timeout;
                            } else {
                                overall_status = WorkerStatus::Failure;
                            }
                            step_results.push(serde_json::json!({
                                "step_id": step.id,
                                "command": cmd,
                                "status": if is_timeout { "timeout" } else { "error" },
                                "error": err_msg,
                            }));
                        }
                    }
                }
                ManifestStepType::WorkerCall => {
                    step_results.push(serde_json::json!({
                        "step_id": step.id,
                        "status": "ok",
                        "type": "worker_call",
                    }));
                }
                ManifestStepType::Llm => {
                    let prompt = step
                        .config
                        .get("prompt")
                        .and_then(|v| v.as_str())
                        .or_else(|| step.operation.as_deref())
                        .unwrap_or("")
                        .to_string();
                    let model = step
                        .config
                        .get("model")
                        .and_then(|v| v.as_str())
                        .unwrap_or("auto")
                        .to_string();
                    let max_tokens = step
                        .config
                        .get("max_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(512);

                    match llm_call(&prompt, &model, max_tokens).await {
                        Ok(content) => {
                            step_results.push(serde_json::json!({
                                "step_id": step.id,
                                "status": "ok",
                                "type": "llm",
                                "model": model,
                                "content": content,
                            }));
                        }
                        Err(e) => {
                            overall_status = WorkerStatus::Failure;
                            step_results.push(serde_json::json!({
                                "step_id": step.id,
                                "status": "error",
                                "type": "llm",
                                "error": format!("{:#}", e),
                            }));
                        }
                    }
                }
                ManifestStepType::Gate | ManifestStepType::Test => {
                    // Stub: gates and tests always pass
                    step_results.push(serde_json::json!({
                        "step_id": step.id,
                        "status": "ok",
                        "type": "gate_or_test",
                    }));
                }
                _ => {
                    step_results.push(serde_json::json!({
                        "step_id": step.id,
                        "status": "skipped",
                        "type": "other",
                    }));
                }
            }
        }
    }

    let elapsed = start.elapsed();
    let diff = build_diff_from_changes(&all_files_changed);

    WorkerOutput {
        worker_id,
        objective_id: manifest.objective_id.clone(),
        status: overall_status,
        diff,
        files_changed: all_files_changed,
        metadata: serde_json::json!({
            "duration_ms": elapsed.as_millis() as u64,
            "worker_type": manifest.worker_type,
            "steps": step_results,
        }),
        completed_at: Utc::now(),
    }
}

/// Run a shell command and capture its stdout.
///
/// Spawns the platform's default shell (`sh -c` on Unix, `cmd /C` on Windows)
/// and waits for the command to finish.  If `timeout_s` is `Some`, the
/// command is killed after that many seconds.
pub async fn run_shell_command(command: &str, timeout_s: Option<u64>) -> anyhow::Result<String> {
    use anyhow::Context;

    let shell = if cfg!(target_os = "windows") {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };

    let child = tokio::process::Command::new(shell.0)
        .arg(shell.1)
        .arg(command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("failed to spawn shell command")?;

    let output = match timeout_s {
        Some(secs) => {
            let duration = Duration::from_secs(secs);
            tokio::time::timeout(duration, child.wait_with_output())
                .await
                .map_err(|_| anyhow::anyhow!("command timed out after {}s", secs))?
                .context("failed to wait for command")?
        }
        None => child.wait_with_output().await.context("failed to wait for command")?,
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let exit_code = output
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".into());
        anyhow::bail!("command exited with {}: {}", exit_code, stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(stdout)
}

/// Call an OpenAI-compatible chat-completions endpoint.
///
/// Credentials come exclusively from the environment:
/// `AI_OS_LLM_BASE_URL` (default `http://localhost:3001/v1`) and
/// `AI_OS_LLM_API_KEY`. The key is sent only in the `Authorization` header and
/// is never logged, serialized into the worker output, or written anywhere.
pub async fn llm_call(prompt: &str, model: &str, max_tokens: u64) -> anyhow::Result<String> {
    use anyhow::Context;

    let base_url = std::env::var("AI_OS_LLM_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:3001/v1".to_string());
    let api_key = std::env::var("AI_OS_LLM_API_KEY")
        .context("AI_OS_LLM_API_KEY is not set; cannot perform LLM step")?;

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model,
        "messages": [{ "role": "user", "content": prompt }],
        "max_tokens": max_tokens,
    });

    let resp = client
        .post(format!("{}/chat/completions", base_url.trim_end_matches('/')))
        .bearer_auth(&api_key)
        .json(&body)
        .send()
        .await
        .context("LLM request failed")?;

    if !resp.status().is_success() {
        anyhow::bail!("LLM endpoint returned status {}", resp.status());
    }

    let value: Value = resp.json().await.context("failed to parse LLM response")?;
    let content = value
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .to_string();
    Ok(content)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Try to parse a `changes` array from an arbitrary JSON config value.
///
/// The expected shape is `{ "changes": [{ "path": "...", "operation": "...", "content": "..." }] }`.
fn parse_file_changes(config: &Value) -> Option<Vec<FileEntry>> {
    let changes = config.get("changes")?;
    serde_json::from_value(changes.clone()).ok()
}

/// Build a unified-diff-like string from a list of file entries.
///
/// Returns `None` when there are no entries.
pub fn build_diff_from_changes(changes: &[FileEntry]) -> Option<String> {
    if changes.is_empty() {
        return None;
    }

    let mut diff = String::new();
    for entry in changes {
        let header = match entry.operation {
            FileOperation::Create => "+++",
            FileOperation::Modify => "---",
            FileOperation::Delete => "---",
        };
        diff.push_str(&format!("{} {}\n", header, entry.path));
        if let Some(ref content) = entry.content {
            for line in content.lines() {
                diff.push_str(&format!(" {}\n", line));
            }
        }
        diff.push('\n');
    }

    Some(diff)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_operation_serde_snake_case() {
        let json = serde_json::to_string(&FileOperation::Create).unwrap();
        assert_eq!(json, "\"create\"");
        let json = serde_json::to_string(&FileOperation::Modify).unwrap();
        assert_eq!(json, "\"modify\"");
        let json = serde_json::to_string(&FileOperation::Delete).unwrap();
        assert_eq!(json, "\"delete\"");
    }

    #[test]
    fn test_worker_status_serde() {
        let json = serde_json::to_string(&WorkerStatus::Success).unwrap();
        assert_eq!(json, "\"Success\"");
    }

    #[test]
    fn test_manifest_step_type_serde_snake_case() {
        let json = serde_json::to_string(&ManifestStepType::Command).unwrap();
        assert_eq!(json, "\"command\"");
        let json = serde_json::to_string(&ManifestStepType::WorkerCall).unwrap();
        assert_eq!(json, "\"worker_call\"");
    }

    #[test]
    fn test_parse_file_changes_none_when_missing_key() {
        let config = serde_json::json!({"foo": "bar"});
        assert!(parse_file_changes(&config).is_none());
    }

    #[test]
    fn test_parse_file_changes_valid() {
        let config = serde_json::json!({
            "changes": [
                {"path": "file.txt", "operation": "create", "content": "hello"}
            ]
        });
        let changes = parse_file_changes(&config).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "file.txt");
        assert!(matches!(changes[0].operation, FileOperation::Create));
    }

    #[test]
    fn test_build_diff_empty() {
        assert!(build_diff_from_changes(&[]).is_none());
    }

    #[test]
    fn test_build_diff_with_entries() {
        let entries = vec![
            FileEntry {
                path: "new.txt".into(),
                operation: FileOperation::Create,
                content: Some("line1\nline2".into()),
            },
        ];
        let diff = build_diff_from_changes(&entries).unwrap();
        assert!(diff.contains("+++ new.txt"));
        assert!(diff.contains(" line1"));
        assert!(diff.contains(" line2"));
    }

    #[tokio::test]
    async fn test_run_shell_command_echo() {
        let output = run_shell_command("echo hello", None).await.unwrap();
        assert_eq!(output.trim(), "hello");
    }

    #[tokio::test]
    async fn test_run_shell_command_nonzero_exit() {
        let result = run_shell_command("exit 42", None).await;
        assert!(result.is_err());
        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("42") || err.contains("exited with"));
    }

    #[tokio::test]
    async fn test_run_shell_command_timeout() {
        let result = run_shell_command("sleep 10", Some(1)).await;
        assert!(result.is_err());
        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("timed out"));
    }

    #[tokio::test]
    async fn test_execute_manifest_empty_groups() {
        let manifest = WorkerManifest {
            manifest_id: "test".into(),
            objective_id: "obj-1".into(),
            title: "empty test".into(),
            groups: vec![],
            allowed_domains: vec![],
            worker_type: None,
            created_at: Utc::now(),
        };
        let output = execute_manifest(&manifest).await;
        assert!(matches!(output.status, WorkerStatus::Success));
        assert_eq!(output.objective_id, "obj-1");
        assert!(output.files_changed.is_empty());
        assert!(output.diff.is_none());
    }

    #[tokio::test]
    async fn test_execute_manifest_with_command() {
        let manifest = WorkerManifest {
            manifest_id: "test-cmd".into(),
            objective_id: "obj-2".into(),
            title: "command test".into(),
            groups: vec![ManifestGroup {
                id: "g1".into(),
                title: "group 1".into(),
                steps: vec![ManifestStep {
                    id: "s1".into(),
                    step_type: ManifestStepType::Command,
                    target: None,
                    operation: Some("echo hello123".into()),
                    config: serde_json::json!({}),
                    summary: "echo test".into(),
                }],
            }],
            allowed_domains: vec![],
            worker_type: None,
            created_at: Utc::now(),
        };
        let output = execute_manifest(&manifest).await;
        assert!(matches!(output.status, WorkerStatus::Success));
        assert_eq!(output.objective_id, "obj-2");

        // Metadata should contain step results
        let steps = output.metadata["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0]["step_id"], "s1");
        assert_eq!(steps[0]["status"], "ok");
        assert!(steps[0]["stdout"].as_str().unwrap().contains("hello123"));
    }

    #[tokio::test]
    async fn test_worker_manifest_deserialize() {
        let json = r#"{
            "manifest_id": "test",
            "objective_id": "obj-1",
            "title": "test",
            "groups": [],
            "allowed_domains": [],
            "worker_type": null,
            "created_at": "2025-01-01T00:00:00Z"
        }"#;
        let manifest: WorkerManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.manifest_id, "test");
        assert_eq!(manifest.objective_id, "obj-1");
    }
}
