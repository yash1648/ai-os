//! Kernel Configuration — hierarchical config from TOML files + env overrides.
//!
//! Load order (later overrides earlier):
//!   1. Built-in defaults
//!   2. Config file (TOML) — path from `--config` CLI arg or `AI_OS_CONFIG` env var
//!   3. Individual env var overrides (`AI_OS_*`)
//!
//! # Example config.toml
//!
//! ```toml
//! [database]
//! url = "sqlite://ai-os.db"
//!
//! [server]
//! bind_address = "127.0.0.1"
//! bind_port = 8080
//!
//! [logging]
//! level = "info"
//! format = "json"
//! file = "logs/kernel.json"
//!
//! [scheduler]
//! max_concurrent_objectives = 4
//! max_retries = 3
//!
//! [execution]
//! simulation_delay_ms = 0
//! ```

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during configuration loading or validation.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to read config file {path}: {source}")]
    FileRead {
        path: String,
        source: std::io::Error,
    },
    #[error("Failed to parse config file: {0}")]
    Parse(String),
    #[error("Config validation failed:\n{0}")]
    Validation(String),
    #[error("Environment variable parse error: {0}")]
    EnvParse(String),
}

// ---------------------------------------------------------------------------
// Log level & format enums
// ---------------------------------------------------------------------------

/// Log verbosity levels understood by the kernel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl Default for LogLevel {
    fn default() -> Self {
        Self::Info
    }
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Trace => write!(f, "trace"),
            Self::Debug => write!(f, "debug"),
            Self::Info => write!(f, "info"),
            Self::Warn => write!(f, "warn"),
            Self::Error => write!(f, "error"),
        }
    }
}

impl std::str::FromStr for LogLevel {
    type Err = ConfigError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "trace" => Ok(Self::Trace),
            "debug" => Ok(Self::Debug),
            "info" => Ok(Self::Info),
            "warn" => Ok(Self::Warn),
            "error" => Ok(Self::Error),
            other => Err(ConfigError::EnvParse(format!(
                "Unknown log level '{other}'. Expected one of: trace, debug, info, warn, error"
            ))),
        }
    }
}

/// Log output format.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Human-readable, colored output (development default).
    Text,
    /// Structured JSON lines (production default).
    Json,
}

impl Default for LogFormat {
    fn default() -> Self {
        Self::Text
    }
}

impl std::str::FromStr for LogFormat {
    type Err = ConfigError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            other => Err(ConfigError::EnvParse(format!(
                "Unknown log format '{other}'. Expected: text or json"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// Sub-configs
// ---------------------------------------------------------------------------

/// Database connection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// SQLite/PostgreSQL connection URL.
    #[serde(default = "default_database_url")]
    pub url: String,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: default_database_url(),
        }
    }
}

fn default_database_url() -> String {
    "sqlite://ai-os.db".to_string()
}

/// HTTP server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Bind address for the HTTP API.
    #[serde(default = "default_bind_address")]
    pub bind_address: String,
    /// Bind port for the HTTP API.
    #[serde(default = "default_bind_port")]
    pub bind_port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_address: default_bind_address(),
            bind_port: default_bind_port(),
        }
    }
}

fn default_bind_address() -> String {
    "127.0.0.1".to_string()
}

fn default_bind_port() -> u16 {
    8080
}

/// Scheduler / concurrency configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerConfig {
    /// Maximum number of objectives executing concurrently.
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_objectives: usize,
    /// Maximum retry attempts before abandoning an objective.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_concurrent_objectives: default_max_concurrent(),
            max_retries: default_max_retries(),
        }
    }
}

fn default_max_concurrent() -> usize {
    4
}

fn default_max_retries() -> u32 {
    3
}

/// Event bus configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventBusConfig {
    /// Capacity of the in-process broadcast channel.
    #[serde(default = "default_event_bus_capacity")]
    pub capacity: usize,
}

impl Default for EventBusConfig {
    fn default() -> Self {
        Self {
            capacity: default_event_bus_capacity(),
        }
    }
}

fn default_event_bus_capacity() -> usize {
    4096
}

/// Logging / observability configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Minimum log level to emit.
    #[serde(default)]
    pub level: LogLevel,
    /// Output format.
    #[serde(default)]
    pub format: LogFormat,
    /// Optional file path for log output. When set, logs are written both
    /// to stdout and to this file.
    pub file: Option<String>,
    /// Log filter directives (e.g. "ai_os_kernel=debug,warn").
    /// Overrides `level` when set.
    pub filter: Option<String>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::default(),
            format: LogFormat::default(),
            file: None,
            filter: None,
        }
    }
}

/// Ownership / domain configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnershipConfig {
    /// Path to the ownership YAML file.
    pub config_path: Option<String>,
    /// Whether to enforce ownership restrictions (can be disabled for Stage 1).
    #[serde(default = "default_enforce_ownership")]
    pub enforce: bool,
}

impl Default for OwnershipConfig {
    fn default() -> Self {
        Self {
            config_path: None,
            enforce: default_enforce_ownership(),
        }
    }
}

fn default_enforce_ownership() -> bool {
    true
}

/// PIL (Python Intelligence Layer) sidecar connection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PilConfig {
    /// Base URL of the PIL sidecar.
    #[serde(default = "default_pil_url")]
    pub url: String,
    /// Request timeout in seconds.
    #[serde(default = "default_pil_timeout")]
    pub timeout_secs: u64,
}

impl Default for PilConfig {
    fn default() -> Self {
        Self {
            url: default_pil_url(),
            timeout_secs: default_pil_timeout(),
        }
    }
}

fn default_pil_url() -> String {
    "http://127.0.0.1:8082".to_string()
}

fn default_pil_timeout() -> u64 {
    30
}

/// Worker execution configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionConfig {
    /// How long (ms) the simulated worker sleeps before returning.
    #[serde(default = "default_simulation_delay")]
    pub simulation_delay_ms: u64,
    /// Objective IDs that should simulate failure.
    #[serde(default)]
    pub fail_objective_ids: Vec<String>,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            simulation_delay_ms: default_simulation_delay(),
            fail_objective_ids: vec![],
        }
    }
}

fn default_simulation_delay() -> u64 {
    0
}

// ---------------------------------------------------------------------------
// Top-level KernelConfig
// ---------------------------------------------------------------------------

/// Complete kernel configuration, typically loaded from `config.toml` with
/// environment variable overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernelConfig {
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub scheduler: SchedulerConfig,
    #[serde(default)]
    pub event_bus: EventBusConfig,
    #[serde(default)]
    pub ownership: OwnershipConfig,
    #[serde(default)]
    pub execution: ExecutionConfig,
    /// Workspace root directory for all file operations.
    #[serde(default = "default_workspace_root")]
    pub workspace_root: String,
    /// Path to an optional config file (used for display/cli round-tripping).
    #[serde(skip)]
    pub config_path: Option<String>,
}

impl Default for KernelConfig {
    fn default() -> Self {
        Self {
            database: DatabaseConfig::default(),
            server: ServerConfig::default(),
            logging: LoggingConfig::default(),
            scheduler: SchedulerConfig::default(),
            event_bus: EventBusConfig::default(),
            ownership: OwnershipConfig::default(),
            execution: ExecutionConfig::default(),
            workspace_root: default_workspace_root(),
            config_path: None,
        }
    }
}

fn default_workspace_root() -> String {
    ".".to_string()
}

impl KernelConfig {
    /// Load configuration from a TOML file path.
    ///
    /// Returns default config when `path` is `None`.
    pub fn load(path: Option<&str>) -> Result<Self, ConfigError> {
        let mut config = match path {
            Some(p) if !p.is_empty() => Self::from_file(p)?,
            _ => Self::default(),
        };
        config.config_path = path.map(|p| p.to_string());

        // Apply env var overrides.
        config.apply_env_overrides()?;

        // Validate.
        config.validate()?;

        Ok(config)
    }

    /// Load configuration from a TOML file.
    fn from_file(path: &str) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path).map_err(|e| ConfigError::FileRead {
            path: path.to_string(),
            source: e,
        })?;
        Self::from_toml(&content)
    }

    /// Parse configuration from a TOML string.
    pub fn from_toml(toml_str: &str) -> Result<Self, ConfigError> {
        toml::from_str(toml_str).map_err(|e| ConfigError::Parse(e.to_string()))
    }

    /// Apply environment variable overrides.
    ///
    /// Uses the `AI_OS_` prefix:
    /// - `AI_OS_DATABASE_URL`
    /// - `AI_OS_BIND_ADDRESS`, `AI_OS_BIND_PORT`
    /// - `AI_OS_LOG_LEVEL`, `AI_OS_LOG_FORMAT`, `AI_OS_LOG_FILE`, `AI_OS_LOG_FILTER`
    /// - `AI_OS_WORKSPACE_ROOT`
    /// - `AI_OS_MAX_CONCURRENT`, `AI_OS_MAX_RETRIES`
    /// - `AI_OS_EVENT_BUS_CAPACITY`
    /// - `AI_OS_OWNERSHIP_CONFIG`, `AI_OS_ENFORCE_OWNERSHIP`
    /// - `AI_OS_EXECUTION_SIMULATION_DELAY_MS`
    fn apply_env_overrides(&mut self) -> Result<(), ConfigError> {
        if let Some(val) = std::env::var("AI_OS_DATABASE_URL").ok().filter(|v| !v.is_empty()) {
            self.database.url = val;
        }
        if let Some(val) = std::env::var("AI_OS_BIND_ADDRESS").ok().filter(|v| !v.is_empty()) {
            self.server.bind_address = val;
        }
        if let Some(val) = std::env::var("AI_OS_BIND_PORT").ok().filter(|v| !v.is_empty()) {
            self.server.bind_port = val.parse::<u16>().map_err(|e| {
                ConfigError::EnvParse(format!("AI_OS_BIND_PORT: {e}"))
            })?;
        }
        if let Some(val) = std::env::var("AI_OS_LOG_LEVEL").ok().filter(|v| !v.is_empty()) {
            self.logging.level = val.parse::<LogLevel>()?;
        }
        if let Some(val) = std::env::var("AI_OS_LOG_FORMAT").ok().filter(|v| !v.is_empty()) {
            self.logging.format = val.parse::<LogFormat>()?;
        }
        if let Some(val) = std::env::var("AI_OS_LOG_FILE").ok().filter(|v| !v.is_empty()) {
            self.logging.file = Some(val);
        }
        if let Some(val) = std::env::var("AI_OS_LOG_FILTER").ok().filter(|v| !v.is_empty()) {
            self.logging.filter = Some(val);
        }
        if let Some(val) = std::env::var("AI_OS_WORKSPACE_ROOT").ok().filter(|v| !v.is_empty()) {
            self.workspace_root = val;
        }
        if let Some(val) = std::env::var("AI_OS_MAX_CONCURRENT").ok().filter(|v| !v.is_empty()) {
            self.scheduler.max_concurrent_objectives = val.parse::<usize>().map_err(|e| {
                ConfigError::EnvParse(format!("AI_OS_MAX_CONCURRENT: {e}"))
            })?;
        }
        if let Some(val) = std::env::var("AI_OS_MAX_RETRIES").ok().filter(|v| !v.is_empty()) {
            self.scheduler.max_retries = val.parse::<u32>().map_err(|e| {
                ConfigError::EnvParse(format!("AI_OS_MAX_RETRIES: {e}"))
            })?;
        }
        if let Some(val) = std::env::var("AI_OS_EVENT_BUS_CAPACITY").ok().filter(|v| !v.is_empty()) {
            self.event_bus.capacity = val.parse::<usize>().map_err(|e| {
                ConfigError::EnvParse(format!("AI_OS_EVENT_BUS_CAPACITY: {e}"))
            })?;
        }
        if let Some(val) = std::env::var("AI_OS_OWNERSHIP_CONFIG").ok().filter(|v| !v.is_empty()) {
            self.ownership.config_path = Some(val);
        }
        if let Some(val) = std::env::var("AI_OS_ENFORCE_OWNERSHIP").ok().filter(|v| !v.is_empty()) {
            self.ownership.enforce = val.parse::<bool>().map_err(|e| {
                ConfigError::EnvParse(format!("AI_OS_ENFORCE_OWNERSHIP: {e}"))
            })?;
        }
        if let Some(val) = std::env::var("AI_OS_EXECUTION_SIMULATION_DELAY_MS")
            .ok()
            .filter(|v| !v.is_empty())
        {
            self.execution.simulation_delay_ms = val.parse::<u64>().map_err(|e| {
                ConfigError::EnvParse(format!("AI_OS_EXECUTION_SIMULATION_DELAY_MS: {e}"))
            })?;
        }
        Ok(())
    }

    /// Validate the configuration, returning an error listing all issues.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let mut errors: Vec<String> = Vec::new();

        // Database URL must not be empty.
        if self.database.url.trim().is_empty() {
            errors.push("database.url must not be empty".to_string());
        }

        // Bind port must be in valid range.
        if self.server.bind_port == 0 {
            errors.push("server.bind_port must be > 0".to_string());
        }

        // Workspace root must be non-empty.
        if self.workspace_root.trim().is_empty() {
            errors.push("workspace_root must not be empty".to_string());
        }

        // Max concurrent must be at least 1.
        if self.scheduler.max_concurrent_objectives == 0 {
            errors.push("scheduler.max_concurrent_objectives must be >= 1".to_string());
        }

        // Validate ownership config path if provided.
        if let Some(ref path) = self.ownership.config_path {
            let p = PathBuf::from(path);
            if !p.exists() {
                errors.push(format!(
                    "ownership.config_path '{}' does not exist",
                    path
                ));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ConfigError::Validation(errors.join("\n")))
        }
    }

    /// Whether the config was loaded from a file (vs. pure defaults).
    pub fn has_config_file(&self) -> bool {
        self.config_path.is_some()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn sample_toml() -> &'static str {
        r#"
workspace_root = "/tmp/test-workspace"

[database]
url = "sqlite://test.db"

[server]
bind_address = "0.0.0.0"
bind_port = 9090

[logging]
level = "debug"
format = "json"
file = "logs/kernel.json"

[scheduler]
max_concurrent_objectives = 8
max_retries = 5

[event_bus]
capacity = 2048

[ownership]
config_path = "./ownership.yaml"
enforce = true

[execution]
simulation_delay_ms = 500
"#
    }

    #[test]
    fn load_from_toml() {
        let config = KernelConfig::from_toml(sample_toml()).unwrap();
        assert_eq!(config.database.url, "sqlite://test.db");
        assert_eq!(config.server.bind_address, "0.0.0.0");
        assert_eq!(config.server.bind_port, 9090);
        assert_eq!(config.logging.level, LogLevel::Debug);
        assert_eq!(config.logging.format, LogFormat::Json);
        assert_eq!(config.logging.file, Some("logs/kernel.json".into()));
        assert_eq!(config.scheduler.max_concurrent_objectives, 8);
        assert_eq!(config.scheduler.max_retries, 5);
        assert_eq!(config.event_bus.capacity, 2048);
        assert_eq!(
            config.ownership.config_path,
            Some("./ownership.yaml".into())
        );
        assert!(config.ownership.enforce);
        assert_eq!(config.workspace_root, "/tmp/test-workspace");
        assert_eq!(config.execution.simulation_delay_ms, 500);
        assert!(config.execution.fail_objective_ids.is_empty());
    }

    #[test]
    fn load_partial_toml_uses_defaults() {
        let toml = r#"
workspace_root = "/custom/workspace"
"#;
        let config = KernelConfig::from_toml(toml).unwrap();
        // Provided field should be set.
        assert_eq!(config.workspace_root, "/custom/workspace");
        // Omitted fields should fall back to defaults.
        assert_eq!(config.database.url, "sqlite://ai-os.db");
        assert_eq!(config.server.bind_address, "127.0.0.1");
        assert_eq!(config.server.bind_port, 8080);
        assert_eq!(config.logging.level, LogLevel::Info);
        assert_eq!(config.scheduler.max_concurrent_objectives, 4);
        assert_eq!(config.event_bus.capacity, 4096);
    }

    #[test]
    fn load_default() {
        let config = KernelConfig::default();
        assert_eq!(config.database.url, "sqlite://ai-os.db");
        assert_eq!(config.server.bind_address, "127.0.0.1");
        assert_eq!(config.server.bind_port, 8080);
        assert_eq!(config.logging.level, LogLevel::Info);
        assert_eq!(config.scheduler.max_concurrent_objectives, 4);
        assert_eq!(config.event_bus.capacity, 4096);
        assert!(config.ownership.enforce);
        assert!(!config.has_config_file());
    }

    #[test]
    #[serial]
    fn env_overrides_apply() {
        // Set env vars atomically for this test.
        unsafe {
            std::env::set_var("AI_OS_DATABASE_URL", "sqlite://env-override.db");
            std::env::set_var("AI_OS_BIND_PORT", "3000");
            std::env::set_var("AI_OS_LOG_LEVEL", "warn");
            std::env::set_var("AI_OS_MAX_CONCURRENT", "16");
            std::env::set_var("AI_OS_ENFORCE_OWNERSHIP", "false");
        }

        let mut config = KernelConfig::default();
        config.apply_env_overrides().unwrap();

        assert_eq!(config.database.url, "sqlite://env-override.db");
        assert_eq!(config.server.bind_port, 3000);
        assert_eq!(config.logging.level, LogLevel::Warn);
        assert_eq!(config.scheduler.max_concurrent_objectives, 16);
        assert!(!config.ownership.enforce);

        // Clean up env vars.
        unsafe {
            std::env::remove_var("AI_OS_DATABASE_URL");
            std::env::remove_var("AI_OS_BIND_PORT");
            std::env::remove_var("AI_OS_LOG_LEVEL");
            std::env::remove_var("AI_OS_MAX_CONCURRENT");
            std::env::remove_var("AI_OS_ENFORCE_OWNERSHIP");
        }
    }

    #[test]
    fn validate_rejects_empty_database_url() {
        let config = KernelConfig {
            database: DatabaseConfig {
                url: "".to_string(),
            },
            ..Default::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("database.url"));
    }

    #[test]
    fn validate_rejects_zero_bind_port() {
        let config = KernelConfig {
            server: ServerConfig {
                bind_port: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("bind_port"));
    }

    #[test]
    fn validate_rejects_zero_max_concurrent() {
        let config = KernelConfig {
            scheduler: SchedulerConfig {
                max_concurrent_objectives: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("max_concurrent_objectives"));
    }

    #[test]
    fn validate_passes_for_valid_config() {
        // Use a config with no custom ownership path (avoids file-existence check).
        let toml = r#"
[database]
url = "sqlite://valid.db"

[server]
bind_address = "0.0.0.0"
bind_port = 3000

workspace_root = "/tmp/valid-workspace"
"#;
        let config = KernelConfig::from_toml(toml).unwrap();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn log_level_from_str() {
        assert_eq!("trace".parse::<LogLevel>().unwrap(), LogLevel::Trace);
        assert_eq!("DEBUG".parse::<LogLevel>().unwrap(), LogLevel::Debug);
        assert_eq!("Info".parse::<LogLevel>().unwrap(), LogLevel::Info);
        assert_eq!("warn".parse::<LogLevel>().unwrap(), LogLevel::Warn);
        assert_eq!("error".parse::<LogLevel>().unwrap(), LogLevel::Error);
        assert!("unknown".parse::<LogLevel>().is_err());
    }

    #[test]
    fn log_format_from_str() {
        assert_eq!("text".parse::<LogFormat>().unwrap(), LogFormat::Text);
        assert_eq!("JSON".parse::<LogFormat>().unwrap(), LogFormat::Json);
        assert!("blob".parse::<LogFormat>().is_err());
    }

    #[test]
    #[serial]
    fn load_returns_default_for_none_path() {
        let config = KernelConfig::load(None).unwrap();
        assert!(!config.has_config_file());
        assert_eq!(config.server.bind_port, 8080);
    }
}
