use std::{fs, path::Path};

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn windows_installer_has_native_paths_and_never_overwrites_codex_cmd() {
    let installer = fs::read_to_string(repo_root().join("install.ps1")).unwrap();

    assert!(installer.contains("$ShimTarget = Join-Path $AgentsBin 'codex.cmd'"));
    assert!(installer.contains("Leaving existing codex entrypoint untouched"));
    assert!(installer.contains("CODEX_MONITOR_SHIM_WRAPPER=1"));
    assert!(installer.contains("codex-monitor-shim.ps1"));
    assert!(installer.contains("[Environment]::SetEnvironmentVariable('Path'"));
    assert!(installer.contains("MSVC Build Tools"));
}

#[test]
fn windows_installer_embeds_app_server_remote_shim() {
    let installer = fs::read_to_string(repo_root().join("install.ps1")).unwrap();

    assert!(installer.contains("app-server"));
    assert!(installer.contains("--listen"));
    assert!(installer.contains("ws://127.0.0.1:0"));
    assert!(installer.contains("--remote"));
    assert!(installer.contains("CODEX_MONITOR_REAL_CODEX"));
}

#[test]
fn readme_documents_windows_native_install() {
    let readme = fs::read_to_string(repo_root().join("README.md")).unwrap();

    assert!(readme.contains("install.ps1"));
    assert!(readme.contains("codex.cmd"));
    assert!(readme.contains("never overwrites"));
    assert!(readme.contains("MSVC Build Tools"));
}

#[test]
fn windows_shim_arg_parser_avoids_reserved_args_automatic_variable() {
    let installer = fs::read_to_string(repo_root().join("install.ps1")).unwrap();

    // `$Args` is a PowerShell automatic variable; binding it as a function
    // parameter silently yields an empty array, which made every `codex <cmd>`
    // miss the passthrough list and spawn an app-server instead of delegating
    // to the real codex. The arg helpers must use a non-reserved parameter.
    assert!(
        !installer.contains("[string[]]$Args"),
        "shim arg helpers must not bind the reserved $Args automatic variable"
    );
    assert!(installer.contains("function Get-FirstNonOption"));
    assert!(installer.contains("function Get-ProjectFromArgs"));
    assert!(installer.contains("[string[]]$Tokens"));
}

#[test]
fn windows_codex_cmd_prefers_powershell_7() {
    let installer = fs::read_to_string(repo_root().join("install.ps1")).unwrap();

    // The generated codex.cmd runs the shim under PowerShell 7 (`pwsh`) when
    // available, falling back to Windows PowerShell (`powershell.exe`).
    assert!(installer.contains("where pwsh"));
    assert!(installer.contains("pwsh -NoProfile -ExecutionPolicy Bypass -File"));
    assert!(installer.contains("powershell.exe -NoProfile -ExecutionPolicy Bypass -File"));
}
