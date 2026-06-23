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
    if seen.insert(label) {
        candidates.push(EndpointCandidate { endpoint, source });
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
fn discover_process_endpoint_candidates() -> Vec<EndpointCandidate> {
    let command = "$ErrorActionPreference='SilentlyContinue'; Get-CimInstance Win32_Process | ForEach-Object { $_.CommandLine }";
    let output = std::process::Command::new("powershell.exe")
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
            discover_endpoint_candidates_from_process_text(&String::from_utf8_lossy(&output.stdout))
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
        let source = if line.contains(" --remote ") || line.contains("--remote=") {
            "codex-cli-remote"
        } else if line.contains("codex-bridge") || line.contains("--app-server") {
            "agmsg-codex-bridge"
        } else {
            "codex-app-server-process"
        };
        endpoints.push((source.to_string(), endpoint));
    }
    endpoints
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
