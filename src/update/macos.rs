#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use anyhow::{bail, Context};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const LAUNCH_AGENT_PREFIX: &str = "com.local.codex-monitor.agmsg.";
const CDXM_LAUNCHER: &str = r#"#!/usr/bin/env sh
set -eu
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec "$SCRIPT_DIR/codex-monitor" "$@"
"#;

#[derive(Debug, Clone)]
pub(crate) struct MacInstallPaths {
    pub root: PathBuf,
    pub binary: PathBuf,
    pub launcher: PathBuf,
    pub update_result: PathBuf,
    pub launch_agents_dir: PathBuf,
    pub legacy_binaries: Vec<PathBuf>,
}

impl MacInstallPaths {
    pub fn for_home(home: &Path) -> Self {
        Self::new(home, home.join(".codex-monitor"))
    }

    fn new(home: &Path, root: PathBuf) -> Self {
        Self {
            binary: root.join("bin/codex-monitor"),
            launcher: root.join("bin/cdxm"),
            update_result: root.join("last-update.json"),
            launch_agents_dir: home.join("Library/LaunchAgents"),
            legacy_binaries: vec![
                home.join(".cargo/bin/codex-monitor"),
                home.join(".cargo/bin/cdxm"),
            ],
            root,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct MacFinalizeSummary {
    pub migrated_agents: usize,
    pub reloaded_agents: usize,
    pub removed_legacy_binaries: usize,
}

pub(crate) trait Launchctl {
    fn is_loaded(&self, label: &str) -> anyhow::Result<bool>;
    fn reload(&self, label: &str, plist: &Path) -> anyhow::Result<()>;
    fn active_arguments(&self, label: &str) -> anyhow::Result<Vec<String>>;
}

struct RealLaunchctl {
    user_id: String,
}

impl RealLaunchctl {
    fn new() -> anyhow::Result<Self> {
        let output = Command::new("id").arg("-u").output()?;
        if !output.status.success() {
            bail!("id -u failed while preparing LaunchAgent migration");
        }
        Ok(Self {
            user_id: String::from_utf8_lossy(&output.stdout).trim().to_string(),
        })
    }

    fn service(&self, label: &str) -> String {
        format!("gui/{}/{label}", self.user_id)
    }

    fn domain(&self) -> String {
        format!("gui/{}", self.user_id)
    }
}

impl Launchctl for RealLaunchctl {
    fn is_loaded(&self, label: &str) -> anyhow::Result<bool> {
        Ok(Command::new("launchctl")
            .args(["print", &self.service(label)])
            .output()
            .with_context(|| format!("failed to inspect LaunchAgent {label}"))?
            .status
            .success())
    }

    fn reload(&self, label: &str, plist: &Path) -> anyhow::Result<()> {
        let service = self.service(label);
        if self.is_loaded(label)? {
            let bootout = Command::new("launchctl")
                .args(["bootout", &service])
                .output()
                .with_context(|| format!("failed to boot out LaunchAgent {label}"))?;
            if !bootout.status.success() {
                bail!(
                    "launchctl bootout failed for {label}: {}{}",
                    String::from_utf8_lossy(&bootout.stderr),
                    String::from_utf8_lossy(&bootout.stdout)
                );
            }
        }
        let plist_text = plist.to_string_lossy();
        let bootstrap = Command::new("launchctl")
            .args(["bootstrap", &self.domain(), &plist_text])
            .output()
            .with_context(|| format!("failed to bootstrap LaunchAgent {label}"))?;
        if !bootstrap.status.success() {
            bail!(
                "launchctl bootstrap failed for {label}: {}{}",
                String::from_utf8_lossy(&bootstrap.stderr),
                String::from_utf8_lossy(&bootstrap.stdout)
            );
        }
        Ok(())
    }

    fn active_arguments(&self, label: &str) -> anyhow::Result<Vec<String>> {
        let output = Command::new("launchctl")
            .args(["print", &self.service(label)])
            .output()
            .with_context(|| format!("failed to verify LaunchAgent {label}"))?;
        if !output.status.success() {
            bail!(
                "launchctl print failed for {label}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(crate::launchd::parse_launchctl_arguments(
            &String::from_utf8_lossy(&output.stdout),
        ))
    }
}

#[derive(Debug)]
struct AgentMigration {
    label: String,
    plist_path: PathBuf,
    original: String,
    migrated: String,
    was_loaded: bool,
}

pub(crate) fn install_paths() -> anyhow::Result<MacInstallPaths> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")?;
    let root = std::env::var_os("CDXM_INSTALL_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex-monitor"));
    Ok(MacInstallPaths::new(&home, root))
}

pub(crate) fn finalize_install() -> anyhow::Result<MacFinalizeSummary> {
    if !cfg!(target_os = "macos") {
        bail!("macOS installation finalization is only available on macOS");
    }
    let paths = install_paths()?;
    let launchctl = RealLaunchctl::new()?;
    finalize_with_launchctl(&paths, &launchctl)
}

pub(crate) fn write_cdxm_launcher(root: &Path) -> anyhow::Result<()> {
    atomic_write(&root.join("bin/cdxm"), CDXM_LAUNCHER.as_bytes(), true)
}

fn owned_paths(home: &Path) -> BTreeSet<PathBuf> {
    let paths = MacInstallPaths::for_home(home);
    owned_paths_for(&paths)
}

fn owned_paths_for(paths: &MacInstallPaths) -> BTreeSet<PathBuf> {
    let mut owned = BTreeSet::from([paths.binary.clone(), paths.launcher.clone()]);
    owned.extend(paths.legacy_binaries.iter().cloned());
    owned
}

fn migrate_plist_program(
    plist: &str,
    canonical: &Path,
    owned: &BTreeSet<PathBuf>,
) -> anyhow::Result<Option<String>> {
    let arguments = crate::launchd::parse_program_arguments_from_plist(plist);
    let Some(program) = arguments.first() else {
        return Ok(None);
    };
    if !owned.contains(Path::new(program)) {
        return Ok(None);
    }
    if Path::new(program) == canonical {
        return Ok(Some(plist.to_string()));
    }

    let key = "<key>ProgramArguments</key>";
    let key_end = plist
        .find(key)
        .map(|offset| offset + key.len())
        .context("owned LaunchAgent plist has no ProgramArguments key")?;
    let array_start = plist[key_end..]
        .find("<array>")
        .map(|offset| key_end + offset + "<array>".len())
        .context("owned LaunchAgent plist has no ProgramArguments array")?;
    let string_open = plist[array_start..]
        .find("<string>")
        .map(|offset| array_start + offset + "<string>".len())
        .context("owned LaunchAgent plist has no executable argument")?;
    let string_close = plist[string_open..]
        .find("</string>")
        .map(|offset| string_open + offset)
        .context("owned LaunchAgent plist executable argument is not closed")?;
    let mut migrated = plist.to_string();
    migrated.replace_range(
        string_open..string_close,
        &crate::launchd::escape_xml(&canonical.to_string_lossy()),
    );
    Ok(Some(migrated))
}

fn inventory_agents(
    paths: &MacInstallPaths,
    launchctl: &dyn Launchctl,
) -> anyhow::Result<Vec<AgentMigration>> {
    let entries = match fs::read_dir(&paths.launch_agents_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let owned = owned_paths_for(paths);
    let mut agents = Vec::new();
    for entry in entries {
        let entry = entry?;
        let file_name = entry.file_name().to_string_lossy().to_string();
        let Some(label) = file_name.strip_suffix(".plist") else {
            continue;
        };
        if !label.starts_with(LAUNCH_AGENT_PREFIX)
            || crate::launchd::parse_agmsg_launch_agent_label(label).is_none()
        {
            continue;
        }
        let plist_path = entry.path();
        let original = fs::read_to_string(&plist_path).with_context(|| {
            format!("failed to read LaunchAgent plist {}", plist_path.display())
        })?;
        let Some(migrated) = migrate_plist_program(&original, &paths.launcher, &owned)? else {
            continue;
        };
        agents.push(AgentMigration {
            label: label.to_string(),
            plist_path,
            original,
            migrated,
            was_loaded: launchctl.is_loaded(label)?,
        });
    }
    agents.sort_by(|left, right| left.label.cmp(&right.label));
    Ok(agents)
}

fn finalize_with_launchctl(
    paths: &MacInstallPaths,
    launchctl: &dyn Launchctl,
) -> anyhow::Result<MacFinalizeSummary> {
    if !paths.binary.is_file() {
        bail!(
            "canonical codex-monitor binary is missing: {}",
            paths.binary.display()
        );
    }
    write_cdxm_launcher(&paths.root)?;
    let agents = inventory_agents(paths, launchctl)?;
    let mut applied = Vec::new();
    let mut reloaded = 0;

    let apply_result = (|| -> anyhow::Result<()> {
        for agent in &agents {
            atomic_write(&agent.plist_path, agent.migrated.as_bytes(), false)?;
            applied.push(agent);
            if agent.was_loaded {
                launchctl.reload(&agent.label, &agent.plist_path)?;
                let active = launchctl.active_arguments(&agent.label)?;
                if active.first().map(Path::new) != Some(paths.launcher.as_path()) {
                    bail!(
                        "LaunchAgent {} did not activate the canonical launcher",
                        agent.label
                    );
                }
                reloaded += 1;
            }
        }
        Ok(())
    })();

    if let Err(error) = apply_result {
        let mut rollback_failures = Vec::new();
        for agent in applied.into_iter().rev() {
            if let Err(restore_error) =
                atomic_write(&agent.plist_path, agent.original.as_bytes(), false)
            {
                rollback_failures.push(format!(
                    "failed to restore {}: {restore_error:#}",
                    agent.label
                ));
                continue;
            }
            if agent.was_loaded {
                if let Err(reload_error) = launchctl.reload(&agent.label, &agent.plist_path) {
                    rollback_failures.push(format!(
                        "failed to reload restored {}: {reload_error:#}",
                        agent.label
                    ));
                }
            }
        }
        if rollback_failures.is_empty() {
            return Err(error);
        }
        bail!(
            "{error:#}; rollback also failed: {}",
            rollback_failures.join("; ")
        );
    }

    let mut removed = 0;
    for legacy in &paths.legacy_binaries {
        match fs::symlink_metadata(legacy) {
            Ok(metadata) if metadata.is_dir() => {
                bail!(
                    "legacy executable path is unexpectedly a directory: {}",
                    legacy.display()
                )
            }
            Ok(_) => {
                fs::remove_file(legacy).with_context(|| {
                    format!("failed to remove legacy executable {}", legacy.display())
                })?;
                removed += 1;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }

    Ok(MacFinalizeSummary {
        migrated_agents: agents.len(),
        reloaded_agents: reloaded,
        removed_legacy_binaries: removed,
    })
}

fn atomic_write(path: &Path, contents: &[u8], executable: bool) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create directory {}", parent.display()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let temporary = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    if fs::symlink_metadata(&temporary).is_ok() {
        fs::remove_file(&temporary).with_context(|| {
            format!(
                "failed to remove stale temporary file {}",
                temporary.display()
            )
        })?;
    }
    fs::write(&temporary, contents)
        .with_context(|| format!("failed to write temporary file {}", temporary.display()))?;
    if executable {
        set_executable(&temporary)?;
    }
    #[cfg(windows)]
    if fs::symlink_metadata(path).is_ok() {
        fs::remove_file(path)
            .with_context(|| format!("failed to replace existing file {}", path.display()))?;
    }
    fs::rename(&temporary, path).with_context(|| {
        format!(
            "failed to publish temporary file {} to {}",
            temporary.display(),
            path.display()
        )
    })?;
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .with_context(|| format!("failed to make {} executable", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::{BTreeMap, BTreeSet},
        fs,
        path::Path,
        sync::Mutex,
    };
    use tempfile::TempDir;

    const LABEL_ONE: &str = "com.local.codex-monitor.agmsg.dev.one";
    const LABEL_TWO: &str = "com.local.codex-monitor.agmsg.dev.two";

    fn fixture_plist(program: &str, marker: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key>\n  <string>{marker}</string>\n  <key>ProgramArguments</key>\n  <array>\n    <string>{program}</string>\n    <string>agmsg</string>\n    <string>watch</string>\n    <string>--team</string>\n    <string>dev</string>\n  </array>\n</dict>\n</plist>\n"
        )
    }

    #[test]
    fn launcher_executes_the_single_native_binary() {
        let temp = TempDir::new().unwrap();
        write_cdxm_launcher(temp.path()).unwrap();
        let text = fs::read_to_string(temp.path().join("bin/cdxm")).unwrap();
        assert!(text.contains("exec \"$SCRIPT_DIR/codex-monitor\" \"$@\""));
    }

    #[test]
    fn plist_migration_changes_only_owned_first_argument() {
        let home = Path::new("/Users/me");
        let original = fixture_plist("/Users/me/.cargo/bin/cdxm", "unchanged-marker");
        let migrated = migrate_plist_program(
            &original,
            Path::new("/Users/me/.codex-monitor/bin/cdxm"),
            &owned_paths(home),
        )
        .unwrap()
        .unwrap();
        let arguments = crate::launchd::parse_program_arguments_from_plist(&migrated);
        assert_eq!(arguments[0], "/Users/me/.codex-monitor/bin/cdxm");
        assert_eq!(&arguments[1..], &["agmsg", "watch", "--team", "dev"]);
        assert!(migrated.contains("unchanged-marker"));
        assert!(migrate_plist_program(
            &fixture_plist("/usr/local/bin/foreign", "foreign"),
            Path::new("/Users/me/.codex-monitor/bin/cdxm"),
            &owned_paths(home),
        )
        .unwrap()
        .is_none());
    }

    struct FakeLaunchctl {
        loaded: BTreeSet<String>,
        active: Mutex<BTreeMap<String, Vec<String>>>,
        fail_once: Mutex<Option<String>>,
    }

    impl Launchctl for FakeLaunchctl {
        fn is_loaded(&self, label: &str) -> anyhow::Result<bool> {
            Ok(self.loaded.contains(label))
        }

        fn reload(&self, label: &str, plist: &Path) -> anyhow::Result<()> {
            if self.fail_once.lock().unwrap().as_deref() == Some(label) {
                *self.fail_once.lock().unwrap() = None;
                anyhow::bail!("injected reload failure for {label}");
            }
            let text = fs::read_to_string(plist)?;
            self.active.lock().unwrap().insert(
                label.to_string(),
                crate::launchd::parse_program_arguments_from_plist(&text),
            );
            Ok(())
        }

        fn active_arguments(&self, label: &str) -> anyhow::Result<Vec<String>> {
            Ok(self
                .active
                .lock()
                .unwrap()
                .get(label)
                .cloned()
                .unwrap_or_default())
        }
    }

    #[test]
    fn migration_failure_restores_changed_plists_and_loaded_services() {
        let temp = TempDir::new().unwrap();
        let home = temp.path();
        let paths = MacInstallPaths::for_home(home);
        fs::create_dir_all(&paths.launch_agents_dir).unwrap();
        fs::create_dir_all(paths.binary.parent().unwrap()).unwrap();
        fs::write(&paths.binary, b"binary").unwrap();
        let first = paths.launch_agents_dir.join(format!("{LABEL_ONE}.plist"));
        let second = paths.launch_agents_dir.join(format!("{LABEL_TWO}.plist"));
        let first_original =
            fixture_plist(&home.join(".cargo/bin/cdxm").to_string_lossy(), LABEL_ONE);
        let second_original = fixture_plist(
            &home.join(".cargo/bin/codex-monitor").to_string_lossy(),
            LABEL_TWO,
        );
        fs::write(&first, &first_original).unwrap();
        fs::write(&second, &second_original).unwrap();
        let fake = FakeLaunchctl {
            loaded: BTreeSet::from([LABEL_ONE.to_string(), LABEL_TWO.to_string()]),
            active: Mutex::new(BTreeMap::new()),
            fail_once: Mutex::new(Some(LABEL_TWO.to_string())),
        };

        assert!(finalize_with_launchctl(&paths, &fake).is_err());
        assert_eq!(fs::read_to_string(first).unwrap(), first_original);
        assert_eq!(fs::read_to_string(second).unwrap(), second_original);
        assert_eq!(
            fake.active_arguments(LABEL_ONE).unwrap().first().cloned(),
            Some(home.join(".cargo/bin/cdxm").to_string_lossy().into())
        );
        assert_eq!(
            fake.active_arguments(LABEL_TWO).unwrap().first().cloned(),
            Some(
                home.join(".cargo/bin/codex-monitor")
                    .to_string_lossy()
                    .into()
            )
        );
    }

    #[test]
    fn successful_migration_reloads_agents_and_removes_legacy_binaries() {
        let temp = TempDir::new().unwrap();
        let home = temp.path();
        let paths = MacInstallPaths::for_home(home);
        fs::create_dir_all(&paths.launch_agents_dir).unwrap();
        fs::create_dir_all(paths.binary.parent().unwrap()).unwrap();
        fs::create_dir_all(home.join(".cargo/bin")).unwrap();
        fs::write(&paths.binary, b"binary").unwrap();
        for legacy in &paths.legacy_binaries {
            fs::write(legacy, b"legacy").unwrap();
        }
        let plist = paths.launch_agents_dir.join(format!("{LABEL_ONE}.plist"));
        fs::write(
            &plist,
            fixture_plist(&home.join(".cargo/bin/cdxm").to_string_lossy(), LABEL_ONE),
        )
        .unwrap();
        let fake = FakeLaunchctl {
            loaded: BTreeSet::from([LABEL_ONE.to_string()]),
            active: Mutex::new(BTreeMap::new()),
            fail_once: Mutex::new(None),
        };

        let summary = finalize_with_launchctl(&paths, &fake).unwrap();
        assert_eq!(
            summary,
            MacFinalizeSummary {
                migrated_agents: 1,
                reloaded_agents: 1,
                removed_legacy_binaries: 2,
            }
        );
        assert_eq!(
            crate::launchd::parse_program_arguments_from_plist(&fs::read_to_string(plist).unwrap())
                [0],
            paths.launcher.to_string_lossy()
        );
        assert!(paths.legacy_binaries.iter().all(|path| !path.exists()));
        assert!(fs::read_to_string(&paths.launcher)
            .unwrap()
            .contains("exec \"$SCRIPT_DIR/codex-monitor\" \"$@\""));
    }
}
