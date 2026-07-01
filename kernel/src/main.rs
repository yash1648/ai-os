// AI-OS Kernel — Stage 1 entry point
//
// Phase 1: Foundation scaffold. For now, just runs tests and exits.
// Phase 4 will wire this into a full daemon with CLI, API server, and
// the end-to-end lifecycle loop.

use ai_os_kernel::state_machine;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ai-os", about = "AI-OS Kernel — deterministic orchestrator")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run kernel daemon (the main entry point in production)
    Serve {
        /// Database URL (default: sqlite://ai-os.db)
        #[arg(long, default_value = "sqlite://ai-os.db")]
        db: String,
    },
    /// Check state machine transitions (read-only validation)
    Validate {
        /// Starting state label
        from: String,
        /// Target state label
        to: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match &cli.command {
        Some(Commands::Serve { db: _ }) => {
            println!("AI-OS Kernel Stage 1 — serve mode coming in Phase 4.");
            println!("For now, run `cargo test` to verify the state machine.");
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
        None => {
            println!("AI-OS Kernel v0.1.0");
            println!("Usage: ai-os <command>");
            println!();
            println!("Commands:");
            println!("  serve      Run the kernel daemon");
            println!("  validate   Check a state machine transition");
        }
    }
}
