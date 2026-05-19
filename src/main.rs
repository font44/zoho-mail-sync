mod config;
mod maildir;
mod oauth;
mod state;
mod sync;
mod zoho;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "zoho-mail-sync", version, about = "One-way Maildir mirror of a Zoho Mail account")]
struct Cli {
    #[arg(long, global = true, value_name = "FILE")]
    config: Option<PathBuf>,

    #[arg(long, global = true, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Auth {
        #[arg(long)]
        code: Option<String>,
    },
    Sync,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();

    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Command::Sync);
    let cfg = config::load(cli.config.as_deref(), cli.data_dir.as_deref())?;

    match command {
        Command::Auth { code } => oauth::run_auth(&cfg, code.as_deref()).await,
        Command::Sync => sync::run(cfg).await,
    }
}
