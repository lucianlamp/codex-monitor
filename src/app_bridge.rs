use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::Context;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio_tungstenite::tungstenite::{protocol::WebSocketConfig, Message};

use crate::target::{Endpoint, EndpointCandidate};

mod monitor_router;
mod stdio_monitor;

pub const APP_BRIDGE_MARKER_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum InvocationKind {
    AppServer { command_index: usize },
    Passthrough,
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

pub fn rewrite_app_server_args(args: &[OsString], endpoint: &str) -> Vec<OsString> {
    let mut rewritten = Vec::with_capacity(args.len() + 2);
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == OsStr::new("--listen") {
            index += 2;
            continue;
        }
        if arg.to_string_lossy().starts_with("--listen=") {
            index += 1;
            continue;
        }
        rewritten.push(arg.clone());
        index += 1;
    }
    rewritten.push(OsString::from("--listen"));
    rewritten.push(OsString::from(endpoint));
    rewritten
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
    match invocation_kind(&args) {
        InvocationKind::Passthrough => run_passthrough(&real_codex, &args).await,
        InvocationKind::AppServer { .. } => {
            run_app_server_bridge(&real_codex, &args, publishes_app_target_marker(&args)).await
        }
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

async fn run_app_server_bridge(
    real_codex: &Path,
    args: &[OsString],
    publish_marker: bool,
) -> anyhow::Result<i32> {
    let port = pick_loopback_port().await?;
    let endpoint = format!("ws://127.0.0.1:{port}");
    let rewritten = rewrite_app_server_args(args, &endpoint);
    let mut child = Command::new(real_codex)
        .args(rewritten)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("failed to start real Codex {}", real_codex.display()))?;
    let server_pid = child
        .id()
        .ok_or_else(|| anyhow::anyhow!("real Codex app-server has no process id"))?;

    if let Err(error) = wait_for_server(&mut child, port).await {
        let _ = child.kill().await;
        return Err(error);
    }

    let _marker_guard = if publish_marker {
        let marker = AppBridgeMarker {
            version: APP_BRIDGE_MARKER_VERSION,
            endpoint: endpoint.clone(),
            bridge_pid: std::process::id(),
            server_pid,
            real_codex: real_codex.to_path_buf(),
        };
        let marker_path = write_marker_atomic(&marker_dir()?, &marker)?;
        Some(MarkerGuard(marker_path))
    } else {
        None
    };

    let input = tokio::io::BufReader::new(tokio::io::stdin());
    let output = tokio::io::stdout();
    let proxy_result = proxy_jsonl_websocket_io(&endpoint, input, output).await;
    let _ = child.kill().await;
    proxy_result?;
    Ok(0)
}

async fn pick_loopback_port() -> anyhow::Result<u16> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

async fn wait_for_server(child: &mut Child, port: u16) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait()? {
            anyhow::bail!("real Codex app-server exited before readiness: {status}");
        }
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("real Codex app-server did not become ready on 127.0.0.1:{port}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

struct MarkerGuard(PathBuf);

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

pub async fn proxy_jsonl_websocket_io<R, W>(
    endpoint: &str,
    mut input: R,
    mut output: W,
) -> anyhow::Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    if !is_safe_loopback_endpoint(endpoint) {
        anyhow::bail!("refusing unsafe Codex App bridge endpoint: {endpoint}");
    }
    // The App can request complete thread snapshots that exceed tungstenite's
    // default 16 MiB frame and 64 MiB message limits. This is a transparent
    // loopback proxy between two trusted Codex processes, so payload limits
    // belong to the App and app-server rather than the bridge.
    let websocket_config = WebSocketConfig::default()
        .max_frame_size(None)
        .max_message_size(None);
    let (mut websocket, _) =
        tokio_tungstenite::connect_async_with_config(endpoint, Some(websocket_config), false)
            .await
            .with_context(|| format!("failed to connect Codex App bridge to {endpoint}"))?;
    let mut line = String::new();

    loop {
        line.clear();
        tokio::select! {
            read = input.read_line(&mut line) => {
                let bytes = read?;
                if bytes == 0 {
                    let _ = websocket.close(None).await;
                    break;
                }
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                websocket.send(Message::Text(trimmed.to_owned().into())).await?;
            }
            incoming = websocket.next() => {
                match incoming.transpose()? {
                    Some(Message::Text(text)) => {
                        output.write_all(text.as_bytes()).await?;
                        output.write_all(b"\n").await?;
                        output.flush().await?;
                    }
                    Some(Message::Binary(bytes)) => {
                        output.write_all(&bytes).await?;
                        output.write_all(b"\n").await?;
                        output.flush().await?;
                    }
                    Some(Message::Ping(payload)) => {
                        websocket.send(Message::Pong(payload)).await?;
                    }
                    Some(Message::Pong(_)) | Some(Message::Frame(_)) => {}
                    Some(Message::Close(_)) | None => break,
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use std::collections::BTreeSet;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio_tungstenite::tungstenite::Message;

    #[tokio::test]
    async fn proxy_forwards_jsonl_and_websocket_messages_bidirectionally() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let message = websocket.next().await.unwrap().unwrap();
            assert_eq!(
                message.into_text().unwrap(),
                r#"{"id":1,"method":"initialize"}"#
            );
            websocket
                .send(Message::Text(r#"{"id":1,"result":{}}"#.into()))
                .await
                .unwrap();
            websocket
                .send(Message::Text(
                    r#"{"id":"server-1","method":"item/tool/requestUserInput","params":{}}"#.into(),
                ))
                .await
                .unwrap();
            websocket.next().await;
        });

        let (test_io, bridge_io) = tokio::io::duplex(8192);
        let (test_read, mut test_write) = tokio::io::split(test_io);
        let (bridge_read, bridge_write) = tokio::io::split(bridge_io);
        let proxy_endpoint = endpoint.clone();
        let proxy = tokio::spawn(async move {
            proxy_jsonl_websocket_io(&proxy_endpoint, BufReader::new(bridge_read), bridge_write)
                .await
        });

        test_write
            .write_all(b"{\"id\":1,\"method\":\"initialize\"}\n")
            .await
            .unwrap();
        test_write.flush().await.unwrap();
        let mut reader = BufReader::new(test_read);
        let mut first = String::new();
        let mut second = String::new();
        reader.read_line(&mut first).await.unwrap();
        reader.read_line(&mut second).await.unwrap();
        assert_eq!(first.trim(), r#"{"id":1,"result":{}}"#);
        assert!(second.contains("item/tool/requestUserInput"));

        test_write.shutdown().await.unwrap();
        proxy.await.unwrap().unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn proxy_forwards_frames_larger_than_tungstenite_default_limit() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let expected_payload_len = (16 * 1024 * 1024) + 1;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let message = format!(
                r#"{{"id":1,"result":{{"payload":"{}"}}}}"#,
                "x".repeat(expected_payload_len)
            );
            websocket.send(Message::Text(message.into())).await.unwrap();
            websocket.next().await;
        });

        let (test_io, bridge_io) = tokio::io::duplex(64 * 1024);
        let (test_read, mut test_write) = tokio::io::split(test_io);
        let (bridge_read, bridge_write) = tokio::io::split(bridge_io);
        let proxy_endpoint = endpoint.clone();
        let proxy = tokio::spawn(async move {
            proxy_jsonl_websocket_io(&proxy_endpoint, BufReader::new(bridge_read), bridge_write)
                .await
        });

        let mut reader = BufReader::new(test_read);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        assert!(line.len() > expected_payload_len);
        assert!(line.ends_with("\n"));

        test_write.shutdown().await.unwrap();
        proxy.await.unwrap().unwrap();
        server.await.unwrap();
    }

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
    fn invocation_rewrites_bridge_owned_listen_argument() {
        let args = vec![
            OsString::from("app-server"),
            OsString::from("--listen"),
            OsString::from("stdio://"),
            OsString::from("--analytics-default-enabled"),
        ];

        assert_eq!(
            rewrite_app_server_args(&args, "ws://127.0.0.1:45454"),
            vec![
                OsString::from("app-server"),
                OsString::from("--analytics-default-enabled"),
                OsString::from("--listen"),
                OsString::from("ws://127.0.0.1:45454"),
            ]
        );
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
