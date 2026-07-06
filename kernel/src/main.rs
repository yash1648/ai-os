// AI-OS Kernel — Stage 1 entry point
//
// Phase 1: Foundation scaffold. For now, just runs tests and exits.
// Phase 4 will wire this into a full daemon with CLI, API server, and
// the end-to-end lifecycle loop.

use ai_os_kernel::api::AppState;
use ai_os_kernel::config::KernelConfig;
use ai_os_kernel::dashboard;
use ai_os_kernel::config::SchedulerConfig;
use ai_os_kernel::coordinator::Coordinator;
use ai_os_kernel::diff_applier::{DiffApplier, StructuredDiff};
use ai_os_kernel::execution_engine::WorkerConfig;
use ai_os_kernel::event_bus::EventBus;
use ai_os_kernel::logging;
use ai_os_kernel::objective::ObjectiveStore;
use ai_os_kernel::scheduler::Scheduler;
use ai_os_kernel::state_machine;
use clap::{Parser, Subcommand};
use metrics_exporter_prometheus::PrometheusBuilder;
use std::io::Read;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "ai-os", about = "AI-OS Kernel — deterministic orchestrator")]
struct Cli {
    /// Path to config file (TOML). Falls back to defaults + env vars when omitted.
    #[arg(long, short = 'c', global = true)]
    config: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run kernel daemon (the main entry point in production)
    Serve {
        /// Database URL (default: from config or sqlite://ai-os.db)
        #[arg(long)]
        db: Option<String>,
    },
    /// Check state machine transitions (read-only validation)
    Validate {
        /// Starting state label
        from: String,
        /// Target state label
        to: String,
    },
    /// Preview a structured diff without applying it
    DryRun {
        /// Path to a JSON file containing a StructuredDiff
        diff: String,
    },
    /// Apply a structured diff to the workspace
    Apply {
        /// Path to a JSON file containing a StructuredDiff
        diff: String,
        #[arg(long, default_value = ".")]
        /// Workspace root directory
        workspace: String,
    },
}

fn load_diff(path: &str) -> Result<StructuredDiff, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("Cannot open {path}: {e}"))?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(|e| format!("Cannot read {path}: {e}"))?;
    serde_json::from_str(&contents).map_err(|e| format!("Invalid diff JSON: {e}"))
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let config = KernelConfig::load(cli.config.as_deref())
        .unwrap_or_else(|e| {
            eprintln!("Configuration error:\n{e}");
            std::process::exit(1);
        });

    let _log_guard = logging::init_logging(&config.logging);

    if config.has_config_file() {
        tracing::info!("Loaded config from: {}", config.config_path.as_deref().unwrap());
    }

    match &cli.command {
        Some(Commands::Serve { db }) => {
            let database_url = db.clone().unwrap_or_else(|| config.database.url.clone());
            let bind_addr: SocketAddr = format!(
                "{}:{}",
                config.server.bind_address,
                config.server.bind_port,
            )
            .parse()
            .expect("Invalid bind address in config");

            tracing::info!(
                "AI-OS Kernel — serving on {}:{}, db: {}",
                config.server.bind_address,
                config.server.bind_port,
                database_url,
            );

            let scheduler_cfg = SchedulerConfig {
                max_concurrent_objectives: config.scheduler.max_concurrent_objectives,
                max_retries: config.scheduler.max_retries,
            };

            let event_bus = EventBus::new();
            let scheduler = Scheduler::new(scheduler_cfg);

            // Initialize persistent store
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(5)
                .connect(&database_url)
                .await
                .expect("Failed to connect to database");
            let objective_store = ObjectiveStore::new(pool.clone())
                .await
                .expect("Failed to initialize objective store");
            let objective_store_arc = Arc::new(objective_store);
            let scheduler_arc = Arc::new(tokio::sync::Mutex::new(scheduler));

            let worker_config = WorkerConfig {
                simulation_delay_ms: config.execution.simulation_delay_ms,
                fail_objective_ids: config.execution.fail_objective_ids.clone(),
            };

            let coordinator = Coordinator::new()
                .with_event_bus(Arc::new(event_bus.clone()))
                .with_objective_store(objective_store_arc.clone())
                .with_scheduler(scheduler_arc.clone())
                .with_worker_config(worker_config);

let metrics_handle = PrometheusBuilder::new()
                .install_recorder()
                .expect("Failed to install Prometheus recorder");

            let state = Arc::new(AppState {
                config: config.clone(),
                scheduler: scheduler_arc,
                coordinator: tokio::sync::Mutex::new(coordinator),
                event_bus: event_bus.clone(),
                objective_store: objective_store_arc,
                started_at: chrono::Utc::now(),
                pool: pool.clone(),
                metrics_handle,
            });

            // Initialize audit table and spawn background consumer task.
            dashboard::init_audit_table(&pool)
                .await
                .expect("Failed to init audit table");
            let audit_bus = event_bus.clone();
            let audit_pool = pool.clone();
            tokio::spawn(async move {
                dashboard::audit_consumer_task(audit_bus, audit_pool).await;
            });

            let app = ai_os_kernel::api::router(state);

            tracing::info!("API server listening on {bind_addr}");
            let listener = tokio::net::TcpListener::bind(&bind_addr)
                .await
                .expect("Failed to bind TCP listener");
            axum::serve(listener, app)
                .await
                .expect("Server exited with error");
        }
        Some(Commands::Validate { from, to }) => {
            let current = state_machine::ObjectiveState::from_label(from);
            let target = state_machine::ObjectiveState::from_label(to);
            let policy = state_machine::RetryPolicy::default();

            match state_machine::transition(current, target, &policy, 0) {
                Ok(state) => println!("✅  {from} → {} [allowed]", state.label()),
                Err(e) => println!("❌  {from} → {to} [denied]: {e}"),
            }
        }
        Some(Commands::DryRun { diff }) => {
            let diff = match load_diff(diff) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            };
            let applier = DiffApplier::new(PathBuf::from("."));
            match applier.dry_run(&diff) {
                Ok(outcome) => println!("{:#?}", outcome),
                Err(e) => eprintln!("Dry-run failed: {e}"),
            }
        }
        Some(Commands::Apply { diff, workspace }) => {
            let diff = match load_diff(diff) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            };
            let applier = DiffApplier::new(PathBuf::from(workspace));
            match applier.apply(&diff) {
                Ok((outcome, snapshot)) => {
                    println!("Applied: {:#?}", outcome);
                    // Keep snapshot in memory — in a real run we'd stash it
                    // for potential rollback. For now just acknowledge it.
                    let _ = snapshot;
                }
                Err(e) => eprintln!("Apply failed: {e}"),
            }
        }
        None => {
            println!("AI-OS Kernel v{}", env!("CARGO_PKG_VERSION"));
            println!("Usage: ai-os <command>");
            println!();
            println!("Commands:");
            println!("  serve      Run the kernel daemon");
            println!("  validate   Check a state machine transition");
            println!("  dry-run    Preview a structured diff");
            println!("  apply      Apply a structured diff to the workspace");
        }
    }
}
