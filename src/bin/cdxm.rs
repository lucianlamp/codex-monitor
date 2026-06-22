#[tokio::main]
async fn main() {
    let code = match codex_monitor::run_cli().await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    };
    std::process::exit(code);
}
