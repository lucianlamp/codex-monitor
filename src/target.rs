use anyhow::{anyhow, bail};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Endpoint {
    Auto,
    Managed,
    App,
    Explicit(String),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct EndpointCandidate {
    pub endpoint: Endpoint,
    pub source: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ThreadSummary {
    pub id: String,
    pub title: Option<String>,
    pub cwd: Option<String>,
}

pub fn endpoint_from_options(endpoint: Option<String>, target: crate::cli::TargetKind) -> Endpoint {
    match endpoint {
        Some(url) => Endpoint::Explicit(url),
        None if target == crate::cli::TargetKind::App => Endpoint::App,
        None if target == crate::cli::TargetKind::Managed => Endpoint::Managed,
        None => Endpoint::Auto,
    }
}

pub fn resolve_default_auto_endpoint(endpoint: Endpoint) -> anyhow::Result<Endpoint> {
    let endpoint = resolve_app_endpoint(endpoint)?;
    if endpoint != Endpoint::Auto {
        return Ok(endpoint);
    }

    let candidates = discover_auto_endpoint_candidates();
    match candidates.as_slice() {
        [] => Ok(Endpoint::Managed),
        [candidate] => Ok(candidate.endpoint.clone()),
        candidates => {
            let choices = candidates
                .iter()
                .map(|candidate| {
                    format!(
                        "{} ({})",
                        endpoint_label(&candidate.endpoint),
                        candidate.source
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "multiple auto endpoints found; pass --endpoint or --target explicitly: {choices}"
            )
        }
    }
}

pub fn resolve_app_endpoint(endpoint: Endpoint) -> anyhow::Result<Endpoint> {
    if endpoint != Endpoint::App {
        return Ok(endpoint);
    }

    #[cfg(unix)]
    {
        Ok(Endpoint::App)
    }
    #[cfg(windows)]
    {
        select_windows_app_endpoint(discover_auto_endpoint_candidates())
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        bail!("--target app is not supported on this platform")
    }
}

fn select_windows_app_endpoint(candidates: Vec<EndpointCandidate>) -> anyhow::Result<Endpoint> {
    let app_endpoints = candidates
        .into_iter()
        .filter(|candidate| candidate.source == "codex-app-bridge")
        .collect::<Vec<_>>();

    match app_endpoints.as_slice() {
        [] => bail!(
            "no live Codex App bridge endpoint found on Windows; enable the app bridge, restart Codex App, and retry"
        ),
        [candidate] => Ok(candidate.endpoint.clone()),
        many => {
            let choices = many
                .iter()
                .map(|candidate| endpoint_label(&candidate.endpoint))
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "multiple live Codex App bridge endpoints found on Windows; pass --endpoint explicitly: {choices}"
            )
        }
    }
}

pub fn discover_auto_endpoint_candidates() -> Vec<EndpointCandidate> {
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();

    for key in [
        "CDXM_ENDPOINT",
        "CODEX_MONITOR_ENDPOINT",
        "CODEX_APP_SERVER_ENDPOINT",
    ] {
        if let Ok(value) = std::env::var(key) {
            if let Some(endpoint) = explicit_endpoint_from_url(value.trim()) {
                push_candidate(&mut candidates, &mut seen, endpoint, key.to_string());
            }
        }
    }

    #[cfg(unix)]
    if default_app_socket_path().exists() {
        push_candidate(
            &mut candidates,
            &mut seen,
            Endpoint::App,
            "codex-app-control-socket".to_string(),
        );
    }

    for candidate in discover_process_endpoint_candidates() {
        push_candidate(
            &mut candidates,
            &mut seen,
            candidate.endpoint,
            candidate.source,
        );
    }

    candidates
}

pub fn endpoint_label(endpoint: &Endpoint) -> String {
    match endpoint {
        Endpoint::Auto => "auto".to_string(),
        Endpoint::Managed => "managed".to_string(),
        Endpoint::App => format!("app:{}", default_app_socket_path().display()),
        Endpoint::Explicit(url) => url.clone(),
    }
}

fn push_candidate(
    candidates: &mut Vec<EndpointCandidate>,
    seen: &mut BTreeSet<String>,
    endpoint: Endpoint,
    source: String,
) {
    let label = endpoint_label(&endpoint);
    if seen.insert(label.clone()) {
        candidates.push(EndpointCandidate { endpoint, source });
    } else if source_priority(&source) > 0 {
        if let Some(candidate) = candidates
            .iter_mut()
            .find(|candidate| endpoint_label(&candidate.endpoint) == label)
        {
            if source_priority(&source) > source_priority(&candidate.source) {
                candidate.source = source;
            }
        }
    }
}

fn source_priority(source: &str) -> u8 {
    match source {
        "codex-app-bridge" => 2,
        "codex-app-server-process" => 1,
        _ => 0,
    }
}

fn explicit_endpoint_from_url(value: &str) -> Option<Endpoint> {
    if value.starts_with("ws://") {
        let parsed = url::Url::parse(value).ok()?;
        if parsed.port() == Some(0) {
            return None;
        }
        Some(Endpoint::Explicit(value.to_string()))
    } else if value.starts_with("unix://") || value == "stdio://" {
        Some(Endpoint::Explicit(value.to_string()))
    } else {
        None
    }
}

#[cfg(unix)]
fn discover_process_endpoint_candidates() -> Vec<EndpointCandidate> {
    let output = std::process::Command::new("ps")
        .args(["-axo", "command="])
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            discover_endpoint_candidates_from_process_text(&text)
        }
        _ => Vec::new(),
    }
}

#[cfg(windows)]
fn windows_powershell_path(system_root: &std::path::Path) -> std::path::PathBuf {
    system_root
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe")
}

#[cfg(windows)]
fn windows_powershell_executable() -> std::path::PathBuf {
    for variable in ["SystemRoot", "WINDIR"] {
        if let Some(root) = std::env::var_os(variable) {
            let candidate = windows_powershell_path(std::path::Path::new(&root));
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    std::path::PathBuf::from("powershell.exe")
}

#[cfg(windows)]
fn discover_process_endpoint_candidates() -> Vec<EndpointCandidate> {
    let command = r#"
$ErrorActionPreference='SilentlyContinue'
'__CDXM_PROCESSES__'
Get-CimInstance Win32_Process | Where-Object { $_.CommandLine } | ForEach-Object {
    "{0}`t{1}" -f $_.ProcessId, ($_.CommandLine -replace "[`r`n]+", " ")
}
'__CDXM_TCP__'
if (Get-Command Get-NetTCPConnection -ErrorAction SilentlyContinue) {
    Get-NetTCPConnection -State Listen | Where-Object {
        $_.LocalAddress -eq '127.0.0.1' -or $_.LocalAddress -eq '::1'
    } | ForEach-Object {
        "{0}`t{1}`t{2}" -f $_.OwningProcess, $_.LocalAddress, $_.LocalPort
    }
}
"#;
    let output = std::process::Command::new(windows_powershell_executable())
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            command,
        ])
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let inventory = String::from_utf8_lossy(&output.stdout);
            let live_bridge_pids = live_windows_app_bridge_pids_from_inventory_text(&inventory);
            let mut candidates =
                discover_windows_endpoint_candidates_from_inventory_text(&inventory);
            let live_endpoints = candidates
                .iter()
                .map(|candidate| endpoint_label(&candidate.endpoint))
                .collect::<BTreeSet<_>>();
            if let Ok(dir) = crate::app_bridge::marker_dir() {
                let mut seen = live_endpoints.clone();
                for marker in crate::app_bridge::read_marker_candidates(
                    &dir,
                    &live_endpoints,
                    &live_bridge_pids,
                ) {
                    push_candidate(&mut candidates, &mut seen, marker.endpoint, marker.source);
                }
            }
            candidates
        }
        _ => Vec::new(),
    }
}

#[cfg(all(not(unix), not(windows)))]
fn discover_process_endpoint_candidates() -> Vec<EndpointCandidate> {
    Vec::new()
}

pub fn discover_endpoint_candidates_from_process_text(text: &str) -> Vec<EndpointCandidate> {
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    for line in text.lines() {
        if !(line.contains("codex") || line.contains("Codex")) {
            continue;
        }
        for (source, endpoint) in endpoints_from_process_line(line) {
            push_candidate(&mut candidates, &mut seen, endpoint, source);
        }
    }
    candidates
}

#[cfg(any(windows, test))]
fn discover_windows_endpoint_candidates_from_process_and_tcp_text(
    process_text: &str,
    tcp_text: &str,
) -> Vec<EndpointCandidate> {
    let mut candidates = discover_endpoint_candidates_from_process_text(process_text);
    let mut seen = candidates
        .iter()
        .map(|candidate| endpoint_label(&candidate.endpoint))
        .collect::<BTreeSet<_>>();
    let tcp_rows = parse_windows_tcp_listen_rows(tcp_text);

    for line in process_text.lines() {
        let Some((pid, command_line)) = parse_windows_process_row(line) else {
            continue;
        };
        if !is_codex_app_server_process(&command_line) {
            continue;
        }
        let source = endpoint_source_from_process_line(&command_line).to_string();
        for row in tcp_rows.iter().filter(|row| row.pid == pid) {
            let Some(url) = loopback_ws_url_from_tcp_listen(&row.local_address, row.port) else {
                continue;
            };
            push_candidate(
                &mut candidates,
                &mut seen,
                Endpoint::Explicit(url),
                source.clone(),
            );
        }
    }

    candidates
}

#[cfg(windows)]
fn discover_windows_endpoint_candidates_from_inventory_text(text: &str) -> Vec<EndpointCandidate> {
    let mut process_text = String::new();
    let mut tcp_text = String::new();
    let mut section = "";
    for line in text.lines() {
        match line.trim() {
            "__CDXM_PROCESSES__" => {
                section = "process";
                continue;
            }
            "__CDXM_TCP__" => {
                section = "tcp";
                continue;
            }
            _ => {}
        }
        match section {
            "process" => {
                process_text.push_str(line);
                process_text.push('\n');
            }
            "tcp" => {
                tcp_text.push_str(line);
                tcp_text.push('\n');
            }
            _ => {}
        }
    }
    discover_windows_endpoint_candidates_from_process_and_tcp_text(&process_text, &tcp_text)
}

#[cfg(any(windows, test))]
fn live_windows_app_bridge_pids_from_inventory_text(text: &str) -> BTreeSet<u32> {
    let mut pids = BTreeSet::new();
    let mut in_process_section = false;
    for line in text.lines() {
        match line.trim() {
            "__CDXM_PROCESSES__" => {
                in_process_section = true;
                continue;
            }
            "__CDXM_TCP__" => break,
            _ => {}
        }
        if !in_process_section {
            continue;
        }
        let Some((pid, command_line)) = parse_windows_process_row(line) else {
            continue;
        };
        if command_line
            .to_ascii_lowercase()
            .contains("cdxm-codex-app-bridge.exe")
        {
            pids.insert(pid);
        }
    }
    pids
}

fn endpoints_from_process_line(line: &str) -> Vec<(String, Endpoint)> {
    let tokens = line.split_whitespace().collect::<Vec<_>>();
    let mut endpoints = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let value = match *token {
            "--remote" | "--app-server" | "--listen" => tokens.get(index + 1).copied(),
            _ => token
                .strip_prefix("--remote=")
                .or_else(|| token.strip_prefix("--app-server="))
                .or_else(|| token.strip_prefix("--listen=")),
        };
        let Some(value) = value else {
            continue;
        };
        if value == "stdio://" || value == "unix://" {
            continue;
        }
        let Some(endpoint) = explicit_endpoint_from_url(value) else {
            continue;
        };
        let source = endpoint_source_from_process_line(line);
        endpoints.push((source.to_string(), endpoint));
    }
    endpoints
}

fn endpoint_source_from_process_line(line: &str) -> &'static str {
    if line.contains(" --remote ") || line.contains("--remote=") {
        "codex-cli-remote"
    } else if line.contains("codex-bridge") || line.contains("--app-server") {
        "agmsg-codex-bridge"
    } else {
        "codex-app-server-process"
    }
}

#[cfg(any(windows, test))]
#[derive(Debug, Clone, Eq, PartialEq)]
struct WindowsTcpListenRow {
    pid: u32,
    local_address: String,
    port: u16,
}

#[cfg(any(windows, test))]
fn parse_windows_process_row(line: &str) -> Option<(u32, String)> {
    let (pid, command_line) = line.trim().split_once('\t')?;
    Some((pid.trim().parse().ok()?, command_line.trim().to_string()))
}

#[cfg(any(windows, test))]
fn parse_windows_tcp_listen_rows(text: &str) -> Vec<WindowsTcpListenRow> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.trim().split('\t');
            let pid = parts.next()?.trim().parse().ok()?;
            let local_address = parts.next()?.trim().to_string();
            let port = parts.next()?.trim().parse().ok()?;
            Some(WindowsTcpListenRow {
                pid,
                local_address,
                port,
            })
        })
        .collect()
}

#[cfg(any(windows, test))]
fn is_codex_app_server_process(command_line: &str) -> bool {
    let lower = command_line.to_ascii_lowercase();
    lower.contains("codex") && lower.contains("app-server")
}

#[cfg(any(windows, test))]
fn loopback_ws_url_from_tcp_listen(local_address: &str, port: u16) -> Option<String> {
    if port == 0 {
        return None;
    }
    let host = match local_address.trim() {
        "127.0.0.1" | "localhost" => "127.0.0.1",
        "::1" => "[::1]",
        _ => return None,
    };
    Some(format!("ws://{host}:{port}"))
}

pub fn default_app_socket_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".codex/app-server-control/app-server-control.sock")
}

pub fn parse_thread_list(value: &Value) -> anyhow::Result<Vec<ThreadSummary>> {
    let raw_threads = value
        .get("threads")
        .or_else(|| value.get("items"))
        .or_else(|| value.get("data"))
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("thread/list response missing threads array"))?;

    let mut threads = Vec::new();
    for raw in raw_threads {
        let thread = raw.get("thread").unwrap_or(raw);
        let Some(id) = thread.get("id").and_then(Value::as_str) else {
            continue;
        };
        threads.push(ThreadSummary {
            id: id.to_string(),
            title: thread
                .get("title")
                .and_then(Value::as_str)
                .or_else(|| thread.get("name").and_then(Value::as_str))
                .or_else(|| thread.get("preview").and_then(Value::as_str))
                .map(str::to_string),
            cwd: thread
                .get("cwd")
                .or_else(|| thread.get("session").and_then(|session| session.get("cwd")))
                .and_then(Value::as_str)
                .map(str::to_string),
        });
    }
    Ok(threads)
}

pub fn resolve_single_thread(threads: &[ThreadSummary]) -> anyhow::Result<String> {
    match threads {
        [thread] => Ok(thread.id.clone()),
        [] => bail!("no matching threads"),
        many => {
            let ids = many
                .iter()
                .map(|thread| thread.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!("multiple matching threads; pass --thread explicitly: {ids}")
        }
    }
}

pub fn parse_loaded_thread_list(value: &Value) -> anyhow::Result<Vec<String>> {
    let raw_threads = value
        .get("data")
        .or_else(|| value.get("threadIds"))
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("thread/loaded/list response missing data array"))?;

    Ok(raw_threads
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn endpoint_explicit_wins() {
        assert_eq!(
            endpoint_from_options(
                Some("ws://127.0.0.1:7777".into()),
                crate::cli::TargetKind::Auto
            ),
            Endpoint::Explicit("ws://127.0.0.1:7777".into())
        );
        assert_eq!(
            endpoint_from_options(None, crate::cli::TargetKind::App),
            Endpoint::App
        );
        assert_eq!(
            endpoint_from_options(None, crate::cli::TargetKind::Managed),
            Endpoint::Managed
        );
        assert_eq!(
            endpoint_from_options(None, crate::cli::TargetKind::Auto),
            Endpoint::Auto
        );
    }

    #[test]
    fn discovers_live_cli_endpoints_from_process_text() {
        let text = r#"
/opt/homebrew/bin/codex --remote unix:///tmp/codex-cli.sock
node /Users/me/.agents/skills/agmsg/scripts/codex-bridge.js --project /tmp/p --app-server unix:///tmp/bridge.sock --thread t1
/opt/homebrew/bin/codex app-server --listen unix:///tmp/server.sock
/opt/homebrew/bin/codex app-server --listen unix://
"C:\Users\me\AppData\Local\OpenAI\Codex\bin\codex.exe" --remote ws://127.0.0.1:54014
"C:\Users\me\AppData\Local\OpenAI\Codex\bin\codex.exe" app-server --listen ws://127.0.0.1:54015
"C:\Users\me\AppData\Local\OpenAI\Codex\bin\codex.exe" app-server --listen ws://127.0.0.1:0
        "#;
        let parsed = discover_endpoint_candidates_from_process_text(text);
        assert_eq!(parsed.len(), 5);
        assert_eq!(parsed[0].source, "codex-cli-remote");
        assert_eq!(
            parsed[0].endpoint,
            Endpoint::Explicit("unix:///tmp/codex-cli.sock".to_string())
        );
        assert_eq!(parsed[1].source, "agmsg-codex-bridge");
        assert_eq!(
            parsed[1].endpoint,
            Endpoint::Explicit("unix:///tmp/bridge.sock".to_string())
        );
        assert_eq!(parsed[2].source, "codex-app-server-process");
        assert_eq!(
            parsed[2].endpoint,
            Endpoint::Explicit("unix:///tmp/server.sock".to_string())
        );
        assert_eq!(parsed[3].source, "codex-cli-remote");
        assert_eq!(
            parsed[3].endpoint,
            Endpoint::Explicit("ws://127.0.0.1:54014".to_string())
        );
        assert_eq!(parsed[4].source, "codex-app-server-process");
        assert_eq!(
            parsed[4].endpoint,
            Endpoint::Explicit("ws://127.0.0.1:54015".to_string())
        );
    }

    #[test]
    fn discovers_windows_dynamic_app_server_listen_ports_by_pid() {
        let process_text = r#"
51896	"C:\Users\me\AppData\Local\OpenAI\Codex\bin\codex.exe" app-server --listen ws://127.0.0.1:0
42544	"C:\Users\me\AppData\Local\OpenAI\Codex\bin\codex.exe" app-server --listen ws://127.0.0.1:0
101428	"C:\Program Files\WindowsApps\OpenAI.Codex\codex.exe" app-server --analytics-default-enabled
        "#;
        let tcp_text = r#"
51896	127.0.0.1	55212
42544	127.0.0.1	63030
101428	0.0.0.0	61234
        "#;

        let parsed =
            discover_windows_endpoint_candidates_from_process_and_tcp_text(process_text, tcp_text);

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].source, "codex-app-server-process");
        assert_eq!(
            parsed[0].endpoint,
            Endpoint::Explicit("ws://127.0.0.1:55212".to_string())
        );
        assert_eq!(parsed[1].source, "codex-app-server-process");
        assert_eq!(
            parsed[1].endpoint,
            Endpoint::Explicit("ws://127.0.0.1:63030".to_string())
        );
    }

    #[test]
    fn discovers_only_live_windows_app_bridge_process_ids() {
        let inventory = concat!(
            "__CDXM_PROCESSES__\n",
            "32868\tC:\\Users\\me\\.codex-monitor\\bin\\cdxm-codex-app-bridge.exe -c features.code_mode_host=true app-server\n",
            "42000\t\"C:\\Users\\me\\.codex-monitor\\runtime\\codex-app-real.exe\" app-server --listen ws://127.0.0.1:55123\n",
            "__CDXM_TCP__\n",
            "42000\t127.0.0.1\t55123\n",
        );

        assert_eq!(
            live_windows_app_bridge_pids_from_inventory_text(inventory),
            BTreeSet::from([32868])
        );
    }

    #[cfg(windows)]
    #[test]
    fn builds_windows_powershell_path_from_system_root() {
        assert_eq!(
            windows_powershell_path(std::path::Path::new(r"C:\Windows")),
            std::path::PathBuf::from(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe")
        );
    }

    #[test]
    fn windows_app_target_selects_single_bridge_candidate() {
        let candidates = vec![
            EndpointCandidate {
                endpoint: Endpoint::Explicit("ws://127.0.0.1:54014".into()),
                source: "codex-app-server-process".into(),
            },
            EndpointCandidate {
                endpoint: Endpoint::Explicit("ws://127.0.0.1:54015".into()),
                source: "codex-app-bridge".into(),
            },
        ];

        assert_eq!(
            select_windows_app_endpoint(candidates).unwrap(),
            Endpoint::Explicit("ws://127.0.0.1:54015".into())
        );
    }

    #[test]
    fn duplicate_endpoint_prefers_app_bridge_source() {
        let endpoint = Endpoint::Explicit("ws://127.0.0.1:54015".into());
        let mut candidates = Vec::new();
        let mut seen = BTreeSet::new();

        push_candidate(
            &mut candidates,
            &mut seen,
            endpoint.clone(),
            "codex-cli-remote".into(),
        );
        push_candidate(
            &mut candidates,
            &mut seen,
            endpoint,
            "codex-app-bridge".into(),
        );

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].source, "codex-app-bridge");
    }

    #[test]
    fn windows_app_target_rejects_ordinary_app_server_process() {
        let error = select_windows_app_endpoint(vec![EndpointCandidate {
            endpoint: Endpoint::Explicit("ws://127.0.0.1:54014".into()),
            source: "codex-app-server-process".into(),
        }])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("no live Codex App bridge endpoint found"));
    }

    #[test]
    fn windows_app_target_rejects_ambiguous_bridges() {
        let error = select_windows_app_endpoint(vec![
            EndpointCandidate {
                endpoint: Endpoint::Explicit("ws://127.0.0.1:54015".into()),
                source: "codex-app-bridge".into(),
            },
            EndpointCandidate {
                endpoint: Endpoint::Explicit("ws://127.0.0.1:54016".into()),
                source: "codex-app-bridge".into(),
            },
        ])
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("multiple live Codex App bridge endpoints found"));
    }

    #[test]
    fn parses_thread_list_shapes() {
        let value = json!({
            "threads": [
                { "id": "t1", "title": "One", "cwd": "/tmp/a" },
                { "thread": { "id": "t2", "title": "Two", "cwd": "/tmp/a" } }
            ]
        });
        let parsed = parse_thread_list(&value).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].id, "t1");
        assert_eq!(parsed[1].id, "t2");
    }

    #[test]
    fn parses_items_and_nested_session_cwd() {
        let value = json!({
            "items": [
                { "thread": { "id": "t1", "title": "One", "session": { "cwd": "/tmp/a" } } }
            ]
        });
        let parsed = parse_thread_list(&value).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].cwd.as_deref(), Some("/tmp/a"));
    }

    #[test]
    fn parses_current_app_server_data_shape() {
        let value = json!({
            "data": [
                {
                    "id": "thread-1",
                    "name": null,
                    "preview": "First user message",
                    "cwd": "/tmp/project"
                }
            ],
            "nextCursor": null
        });
        let parsed = parse_thread_list(&value).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "thread-1");
        assert_eq!(parsed[0].title.as_deref(), Some("First user message"));
        assert_eq!(parsed[0].cwd.as_deref(), Some("/tmp/project"));
    }

    #[test]
    fn rejects_missing_thread_array() {
        let error = parse_thread_list(&json!({ "result": [] })).unwrap_err();
        assert!(error
            .to_string()
            .contains("thread/list response missing threads array"));
    }

    #[test]
    fn single_thread_resolution_rejects_ambiguous_matches() {
        let one = vec![ThreadSummary {
            id: "t1".into(),
            title: None,
            cwd: None,
        }];
        assert_eq!(resolve_single_thread(&one).unwrap(), "t1");

        let two = vec![
            ThreadSummary {
                id: "t1".into(),
                title: None,
                cwd: None,
            },
            ThreadSummary {
                id: "t2".into(),
                title: None,
                cwd: None,
            },
        ];
        let error = resolve_single_thread(&two).unwrap_err();
        assert!(error.to_string().contains("multiple matching threads"));
    }

    #[test]
    fn parses_loaded_thread_list() {
        let parsed = parse_loaded_thread_list(&json!({
            "data": ["thread-1", "thread-2"],
            "nextCursor": null
        }))
        .unwrap();
        assert_eq!(parsed, vec!["thread-1", "thread-2"]);

        let error = parse_loaded_thread_list(&json!({ "threads": [] })).unwrap_err();
        assert!(error
            .to_string()
            .contains("thread/loaded/list response missing data array"));
    }
}
