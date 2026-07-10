fn main() {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build Codex App bridge runtime");
    let code = match runtime.block_on(codex_monitor::app_bridge::run_bridge(args)) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("codex-monitor app bridge: {error:#}");
            1
        }
    };
    std::process::exit(code);
}
