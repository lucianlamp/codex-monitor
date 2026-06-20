use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Parser)]
#[command(name = "codex-control-bridge")]
#[command(
    about = "codex-control-bridge: local-first bridge for Codex app-server control plane events."
)]
pub struct Cli {
    #[arg(long, global = true)]
    pub endpoint: Option<String>,

    #[arg(long, global = true, value_enum, default_value_t = TargetKind::Managed)]
    pub target: TargetKind,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
pub enum TargetKind {
    Managed,
    App,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Commands {
    Threads {
        #[arg(long)]
        cwd: String,
    },
    Send {
        #[arg(long)]
        thread: String,
        #[arg(long)]
        text: String,
    },
    Agmsg {
        #[command(subcommand)]
        command: AgmsgCommand,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum AgmsgCommand {
    Watch {
        #[arg(long)]
        team: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        thread: String,
        #[arg(long)]
        agmsg_db: Option<String>,
    },
}

pub async fn run_from_env() -> anyhow::Result<i32> {
    let cli = Cli::parse();
    run(cli).await
}

pub async fn run(_cli: Cli) -> anyhow::Result<i32> {
    Ok(0)
}
