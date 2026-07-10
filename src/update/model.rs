use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

pub const MANIFEST_VERSION: u32 = 1;
pub const RESULT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManagedFile {
    CodexMonitor,
    Cdxm,
}

impl ManagedFile {
    pub const ALL: [Self; 2] = [Self::CodexMonitor, Self::Cdxm];

    pub const RELEASE: [Self; 2] = Self::ALL;

    pub fn destination(self, install_root: &Path) -> PathBuf {
        let (directory, name) = match self {
            Self::CodexMonitor => ("bin", "codex-monitor.exe"),
            Self::Cdxm => ("bin", "cdxm.exe"),
        };
        install_root.join(directory).join(name)
    }

    pub fn staged_name(self) -> &'static str {
        match self {
            Self::CodexMonitor => "codex-monitor.exe",
            Self::Cdxm => "cdxm.exe",
        }
    }

    pub fn is_required(self) -> bool {
        true
    }

    pub fn from_release_name(name: &str) -> Option<Self> {
        Self::RELEASE
            .into_iter()
            .find(|managed| managed.staged_name() == name)
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedFile {
    pub id: ManagedFile,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateManifest {
    pub version: u32,
    pub install_root: PathBuf,
    pub staging_root: PathBuf,
    pub files: Vec<StagedFile>,
}

impl UpdateManifest {
    pub fn validate_shape(&self) -> anyhow::Result<()> {
        if self.version != MANIFEST_VERSION {
            bail!("unsupported update manifest version {}", self.version);
        }
        if !self.install_root.is_absolute() || !self.staging_root.is_absolute() {
            bail!("update manifest paths must be absolute");
        }
        if !self.staging_root.starts_with(&self.install_root) {
            bail!("update staging directory must be inside the install root");
        }

        let mut seen = BTreeSet::new();
        for file in &self.files {
            if !seen.insert(file.id) {
                bail!("duplicate update manifest entry: {:?}", file.id);
            }
            match &file.sha256 {
                Some(hash) if is_sha256(hash) => {}
                Some(_) => bail!("invalid staged SHA-256 for {:?}", file.id),
                None if file.id.is_required() => {
                    bail!("required update file is absent: {:?}", file.id)
                }
                None => {}
            }
        }
        for managed in ManagedFile::ALL {
            if !seen.contains(&managed) {
                bail!("update manifest entry is missing: {managed:?}");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct InstallPaths {
    pub root: PathBuf,
    pub app_bridge_backup: PathBuf,
    pub update_result: PathBuf,
}

impl InstallPaths {
    pub fn new(root: PathBuf) -> Self {
        Self {
            app_bridge_backup: root.join("app-bridge-env.json"),
            update_result: root.join("last-update.json"),
            root,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateResult {
    pub version: u32,
    pub success: bool,
    pub message: String,
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    digest_to_hex(&Sha256::digest(bytes))
}

pub fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut file = File::open(path)
        .with_context(|| format!("failed to open update file {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("failed to hash update file {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(digest_to_hex(&hasher.finalize()))
}

pub fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn digest_to_hex(digest: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn managed_files_map_only_to_fixed_destinations() {
        let root = Path::new(r"C:\Users\me\.codex-monitor");
        assert_eq!(
            ManagedFile::CodexMonitor.destination(root),
            root.join("bin/codex-monitor.exe")
        );
        assert_eq!(
            ManagedFile::Cdxm.destination(root),
            root.join("bin/cdxm.exe")
        );
    }

    #[test]
    fn required_and_optional_file_sets_are_fixed() {
        assert!(ManagedFile::CodexMonitor.is_required());
        assert!(ManagedFile::Cdxm.is_required());
        assert_eq!(
            ManagedFile::ALL,
            [ManagedFile::CodexMonitor, ManagedFile::Cdxm]
        );
        assert_eq!(ManagedFile::RELEASE, ManagedFile::ALL);
    }

    #[test]
    fn manifest_rejects_duplicate_and_missing_required_entries() {
        let root = std::env::current_dir().unwrap();
        let mut files = ManagedFile::ALL
            .into_iter()
            .map(|id| StagedFile {
                id,
                sha256: id.is_required().then(|| "a".repeat(64)),
            })
            .collect::<Vec<_>>();
        let manifest = UpdateManifest {
            version: MANIFEST_VERSION,
            install_root: root.join("install"),
            staging_root: root.join("install").join("staging"),
            files: files.clone(),
        };
        assert!(manifest.validate_shape().is_ok());

        files.push(files[0].clone());
        assert!(UpdateManifest {
            files,
            ..manifest.clone()
        }
        .validate_shape()
        .is_err());

        let files = manifest
            .files
            .iter()
            .filter(|file| file.id != ManagedFile::Cdxm)
            .cloned()
            .collect();
        assert!(UpdateManifest { files, ..manifest }
            .validate_shape()
            .is_err());
    }
}
