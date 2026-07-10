use std::{fs, path::Path};

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn shared_shim() -> String {
    fs::read_to_string(repo_root().join("skills/codex-monitor/scripts/codex-shim.sh")).unwrap()
}

fn apply_helper() -> String {
    fs::read_to_string(repo_root().join("skills/codex-monitor/scripts/cdxm-agmsg-apply.sh"))
        .unwrap()
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
    // An explicit shim install takes over an existing entrypoint while keeping a
    // timestamped backup, and still leaves it untouched when the user declines.
    assert!(installer.contains("Leaving existing codex entrypoint untouched"));
    assert!(installer.contains("Backed up existing"));
    assert!(installer.contains(".bak-$stamp"));
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
fn agmsg_apply_uses_windows_background_watch_instead_of_launch_agent() {
    let helper = apply_helper();

    assert!(helper.contains("windows background watch"));
    assert!(helper.contains("monitor watch agmsg"));
    assert!(helper.contains("LOCALAPPDATA"));
    assert!(helper.contains("nohup"));
    assert!(helper.contains("agmsg launch-agent install"));
}

#[test]
fn agmsg_apply_can_pin_the_codex_app_target() {
    let helper = apply_helper();

    assert!(helper.contains("--target auto|app|managed"));
    assert!(helper.contains("target=\"${CDXM_MONITOR_TARGET:-auto}\""));
    assert!(helper.contains("target_args=(--target \"$target\")"));
    assert!(helper.contains("\"${target_args[@]}\" monitor watch agmsg"));
}

#[test]
fn agmsg_apply_can_replace_legacy_codex_bridge_on_windows() {
    let helper = apply_helper();

    assert!(helper.contains("taskkill.exe"));
    assert!(helper.contains("MSYS2_ARG_CONV_EXCL='*'"));
    assert!(helper.contains("extract_tab_field \"$line\" command"));
    assert!(helper.contains("Get-CimInstance Win32_Process"));
    assert!(helper.contains("Stop-Process -Id $_.ProcessId"));
}

#[test]
fn agmsg_apply_does_not_disable_project_wide_legacy_agmsg_monitor_delivery() {
    let helper = apply_helper();
    let context =
        fs::read_to_string(repo_root().join("skills/codex-monitor/scripts/cdxm-context.sh"))
            .unwrap();

    assert!(helper.contains("delivery.sh"));
    assert!(helper.contains("status codex \"$project\""));
    assert!(helper.contains("codex_shim=agmsg"));
    assert!(!helper.contains("set off codex \"$project\""));
    assert!(!helper.contains("disable legacy agmsg monitor"));
    assert!(helper.contains("leaving project-wide legacy agmsg monitor mode unchanged"));
    assert!(helper.contains("stop_legacy_codex_bridge_consumers \"$legacy_target_consumer\""));
    assert!(context.contains("status codex \"$project\""));
}

#[test]
fn windows_installer_manages_codex_app_bridge_reversibly() {
    let installer = fs::read_to_string(repo_root().join("install.ps1")).unwrap();

    for required in [
        "[switch]$InstallAppBridge",
        "[switch]$RemoveAppBridge",
        "[string]$RealCodexPath",
        "cdxm-codex-app-bridge.exe",
        "codex-app-real.exe",
        "codex-code-mode-host.exe",
        "codex-command-runner.exe",
        "codex-windows-sandbox-setup.exe",
        "app-bridge-env.json",
        "CODEX_CLI_PATH",
        "CDXM_REAL_CODEX",
        "function Resolve-CodexRuntimeCompanionPath",
        "function Copy-CdxmRuntimeFile",
        "function Enable-CdxmAppBridge",
        "function Disable-CdxmAppBridge",
    ] {
        assert!(
            installer.contains(required),
            "installer is missing app bridge contract `{required}`"
        );
    }
    assert!(installer.contains("InstallAppBridge.IsPresent -and $RemoveAppBridge.IsPresent"));
    assert!(installer.contains("Test-CdxmOwnedAppBridge"));
    assert!(installer.contains("Copy-CdxmRuntimeFile $ResolvedRealCodexPath $ManagedRealCodex"));
    assert!(installer.contains("$ManagedCodeModeHost"));
    assert!(installer.contains("$ManagedCommandRunner"));
    assert!(installer.contains("$ManagedSandboxSetup"));
    assert!(installer.contains("Preserving the current user environment"));
    assert!(installer.contains("Codex App must be restarted"));
}

#[test]
fn windows_installer_has_prebuilt_download_path() {
    let installer = fs::read_to_string(repo_root().join("install.ps1")).unwrap();
    assert!(installer.contains("releases/latest/download"));
    assert!(installer.contains("CDXM_INSTALL_RELEASE_BASE"));
    assert!(installer.contains("x86_64-pc-windows-msvc.zip"));
    assert!(installer.contains("Get-FileHash"));
    assert!(installer.contains("BuildFromSource"));
    assert!(installer
        .contains("$allowed = @('codex-monitor.exe', 'cdxm.exe', 'cdxm-codex-app-bridge.exe')"));
    assert!(installer.contains("Join-Path $BinDir 'cdxm-codex-app-bridge.exe'"));
}

#[test]
fn readme_documents_windows_native_install() {
    let readme = fs::read_to_string(repo_root().join("README.md")).unwrap();

    assert!(readme.contains("install.ps1"));
    assert!(readme.contains("codex.cmd"));
    assert!(readme.contains("keeps a backup"));
    assert!(readme.contains("MSVC Build Tools"));
    // The Windows shim now routes through Git Bash.
    assert!(readme.contains("Git Bash"));
    assert!(readme.contains("-InstallAppBridge"));
    assert!(readme.contains("-RemoveAppBridge"));
    assert!(readme.contains("codex-app-bridge"));
    assert!(readme.contains("codex-code-mode-host.exe"));
    assert!(readme.contains("codex-monitor update"));
    assert!(readme.contains("fully quit Codex App"));
}
