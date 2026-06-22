pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CLIENT_INFO_NAME: &str = "codex-monitor";
pub const CLIENT_INFO_TITLE: &str = "Codex Monitor";

pub mod cli;
pub mod client;
pub mod delivery;
pub mod launchd;
pub mod protocol;
pub mod remote_control;
pub mod sources;
pub mod state;
pub mod target;
pub mod transport;

pub async fn run_cli() -> anyhow::Result<i32> {
    cli::run_from_env().await
}
