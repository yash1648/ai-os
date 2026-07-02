//! Logging initialization — configures the global tracing subscriber.
//!
//! Call `init_from_config(&config.logging)` once at startup, before any
//! `tracing::info!()` / `tracing::error!()` calls. The returned guard MUST
//! be held for the program's lifetime — dropping it flushes and shuts down
//! the non-blocking file writer.

use crate::config::{LogFormat, LoggingConfig};
use std::path::Path;

/// Initialize the global tracing subscriber from a `LoggingConfig`.
///
/// Supports:
/// - Text (human-readable, default) or JSON (structured) output
/// - stdout-only or stdout + file output via `tracing_appender`
/// - EnvFilter directives (e.g. `ai_os_kernel=debug,warn`) or fallback level
/// - Idempotent: if a subscriber is already set, this is a silent no-op.
pub fn init_logging(config: &LoggingConfig) -> LoggingGuard {
    let filter = build_filter(config);

    if let Some(file_path) = &config.file {
        init_dual_output(file_path, config.format == LogFormat::Json, filter)
    } else {
        init_stdout_only(config.format == LogFormat::Json, filter)
    }
}

/// Build an `EnvFilter` from the config's optional filter directive, falling
/// back to the configured log level.
fn build_filter(config: &LoggingConfig) -> tracing_subscriber::EnvFilter {
    config
        .filter
        .as_ref()
        .and_then(|f| tracing_subscriber::EnvFilter::try_new(f).ok())
        .unwrap_or_else(|| tracing_subscriber::EnvFilter::new(config.level.to_string()))
}

/// Initialize tracing with stdout-only output.
fn init_stdout_only(
    json: bool,
    filter: tracing_subscriber::EnvFilter,
) -> LoggingGuard {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::fmt;

    let layer = if json {
        fmt::layer().json().with_filter(filter).boxed()
    } else {
        fmt::layer().with_filter(filter).boxed()
    };

    let did_init = tracing_subscriber::registry().with(layer).try_init().is_ok();
    LoggingGuard { _guard: None, did_init }
}

/// Initialize tracing with dual stdout + file output.
fn init_dual_output(
    file_path: &str,
    json: bool,
    filter: tracing_subscriber::EnvFilter,
) -> LoggingGuard {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::fmt;

    let (non_blocking, guard) = make_file_appender(file_path);

    let file_layer = if json {
        fmt::layer()
            .json()
            .with_writer(non_blocking.clone())
            .with_filter(filter.clone())
            .boxed()
    } else {
        fmt::layer()
            .with_writer(non_blocking.clone())
            .with_filter(filter.clone())
            .boxed()
    };

    let stdout_layer = if json {
        fmt::layer().json().with_filter(filter).boxed()
    } else {
        fmt::layer().with_filter(filter).boxed()
    };

    let did_init = tracing_subscriber::registry()
        .with(file_layer)
        .with(stdout_layer)
        .try_init()
        .is_ok();

    LoggingGuard { _guard: Some(guard), did_init }
}

/// Create a `tracing_appender::non_blocking` file writer.
fn make_file_appender(
    path: &str,
) -> (
    tracing_appender::non_blocking::NonBlocking,
    tracing_appender::non_blocking::WorkerGuard,
) {
    let parent = Path::new(path)
        .parent()
        .unwrap_or(Path::new("."));
    let file_name = Path::new(path)
        .file_name()
        .unwrap_or_default();

    // Create parent directory if it doesn't exist.
    let _ = std::fs::create_dir_all(parent);

    let file_appender = tracing_appender::rolling::never(parent, file_name);
    tracing_appender::non_blocking(file_appender)
}

/// Guard returned by `init_logging`. Held for the program lifetime.
///
/// When dropped, the inner `WorkerGuard` flushes and shuts down the
/// non-blocking file writer, ensuring no log lines are lost.
#[must_use]
pub struct LoggingGuard {
    _guard: Option<tracing_appender::non_blocking::WorkerGuard>,
    /// Whether the global subscriber was actually installed (false if already set).
    pub did_init: bool,
}

impl LoggingGuard {
    /// Create a no-op guard (useful when logging is disabled in tests).
    pub fn none() -> Self {
        Self { _guard: None, did_init: false }
    }
}

impl Default for LoggingGuard {
    fn default() -> Self {
        Self::none()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LogLevel;

    /// Helper: create a LoggingConfig with the given level.
    fn cfg(level: LogLevel) -> LoggingConfig {
        LoggingConfig {
            level,
            format: LogFormat::Text,
            file: None,
            filter: None,
        }
    }

    #[test]
    fn init_twice_is_idempotent() {
        let _guard = init_logging(&cfg(LogLevel::Info));
        // Second init should not panic (try_init returns Err on duplicate).
        let _guard2 = init_logging(&cfg(LogLevel::Debug));
        // If we reach here, the second init didn't crash.
    }

    #[test]
    fn init_with_json_format() {
        let mut config = cfg(LogLevel::Info);
        config.format = LogFormat::Json;
        let _guard = init_logging(&config);
        // Should not panic.
    }

    #[test]
    fn init_with_filter_directive() {
        let mut config = cfg(LogLevel::Warn);
        config.filter = Some("ai_os_kernel=debug,info".into());
        let _guard = init_logging(&config);
    }

    #[test]
    fn file_output_creates_parent_dir() {
        let dir = std::env::temp_dir().join(format!("ai_os_log_test_{}", std::process::id()));
        let file_path = dir.join("test.log");
        let file_str = file_path.to_str().unwrap().to_string();

        let mut config = cfg(LogLevel::Info);
        config.file = Some(file_str);
        let _guard = init_logging(&config);

        assert!(dir.exists(), "Parent directory should have been created");
        // Clean up.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn guard_drop_flushes() {
        let dir = std::env::temp_dir().join(format!("ai_os_log_guard_{}", std::process::id()));
        let file_path = dir.join("flush.log");
        let file_str = file_path.to_str().unwrap().to_string();

        let mut config = cfg(LogLevel::Trace);
        config.file = Some(file_str.clone());
        config.format = LogFormat::Text;

        let guard = init_logging(&config);

        // If the subscriber was already set by another test, skip the
        // assertion — only one global subscriber can exist per process.
        if !guard.did_init {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }

        // Emit a log line.
        tracing::info!("test log line for guard flush");

        // Drop guard explicitly to flush.
        drop(guard);

        // File should now contain the log line.
        let content = std::fs::read_to_string(&file_path).unwrap_or_default();
        assert!(
            content.contains("test log line for guard flush"),
            "Expected log line in file, got: {content:?}"
        );

        // Clean up.
        let _ = std::fs::remove_dir_all(&dir);
    }
}
