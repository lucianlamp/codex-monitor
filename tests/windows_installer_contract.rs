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

fn codex_monitor_skill() -> String {
    fs::read_to_string(repo_root().join("skills/codex-monitor/SKILL.md")).unwrap()
}

fn foreground_helper() -> String {
    fs::read_to_string(repo_root().join("skills/codex-monitor/scripts/cdxm-agmsg-foreground.sh"))
        .unwrap()
}

#[test]
fn foreground_helper_contract() {
    let helper = foreground_helper();
    assert!(helper.contains("inbox.sh"));
    assert!(helper.contains("No new messages."));
    assert!(helper.contains("while :"));
    assert!(helper.contains("exit 0"));
    for forbidden in ["nohup", "pidfile", "monitor watch", "launch-agent"] {
        assert!(
            !helper.contains(forbidden),
            "foreground helper contains forbidden lifecycle behavior `{forbidden}`"
        );
    }
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
fn docs_define_native_app_monitor_shortcuts() {
    let skill = codex_monitor_skill();
    for required in [
        "## Codex App Shortcuts",
        "cdxm-agmsg-foreground.sh",
        "$codex-monitor heartbeat",
        "one-minute heartbeat",
        "automation_update",
        "$codex-monitor off",
        "target thread",
        "No new messages.",
        "only the installed agmsg scripts",
        "Never start, stop, kill, restart, replace, or install any watcher or process.",
        "refusing Windows Desktop Codex fallback",
        "active orphaned legacy runtime files",
    ] {
        assert!(skill.contains(required), "missing `{required}`");
    }
    for forbidden in [
        "-InstallAppBridge",
        "-RemoveAppBridge",
        "cdxm-codex-app-bridge.exe",
        "enable the app bridge",
    ] {
        assert!(
            !skill.contains(forbidden),
            "stale App bridge text `{forbidden}`"
        );
    }
}

#[test]
fn skill_routes_cli_codex_monitor_to_durable_apply_before_app_foreground_wait() {
    let skill = codex_monitor_skill();
    let normalized = skill.split_whitespace().collect::<Vec<_>>().join(" ");
    let routing = skill
        .find("## Shortcut Runtime Routing")
        .expect("skill must define shortcut runtime routing");
    let app = skill
        .find("## Codex App Shortcuts")
        .expect("skill must define App shortcuts");

    assert!(routing < app, "runtime routing must precede App shortcuts");
    for required in [
        "Treat the host as Codex App only when the runtime explicitly identifies itself as Codex App",
        "Otherwise treat the host as Codex CLI",
        "Never run `cdxm-agmsg-foreground.sh` for the CLI shortcut",
        "do not pass `--foreground`",
        "durable receiver",
    ] {
        assert!(
            normalized.contains(required),
            "missing CLI routing contract `{required}`"
        );
    }
}

#[test]
fn windows_installer_isolates_the_public_cli_path() {
    let installer = fs::read_to_string(repo_root().join("install.ps1")).unwrap();

    assert!(installer.contains("user-path-backup.json"));
    assert!(installer.contains("function Get-CdxmNormalizedPath"));
    assert!(installer.contains("function Repair-CdxmUserPath"));
    assert!(installer.contains("OpenAI\\Codex\\bin"));
    assert!(installer.contains("Join-Path $env:APPDATA 'npm'"));
    assert!(installer.contains("Repair-CdxmUserPath"));
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

    assert!(helper.contains("apply=1"));
    assert!(helper.contains("foreground=0"));
    assert!(helper.contains("windows background watch"));
    assert!(helper.contains("monitor watch agmsg"));
    assert!(helper.contains("LOCALAPPDATA"));
    assert!(helper.contains("nohup"));
    assert!(helper.contains("agmsg launch-agent install"));
}

#[test]
fn agmsg_apply_can_hash_projects_without_macos_shasum() {
    let helper = apply_helper();

    assert!(helper.contains("command -v shasum"));
    assert!(helper.contains("command -v sha1sum"));
    assert!(helper.contains("openssl dgst -sha1"));
    assert!(helper.contains("| sha1_digest"));
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
fn windows_installer_migrates_legacy_bridge_without_managing_processes() {
    let installer = fs::read_to_string(repo_root().join("install.ps1")).unwrap();
    assert!(!installer.contains("[switch]$InstallAppBridge"));
    assert!(!installer.contains("[switch]$RemoveAppBridge"));
    assert!(!installer.contains("[string]$RealCodexPath"));
    assert!(installer.contains("function Migrate-CdxmLegacyAppBridge"));
    assert!(installer.contains("previousCodexCliPath"));
    assert!(installer.contains("previousCdxmRealCodex"));
    assert!(installer.contains("Get-CimInstance Win32_Process"));
    assert!(installer.contains("ExecutablePath"));
    assert!(installer.contains("if ($owned -and $active.Count -gt 0)"));
    assert!(installer.contains("$runtimeActive"));
    assert!(installer.contains("Deferring cleanup of active legacy runtime"));
    assert!(!installer.contains("Stop-Process"));
}

#[test]
fn windows_installer_has_prebuilt_download_path() {
    let installer = fs::read_to_string(repo_root().join("install.ps1")).unwrap();
    assert!(installer.contains("releases/latest/download"));
    assert!(installer.contains("CDXM_INSTALL_RELEASE_BASE"));
    assert!(installer.contains("x86_64-pc-windows-msvc.zip"));
    assert!(installer.contains("Get-FileHash"));
    assert!(installer.contains("BuildFromSource"));
    assert!(installer.contains("$CdxmCompatTarget = Join-Path $AgentsBin 'cdxm.cmd'"));
    assert!(installer.contains("function Write-CdxmCompatibilityLauncher"));
    assert!(installer.contains("\"$monitorTarget\" %*"));
    assert!(installer.contains("cargo build"));
    assert!(installer.contains("--bin codex-monitor"));
    assert!(!installer.contains("cargo install --path $SourceDir"));
    assert!(installer.contains("$allowed = @('codex-monitor.exe')"));
    assert!(!installer.contains("Join-Path $releaseDir 'cdxm.exe'"));
    assert!(installer.contains("Deferring cleanup of active legacy cdxm.exe"));
    assert!(!installer.contains("Stop-Process"));
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
    assert!(readme.contains("$codex-monitor heartbeat"));
    assert!(readme.contains("$codex-monitor off"));
    assert!(readme.contains("signed native"));
    assert!(readme.contains("codex-monitor update"));
    assert!(readme.contains("single native"));
    assert!(readme.contains("cdxm.cmd"));
    assert!(!readme.contains("-InstallAppBridge"));
    assert!(!readme.contains("-RemoveAppBridge"));
    assert!(!readme.contains("codex-app-bridge"));
}
