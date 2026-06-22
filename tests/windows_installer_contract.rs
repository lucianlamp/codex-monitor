use std::{fs, path::Path};

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn shared_shim() -> String {
    fs::read_to_string(repo_root().join("skills/codex-monitor/scripts/codex-shim.sh")).unwrap()
}

#[test]
fn windows_installer_routes_codex_through_git_bash_to_shared_shim() {
    let installer = fs::read_to_string(repo_root().join("install.ps1")).unwrap();

    assert!(installer.contains("$ShimTarget = Join-Path $AgentsBin 'codex.cmd'"));
    // The generated codex.cmd is a thin launcher that runs the shared bash
    // shim through Git Bash (same logic as macOS/Linux), not a separate
    // PowerShell reimplementation.
    assert!(installer.contains("scripts\\codex-shim.sh"));
    assert!(installer.contains("CDXM_BASH"));
    assert!(installer.contains("-l \"$shimBashPath\""));
    assert!(installer.contains("CODEX_MONITOR_SHIM_WRAPPER=1"));
    // Still refuses to clobber an existing codex entrypoint.
    assert!(installer.contains("Leaving existing codex entrypoint untouched"));
    // Still documents the bundled-rusqlite build requirement and updates PATH.
    assert!(installer.contains("MSVC Build Tools"));
    assert!(installer.contains("[Environment]::SetEnvironmentVariable('Path'"));
    // The PowerShell shim reimplementation is gone.
    assert!(!installer.contains("codex-monitor-shim.ps1"));
}

#[test]
fn shared_shim_starts_app_server_and_runs_codex_with_remote() {
    let shim = shared_shim();

    assert!(shim.contains("CODEX_MONITOR_SHIM_WRAPPER=1"));
    assert!(shim.contains("Codex monitor shim"));
    assert!(shim.contains("app-server --listen"));
    assert!(shim.contains("ws://127.0.0.1:0"));
    assert!(shim.contains("--remote"));
    assert!(shim.contains("CODEX_MONITOR_REAL_CODEX"));
    // codex prints "listening on: ws://..." to stderr, so the shim must merge
    // stderr into the log it scans for the port.
    assert!(shim.contains("2>&1"));
}

#[test]
fn shared_shim_passes_through_non_interactive_codex_commands() {
    let shim = shared_shim();

    // These are forwarded straight to the real codex instead of starting an
    // app-server.
    for cmd in [
        "app-server",
        "exec",
        "login",
        "logout",
        "mcp",
        "--help",
        "--version",
    ] {
        assert!(
            shim.contains(cmd),
            "shim passthrough list is missing `{cmd}`"
        );
    }
}

#[test]
fn readme_documents_windows_native_install() {
    let readme = fs::read_to_string(repo_root().join("README.md")).unwrap();

    assert!(readme.contains("install.ps1"));
    assert!(readme.contains("codex.cmd"));
    assert!(readme.contains("never overwrites"));
    assert!(readme.contains("MSVC Build Tools"));
    // The Windows shim now routes through Git Bash.
    assert!(readme.contains("Git Bash"));
}
