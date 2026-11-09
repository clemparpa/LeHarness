use clap::Parser;

#[derive(Parser)]
#[command(
    name = "le-harness",
    version,
    about = "AI agent harness: session core, in-memory secrets, confined subprocess tools"
)]
struct Cli {}

fn main() {
    let _ = Cli::parse();
}
