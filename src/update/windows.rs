use super::model::InstallPaths;
use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
};

const INVENTORY_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$processes = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
    Where-Object { $_.ExecutablePath } |
    ForEach-Object {
        [ordered]@{
            pid = [uint32]$_.ProcessId
            path = $_.ExecutablePath
        }
    })
[ordered]@{
    userCodexCliPath = [Environment]::GetEnvironmentVariable('CODEX_CLI_PATH', 'User')
    userRealCodex = [Environment]::GetEnvironmentVariable('CDXM_REAL_CODEX', 'User')
    userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    processes = @($processes)
} | ConvertTo-Json -Compress -Depth 4
"#;

const ENVIRONMENT_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
if ($env:CDXM_UPDATE_RESTORE_ENV -eq '1') {
    $cli = if ($env:CDXM_UPDATE_CLI_PRESENT -eq '1') { $env:CDXM_UPDATE_CLI } else { $null }
    $real = if ($env:CDXM_UPDATE_REAL_PRESENT -eq '1') { $env:CDXM_UPDATE_REAL } else { $null }
    [Environment]::SetEnvironmentVariable('CODEX_CLI_PATH', $cli, 'User')
    [Environment]::SetEnvironmentVariable('CDXM_REAL_CODEX', $real, 'User')
}
[Environment]::SetEnvironmentVariable('Path', $env:CDXM_UPDATE_USER_PATH, 'User')
"#;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyProcess {
    pid: u32,
    path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WindowsInventory {
    user_codex_cli_path: Option<PathBuf>,
    user_path: Option<String>,
    #[serde(default)]
    processes: Vec<LegacyProcess>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppBridgeBackup {
    version: u32,
    previous_codex_cli_path: Option<PathBuf>,
    previous_cdxm_real_codex: Option<PathBuf>,
    bridge_path: PathBuf,
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum LegacyEnvironmentAction {
    Preserve,
    Restore {
        codex_cli_path: Option<PathBuf>,
        cdxm_real_codex: Option<PathBuf>,
    },
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
    let backup = read_owned_bridge_backup(&paths, &inventory)?;
    let action = plan_legacy_environment(&paths, &inventory, backup.as_ref())?;
    ensure_owned_migration_not_running(&paths, &inventory, &action)?;
    Ok(WindowsPreflight { paths })
}

pub fn install_paths() -> anyhow::Result<InstallPaths> {
    Ok(InstallPaths::new(resolve_install_root()?))
}

pub fn ensure_legacy_bridge_not_running() -> anyhow::Result<()> {
    let paths = install_paths()?;
    let inventory = query_inventory()?;
    let backup = read_owned_bridge_backup(&paths, &inventory)?;
    let action = plan_legacy_environment(&paths, &inventory, backup.as_ref())?;
    ensure_owned_migration_not_running(&paths, &inventory, &action)
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

pub fn finalize_environment(paths: &InstallPaths) -> anyhow::Result<()> {
    let inventory = query_inventory()?;
    let had_backup = paths.app_bridge_backup.is_file();
    let backup = read_owned_bridge_backup(paths, &inventory)?;
    let action = plan_legacy_environment(paths, &inventory, backup.as_ref())?;
    ensure_owned_migration_not_running(paths, &inventory, &action)?;

    let (preferred_path_entries, removed_path_entries) = public_cli_path_entries(paths)?;
    preserve_user_path_backup(paths, inventory.user_path.as_deref())?;
    let user_path = normalize_user_path(
        inventory.user_path.as_deref(),
        &preferred_path_entries,
        &removed_path_entries,
    );
    apply_environment(&action, &user_path)?;
    write_cdxm_compat_launcher(paths)?;
    cleanup_legacy_cdxm_binary(paths, &inventory)?;
    cleanup_legacy_files(paths, &inventory)?;

    if matches!(action, LegacyEnvironmentAction::Restore { .. }) {
        std::fs::remove_file(&paths.app_bridge_backup).with_context(|| {
            format!(
                "failed to remove migrated App bridge ownership file {}",
                paths.app_bridge_backup.display()
            )
        })?;
    } else if had_backup {
        eprintln!(
            "codex-monitor update preserved CODEX_CLI_PATH because it is not owned by this installation"
        );
    }
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

fn legacy_bridge_path(paths: &InstallPaths) -> PathBuf {
    paths.root.join("bin").join("cdxm-codex-app-bridge.exe")
}

fn legacy_cdxm_path(paths: &InstallPaths) -> PathBuf {
    paths.root.join("bin").join("cdxm.exe")
}

fn legacy_runtime_dir(paths: &InstallPaths) -> PathBuf {
    paths.root.join("runtime")
}

fn legacy_paths(paths: &InstallPaths) -> Vec<PathBuf> {
    let runtime = legacy_runtime_dir(paths);
    vec![
        legacy_bridge_path(paths),
        runtime.join("codex-app-real.exe"),
        runtime.join("codex-code-mode-host.exe"),
        runtime.join("codex-command-runner.exe"),
        runtime.join("codex-windows-sandbox-setup.exe"),
    ]
}

fn plan_legacy_environment(
    paths: &InstallPaths,
    inventory: &WindowsInventory,
    backup: Option<&AppBridgeBackup>,
) -> anyhow::Result<LegacyEnvironmentAction> {
    let expected = legacy_bridge_path(paths);
    let Some(current) = inventory.user_codex_cli_path.as_deref() else {
        return Ok(LegacyEnvironmentAction::Preserve);
    };
    let is_bridge = paths_equal(current, &expected);
    let is_app_bin = is_codex_app_bin_path(current);
    if !is_bridge && !is_app_bin {
        return Ok(LegacyEnvironmentAction::Preserve);
    }
    // A CODEX_CLI_PATH that drifted into the Codex App's `OpenAI\Codex\bin` tree
    // is only ours to reclaim while we still hold the ownership backup. Without
    // one it is the user's native Codex App CLI and must be kept -- even though
    // a stale build hash there is exactly what breaks the App's code-mode host.
    if is_app_bin && !is_bridge && backup.is_none() {
        return Ok(LegacyEnvironmentAction::Preserve);
    }

    let backup = backup.with_context(|| {
        format!(
            "CODEX_CLI_PATH is the legacy bridge but its ownership file is missing: {}",
            paths.app_bridge_backup.display()
        )
    })?;
    if backup.version != 1 || !paths_equal(&backup.bridge_path, &expected) {
        bail!(
            "legacy App bridge ownership file does not match this installation: {}",
            paths.app_bridge_backup.display()
        );
    }
    Ok(LegacyEnvironmentAction::Restore {
        codex_cli_path: backup.previous_codex_cli_path.clone(),
        cdxm_real_codex: backup.previous_cdxm_real_codex.clone(),
    })
}

fn active_legacy_processes<'a>(
    paths: &InstallPaths,
    inventory: &'a WindowsInventory,
) -> Vec<&'a LegacyProcess> {
    let legacy = legacy_paths(paths)
        .into_iter()
        .map(|path| normalized_path_key(&path.to_string_lossy()))
        .collect::<HashSet<_>>();
    inventory
        .processes
        .iter()
        .filter(|process| legacy.contains(&normalized_path_key(&process.path.to_string_lossy())))
        .collect()
}

fn ensure_owned_migration_not_running(
    paths: &InstallPaths,
    inventory: &WindowsInventory,
    action: &LegacyEnvironmentAction,
) -> anyhow::Result<()> {
    if matches!(action, LegacyEnvironmentAction::Preserve) {
        return Ok(());
    }
    let active = active_legacy_processes(paths, inventory)
        .into_iter()
        .map(|process| format!("PID {} ({})", process.pid, process.path.display()))
        .collect::<Vec<_>>();
    if !active.is_empty() {
        bail!(
            "fully quit Codex App before migrating the legacy bridge (active: {})",
            active.join(", ")
        );
    }
    Ok(())
}

fn read_owned_bridge_backup(
    paths: &InstallPaths,
    inventory: &WindowsInventory,
) -> anyhow::Result<Option<AppBridgeBackup>> {
    let Some(current) = inventory.user_codex_cli_path.as_deref() else {
        return Ok(None);
    };
    if !paths_equal(current, &legacy_bridge_path(paths)) && !is_codex_app_bin_path(current) {
        return Ok(None);
    }
    if !paths.app_bridge_backup.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(&paths.app_bridge_backup).with_context(|| {
        format!(
            "failed to read App bridge ownership file {}",
            paths.app_bridge_backup.display()
        )
    })?;
    serde_json::from_slice(&bytes).map(Some).with_context(|| {
        format!(
            "App bridge ownership file is invalid: {}",
            paths.app_bridge_backup.display()
        )
    })
}

fn apply_environment(action: &LegacyEnvironmentAction, user_path: &str) -> anyhow::Result<()> {
    let (restore, cli, real) = match action {
        LegacyEnvironmentAction::Preserve => (false, None, None),
        LegacyEnvironmentAction::Restore {
            codex_cli_path,
            cdxm_real_codex,
        } => (true, codex_cli_path.as_ref(), cdxm_real_codex.as_ref()),
    };
    let output = Command::new(windows_powershell_executable())
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            ENVIRONMENT_SCRIPT,
        ])
        .env("CDXM_UPDATE_RESTORE_ENV", if restore { "1" } else { "0" })
        .env(
            "CDXM_UPDATE_CLI_PRESENT",
            if cli.is_some() { "1" } else { "0" },
        )
        .env(
            "CDXM_UPDATE_CLI",
            cli.map_or_else(String::new, |p| p.to_string_lossy().into()),
        )
        .env(
            "CDXM_UPDATE_REAL_PRESENT",
            if real.is_some() { "1" } else { "0" },
        )
        .env(
            "CDXM_UPDATE_REAL",
            real.map_or_else(String::new, |p| p.to_string_lossy().into()),
        )
        .env("CDXM_UPDATE_USER_PATH", user_path)
        .output()
        .context("failed to start PowerShell environment update")?;
    require_powershell_success(output, "updating codex-monitor user environment")?;
    Ok(())
}

fn cleanup_legacy_files(paths: &InstallPaths, inventory: &WindowsInventory) -> anyhow::Result<()> {
    let active = active_legacy_processes(paths, inventory);
    let mut deferred_paths = active
        .iter()
        .map(|process| normalized_path_key(&process.path.to_string_lossy()))
        .collect::<HashSet<_>>();
    let runtime_key = normalized_path_key(&legacy_runtime_dir(paths).to_string_lossy());
    let runtime_is_active = active.iter().any(|process| {
        let process_key = normalized_path_key(&process.path.to_string_lossy());
        process_key.starts_with(&format!("{runtime_key}\\"))
    });
    if runtime_is_active {
        deferred_paths.extend(
            legacy_paths(paths)
                .into_iter()
                .filter(|path| path.starts_with(legacy_runtime_dir(paths)))
                .map(|path| normalized_path_key(&path.to_string_lossy())),
        );
    }
    for path in legacy_paths(paths) {
        if deferred_paths.contains(&normalized_path_key(&path.to_string_lossy())) {
            continue;
        }
        if path.is_file() {
            std::fs::remove_file(&path).with_context(|| {
                format!(
                    "failed to remove obsolete codex-monitor file {}",
                    path.display()
                )
            })?;
        }
    }
    let runtime = legacy_runtime_dir(paths);
    if runtime.is_dir() && std::fs::read_dir(&runtime)?.next().is_none() {
        std::fs::remove_dir(&runtime).with_context(|| {
            format!(
                "failed to remove empty legacy runtime directory {}",
                runtime.display()
            )
        })?;
    }
    if !active.is_empty() {
        let details = active
            .iter()
            .map(|process| format!("PID {} ({})", process.pid, process.path.display()))
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!(
            "codex-monitor update deferred cleanup of active legacy runtime files: {details}"
        );
    }
    Ok(())
}

fn cleanup_legacy_cdxm_binary(
    paths: &InstallPaths,
    inventory: &WindowsInventory,
) -> anyhow::Result<bool> {
    let legacy = legacy_cdxm_path(paths);
    let active = inventory
        .processes
        .iter()
        .filter(|process| paths_equal(&process.path, &legacy))
        .collect::<Vec<_>>();
    if !active.is_empty() {
        let pids = active
            .iter()
            .map(|process| process.pid.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!("codex-monitor update deferred cleanup of active legacy cdxm.exe (PID: {pids})");
        return Ok(true);
    }
    if legacy.is_file() {
        std::fs::remove_file(&legacy).with_context(|| {
            format!(
                "failed to remove inactive legacy binary {}",
                legacy.display()
            )
        })?;
    }
    Ok(false)
}

fn write_cdxm_compat_launcher(paths: &InstallPaths) -> anyhow::Result<()> {
    let base = directories::BaseDirs::new()
        .context("could not resolve the current user's home directory")?;
    let launcher = base.home_dir().join(".agents").join("bin").join("cdxm.cmd");
    let target = paths.root.join("bin").join("codex-monitor.exe");
    write_cdxm_compat_launcher_at(&launcher, &target)
}

fn write_cdxm_compat_launcher_at(launcher: &Path, target: &Path) -> anyhow::Result<()> {
    let parent = launcher.parent().with_context(|| {
        format!(
            "compatibility launcher has no parent: {}",
            launcher.display()
        )
    })?;
    std::fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create compatibility launcher directory {}",
            parent.display()
        )
    })?;
    let temporary = launcher.with_extension(format!("cmd.tmp-{}", std::process::id()));
    let content = format!(
        "@echo off\r\n\"{}\" %*\r\nexit /b %ERRORLEVEL%\r\n",
        target.display()
    );
    std::fs::write(&temporary, content).with_context(|| {
        format!(
            "failed to write compatibility launcher {}",
            temporary.display()
        )
    })?;
    if launcher.is_file() {
        std::fs::remove_file(launcher).with_context(|| {
            format!(
                "failed to replace compatibility launcher {}",
                launcher.display()
            )
        })?;
    }
    std::fs::rename(&temporary, launcher).with_context(|| {
        format!(
            "failed to publish compatibility launcher {}",
            launcher.display()
        )
    })?;
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
        .context("failed to start PowerShell for codex-monitor inventory")?;
    let stdout = require_powershell_success(output, "reading codex-monitor inventory")?;
    parse_inventory(&stdout)
}

fn parse_inventory(text: &str) -> anyhow::Result<WindowsInventory> {
    serde_json::from_str(text.trim()).context("codex-monitor inventory was not valid JSON")
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

pub(crate) fn paths_equal(left: &Path, right: &Path) -> bool {
    normalize_path(left.as_os_str()).eq_ignore_ascii_case(&normalize_path(right.as_os_str()))
}

/// True when `path` lives inside the Codex App's per-user `OpenAI\Codex\bin`
/// tree. codex-monitor's App bridge fronts exactly that directory, so once we
/// hold an ownership backup a CODEX_CLI_PATH value there is bridge residue we
/// can safely reclaim rather than a native value we must preserve.
fn is_codex_app_bin_path(path: &Path) -> bool {
    normalized_path_key(&path.to_string_lossy()).contains("\\openai\\codex\\bin\\")
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
    use tempfile::TempDir;

    fn fixture_inventory(user_codex_cli_path: Option<PathBuf>) -> WindowsInventory {
        WindowsInventory {
            user_codex_cli_path,
            user_path: Some(r"C:\Tools".into()),
            processes: Vec::new(),
        }
    }

    #[test]
    fn native_environment_is_preserved() {
        let paths = InstallPaths::new(PathBuf::from(r"C:\Users\me\.codex-monitor"));
        let inventory = fixture_inventory(Some(PathBuf::from(
            r"C:\Users\me\AppData\Local\OpenAI\Codex\bin\signed\codex.exe",
        )));
        assert_eq!(
            plan_legacy_environment(&paths, &inventory, None).unwrap(),
            LegacyEnvironmentAction::Preserve
        );
    }

    #[test]
    fn owned_bridge_restores_saved_environment() {
        let paths = InstallPaths::new(PathBuf::from(r"C:\Users\me\.codex-monitor"));
        let expected = legacy_bridge_path(&paths);
        let inventory = fixture_inventory(Some(expected.clone()));
        let backup = AppBridgeBackup {
            version: 1,
            previous_codex_cli_path: Some(PathBuf::from(r"C:\Native\codex.exe")),
            previous_cdxm_real_codex: None,
            bridge_path: expected,
        };
        assert_eq!(
            plan_legacy_environment(&paths, &inventory, Some(&backup)).unwrap(),
            LegacyEnvironmentAction::Restore {
                codex_cli_path: Some(PathBuf::from(r"C:\Native\codex.exe")),
                cdxm_real_codex: None,
            }
        );
    }

    #[test]
    fn owned_bridge_requires_valid_backup() {
        let paths = InstallPaths::new(PathBuf::from(r"C:\Users\me\.codex-monitor"));
        let inventory = fixture_inventory(Some(legacy_bridge_path(&paths)));
        assert!(plan_legacy_environment(&paths, &inventory, None).is_err());
    }

    #[test]
    fn drifted_app_bin_cli_with_backup_restores_saved_environment() {
        // Regression: CODEX_CLI_PATH that drifted from the bridge exe into the
        // Codex App's `OpenAI\Codex\bin` tree (a stale build hash after the App
        // updated) must be reclaimed, not stranded, so the App's code-mode host
        // resolves against the current build again.
        let paths = InstallPaths::new(PathBuf::from(r"C:\Users\me\.codex-monitor"));
        let inventory = fixture_inventory(Some(PathBuf::from(
            r"C:\Users\me\AppData\Local\OpenAI\Codex\bin\a7c12ebff69fb123\codex.exe",
        )));
        let backup = AppBridgeBackup {
            version: 1,
            previous_codex_cli_path: None,
            previous_cdxm_real_codex: None,
            bridge_path: legacy_bridge_path(&paths),
        };
        assert_eq!(
            plan_legacy_environment(&paths, &inventory, Some(&backup)).unwrap(),
            LegacyEnvironmentAction::Restore {
                codex_cli_path: None,
                cdxm_real_codex: None,
            }
        );
    }

    #[test]
    fn drifted_app_bin_cli_without_backup_is_preserved() {
        // No ownership backup means codex-monitor never recorded touching
        // CODEX_CLI_PATH, so an App-bin value is the user's native CLI: keep it.
        let paths = InstallPaths::new(PathBuf::from(r"C:\Users\me\.codex-monitor"));
        let inventory = fixture_inventory(Some(PathBuf::from(
            r"C:\Users\me\AppData\Local\OpenAI\Codex\bin\a7c12ebff69fb123\codex.exe",
        )));
        assert_eq!(
            plan_legacy_environment(&paths, &inventory, None).unwrap(),
            LegacyEnvironmentAction::Preserve
        );
    }

    #[test]
    fn read_backup_loads_for_drifted_app_bin_cli() {
        let temp = TempDir::new().unwrap();
        let paths = InstallPaths::new(temp.path().join("install"));
        std::fs::create_dir_all(&paths.root).unwrap();
        let backup = serde_json::json!({
            "version": 1,
            "previousCodexCliPath": null,
            "previousCdxmRealCodex": null,
            "bridgePath": legacy_bridge_path(&paths).to_string_lossy(),
        });
        std::fs::write(
            &paths.app_bridge_backup,
            serde_json::to_vec(&backup).unwrap(),
        )
        .unwrap();
        let inventory = fixture_inventory(Some(PathBuf::from(
            r"C:\Users\me\AppData\Local\OpenAI\Codex\bin\a7c12ebff69fb123\codex.exe",
        )));
        assert!(read_owned_bridge_backup(&paths, &inventory)
            .unwrap()
            .is_some());
    }

    #[test]
    fn unowned_environment_ignores_stale_invalid_backup() {
        let temp = TempDir::new().unwrap();
        let paths = InstallPaths::new(temp.path().join("install"));
        std::fs::create_dir_all(&paths.root).unwrap();
        std::fs::write(&paths.app_bridge_backup, b"not json").unwrap();
        let inventory = fixture_inventory(Some(PathBuf::from(r"C:\Native\codex.exe")));

        assert!(read_owned_bridge_backup(&paths, &inventory)
            .unwrap()
            .is_none());
    }

    #[test]
    fn inventory_json_maps_environment_and_exact_process_paths() {
        let inventory = parse_inventory(
            r#"{"userCodexCliPath":"C:\\Native\\codex.exe","userRealCodex":null,"userPath":"C:\\Tools","processes":[{"pid":42,"path":"C:\\Users\\me\\.codex-monitor\\bin\\cdxm-codex-app-bridge.exe"}]}"#,
        )
        .unwrap();
        assert_eq!(inventory.user_path.as_deref(), Some(r"C:\Tools"));
        assert_eq!(inventory.processes[0].pid, 42);
        assert!(inventory.processes[0]
            .path
            .ends_with("cdxm-codex-app-bridge.exe"));
    }

    #[test]
    fn exact_active_legacy_path_blocks_owned_migration() {
        let paths = InstallPaths::new(PathBuf::from(r"C:\Users\me\.codex-monitor"));
        let mut inventory = fixture_inventory(Some(legacy_bridge_path(&paths)));
        inventory.processes.push(LegacyProcess {
            pid: 42,
            path: legacy_bridge_path(&paths),
        });
        let action = LegacyEnvironmentAction::Restore {
            codex_cli_path: Some(PathBuf::from(r"C:\Native\codex.exe")),
            cdxm_real_codex: None,
        };
        let error = ensure_owned_migration_not_running(&paths, &inventory, &action).unwrap_err();
        assert!(error.to_string().contains("PID 42"));
        assert!(error.to_string().contains("fully quit Codex App"));
    }

    #[test]
    fn active_legacy_path_is_deferred_for_native_environment() {
        let paths = InstallPaths::new(PathBuf::from(r"C:\Users\me\.codex-monitor"));
        let mut inventory = fixture_inventory(Some(PathBuf::from(r"C:\Native\codex.exe")));
        inventory.processes.push(LegacyProcess {
            pid: 42,
            path: legacy_bridge_path(&paths),
        });

        assert!(ensure_owned_migration_not_running(
            &paths,
            &inventory,
            &LegacyEnvironmentAction::Preserve,
        )
        .is_ok());
    }

    #[test]
    fn cleanup_removes_only_fixed_legacy_files() {
        let temp = TempDir::new().unwrap();
        let paths = InstallPaths::new(temp.path().join("install"));
        for path in legacy_paths(&paths) {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"legacy").unwrap();
        }
        let keep = paths.root.join("runtime").join("keep.txt");
        std::fs::write(&keep, b"keep").unwrap();
        cleanup_legacy_files(&paths, &fixture_inventory(None)).unwrap();
        assert!(legacy_paths(&paths).iter().all(|path| !path.exists()));
        assert!(keep.is_file());
    }

    #[test]
    fn cleanup_defers_only_active_legacy_files() {
        let temp = TempDir::new().unwrap();
        let paths = InstallPaths::new(temp.path().join("install"));
        let legacy = legacy_paths(&paths);
        for path in &legacy {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"legacy").unwrap();
        }
        let mut inventory = fixture_inventory(Some(PathBuf::from(r"C:\Native\codex.exe")));
        inventory.processes.push(LegacyProcess {
            pid: 42,
            path: legacy[1].clone(),
        });

        cleanup_legacy_files(&paths, &inventory).unwrap();

        assert!(!legacy[0].exists());
        assert!(legacy[1..].iter().all(|path| path.exists()));
    }

    #[test]
    fn active_legacy_cdxm_binary_is_deferred() {
        let temp = TempDir::new().unwrap();
        let paths = InstallPaths::new(temp.path().join("install"));
        let legacy = legacy_cdxm_path(&paths);
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, b"old cdxm").unwrap();
        let mut inventory = fixture_inventory(Some(PathBuf::from(r"C:\Native\codex.exe")));
        inventory.processes.push(LegacyProcess {
            pid: 42,
            path: legacy.clone(),
        });

        assert!(cleanup_legacy_cdxm_binary(&paths, &inventory).unwrap());
        assert!(legacy.exists());
    }

    #[test]
    fn inactive_legacy_cdxm_binary_is_removed() {
        let temp = TempDir::new().unwrap();
        let paths = InstallPaths::new(temp.path().join("install"));
        let legacy = legacy_cdxm_path(&paths);
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, b"old cdxm").unwrap();

        assert!(!cleanup_legacy_cdxm_binary(&paths, &fixture_inventory(None)).unwrap());
        assert!(!legacy.exists());
    }

    #[test]
    fn compatibility_launcher_forwards_to_installed_monitor() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("install/bin/codex-monitor.exe");
        let launcher = temp.path().join("agents/bin/cdxm.cmd");

        write_cdxm_compat_launcher_at(&launcher, &target).unwrap();

        let content = std::fs::read_to_string(launcher).unwrap();
        assert!(content.contains("@echo off"));
        assert!(content.contains(&format!("\"{}\" %*", target.display())));
        assert!(content.contains("exit /b %ERRORLEVEL%"));
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
