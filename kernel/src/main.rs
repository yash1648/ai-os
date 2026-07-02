// AI-OS Kernel — Stage 1 entry point
//
// Phase 1: Foundation scaffold. For now, just runs tests and exits.
// Phase 4 will wire this into a full daemon with CLI, API server, and
// the end-to-end lifecycle loop.

use ai_os_kernel::config::KernelConfig;
use ai_os_kernel::diff_applier::{DiffApplier, StructuredDiff};
use ai_os_kernel::logging;
use ai_os_kernel::state_machine;
use clap::{Parser, Subcommand};
use std::io::Read;
use std::path::PathBuf;

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
            tracing::info!(
                "AI-OS Kernel — serving on {}:{}, db: {}",
                config.server.bind_address,
                config.server.bind_port,
                database_url,
            );
            println!("Serve mode coming in Phase 4.");
            println!("For now, run `cargo test` to verify the kernel modules.");
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
            println!("AI-OS Kernel v0.1.0");
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
