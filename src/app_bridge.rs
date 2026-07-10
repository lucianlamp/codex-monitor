use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::target::{Endpoint, EndpointCandidate};

mod monitor_router;
mod stdio_monitor;

pub const APP_BRIDGE_MARKER_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum InvocationKind {
    AppServer { command_index: usize },
    Passthrough,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum BridgeMode {
    Passthrough,
    StdioMonitor,
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct RealCodexSources {
    pub explicit: Option<PathBuf>,
    pub resources_dir: Option<PathBuf>,
    pub user_install: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct AppBridgeMarker {
    pub version: u32,
    pub endpoint: String,
    pub bridge_pid: u32,
    pub server_pid: u32,
    pub real_codex: PathBuf,
}

pub fn invocation_kind(args: &[OsString]) -> InvocationKind {
    args.iter()
        .position(|arg| arg == OsStr::new("app-server"))
        .map(|command_index| InvocationKind::AppServer { command_index })
        .unwrap_or(InvocationKind::Passthrough)
}

pub fn publishes_app_target_marker(args: &[OsString]) -> bool {
    if !matches!(invocation_kind(args), InvocationKind::AppServer { .. }) {
        return false;
    }
    let has_code_mode_config = args.windows(2).any(|pair| {
        matches!(pair[0].to_str(), Some("-c" | "--config"))
            && pair[1] == OsStr::new("features.code_mode_host=true")
    }) || args
        .iter()
        .any(|arg| arg == OsStr::new("--config=features.code_mode_host=true"));
    let has_app_analytics_flag = args
        .iter()
        .any(|arg| arg == OsStr::new("--analytics-default-enabled"));
    has_code_mode_config && has_app_analytics_flag
}

fn bridge_mode(args: &[OsString]) -> BridgeMode {
    if publishes_app_target_marker(args) {
        BridgeMode::StdioMonitor
    } else {
        BridgeMode::Passthrough
    }
}

pub fn real_codex_sources_from_env() -> RealCodexSources {
    let explicit = std::env::var_os("CDXM_REAL_CODEX").map(PathBuf::from);
    let resources_dir = std::env::var_os("CODEX_ELECTRON_RESOURCES_PATH").map(PathBuf::from);
    let user_install = std::env::var_os("LOCALAPPDATA").map(|root| {
        PathBuf::from(root)
            .join("OpenAI")
            .join("Codex")
            .join("bin")
            .join("codex.exe")
    });
    RealCodexSources {
        explicit,
        resources_dir,
        user_install,
    }
}

pub fn resolve_real_codex(
    sources: &RealCodexSources,
    current_executable: &Path,
) -> anyhow::Result<PathBuf> {
    let candidates = [
        sources.explicit.clone(),
        sources
            .resources_dir
            .as_ref()
            .map(|resources| resources.join("codex.exe")),
        sources.user_install.clone(),
    ];
    let current = std::fs::canonicalize(current_executable).with_context(|| {
        format!(
            "failed to resolve app bridge executable {}",
            current_executable.display()
        )
    })?;
    for candidate in candidates.into_iter().flatten() {
        if !candidate.is_file() {
            continue;
        }
        let resolved = std::fs::canonicalize(&candidate)
            .with_context(|| format!("failed to resolve real Codex {}", candidate.display()))?;
        if resolved == current {
            anyhow::bail!(
                "real Codex executable resolves to the bridge itself: {}",
                resolved.display()
            );
        }
        return Ok(resolved);
    }
    anyhow::bail!("real Codex executable was not found; set CDXM_REAL_CODEX to codex.exe")
}

pub async fn run_bridge(args: Vec<OsString>) -> anyhow::Result<i32> {
    let current_executable = std::env::current_exe()?;
    let real_codex = resolve_real_codex(&real_codex_sources_from_env(), &current_executable)?;
    match bridge_mode(&args) {
        BridgeMode::Passthrough => run_passthrough(&real_codex, &args).await,
        BridgeMode::StdioMonitor => stdio_monitor::run(&real_codex, &args).await,
    }
}

async fn run_passthrough(real_codex: &Path, args: &[OsString]) -> anyhow::Result<i32> {
    let status = Command::new(real_codex)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .with_context(|| format!("failed to run real Codex {}", real_codex.display()))?;
    Ok(status.code().unwrap_or(1))
}

pub(super) struct MarkerGuard(pub(super) PathBuf);

impl Drop for MarkerGuard {
    fn drop(&mut self) {
        remove_marker(&self.0);
    }
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
    live_bridge_pids: &BTreeSet<u32>,
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
            || !live_bridge_pids.contains(&marker.bridge_pid)
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
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[test]
    fn invocation_classifies_app_server_after_global_config_args() {
        let args = vec![
            OsString::from("-c"),
            OsString::from("features.code_mode_host=true"),
            OsString::from("app-server"),
            OsString::from("--analytics-default-enabled"),
        ];

        assert_eq!(
            invocation_kind(&args),
            InvocationKind::AppServer { command_index: 2 }
        );
        assert_eq!(
            invocation_kind(&[OsString::from("--version")]),
            InvocationKind::Passthrough
        );
    }

    #[test]
    fn only_codex_app_signature_publishes_app_target_marker() {
        let app_args = vec![
            OsString::from("-c"),
            OsString::from("features.code_mode_host=true"),
            OsString::from("app-server"),
            OsString::from("--analytics-default-enabled"),
        ];
        let generic_args = vec![
            OsString::from("app-server"),
            OsString::from("--listen"),
            OsString::from("stdio://"),
        ];

        assert!(publishes_app_target_marker(&app_args));
        assert!(!publishes_app_target_marker(&generic_args));
    }

    #[test]
    fn app_signature_selects_stdio_monitor_without_rewriting_args() {
        let app_args = vec![
            OsString::from("-c"),
            OsString::from("features.code_mode_host=true"),
            OsString::from("app-server"),
            OsString::from("--analytics-default-enabled"),
        ];
        let generic = vec![
            OsString::from("app-server"),
            OsString::from("--listen"),
            OsString::from("ws://127.0.0.1:45454"),
        ];

        assert_eq!(bridge_mode(&app_args), BridgeMode::StdioMonitor);
        assert_eq!(bridge_mode(&generic), BridgeMode::Passthrough);
        assert!(!app_args
            .iter()
            .any(|argument| argument.to_string_lossy().starts_with("--listen")));
    }

    #[test]
    fn real_codex_prefers_explicit_then_resources_then_user_install() {
        let dir = tempfile::tempdir().unwrap();
        let explicit = dir.path().join("explicit.exe");
        let resources = dir.path().join("resources");
        let bundled = resources.join("codex.exe");
        let user = dir.path().join("user-codex.exe");
        std::fs::create_dir_all(&resources).unwrap();
        std::fs::write(&explicit, b"explicit").unwrap();
        std::fs::write(&bundled, b"bundled").unwrap();
        std::fs::write(&user, b"user").unwrap();
        let current = dir.path().join("bridge.exe");
        std::fs::write(&current, b"bridge").unwrap();

        let sources = RealCodexSources {
            explicit: Some(explicit.clone()),
            resources_dir: Some(resources.clone()),
            user_install: Some(user.clone()),
        };
        assert_eq!(
            resolve_real_codex(&sources, &current).unwrap(),
            std::fs::canonicalize(explicit).unwrap()
        );

        let sources = RealCodexSources {
            explicit: None,
            resources_dir: Some(resources),
            user_install: Some(user.clone()),
        };
        assert_eq!(
            resolve_real_codex(&sources, &current).unwrap(),
            std::fs::canonicalize(bundled).unwrap()
        );

        let sources = RealCodexSources {
            explicit: None,
            resources_dir: None,
            user_install: Some(user.clone()),
        };
        assert_eq!(
            resolve_real_codex(&sources, &current).unwrap(),
            std::fs::canonicalize(user).unwrap()
        );
    }

    #[test]
    fn real_codex_rejects_bridge_recursion() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = dir.path().join("bridge.exe");
        std::fs::write(&bridge, b"bridge").unwrap();
        let sources = RealCodexSources {
            explicit: Some(bridge.clone()),
            resources_dir: None,
            user_install: None,
        };

        assert!(resolve_real_codex(&sources, &bridge)
            .unwrap_err()
            .to_string()
            .contains("resolves to the bridge itself"));
    }

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
    fn marker_guard_removes_only_its_owned_marker() {
        let dir = tempfile::tempdir().unwrap();
        let owned = dir.path().join("app-target.1.json");
        let unrelated = dir.path().join("app-target.2.json");
        std::fs::write(&owned, b"owned").unwrap();
        std::fs::write(&unrelated, b"unrelated").unwrap();

        {
            let _guard = MarkerGuard(owned.clone());
            assert!(owned.is_file());
            assert!(unrelated.is_file());
        }

        assert!(!owned.exists());
        assert!(unrelated.is_file());
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

        let live_bridge_pids = BTreeSet::from([marker.bridge_pid]);
        assert!(read_marker_candidates(dir.path(), &BTreeSet::new(), &live_bridge_pids).is_empty());

        let live = BTreeSet::from([marker.endpoint.clone()]);
        assert!(read_marker_candidates(dir.path(), &live, &BTreeSet::new()).is_empty());

        let candidates = read_marker_candidates(dir.path(), &live, &live_bridge_pids);
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

        let live_bridge_pids = BTreeSet::from([1, 2, 3]);
        assert!(read_marker_candidates(dir.path(), &live, &live_bridge_pids).is_empty());
    }
}
