pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CLIENT_INFO_NAME: &str = "codex-control-bridge";
pub const CLIENT_INFO_TITLE: &str = "Codex Control Bridge";

pub mod cli;
pub mod protocol;

pub async fn run_cli() -> anyhow::Result<i32> {
    cli::run_from_env().await
}
