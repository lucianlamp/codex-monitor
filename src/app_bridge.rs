use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::target::{Endpoint, EndpointCandidate};

pub const APP_BRIDGE_MARKER_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct AppBridgeMarker {
    pub version: u32,
    pub endpoint: String,
    pub bridge_pid: u32,
    pub server_pid: u32,
    pub real_codex: PathBuf,
}

pub fn marker_dir() -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os("CDXM_APP_BRIDGE_DIR") {
        return Ok(PathBuf::from(path));
    }
    let dirs = directories::ProjectDirs::from("", "", "codex-monitor")
        .ok_or_else(|| anyhow::anyhow!("could not resolve codex-monitor runtime directory"))?;
    Ok(dirs.data_local_dir().join("app-bridge"))
}

pub fn marker_path(dir: &Path, bridge_pid: u32) -> PathBuf {
    dir.join(format!("app-target.{bridge_pid}.json"))
}

pub fn write_marker_atomic(dir: &Path, marker: &AppBridgeMarker) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("failed to create app bridge marker dir {}", dir.display()))?;
    let path = marker_path(dir, marker.bridge_pid);
    let temporary = dir.join(format!("app-target.{}.json.tmp", marker.bridge_pid));
    let encoded = serde_json::to_vec_pretty(marker)?;
    std::fs::write(&temporary, encoded)
        .with_context(|| format!("failed to write app bridge marker {}", temporary.display()))?;
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("failed to replace app bridge marker {}", path.display()))?;
    }
    std::fs::rename(&temporary, &path)
        .with_context(|| format!("failed to publish app bridge marker {}", path.display()))?;
    Ok(path)
}

pub fn remove_marker(path: &Path) {
    let _ = std::fs::remove_file(path);
}

pub fn read_marker_candidates(
    dir: &Path,
    live_endpoints: &BTreeSet<String>,
) -> Vec<EndpointCandidate> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("app-target.") || !name.ends_with(".json") {
            continue;
        }
        let Ok(raw) = std::fs::read(&path) else {
            continue;
        };
        let Ok(marker) = serde_json::from_slice::<AppBridgeMarker>(&raw) else {
            continue;
        };
        if marker.version != APP_BRIDGE_MARKER_VERSION
            || !live_endpoints.contains(&marker.endpoint)
            || !is_safe_loopback_endpoint(&marker.endpoint)
        {
            continue;
        }
        candidates.push(EndpointCandidate {
            endpoint: Endpoint::Explicit(marker.endpoint),
            source: "codex-app-bridge".to_string(),
        });
    }
    candidates
}

fn is_safe_loopback_endpoint(endpoint: &str) -> bool {
    let Ok(url) = url::Url::parse(endpoint) else {
        return false;
    };
    url.scheme() == "ws"
        && url.port().is_some_and(|port| port != 0)
        && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    #[test]
    fn app_marker_round_trips() {
        let marker = AppBridgeMarker {
            version: APP_BRIDGE_MARKER_VERSION,
            endpoint: "ws://127.0.0.1:45454".into(),
            bridge_pid: 101,
            server_pid: 202,
            real_codex: PathBuf::from(r"C:\Codex\codex.exe"),
        };

        let encoded = serde_json::to_string(&marker).unwrap();
        let decoded: AppBridgeMarker = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, marker);
    }

    #[test]
    fn app_marker_candidates_require_live_loopback_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let marker = AppBridgeMarker {
            version: APP_BRIDGE_MARKER_VERSION,
            endpoint: "ws://127.0.0.1:45454".into(),
            bridge_pid: 101,
            server_pid: 202,
            real_codex: PathBuf::from(r"C:\Codex\codex.exe"),
        };
        write_marker_atomic(dir.path(), &marker).unwrap();

        assert!(read_marker_candidates(dir.path(), &BTreeSet::new()).is_empty());

        let live = BTreeSet::from([marker.endpoint.clone()]);
        let candidates = read_marker_candidates(dir.path(), &live);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].source, "codex-app-bridge");
        assert_eq!(
            crate::target::endpoint_label(&candidates[0].endpoint),
            marker.endpoint
        );
    }

    #[test]
    fn app_marker_candidates_ignore_malformed_unsafe_and_wrong_version_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app-target.bad.json"), "not-json").unwrap();
        for (pid, version, endpoint) in [
            (1, APP_BRIDGE_MARKER_VERSION + 1, "ws://127.0.0.1:45451"),
            (2, APP_BRIDGE_MARKER_VERSION, "ws://192.168.1.2:45452"),
            (3, APP_BRIDGE_MARKER_VERSION, "ws://127.0.0.1:0"),
        ] {
            let marker = AppBridgeMarker {
                version,
                endpoint: endpoint.into(),
                bridge_pid: pid,
                server_pid: pid + 100,
                real_codex: PathBuf::from(r"C:\Codex\codex.exe"),
            };
            std::fs::write(
                marker_path(dir.path(), pid),
                serde_json::to_vec(&marker).unwrap(),
            )
            .unwrap();
        }
        let live = BTreeSet::from([
            "ws://127.0.0.1:45451".to_string(),
            "ws://192.168.1.2:45452".to_string(),
            "ws://127.0.0.1:0".to_string(),
        ]);

        assert!(read_marker_candidates(dir.path(), &live).is_empty());
    }
}
