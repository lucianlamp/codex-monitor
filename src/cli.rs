use crate::client::AppServerClient;
use crate::target::endpoint_from_options;
use crate::transport::open_endpoint_transport;
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

pub async fn run(cli: Cli) -> anyhow::Result<i32> {
    let endpoint = endpoint_from_options(cli.endpoint.clone(), cli.target);
    match cli.command {
        Commands::Threads { cwd } => {
            let transport = open_endpoint_transport(endpoint).await?;
            let mut client = AppServerClient::new(transport);
            let operation = async {
                client.initialize().await?;
                let result = client.thread_list_by_cwd(&cwd).await?;
                crate::target::parse_thread_list(&result)
            }
            .await;
            let close_result = client.close().await;
            let threads = operation?;
            close_result?;
            for thread in threads {
                println!(
                    "{}\t{}\t{}",
                    thread.id,
                    thread.title.unwrap_or_else(|| "-".into()),
                    thread.cwd.unwrap_or_else(|| "-".into())
                );
            }
            Ok(0)
        }
        Commands::Send { thread, text } => {
            let transport = open_endpoint_transport(endpoint).await?;
            let mut client = AppServerClient::new(transport);
            let operation = async {
                client.initialize().await?;
                client.turn_start_and_wait(&thread, &text).await
            }
            .await;
            let close_result = client.close().await;
            operation?;
            close_result?;
            Ok(0)
        }
        Commands::Agmsg { command } => match command {
            AgmsgCommand::Watch {
                team,
                name,
                thread,
                agmsg_db,
            } => crate::delivery::run_agmsg_watch(endpoint, team, name, thread, agmsg_db).await,
        },
    }
}
