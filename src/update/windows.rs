use super::model::InstallPaths;
use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashSet},
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
    userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
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
    user_path: Option<String>,
    #[serde(default)]
    processes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppBridgeBackup {
    version: u32,
    bridge_path: PathBuf,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UserPathBackup<'a> {
    version: u32,
    user_path: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct WindowsPreflight {
    pub paths: InstallPaths,
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

    Ok(WindowsPreflight { paths })
}

pub fn install_paths() -> anyhow::Result<InstallPaths> {
    Ok(InstallPaths::new(resolve_install_root()?))
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
    let expected_bridge = paths.root.join("bin").join("cdxm-codex-app-bridge.exe");
    let user_bridge = inventory
        .user_codex_cli_path
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("CODEX_CLI_PATH is no longer owned by codex-monitor"))?;
    verify_owned_bridge(paths, &backup, user_bridge)?;

    let real_codex = paths.root.join("runtime").join("codex-app-real.exe");
    let (preferred_path_entries, removed_path_entries) = public_cli_path_entries(paths)?;
    preserve_user_path_backup(paths, inventory.user_path.as_deref())?;
    let user_path = normalize_user_path(
        inventory.user_path.as_deref(),
        &preferred_path_entries,
        &removed_path_entries,
    );
    let script = r#"
$ErrorActionPreference = 'Stop'
[Environment]::SetEnvironmentVariable('CODEX_CLI_PATH', $env:CDXM_UPDATE_BRIDGE, 'User')
[Environment]::SetEnvironmentVariable('CDXM_REAL_CODEX', $env:CDXM_UPDATE_REAL_CODEX, 'User')
[Environment]::SetEnvironmentVariable('Path', $env:CDXM_UPDATE_USER_PATH, 'User')
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
        .env("CDXM_UPDATE_USER_PATH", &user_path)
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
        .collect::<BTreeSet<_>>();
    if !blockers.is_empty() {
        bail!(
            "fully quit Codex App before running codex-monitor update (active: {})",
            blockers.into_iter().collect::<Vec<_>>().join(", ")
        );
    }
    Ok(())
}

fn blocks_update_process(name: &str) -> bool {
    ["cdxm-codex-app-bridge.exe", "codex-app-real.exe"]
        .iter()
        .any(|blocked| name.eq_ignore_ascii_case(blocked))
}

fn public_cli_path_entries(paths: &InstallPaths) -> anyhow::Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    let base = directories::BaseDirs::new()
        .context("could not resolve the current user's home directory")?;
    let app_data = std::env::var_os("APPDATA").context("APPDATA is not set")?;
    let local_app_data = std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not set")?;
    Ok((
        vec![
            base.home_dir().join(".agents").join("bin"),
            paths.root.join("bin"),
            PathBuf::from(app_data).join("npm"),
        ],
        vec![PathBuf::from(local_app_data)
            .join("OpenAI")
            .join("Codex")
            .join("bin")],
    ))
}

fn normalize_user_path(
    current: Option<&str>,
    preferred: &[PathBuf],
    removed: &[PathBuf],
) -> String {
    let removed = removed
        .iter()
        .map(|entry| normalized_path_key(&entry.to_string_lossy()))
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    let entries = preferred
        .iter()
        .map(|entry| entry.to_string_lossy().into_owned())
        .chain(
            current
                .into_iter()
                .flat_map(|value| value.split(';').map(str::to_owned).collect::<Vec<_>>()),
        );

    for entry in entries {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let key = normalized_path_key(entry);
        if removed.contains(&key) || !seen.insert(key) {
            continue;
        }
        result.push(entry.to_owned());
    }
    result.join(";")
}

fn normalized_path_key(path: &str) -> String {
    path.replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn preserve_user_path_backup(paths: &InstallPaths, user_path: Option<&str>) -> anyhow::Result<()> {
    let backup_path = paths.root.join("user-path-backup.json");
    if backup_path.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(&paths.root).with_context(|| {
        format!(
            "failed to create install root for PATH backup {}",
            paths.root.display()
        )
    })?;
    let temporary = paths
        .root
        .join(format!(".user-path-backup-{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec_pretty(&UserPathBackup {
        version: 1,
        user_path,
    })
    .context("failed to serialize the user PATH backup")?;
    std::fs::write(&temporary, bytes).with_context(|| {
        format!(
            "failed to write temporary user PATH backup {}",
            temporary.display()
        )
    })?;
    if let Err(error) = std::fs::rename(&temporary, &backup_path) {
        let _ = std::fs::remove_file(&temporary);
        if backup_path.exists() {
            return Ok(());
        }
        return Err(error).with_context(|| {
            format!(
                "failed to publish user PATH backup {}",
                backup_path.display()
            )
        });
    }
    Ok(())
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
    let expected = paths.root.join("bin").join("cdxm-codex-app-bridge.exe");
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
    use crate::update::model::InstallPaths;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    #[test]
    fn app_process_inventory_blocks_update_case_insensitively() {
        assert!(blocks_update_process("CDXM-CODEX-APP-BRIDGE.EXE"));
        assert!(blocks_update_process("codex-app-real.exe"));
        assert!(!blocks_update_process("Codex.exe"));
        assert!(!blocks_update_process("codex-code-mode-host.exe"));
        assert!(!blocks_update_process("codex-monitor.exe"));
    }

    #[test]
    fn ownership_requires_backup_and_matching_user_bridge() {
        let paths = InstallPaths::new(PathBuf::from(r"C:\Users\me\.codex-monitor"));
        let backup = AppBridgeBackup {
            version: 1,
            bridge_path: paths.root.join("bin").join("cdxm-codex-app-bridge.exe"),
        };
        let expected = paths.root.join("bin").join("cdxm-codex-app-bridge.exe");
        assert!(verify_owned_bridge(&paths, &backup, &expected).is_ok());
        assert!(verify_owned_bridge(&paths, &backup, Path::new(r"C:\other.exe")).is_err());
    }

    #[test]
    fn inventory_json_maps_app_package_environment_and_processes() {
        let inventory = parse_inventory(
            r#"{"installLocation":"C:\\Program Files\\WindowsApps\\OpenAI.Codex_1","userCodexCliPath":"C:\\Users\\me\\.codex-monitor\\bin\\cdxm-codex-app-bridge.exe","userRealCodex":null,"userPath":"C:\\Tools","processes":["explorer.exe","Codex.exe"]}"#,
        )
        .unwrap();
        assert!(inventory
            .install_location
            .unwrap()
            .ends_with("OpenAI.Codex_1"));
        assert_eq!(inventory.user_path.as_deref(), Some(r"C:\Tools"));
        assert_eq!(inventory.processes, ["explorer.exe", "Codex.exe"]);
    }

    #[test]
    fn public_cli_path_is_ordered_and_desktop_is_removed() {
        let preferred = [
            PathBuf::from(r"C:\Users\me\.agents\bin"),
            PathBuf::from(r"C:\Users\me\.codex-monitor\bin"),
            PathBuf::from(r"C:\Users\me\AppData\Roaming\npm"),
        ];
        let removed = [PathBuf::from(r"C:\Users\me\AppData\Local\OpenAI\Codex\bin")];
        let actual = normalize_user_path(
            Some(
                r"C:\Tools;C:\Users\me\AppData\Local\OpenAI\Codex\bin;C:\Users\me\AppData\Roaming\npm",
            ),
            &preferred,
            &removed,
        );

        assert_eq!(
            actual,
            r"C:\Users\me\.agents\bin;C:\Users\me\.codex-monitor\bin;C:\Users\me\AppData\Roaming\npm;C:\Tools"
        );
        assert_eq!(
            normalize_user_path(Some(&actual), &preferred, &removed),
            actual
        );
    }

    #[test]
    fn user_path_backup_preserves_the_first_value() {
        let root = TempDir::new().unwrap();
        let paths = InstallPaths::new(root.path().join("install"));

        preserve_user_path_backup(&paths, Some(r"C:\Original")).unwrap();
        preserve_user_path_backup(&paths, Some(r"C:\AlreadyNormalized")).unwrap();

        let backup: serde_json::Value = serde_json::from_slice(
            &std::fs::read(paths.root.join("user-path-backup.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(backup["version"], 1);
        assert_eq!(backup["userPath"], r"C:\Original");
        assert!(std::fs::read_dir(&paths.root).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
    }

}
