use super::model::{sha256_file, ManagedFile, UpdateManifest, UpdateResult, RESULT_VERSION};
use anyhow::{bail, Context};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ApplySummary {
    pub changed: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub deferred_cleanup: Option<PathBuf>,
}

struct PreparedOperation {
    id: ManagedFile,
    destination: PathBuf,
    prepared: Option<PathBuf>,
    backup: PathBuf,
}

struct AppliedOperation {
    destination: PathBuf,
    backup: Option<PathBuf>,
    installed_new: bool,
}

pub fn apply_manifest(manifest: &UpdateManifest) -> anyhow::Result<ApplySummary> {
    apply_manifest_with_hook(manifest, |_| Ok(()))
}

fn apply_manifest_with_hook<F>(
    manifest: &UpdateManifest,
    mut before_replace: F,
) -> anyhow::Result<ApplySummary>
where
    F: FnMut(ManagedFile) -> anyhow::Result<()>,
{
    manifest.validate_shape()?;
    let suffix = transaction_suffix()?;
    let backup_root = manifest
        .install_root
        .join(format!(".update-backup-{suffix}"));
    if backup_root.exists() {
        bail!(
            "update backup directory already exists: {}",
            backup_root.display()
        );
    }

    let files = manifest
        .files
        .iter()
        .map(|file| (file.id, file))
        .collect::<BTreeMap<_, _>>();
    let mut operations = Vec::new();
    let mut unchanged = 0;
    let mut changed = 0;
    let mut removed = 0;

    for id in ManagedFile::ALL {
        let file = files
            .get(&id)
            .expect("manifest shape validation requires every managed file");
        let destination = id.destination(&manifest.install_root);
        let backup = backup_root.join(id.staged_name());
        match file.sha256.as_deref() {
            Some(expected_hash) => {
                let staged = manifest.staging_root.join(id.staged_name());
                let staged_hash = sha256_file(&staged)
                    .with_context(|| format!("failed to validate staged update file for {id:?}"))?;
                if staged_hash != expected_hash {
                    bail!(
                        "staged update hash changed for {} (expected {expected_hash}, got {staged_hash})",
                        staged.display()
                    );
                }
                if destination.is_file() && sha256_file(&destination)? == expected_hash {
                    unchanged += 1;
                    continue;
                }
                let parent = destination.parent().with_context(|| {
                    format!(
                        "update destination has no parent: {}",
                        destination.display()
                    )
                })?;
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create update destination {}", parent.display())
                })?;
                let prepared =
                    parent.join(format!(".cdxm-update-new-{suffix}-{}", id.staged_name()));
                std::fs::copy(&staged, &prepared).with_context(|| {
                    format!(
                        "failed to prepare update file {} from {}",
                        prepared.display(),
                        staged.display()
                    )
                })?;
                if sha256_file(&prepared)? != expected_hash {
                    let _ = std::fs::remove_file(&prepared);
                    bail!("prepared update hash mismatch for {}", prepared.display());
                }
                operations.push(PreparedOperation {
                    id,
                    destination,
                    prepared: Some(prepared),
                    backup,
                });
                changed += 1;
            }
            None => {
                if destination.exists() {
                    operations.push(PreparedOperation {
                        id,
                        destination,
                        prepared: None,
                        backup,
                    });
                    removed += 1;
                } else {
                    unchanged += 1;
                }
            }
        }
    }

    if operations.is_empty() {
        return Ok(ApplySummary {
            changed,
            removed,
            unchanged,
            deferred_cleanup: None,
        });
    }

    std::fs::create_dir(&backup_root).with_context(|| {
        format!(
            "failed to create update backup directory {}",
            backup_root.display()
        )
    })?;
    let mut applied = Vec::new();
    let apply_result = (|| -> anyhow::Result<()> {
        for operation in &operations {
            before_replace(operation.id)?;
            let backup = if operation.destination.exists() {
                std::fs::rename(&operation.destination, &operation.backup).with_context(|| {
                    format!(
                        "failed to back up update destination {}",
                        operation.destination.display()
                    )
                })?;
                Some(operation.backup.clone())
            } else {
                None
            };
            applied.push(AppliedOperation {
                destination: operation.destination.clone(),
                backup,
                installed_new: false,
            });
            if let Some(prepared) = &operation.prepared {
                std::fs::rename(prepared, &operation.destination).with_context(|| {
                    format!(
                        "failed to install prepared update {}",
                        operation.destination.display()
                    )
                })?;
                applied
                    .last_mut()
                    .expect("applied operation was just inserted")
                    .installed_new = true;
            }
        }
        verify_installed_state(manifest)
    })();

    if let Err(error) = apply_result {
        let rollback_error = rollback(&applied);
        cleanup_prepared(&operations);
        let _ = std::fs::remove_dir_all(&backup_root);
        if let Err(rollback_error) = rollback_error {
            bail!("{error:#}; rollback also failed: {rollback_error:#}");
        }
        return Err(error);
    }

    cleanup_prepared(&operations);
    let cleanup_marker = backup_root.join(".cleanup-ready");
    std::fs::write(&cleanup_marker, b"verified update backup\n").with_context(|| {
        format!(
            "update succeeded but cleanup marker creation failed: {}",
            cleanup_marker.display()
        )
    })?;
    let deferred_cleanup = match std::fs::remove_dir_all(&backup_root) {
        Ok(()) => None,
        Err(_) => Some(backup_root),
    };
    Ok(ApplySummary {
        changed,
        removed,
        unchanged,
        deferred_cleanup,
    })
}

pub fn cleanup_ready_backups(install_root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    if !install_root.is_dir() {
        return Ok(Vec::new());
    }
    let mut remaining = Vec::new();
    for entry in std::fs::read_dir(install_root).with_context(|| {
        format!(
            "failed to inspect codex-monitor install root {}",
            install_root.display()
        )
    })? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !file_type.is_dir() || !name.starts_with(".update-backup-") {
            continue;
        }
        let path = entry.path();
        if !path.join(".cleanup-ready").is_file() {
            continue;
        }
        if std::fs::remove_dir_all(&path).is_err() {
            remaining.push(path);
        }
    }
    Ok(remaining)
}

fn verify_installed_state(manifest: &UpdateManifest) -> anyhow::Result<()> {
    for file in &manifest.files {
        let destination = file.id.destination(&manifest.install_root);
        match file.sha256.as_deref() {
            Some(expected) => {
                let actual = sha256_file(&destination)?;
                if actual != expected {
                    bail!(
                        "installed update hash mismatch for {} (expected {expected}, got {actual})",
                        destination.display()
                    );
                }
            }
            None if destination.exists() => {
                bail!(
                    "obsolete optional runtime was not removed: {}",
                    destination.display()
                );
            }
            None => {}
        }
    }
    Ok(())
}

fn rollback(applied: &[AppliedOperation]) -> anyhow::Result<()> {
    let mut failures = Vec::new();
    for operation in applied.iter().rev() {
        if operation.installed_new && operation.destination.exists() {
            if let Err(error) = std::fs::remove_file(&operation.destination) {
                failures.push(format!(
                    "failed to remove {}: {error}",
                    operation.destination.display()
                ));
                continue;
            }
        }
        if let Some(backup) = &operation.backup {
            if let Err(error) = std::fs::rename(backup, &operation.destination) {
                failures.push(format!(
                    "failed to restore {}: {error}",
                    operation.destination.display()
                ));
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        bail!(failures.join("; "))
    }
}

fn cleanup_prepared(operations: &[PreparedOperation]) {
    for operation in operations {
        if let Some(prepared) = &operation.prepared {
            let _ = std::fs::remove_file(prepared);
        }
    }
}

fn transaction_suffix() -> anyhow::Result<String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_nanos();
    Ok(format!("{}-{nanos}", std::process::id()))
}

pub fn write_result_atomic(path: &Path, result: &UpdateResult) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("update result path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create update result directory {}",
            parent.display()
        )
    })?;
    let temporary = parent.join(format!(
        ".{}-{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("last-update.json"),
        std::process::id()
    ));
    let encoded = serde_json::to_vec_pretty(result).context("failed to encode update result")?;
    std::fs::write(&temporary, encoded).with_context(|| {
        format!(
            "failed to write temporary update result {}",
            temporary.display()
        )
    })?;
    if path.exists() {
        std::fs::remove_file(path)
            .with_context(|| format!("failed to replace prior update result {}", path.display()))?;
    }
    std::fs::rename(&temporary, path)
        .with_context(|| format!("failed to publish update result {}", path.display()))?;
    Ok(())
}

pub fn take_previous_failure(path: &Path) -> anyhow::Result<Option<String>> {
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read prior update result {}", path.display()))?;
    let result: UpdateResult = serde_json::from_slice(&bytes)
        .with_context(|| format!("prior update result is invalid: {}", path.display()))?;
    if result.version != RESULT_VERSION {
        bail!("unsupported update result version {}", result.version);
    }
    std::fs::remove_file(path)
        .with_context(|| format!("failed to consume prior update result {}", path.display()))?;
    Ok((!result.success).then_some(result.message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::model::{
        sha256_file, ManagedFile, StagedFile, UpdateManifest, UpdateResult, MANIFEST_VERSION,
        RESULT_VERSION,
    };
    use tempfile::TempDir;

    struct Fixture {
        _temp: TempDir,
        manifest: UpdateManifest,
    }

    fn fixture(absent_optional: Option<ManagedFile>) -> Fixture {
        let temp = TempDir::new().unwrap();
        let install_root = temp.path().join("install");
        let staging_root = install_root.join("staging");
        std::fs::create_dir_all(&staging_root).unwrap();
        let mut files = Vec::new();
        for id in ManagedFile::ALL {
            let destination = id.destination(&install_root);
            std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
            std::fs::write(&destination, format!("old-{id:?}")).unwrap();
            let sha256 = if Some(id) == absent_optional {
                None
            } else {
                let staged = staging_root.join(id.staged_name());
                std::fs::write(&staged, format!("new-{id:?}")).unwrap();
                Some(sha256_file(&staged).unwrap())
            };
            files.push(StagedFile { id, sha256 });
        }
        Fixture {
            manifest: UpdateManifest {
                version: MANIFEST_VERSION,
                install_root,
                staging_root,
                files,
            },
            _temp: temp,
        }
    }

    #[test]
    fn apply_installs_complete_manifest_and_removes_absent_optional_files() {
        let fixture = fixture(Some(ManagedFile::CommandRunner));
        let summary = apply_manifest(&fixture.manifest).unwrap();
        assert_eq!(summary.changed, 6);
        assert_eq!(summary.removed, 1);
        assert!(!ManagedFile::CommandRunner
            .destination(&fixture.manifest.install_root)
            .exists());
        assert_eq!(
            std::fs::read(ManagedFile::CodexMonitor.destination(&fixture.manifest.install_root))
                .unwrap(),
            b"new-CodexMonitor"
        );
    }

    #[test]
    fn apply_skips_identical_files_without_leaving_backups() {
        let fixture = fixture(None);
        for file in &fixture.manifest.files {
            let staged = fixture.manifest.staging_root.join(file.id.staged_name());
            std::fs::copy(&staged, file.id.destination(&fixture.manifest.install_root)).unwrap();
        }
        let summary = apply_manifest(&fixture.manifest).unwrap();
        assert_eq!(summary.unchanged, ManagedFile::ALL.len());
        assert_eq!(summary.changed, 0);
        assert!(std::fs::read_dir(&fixture.manifest.install_root)
            .unwrap()
            .flatten()
            .all(|entry| !entry
                .file_name()
                .to_string_lossy()
                .starts_with(".update-backup-")));
    }

    #[test]
    fn apply_failure_restores_every_destination() {
        let fixture = fixture(None);
        let result = apply_manifest_with_hook(&fixture.manifest, |id| {
            if id == ManagedFile::CodeModeHost {
                anyhow::bail!("injected replacement failure");
            }
            Ok(())
        });
        assert!(result.is_err());
        for id in ManagedFile::ALL {
            assert_eq!(
                std::fs::read(id.destination(&fixture.manifest.install_root)).unwrap(),
                format!("old-{id:?}").as_bytes()
            );
        }
    }

    #[test]
    fn update_result_is_atomic_and_failure_is_reported_once() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("last-update.json");
        write_result_atomic(
            &path,
            &UpdateResult {
                version: RESULT_VERSION,
                success: false,
                message: "broken".into(),
            },
        )
        .unwrap();
        assert_eq!(
            take_previous_failure(&path).unwrap().as_deref(),
            Some("broken")
        );
        assert_eq!(take_previous_failure(&path).unwrap(), None);
    }

    #[test]
    fn cleanup_removes_only_verified_backup_directories() {
        let temp = TempDir::new().unwrap();
        let ready = temp.path().join(".update-backup-ready");
        let unverified = temp.path().join(".update-backup-unverified");
        std::fs::create_dir(&ready).unwrap();
        std::fs::create_dir(&unverified).unwrap();
        std::fs::write(ready.join(".cleanup-ready"), b"verified").unwrap();
        std::fs::write(ready.join("old.exe"), b"old").unwrap();
        std::fs::write(unverified.join("old.exe"), b"recoverable").unwrap();

        assert!(cleanup_ready_backups(temp.path()).unwrap().is_empty());
        assert!(!ready.exists());
        assert!(unverified.exists());
    }
}
