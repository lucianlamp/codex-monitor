use super::model::{sha256_file, InstallPaths, ManagedFile, StagedFile};
use anyhow::{bail, Context};
use serde::Deserialize;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
};

const INVENTORY_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$package = Get-AppxPackage -Name 'OpenAI.Codex' -ErrorAction SilentlyContinue |
    Sort-Object Version -Descending |
    Select-Object -First 1
$processes = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
    ForEach-Object { $_.Name })
[ordered]@{
    installLocation = if ($package) { $package.InstallLocation } else { $null }
    userCodexCliPath = [Environment]::GetEnvironmentVariable('CODEX_CLI_PATH', 'User')
    userRealCodex = [Environment]::GetEnvironmentVariable('CDXM_REAL_CODEX', 'User')
    processes = $processes
} | ConvertTo-Json -Compress
"#;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WindowsInventory {
    install_location: Option<PathBuf>,
    user_codex_cli_path: Option<PathBuf>,
    #[allow(dead_code)]
    user_real_codex: Option<PathBuf>,
    #[serde(default)]
    processes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppBridgeBackup {
    version: u32,
    bridge_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct RuntimeSources {
    files: Vec<(ManagedFile, Option<PathBuf>)>,
}

impl RuntimeSources {
    pub fn from_resources_dir(resources_dir: PathBuf) -> anyhow::Result<Self> {
        let resources_dir = resources_dir.canonicalize().with_context(|| {
            format!(
                "failed to resolve Codex App resources directory {}",
                resources_dir.display()
            )
        })?;
        let mut files = Vec::new();
        for id in [
            ManagedFile::RealCodex,
            ManagedFile::CodeModeHost,
            ManagedFile::CommandRunner,
            ManagedFile::SandboxSetup,
        ] {
            let name = id
                .runtime_source_name()
                .expect("runtime identifier must have a source name");
            let candidate = resources_dir.join(name);
            let source = if candidate.is_file() {
                let source = candidate.canonicalize().with_context(|| {
                    format!(
                        "failed to resolve Codex App runtime {}",
                        candidate.display()
                    )
                })?;
                if !source.starts_with(&resources_dir) {
                    bail!(
                        "Codex App runtime escaped its package resources directory: {}",
                        source.display()
                    );
                }
                Some(source)
            } else if id.is_required() {
                bail!(
                    "required Codex App runtime companion is missing: {}",
                    candidate.display()
                );
            } else {
                None
            };
            files.push((id, source));
        }
        Ok(Self { files })
    }
}

#[derive(Debug, Clone)]
pub struct WindowsPreflight {
    pub paths: InstallPaths,
    pub runtime_sources: RuntimeSources,
}

pub fn preflight() -> anyhow::Result<WindowsPreflight> {
    let paths = install_paths()?;
    let inventory = query_inventory()?;
    ensure_app_not_running_in(&inventory)?;
    let backup = read_bridge_backup(&paths)?;
    let user_bridge = inventory.user_codex_cli_path.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "Codex App bridge is not enabled; run the one-time -InstallAppBridge installer first"
        )
    })?;
    verify_owned_bridge(&paths, &backup, user_bridge)?;

    let install_location = inventory
        .install_location
        .context("OpenAI.Codex AppX package is not installed for this user")?;
    let resources_dir = install_location.join("app").join("resources");
    let runtime_sources = RuntimeSources::from_resources_dir(resources_dir)?;
    Ok(WindowsPreflight {
        paths,
        runtime_sources,
    })
}

pub fn install_paths() -> anyhow::Result<InstallPaths> {
    Ok(InstallPaths::new(resolve_install_root()?))
}

pub fn stage_runtime(sources: &RuntimeSources, staging: &Path) -> anyhow::Result<Vec<StagedFile>> {
    std::fs::create_dir_all(staging).with_context(|| {
        format!(
            "failed to create runtime staging directory {}",
            staging.display()
        )
    })?;
    let mut staged = Vec::new();
    for (id, source) in &sources.files {
        let sha256 = if let Some(source) = source {
            let destination = staging.join(id.staged_name());
            std::fs::copy(source, &destination).with_context(|| {
                format!(
                    "failed to stage Codex App runtime {} to {}",
                    source.display(),
                    destination.display()
                )
            })?;
            Some(sha256_file(&destination)?)
        } else {
            None
        };
        staged.push(StagedFile { id: *id, sha256 });
    }
    Ok(staged)
}

pub fn ensure_app_not_running() -> anyhow::Result<()> {
    ensure_app_not_running_in(&query_inventory()?)
}

pub fn wait_for_process_exit(pid: u32) -> anyhow::Result<()> {
    let script = r#"
$ErrorActionPreference = 'Stop'
$parent = Get-Process -Id ([int]$env:CDXM_UPDATE_PARENT_PID) -ErrorAction SilentlyContinue
if ($parent) { $parent.WaitForExit() }
"#;
    let output = Command::new(windows_powershell_executable())
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .env("CDXM_UPDATE_PARENT_PID", pid.to_string())
        .output()
        .context("failed to start PowerShell parent-process wait")?;
    require_powershell_success(output, "waiting for the update parent process")?;
    Ok(())
}

pub fn reassert_owned_environment(paths: &InstallPaths) -> anyhow::Result<()> {
    let inventory = query_inventory()?;
    let backup = read_bridge_backup(paths)?;
    let expected_bridge = paths.destination(ManagedFile::AppBridge);
    let user_bridge = inventory
        .user_codex_cli_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("CODEX_CLI_PATH is no longer owned by codex-monitor"))?;
    verify_owned_bridge(paths, &backup, user_bridge)?;

    let real_codex = paths.destination(ManagedFile::RealCodex);
    let script = r#"
$ErrorActionPreference = 'Stop'
[Environment]::SetEnvironmentVariable('CODEX_CLI_PATH', $env:CDXM_UPDATE_BRIDGE, 'User')
[Environment]::SetEnvironmentVariable('CDXM_REAL_CODEX', $env:CDXM_UPDATE_REAL_CODEX, 'User')
"#;
    let output = Command::new(windows_powershell_executable())
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .env("CDXM_UPDATE_BRIDGE", &expected_bridge)
        .env("CDXM_UPDATE_REAL_CODEX", &real_codex)
        .output()
        .context("failed to start PowerShell environment update")?;
    require_powershell_success(output, "updating the owned Codex App environment")?;
    Ok(())
}

pub fn schedule_staging_cleanup(staging: &Path, helper_pid: u32) -> anyhow::Result<()> {
    use std::process::Stdio;

    let name = staging
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if !staging.is_absolute() || !name.starts_with(".update-staging-") {
        bail!(
            "refusing unsafe update staging cleanup path: {}",
            staging.display()
        );
    }
    let script = r#"
$ErrorActionPreference = 'SilentlyContinue'
$helper = Get-Process -Id ([int]$env:CDXM_UPDATE_HELPER_PID) -ErrorAction SilentlyContinue
if ($helper) { $helper.WaitForExit() }
Remove-Item -LiteralPath $env:CDXM_UPDATE_STAGING -Recurse -Force -ErrorAction SilentlyContinue
"#;
    Command::new(windows_powershell_executable())
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .env("CDXM_UPDATE_HELPER_PID", helper_pid.to_string())
        .env("CDXM_UPDATE_STAGING", staging)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to launch update staging cleanup")?;
    Ok(())
}

fn resolve_install_root() -> anyhow::Result<PathBuf> {
    if let Some(root) = std::env::var_os("CDXM_INSTALL_ROOT") {
        let root = PathBuf::from(root);
        if !root.is_absolute() {
            bail!("CDXM_INSTALL_ROOT must be an absolute path");
        }
        return Ok(root);
    }
    let base = directories::BaseDirs::new()
        .context("could not resolve the current user's home directory")?;
    Ok(base.home_dir().join(".codex-monitor"))
}

fn query_inventory() -> anyhow::Result<WindowsInventory> {
    let output = Command::new(windows_powershell_executable())
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            INVENTORY_SCRIPT,
        ])
        .output()
        .context("failed to start PowerShell for Codex App inventory")?;
    let stdout = require_powershell_success(output, "reading Codex App inventory")?;
    parse_inventory(&stdout)
}

fn parse_inventory(text: &str) -> anyhow::Result<WindowsInventory> {
    serde_json::from_str(text.trim()).context("Codex App inventory was not valid JSON")
}

fn ensure_app_not_running_in(inventory: &WindowsInventory) -> anyhow::Result<()> {
    let blockers = inventory
        .processes
        .iter()
        .filter(|name| blocks_update_process(name))
        .cloned()
        .collect::<Vec<_>>();
    if !blockers.is_empty() {
        bail!(
            "fully quit Codex App before running codex-monitor update (active: {})",
            blockers.join(", ")
        );
    }
    Ok(())
}

fn blocks_update_process(name: &str) -> bool {
    [
        "cdxm-codex-app-bridge.exe",
        "codex-app-real.exe",
        "codex-code-mode-host.exe",
        "codex-command-runner.exe",
        "codex-windows-sandbox-setup.exe",
    ]
    .iter()
    .any(|blocked| name.eq_ignore_ascii_case(blocked))
}

fn read_bridge_backup(paths: &InstallPaths) -> anyhow::Result<AppBridgeBackup> {
    let bytes = std::fs::read(&paths.app_bridge_backup).with_context(|| {
        format!(
            "Codex App bridge ownership file is missing: {}; run the one-time -InstallAppBridge installer first",
            paths.app_bridge_backup.display()
        )
    })?;
    serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "Codex App bridge ownership file is invalid: {}",
            paths.app_bridge_backup.display()
        )
    })
}

fn verify_owned_bridge(
    paths: &InstallPaths,
    backup: &AppBridgeBackup,
    user_bridge: &Path,
) -> anyhow::Result<()> {
    let expected = paths.destination(ManagedFile::AppBridge);
    if backup.version != 1
        || !paths_equal(&backup.bridge_path, &expected)
        || !paths_equal(user_bridge, &expected)
    {
        bail!(
            "Codex App bridge is not owned by this codex-monitor installation: {}",
            expected.display()
        );
    }
    Ok(())
}

pub(crate) fn paths_equal(left: &Path, right: &Path) -> bool {
    normalize_path(left.as_os_str()).eq_ignore_ascii_case(&normalize_path(right.as_os_str()))
}

fn normalize_path(path: &OsStr) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_owned()
}

fn windows_powershell_executable() -> PathBuf {
    for variable in ["SystemRoot", "WINDIR"] {
        if let Some(root) = std::env::var_os(variable) {
            let candidate = PathBuf::from(root)
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe");
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from("powershell.exe")
}

fn require_powershell_success(
    output: std::process::Output,
    operation: &str,
) -> anyhow::Result<String> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("PowerShell failed while {operation}: {}", stderr.trim());
    }
    String::from_utf8(output.stdout).context("PowerShell returned non-UTF-8 output")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::model::{InstallPaths, ManagedFile};
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    #[test]
    fn app_process_inventory_blocks_update_case_insensitively() {
        assert!(blocks_update_process("CDXM-CODEX-APP-BRIDGE.EXE"));
        assert!(blocks_update_process("codex-app-real.exe"));
        assert!(!blocks_update_process("Codex.exe"));
        assert!(!blocks_update_process("codex-monitor.exe"));
    }

    #[test]
    fn ownership_requires_backup_and_matching_user_bridge() {
        let paths = InstallPaths::new(PathBuf::from(r"C:\Users\me\.codex-monitor"));
        let backup = AppBridgeBackup {
            version: 1,
            bridge_path: paths.destination(ManagedFile::AppBridge),
        };
        let expected = paths.destination(ManagedFile::AppBridge);
        assert!(verify_owned_bridge(&paths, &backup, &expected).is_ok());
        assert!(verify_owned_bridge(&paths, &backup, Path::new(r"C:\other.exe")).is_err());
    }

    #[test]
    fn inventory_json_maps_app_package_environment_and_processes() {
        let inventory = parse_inventory(
            r#"{"installLocation":"C:\\Program Files\\WindowsApps\\OpenAI.Codex_1","userCodexCliPath":"C:\\Users\\me\\.codex-monitor\\bin\\cdxm-codex-app-bridge.exe","userRealCodex":null,"processes":["explorer.exe","Codex.exe"]}"#,
        )
        .unwrap();
        assert!(inventory
            .install_location
            .unwrap()
            .ends_with("OpenAI.Codex_1"));
        assert_eq!(inventory.processes, ["explorer.exe", "Codex.exe"]);
    }

    #[test]
    fn runtime_staging_records_absent_optional_companions() {
        let source = TempDir::new().unwrap();
        let staging = TempDir::new().unwrap();
        std::fs::write(source.path().join("codex.exe"), b"codex").unwrap();
        std::fs::write(source.path().join("codex-code-mode-host.exe"), b"host").unwrap();
        let sources = RuntimeSources::from_resources_dir(source.path().to_path_buf()).unwrap();
        let staged = stage_runtime(&sources, staging.path()).unwrap();
        assert_eq!(staged.len(), 4);
        assert!(staged
            .iter()
            .any(|file| file.id == ManagedFile::RealCodex && file.sha256.is_some()));
        assert!(staged
            .iter()
            .any(|file| file.id == ManagedFile::CommandRunner && file.sha256.is_none()));
    }
}
