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
}
