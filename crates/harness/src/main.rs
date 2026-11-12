use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};

use harness::run::{run, RunConfig};

#[derive(Parser)]
#[command(
    name = "le-harness",
    version,
    about = "AI agent harness: session core, in-memory secrets, confined subprocess tools"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start an interactive session on a local repository.
    Run {
        /// Path to the local repository root (must contain trusted.txt and
        /// sensitives.txt).
        #[arg(long, value_name = "PATH")]
        repo: PathBuf,
        /// External tool binary to forward input lines to (line-JSON protocol).
        #[arg(long, value_name = "BINARY")]
        tool: Option<PathBuf>,
        /// Optional NAME=value secrets file (in-memory only, never written).
        #[arg(long, value_name = "PATH")]
        secrets_file: Option<PathBuf>,
        /// Per-call timeout for the tool, in seconds (default 30).
        #[arg(long, value_name = "SECS", default_value_t = 30)]
        tool_timeout: u64,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Commands::Run {
            repo,
            tool,
            secrets_file,
            tool_timeout,
        } => {
            let config = RunConfig {
                repo,
                tool,
                secrets_file,
                tool_timeout: Duration::from_secs(tool_timeout),
            };
            let stdin = std::io::stdin();
            let stdout = std::io::stdout();
            match run(&config, stdin.lock(), stdout.lock()) {
                Ok(_) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("le-harness: {e}");
                    ExitCode::from(2)
                }
            }
        }
    }
}
