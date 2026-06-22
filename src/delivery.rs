use crate::sources::BridgeEventSource;

pub struct AgmsgWatchOptions {
    pub endpoint: crate::target::Endpoint,
    pub team: String,
    pub name: String,
    pub thread: Option<String>,
    pub cwd: Option<std::path::PathBuf>,
    pub mode: crate::cli::SendMode,
    pub agmsg_db: Option<String>,
    pub dry_run: bool,
}

pub struct MonitorWatchOptions {
    pub endpoint: crate::target::Endpoint,
    pub source_label: String,
    pub state_key: String,
    pub source: Box<dyn BridgeEventSource>,
    pub thread: Option<String>,
    pub cwd: Option<std::path::PathBuf>,
    pub mode: crate::cli::SendMode,
    pub dry_run: bool,
}

pub async fn run_agmsg_watch(options: AgmsgWatchOptions) -> anyhow::Result<i32> {
    let db_path = options
        .agmsg_db
        .map(std::path::PathBuf::from)
        .unwrap_or_else(crate::sources::agmsg::AgmsgSource::default_db_path);
    let state_key = format!("agmsg:{}:{}", options.team, options.name);
    let source = crate::sources::agmsg::AgmsgSource::new(db_path, options.team, options.name);
    run_monitor_watch(MonitorWatchOptions {
        endpoint: options.endpoint,
        source_label: "agmsg".to_string(),
        state_key,
        source: Box::new(source),
        thread: options.thread,
        cwd: options.cwd,
        mode: options.mode,
        dry_run: options.dry_run,
    })
    .await
}

pub async fn run_monitor_watch(options: MonitorWatchOptions) -> anyhow::Result<i32> {
    let (endpoint, thread) =
        crate::cli::resolve_endpoint_and_thread(options.endpoint, options.thread, options.cwd)
            .await?;
    let requires_loaded_thread = !matches!(endpoint, crate::target::Endpoint::Managed);
    let state_path = default_state_path()?;
    let store = crate::state::StateStore::new(state_path);
    let mut state = store.load().await?;
    let endpoint_label = crate::target::endpoint_label(&endpoint);

    if options.dry_run {
        let last_seen = state.last_seen(&options.state_key);
        let events = options.source.poll_after(last_seen)?;
        println!(
            "dry-run\ttarget\tendpoint={}\tthread={}\tmode={}\tsource={}",
            sanitize_field(&endpoint_label),
            sanitize_field(&thread),
            options.mode.as_str(),
            sanitize_field(&options.source_label)
        );
        println!(
            "dry-run\tstate\tkey={}\tlast_seen={}\tpath={}",
            sanitize_field(&options.state_key),
            last_seen,
            sanitize_field(&store.path().display().to_string())
        );
        if events.is_empty() {
            println!("dry-run\tdelivery\tnone");
        } else {
            for event in events {
                println!("{}", format_dry_run_delivery_line(&event));
            }
        }
        println!("dry-run\tnote\tno state update, no app-server turn sent");
        return Ok(0);
    }

    let transport = crate::transport::open_endpoint_transport(endpoint).await?;
    let mut client = crate::client::AppServerClient::new(transport);
    client.initialize().await?;
    if requires_loaded_thread {
        client.ensure_thread_loaded(&thread).await?;
    }

    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    loop {
        let last_seen = state.last_seen(&options.state_key);
        let events = match options.source.poll_after(last_seen) {
            Ok(events) => events,
            Err(error) => {
                let _ = client.close().await;
                return Err(error);
            }
        };
        for event in events {
            let text = options.source.format_event_for_turn(&event);
            let delivery = match options.mode {
                crate::cli::SendMode::Start => client.turn_start(&thread, &text).await.map(|_| ()),
                crate::cli::SendMode::Steer => {
                    let active_turn = client.active_turn_id(&thread).await?;
                    match active_turn {
                        Some(active_turn) => client.turn_steer(&thread, &active_turn, &text).await,
                        None => anyhow::bail!(
                            "thread {thread} has no active turn; use --mode start or --mode auto"
                        ),
                    }
                }
                crate::cli::SendMode::Auto => {
                    client.turn_start_or_steer(&thread, &text, None).await
                }
            };
            if let Err(error) = delivery {
                eprintln!("delivery failed for {}: {error:#}", event.event_id);
                let _ = client.close().await;
                return Err(error);
            }
            state.mark_seen(options.state_key.clone(), event.cursor);
            store.save(&state).await?;
        }

        #[cfg(unix)]
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                client.close().await?;
                return Ok(0);
            },
            _ = sigterm.recv() => {
                client.close().await?;
                return Ok(0);
            },
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
        }

        #[cfg(not(unix))]
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                client.close().await?;
                return Ok(0);
            },
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
        }
    }
}

pub fn default_state_path() -> anyhow::Result<std::path::PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "codex-monitor")
        .ok_or_else(|| anyhow::anyhow!("could not resolve local state directory"))?;
    #[cfg(windows)]
    let dir = dirs.data_local_dir();
    #[cfg(not(windows))]
    let dir = dirs.state_dir().unwrap_or_else(|| dirs.cache_dir());
    Ok(dir.join("state.json"))
}

fn sanitize_field(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

fn format_dry_run_delivery_line(event: &crate::sources::BridgeEvent) -> String {
    let mut fields = vec![
        format!("source={}", sanitize_field(&event.source)),
        format!("cursor={}", event.cursor),
        format!("event_id={}", sanitize_field(&event.event_id)),
    ];
    for (key, value) in &event.metadata {
        fields.push(format!("{}={}", sanitize_field(key), sanitize_field(value)));
    }
    format!("dry-run\tdelivery\t{}", fields.join("\t"))
}

#[cfg(test)]
mod tests {
    use crate::sources::BridgeEvent;
    use std::collections::BTreeMap;

    #[test]
    fn formats_agmsg_event_with_reply_instruction() {
        let mut metadata = BTreeMap::new();
        metadata.insert("team".to_string(), "dev".to_string());
        metadata.insert("recipient".to_string(), "sally".to_string());
        metadata.insert("sender".to_string(), "kimura".to_string());
        let event = BridgeEvent {
            source: "agmsg".into(),
            cursor: 1,
            event_id: "agmsg:dev:sally:1".into(),
            observed_at: "2026-06-20T00:00:00Z".into(),
            title: "agmsg from kimura".into(),
            body: "please check status".into(),
            cwd_hint: None,
            reply_hint: None,
            metadata,
        };
        let text = crate::sources::agmsg::format_agmsg_event_for_turn(&event);
        assert!(text.contains("agmsg monitor event"));
        assert!(text.contains("Team: dev"));
        assert!(text.contains("Recipient: sally"));
        assert!(text.contains("Sender: kimura"));
        assert!(text.contains("please check status"));
        assert!(text.contains("If this requires a reply, use the agmsg scripts"));
    }

    #[test]
    fn formats_unknown_source_with_title_and_body() {
        let event = BridgeEvent {
            source: "other".into(),
            cursor: 1,
            event_id: "other:1".into(),
            observed_at: "2026-06-20T00:00:00Z".into(),
            title: "External update".into(),
            body: "details".into(),
            cwd_hint: None,
            reply_hint: None,
            metadata: BTreeMap::new(),
        };
        let text = crate::sources::format_generic_event_for_turn(&event);
        assert!(text.contains("External update"));
        assert!(text.contains("details"));
    }

    #[test]
    fn dry_run_delivery_line_is_source_agnostic() {
        let mut metadata = BTreeMap::new();
        metadata.insert("recipient".to_string(), "sally".to_string());
        metadata.insert("sender".to_string(), "kimura".to_string());
        let event = BridgeEvent {
            source: "hmsg".into(),
            cursor: 42,
            event_id: "hmsg:dev:sally:42".into(),
            observed_at: "2026-06-20T00:00:00Z".into(),
            title: "hmsg from kimura".into(),
            body: "status?".into(),
            cwd_hint: None,
            reply_hint: None,
            metadata,
        };

        let line = super::format_dry_run_delivery_line(&event);

        assert!(line.contains("dry-run\tdelivery"));
        assert!(line.contains("source=hmsg"));
        assert!(line.contains("cursor=42"));
        assert!(line.contains("event_id=hmsg:dev:sally:42"));
        assert!(line.contains("recipient=sally"));
        assert!(line.contains("sender=kimura"));
        assert!(!line.contains("message_id=-"));
    }

    #[test]
    fn default_state_path_points_to_state_json() {
        let path = super::default_state_path().unwrap();
        assert_eq!(path.file_name().unwrap(), "state.json");
    }
}
