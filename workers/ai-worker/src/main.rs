//! AI-OS Worker Binary
//!
//! Reads an Execution Manifest from a file (or stdin via `-`), executes the
//! manifest steps, and writes structured `WorkerOutput` JSON to stdout.
//!
//! Usage:
//!   ai-worker --manifest <PATH>
//!   cat manifest.json | ai-worker --manifest -
//!
//! Exit codes: 0 on success, 1 on error (errors go to stderr).

use anyhow::Context;
use clap::Parser;
use std::io::Read;

#[derive(Parser)]
#[command(name = "ai-worker", version, about = "AI-OS stateless worker process")]
struct Cli {
    /// Path to Execution Manifest JSON file. Use "-" to read from stdin.
    #[arg(long, short)]
    manifest: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Read manifest JSON from file or stdin
    let manifest_json = if cli.manifest == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("failed to read manifest from stdin")?;
        buf
    } else {
        std::fs::read_to_string(&cli.manifest)
            .with_context(|| format!("failed to read manifest file '{}'", cli.manifest))?
    };

    // Parse manifest
    let manifest: ai_worker::WorkerManifest = serde_json::from_str(&manifest_json)
        .context("failed to parse manifest JSON")?;

    // Execute all steps
    let output = ai_worker::execute_manifest(&manifest).await;

    // Serialize and emit output
    let output_json = serde_json::to_string_pretty(&output)
        .context("failed to serialize worker output")?;
    println!("{}", output_json);

    Ok(())
}
