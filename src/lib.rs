pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CLIENT_INFO_NAME: &str = "codex-monitor";
pub const CLIENT_INFO_TITLE: &str = "Codex Monitor";

pub mod app_bridge;
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
pub mod update;

pub async fn run_cli() -> anyhow::Result<i32> {
    cli::run_from_env().await
}

/// Blocking entry point used by the binaries.
///
/// The async future tree of some commands (e.g. `send` / `agmsg watch`) is
/// large. On Windows the process main thread's default stack is ~1 MiB —
/// much smaller than the 8 MiB Linux/macOS give it — so driving that future
/// directly on the main thread (as `#[tokio::main]` does) overflows the stack
/// at runtime. Run the Tokio runtime on a dedicated thread with a large stack
/// so behavior is consistent across platforms.
pub fn run_cli_blocking() -> i32 {
    const STACK_SIZE: usize = 16 * 1024 * 1024;

    let run = || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_stack_size(STACK_SIZE)
            .build()
            .expect("failed to build Tokio runtime");
        runtime.block_on(async {
            match run_cli().await {
                Ok(code) => code,
                Err(error) => {
                    eprintln!("{error:#}");
                    1
                }
            }
        })
    };

    std::thread::Builder::new()
        .name("codex-monitor".into())
        .stack_size(STACK_SIZE)
        .spawn(run)
        .expect("failed to spawn runtime thread")
        .join()
        .expect("runtime thread panicked")
}
