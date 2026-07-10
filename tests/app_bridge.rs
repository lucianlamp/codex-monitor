#[cfg(windows)]
#[test]
fn bridge_passes_non_app_server_invocations_to_real_codex() {
    let bridge = env!("CARGO_BIN_EXE_cdxm-codex-app-bridge");
    let command = std::env::var_os("COMSPEC").expect("COMSPEC is set on Windows");
    let status = std::process::Command::new(bridge)
        .env("CDXM_REAL_CODEX", command)
        .args(["/d", "/c", "exit", "7"])
        .status()
        .unwrap();

    assert_eq!(status.code(), Some(7));
}
