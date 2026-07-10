use anyhow::Result;
use std::path::Path;

mod archive;
mod model;

#[cfg(windows)]
pub async fn run_update() -> Result<i32> {
    windows_impl::run_update().await
}

#[cfg(not(windows))]
pub async fn run_update() -> Result<i32> {
    anyhow::bail!("codex-monitor update with Codex App runtime refresh is currently Windows-only")
}

#[cfg(windows)]
pub fn run_apply(manifest: &Path, parent_pid: u32) -> Result<i32> {
    windows_impl::run_apply(manifest, parent_pid)
}

#[cfg(not(windows))]
pub fn run_apply(_manifest: &Path, _parent_pid: u32) -> Result<i32> {
    anyhow::bail!("the internal update worker is currently Windows-only")
}

pub fn report_previous_failure() -> Result<()> {
    Ok(())
}

#[cfg(windows)]
mod windows_impl {
    use anyhow::Result;
    use std::path::Path;

    pub async fn run_update() -> Result<i32> {
        anyhow::bail!("update staging is not implemented")
    }

    pub fn run_apply(_manifest: &Path, _parent_pid: u32) -> Result<i32> {
        anyhow::bail!("update apply is not implemented")
    }
}
