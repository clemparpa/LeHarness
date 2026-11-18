use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};

use harness::run::{run, RunConfig};
use harness::serve::{serve, ServeConfig, DEFAULT_MAX_SESSIONS, DEFAULT_SESSION_TIMEOUT_SECS};
use harness::session::SessionId;

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
        /// Resume an existing session instead of starting a new one.
        #[arg(long, value_name = "SESSION_ID")]
        resume: Option<SessionId>,
        /// Directory holding the per-session JSON snapshots
        /// (default: $HOME/.local/share/le-harness/sessions).
        #[arg(long, value_name = "PATH")]
        state_dir: Option<PathBuf>,
        /// Line-JSON protocol mode (one ToolRequest line in, one ToolResponse
        /// line out per request); used when spawned by `serve`.
        #[arg(long)]
        json: bool,
    },
    /// Multi-session orchestrator: one `le-harness run --json` subprocess per
    /// session under --repos-dir, line-JSON protocol on stdin/stdout.
    Serve {
        /// Directory whose subdirectories are the session repos.
        #[arg(long, value_name = "DIR")]
        repos_dir: PathBuf,
        /// Maximum number of concurrent sessions (fail-closed refusal beyond).
        #[arg(long, value_name = "N", default_value_t = DEFAULT_MAX_SESSIONS)]
        max_sessions: usize,
        /// Per-message session timeout, in seconds.
        #[arg(long, value_name = "SECS", default_value_t = DEFAULT_SESSION_TIMEOUT_SECS)]
        session_timeout: u64,
        /// External tool binary forwarded by each session (line-JSON protocol).
        #[arg(long, value_name = "BINARY")]
        tool: Option<PathBuf>,
        /// State root (default: $HOME/.local/share/le-harness): per-session
        /// snapshots in <dir>/sessions, audit logs in <dir>/audit.
        #[arg(long, value_name = "PATH")]
        state_dir: Option<PathBuf>,
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
            resume,
            state_dir,
            json,
        } => {
            let config = RunConfig {
                repo,
                tool,
                secrets_file,
                tool_timeout: Duration::from_secs(tool_timeout),
                state_dir,
                resume,
                json,
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
        Commands::Serve {
            repos_dir,
            max_sessions,
            session_timeout,
            tool,
            state_dir,
        } => {
            let state_dir = match state_dir {
                Some(dir) => dir,
                None => match ServeConfig::default_state_dir() {
                    Ok(dir) => dir,
                    Err(e) => {
                        eprintln!("le-harness: {e}");
                        return ExitCode::from(2);
                    }
                },
            };
            let config = ServeConfig {
                repos_dir,
                max_sessions,
                session_timeout: Duration::from_secs(session_timeout),
                tool,
                state_dir,
                harness_bin: std::env::current_exe()
                    .unwrap_or_else(|_| PathBuf::from("le-harness")),
            };
            let stdout = std::io::stdout();
            match serve(&config, stdout.lock()) {
                Ok(_) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("le-harness: {e}");
                    ExitCode::from(2)
                }
            }
        }
    }
}
