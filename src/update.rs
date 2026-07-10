use anyhow::{Context, Result};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

mod apply;
mod archive;
mod model;
#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub async fn run_update() -> Result<i32> {
    run_update_windows().await
}

#[cfg(not(windows))]
pub async fn run_update() -> Result<i32> {
    anyhow::bail!("codex-monitor update with Codex App runtime refresh is currently Windows-only")
}

#[cfg(windows)]
pub fn run_apply(manifest: &Path, parent_pid: u32) -> Result<i32> {
    run_apply_windows(manifest, parent_pid)
}

#[cfg(not(windows))]
pub fn run_apply(_manifest: &Path, _parent_pid: u32) -> Result<i32> {
    anyhow::bail!("the internal update worker is currently Windows-only")
}

pub fn report_previous_failure() -> Result<()> {
    #[cfg(windows)]
    {
        let paths = windows::install_paths()?;
        let _ = apply::cleanup_ready_backups(&paths.root)?;
        if let Some(message) = apply::take_previous_failure(&paths.update_result)? {
            eprintln!("Previous codex-monitor update failed: {message}");
        }
    }
    Ok(())
}

#[cfg(windows)]
async fn run_update_windows() -> Result<i32> {
    use model::{UpdateManifest, MANIFEST_VERSION};
    use std::process::{Command, Stdio};

    let preflight = windows::preflight()?;
    std::fs::create_dir_all(&preflight.paths.root).with_context(|| {
        format!(
            "failed to create codex-monitor install root {}",
            preflight.paths.root.display()
        )
    })?;
    let staging_root = preflight
        .paths
        .root
        .join(format!(".update-staging-{}", unique_suffix()?));
    std::fs::create_dir(&staging_root).with_context(|| {
        format!(
            "failed to create update staging directory {}",
            staging_root.display()
        )
    })?;
    let mut guard = StagingGuard::new(staging_root.clone());

    let release_base = std::env::var("CDXM_INSTALL_RELEASE_BASE").unwrap_or_else(|_| {
        "https://github.com/lucianlamp/codex-monitor/releases/latest/download".into()
    });
    println!("Downloading and verifying the latest codex-monitor release...");
    let files = archive::download_latest_release(&release_base, &staging_root).await?;

    let manifest = UpdateManifest {
        version: MANIFEST_VERSION,
        install_root: preflight.paths.root.clone(),
        staging_root: staging_root.clone(),
        files,
    };
    manifest.validate_shape()?;
    let manifest_path = staging_root.join("manifest.json");
    write_manifest_atomic(&manifest_path, &manifest)?;

    let helper_path = staging_root.join("update-helper.exe");
    let current_exe =
        std::env::current_exe().context("failed to resolve the updater executable")?;
    std::fs::copy(&current_exe, &helper_path).with_context(|| {
        format!(
            "failed to stage update helper from {} to {}",
            current_exe.display(),
            helper_path.display()
        )
    })?;
    Command::new(&helper_path)
        .args(helper_args(&manifest_path, std::process::id()))
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("failed to launch update helper {}", helper_path.display()))?;

    guard.keep();
    println!(
        "Verified update staged. Applying it now; reopen Codex App after the completion message."
    );
    Ok(0)
}

#[cfg(windows)]
fn run_apply_windows(manifest_path: &Path, parent_pid: u32) -> Result<i32> {
    use model::{UpdateManifest, UpdateResult, RESULT_VERSION};

    let expected_paths = windows::install_paths()?;
    let apply_result = (|| -> Result<apply::ApplySummary> {
        if !manifest_path.is_absolute() {
            anyhow::bail!("update manifest path must be absolute");
        }
        let bytes = std::fs::read(manifest_path).with_context(|| {
            format!(
                "failed to read staged update manifest {}",
                manifest_path.display()
            )
        })?;
        let manifest: UpdateManifest =
            serde_json::from_slice(&bytes).context("staged update manifest is not valid JSON")?;
        manifest.validate_shape()?;
        if !windows::paths_equal(&manifest.install_root, &expected_paths.root) {
            anyhow::bail!("update manifest install root does not match this installation");
        }
        let manifest_parent = manifest_path
            .parent()
            .context("update manifest has no staging parent")?;
        if !windows::paths_equal(manifest_parent, &manifest.staging_root) {
            anyhow::bail!("update manifest is outside its declared staging directory");
        }
        let helper = std::env::current_exe().context("failed to resolve update helper path")?;
        let helper_parent = helper
            .parent()
            .context("update helper has no parent directory")?;
        if !windows::paths_equal(helper_parent, &manifest.staging_root) {
            anyhow::bail!("internal update worker must run from its staging directory");
        }

        windows::wait_for_process_exit(parent_pid)?;
        windows::ensure_app_not_running()?;
        let summary = apply::apply_manifest(&manifest)?;
        windows::reassert_owned_environment(&expected_paths)?;
        apply::write_result_atomic(
            &expected_paths.update_result,
            &UpdateResult {
                version: RESULT_VERSION,
                success: true,
                message: format!(
                    "updated {} files, removed {} obsolete files, left {} identical files{}",
                    summary.changed,
                    summary.removed,
                    summary.unchanged,
                    if summary.deferred_cleanup.is_some() {
                        "; old running executable cleanup is deferred"
                    } else {
                        ""
                    }
                ),
            },
        )?;
        if let Err(error) =
            windows::schedule_staging_cleanup(&manifest.staging_root, std::process::id())
        {
            eprintln!(
                "codex-monitor update warning: could not schedule staging cleanup for {}: {error:#}",
                manifest.staging_root.display()
            );
        }
        Ok(summary)
    })();

    match apply_result {
        Ok(summary) => {
            println!(
                "codex-monitor update complete: {} updated, {} removed, {} unchanged. Reopen Codex App.",
                summary.changed, summary.removed, summary.unchanged
            );
            if let Some(path) = summary.deferred_cleanup {
                println!(
                    "Old running executable backup will be removed after its watcher exits: {}",
                    path.display()
                );
            }
            Ok(0)
        }
        Err(error) => {
            let message = format!("{error:#}");
            let result = model::UpdateResult {
                version: model::RESULT_VERSION,
                success: false,
                message: message.clone(),
            };
            if let Err(write_error) =
                apply::write_result_atomic(&expected_paths.update_result, &result)
            {
                eprintln!(
                    "codex-monitor update failed: {message}; also failed to persist the result: {write_error:#}"
                );
            }
            Err(error)
        }
    }
}

fn helper_args(manifest: &Path, parent_pid: u32) -> Vec<OsString> {
    vec![
        OsString::from("__apply-update"),
        OsString::from("--manifest"),
        manifest.as_os_str().to_owned(),
        OsString::from("--parent-pid"),
        OsString::from(parent_pid.to_string()),
    ]
}

fn write_manifest_atomic(path: &Path, manifest: &model::UpdateManifest) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("update manifest path has no parent: {}", path.display()))?;
    let temporary = parent.join("manifest.json.tmp");
    let encoded =
        serde_json::to_vec_pretty(manifest).context("failed to encode update manifest")?;
    std::fs::write(&temporary, encoded).with_context(|| {
        format!(
            "failed to write temporary update manifest {}",
            temporary.display()
        )
    })?;
    std::fs::rename(&temporary, path)
        .with_context(|| format!("failed to publish update manifest {}", path.display()))?;
    Ok(())
}

fn unique_suffix() -> anyhow::Result<String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_nanos();
    Ok(format!("{}-{nanos}", std::process::id()))
}

struct StagingGuard {
    path: PathBuf,
    keep: bool,
}

impl StagingGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, keep: false }
    }

    fn keep(&mut self) {
        self.keep = true;
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsString, path::Path};

    #[test]
    fn helper_arguments_use_absolute_manifest_and_parent_pid() {
        let args = helper_args(Path::new(r"C:\tmp\manifest.json"), 42);
        assert_eq!(
            args,
            [
                OsString::from("__apply-update"),
                OsString::from("--manifest"),
                OsString::from(r"C:\tmp\manifest.json"),
                OsString::from("--parent-pid"),
                OsString::from("42"),
            ]
        );
    }
}
