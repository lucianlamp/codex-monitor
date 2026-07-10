use crate::sources::BridgeEventSource;
use std::future::Future;
use std::time::Duration;

const WATCH_INTERVAL: Duration = Duration::from_secs(2);

enum TargetGuard {
    Current,
    Drifted {
        endpoint: crate::target::Endpoint,
        thread: String,
    },
}

enum DeliveryPass {
    Healthy,
    TargetDrifted {
        endpoint: crate::target::Endpoint,
        thread: String,
    },
    SessionFailed {
        event_id: String,
        error: anyhow::Error,
    },
}

struct DeliveryContext<'a> {
    source: &'a dyn BridgeEventSource,
    state_key: &'a str,
    store: &'a crate::state::StateStore,
    mode: crate::cli::SendMode,
    thread: &'a str,
}

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
    let state_path = default_state_path()?;
    let store = crate::state::StateStore::new(state_path);
    let mut state = store.load().await?;

    if options.dry_run {
        let (endpoint, thread) =
            crate::cli::resolve_endpoint_and_thread(options.endpoint, options.thread, options.cwd)
                .await?;
        let endpoint_label = crate::target::endpoint_label(&endpoint);
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

    let logical_endpoint = options.endpoint;
    let requested_thread = options.thread;
    let cwd = options.cwd;

    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    'sessions: loop {
        let (endpoint, thread, mut client) = match open_monitor_session(
            logical_endpoint.clone(),
            requested_thread.clone(),
            cwd.clone(),
        )
        .await
        {
            Ok(session) => session,
            Err(error) => {
                eprintln!("monitor session unavailable; retrying: {error:#}");
                #[cfg(unix)]
                let shutdown = wait_for_shutdown_or_delay(WATCH_INTERVAL, &mut sigterm).await?;
                #[cfg(not(unix))]
                let shutdown = wait_for_shutdown_or_delay(WATCH_INTERVAL).await?;
                if shutdown {
                    return Ok(0);
                }
                continue;
            }
        };
        eprintln!(
            "monitor connected: endpoint={} thread={}",
            crate::target::endpoint_label(&endpoint),
            sanitize_field(&thread)
        );

        loop {
            let events = match options
                .source
                .poll_after(state.last_seen(&options.state_key))
            {
                Ok(events) => events,
                Err(error) => {
                    let _ = client.close().await;
                    return Err(error);
                }
            };
            let guard = if events.is_empty() {
                TargetGuard::Current
            } else {
                match revalidate_dynamic_target(
                    &logical_endpoint,
                    &requested_thread,
                    &cwd,
                    &endpoint,
                    &thread,
                )
                .await
                {
                    Ok(guard) => guard,
                    Err(error) => {
                        eprintln!("monitor target revalidation failed; reconnecting: {error:#}");
                        let _ = client.close().await;
                        #[cfg(unix)]
                        let shutdown =
                            wait_for_shutdown_or_delay(WATCH_INTERVAL, &mut sigterm).await?;
                        #[cfg(not(unix))]
                        let shutdown = wait_for_shutdown_or_delay(WATCH_INTERVAL).await?;
                        if shutdown {
                            return Ok(0);
                        }
                        continue 'sessions;
                    }
                }
            };
            let delivery_context = DeliveryContext {
                source: options.source.as_ref(),
                state_key: &options.state_key,
                store: &store,
                mode: options.mode,
                thread: &thread,
            };
            let pass = match deliver_polled_events(
                &delivery_context,
                events,
                guard,
                &mut state,
                &mut client,
            )
            .await
            {
                Ok(pass) => pass,
                Err(error) => {
                    let _ = client.close().await;
                    return Err(error);
                }
            };

            match pass {
                DeliveryPass::Healthy => {}
                DeliveryPass::TargetDrifted {
                    endpoint: current_endpoint,
                    thread: current_thread,
                } => {
                    eprintln!(
                        "monitor target changed: endpoint={} -> {} thread={} -> {}; reconnecting",
                        crate::target::endpoint_label(&endpoint),
                        crate::target::endpoint_label(&current_endpoint),
                        sanitize_field(&thread),
                        sanitize_field(&current_thread)
                    );
                    let _ = client.close().await;
                    continue 'sessions;
                }
                DeliveryPass::SessionFailed { event_id, error } => {
                    eprintln!("delivery failed for {event_id}; reconnecting: {error:#}");
                    let _ = client.close().await;
                    #[cfg(unix)]
                    let shutdown = wait_for_shutdown_or_delay(WATCH_INTERVAL, &mut sigterm).await?;
                    #[cfg(not(unix))]
                    let shutdown = wait_for_shutdown_or_delay(WATCH_INTERVAL).await?;
                    if shutdown {
                        return Ok(0);
                    }
                    continue 'sessions;
                }
            }

            #[cfg(unix)]
            let shutdown = wait_for_shutdown_or_delay(WATCH_INTERVAL, &mut sigterm).await?;
            #[cfg(not(unix))]
            let shutdown = wait_for_shutdown_or_delay(WATCH_INTERVAL).await?;
            if shutdown {
                let _ = client.close().await;
                return Ok(0);
            }
        }
    }
}

async fn open_monitor_session(
    endpoint: crate::target::Endpoint,
    thread: Option<String>,
    cwd: Option<std::path::PathBuf>,
) -> anyhow::Result<(
    crate::target::Endpoint,
    String,
    crate::client::AppServerClient<Box<dyn crate::transport::AppServerTransport>>,
)> {
    let (endpoint, thread) = crate::cli::resolve_endpoint_and_thread(endpoint, thread, cwd).await?;
    let transport = crate::transport::open_endpoint_transport(endpoint.clone()).await?;
    let mut client = crate::client::AppServerClient::new(transport);
    let setup = async {
        client.initialize().await?;
        client.ensure_thread_loaded(&thread).await
    }
    .await;
    if let Err(error) = setup {
        let _ = client.close().await;
        return Err(error);
    }
    Ok((endpoint, thread, client))
}

async fn revalidate_dynamic_target(
    logical_endpoint: &crate::target::Endpoint,
    requested_thread: &Option<String>,
    cwd: &Option<std::path::PathBuf>,
    connected_endpoint: &crate::target::Endpoint,
    connected_thread: &str,
) -> anyhow::Result<TargetGuard> {
    if !matches!(
        logical_endpoint,
        crate::target::Endpoint::App | crate::target::Endpoint::Auto
    ) {
        return Ok(TargetGuard::Current);
    }

    let (resolved_endpoint, resolved_thread) = crate::cli::resolve_endpoint_and_thread(
        logical_endpoint.clone(),
        requested_thread.clone(),
        cwd.clone(),
    )
    .await?;
    Ok(classify_resolved_target(
        logical_endpoint,
        connected_endpoint,
        connected_thread,
        resolved_endpoint,
        resolved_thread,
    ))
}

fn classify_resolved_target(
    logical_endpoint: &crate::target::Endpoint,
    connected_endpoint: &crate::target::Endpoint,
    connected_thread: &str,
    resolved_endpoint: crate::target::Endpoint,
    resolved_thread: String,
) -> TargetGuard {
    if !matches!(
        logical_endpoint,
        crate::target::Endpoint::App | crate::target::Endpoint::Auto
    ) || (connected_endpoint == &resolved_endpoint && connected_thread == resolved_thread)
    {
        return TargetGuard::Current;
    }

    TargetGuard::Drifted {
        endpoint: resolved_endpoint,
        thread: resolved_thread,
    }
}

async fn deliver_polled_events<T: crate::transport::AppServerTransport>(
    context: &DeliveryContext<'_>,
    events: Vec<crate::sources::BridgeEvent>,
    guard: TargetGuard,
    state: &mut crate::state::State,
    client: &mut crate::client::AppServerClient<T>,
) -> anyhow::Result<DeliveryPass> {
    if let TargetGuard::Drifted { endpoint, thread } = guard {
        return Ok(DeliveryPass::TargetDrifted { endpoint, thread });
    }

    for event in events {
        let text = context.source.format_event_for_turn(&event);
        if let Err(error) = deliver_event(client, context.thread, context.mode, &text).await {
            return Ok(DeliveryPass::SessionFailed {
                event_id: event.event_id,
                error,
            });
        }
        state.mark_seen(context.state_key.to_string(), event.cursor);
        context.store.save(state).await?;
    }
    Ok(DeliveryPass::Healthy)
}

async fn deliver_event<T: crate::transport::AppServerTransport>(
    client: &mut crate::client::AppServerClient<T>,
    thread: &str,
    mode: crate::cli::SendMode,
    text: &str,
) -> anyhow::Result<()> {
    match mode {
        crate::cli::SendMode::Start => client.turn_start(thread, text).await.map(|_| ()),
        crate::cli::SendMode::Steer => match client.active_turn_id(thread).await? {
            Some(active_turn) => client.turn_steer(thread, &active_turn, text).await,
            None => {
                anyhow::bail!("thread {thread} has no active turn; use --mode start or --mode auto")
            }
        },
        crate::cli::SendMode::Auto => client.turn_start_or_steer(thread, text, None).await,
    }
}

async fn wait_for_delay_or_shutdown<F>(delay: Duration, shutdown: F) -> anyhow::Result<bool>
where
    F: Future<Output = anyhow::Result<()>>,
{
    tokio::select! {
        result = shutdown => {
            result?;
            Ok(true)
        }
        _ = tokio::time::sleep(delay) => Ok(false),
    }
}

#[cfg(unix)]
async fn wait_for_shutdown_or_delay(
    delay: Duration,
    sigterm: &mut tokio::signal::unix::Signal,
) -> anyhow::Result<bool> {
    wait_for_delay_or_shutdown(delay, async {
        tokio::select! {
            result = tokio::signal::ctrl_c() => result.map_err(anyhow::Error::from),
            _ = sigterm.recv() => Ok(()),
        }
    })
    .await
}

#[cfg(not(unix))]
async fn wait_for_shutdown_or_delay(delay: Duration) -> anyhow::Result<bool> {
    wait_for_delay_or_shutdown(delay, async {
        tokio::signal::ctrl_c().await.map_err(anyhow::Error::from)
    })
    .await
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
    use crate::client::AppServerClient;
    use crate::sources::{BridgeEvent, BridgeEventSource};
    use crate::transport::memory::MemoryTransport;
    use serde_json::json;
    use std::collections::BTreeMap;

    struct FixedSource {
        event: BridgeEvent,
    }

    impl BridgeEventSource for FixedSource {
        fn poll_after(&self, last_seen_id: u64) -> anyhow::Result<Vec<BridgeEvent>> {
            Ok((self.event.cursor > last_seen_id)
                .then(|| self.event.clone())
                .into_iter()
                .collect())
        }

        fn format_event_for_turn(&self, event: &BridgeEvent) -> String {
            event.body.clone()
        }
    }

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

    #[test]
    fn dynamic_target_drift_is_detected() {
        let guard = super::classify_resolved_target(
            &crate::target::Endpoint::App,
            &crate::target::Endpoint::Explicit("ws://127.0.0.1:60498".into()),
            "thread-1",
            crate::target::Endpoint::Explicit("ws://127.0.0.1:56473".into()),
            "thread-1".into(),
        );

        assert!(matches!(
            guard,
            super::TargetGuard::Drifted {
                endpoint: crate::target::Endpoint::Explicit(ref endpoint),
                ref thread,
            } if endpoint == "ws://127.0.0.1:56473" && thread == "thread-1"
        ));
    }

    #[tokio::test]
    async fn drifted_target_does_not_send_or_advance_state() {
        let event = BridgeEvent {
            source: "agmsg".into(),
            cursor: 43,
            event_id: "agmsg:dev:codex:43".into(),
            observed_at: "2026-07-10T00:00:01Z".into(),
            title: "agmsg from codex".into(),
            body: "do not send to stale endpoint".into(),
            cwd_hint: None,
            reply_hint: None,
            metadata: BTreeMap::new(),
        };
        let source = FixedSource {
            event: event.clone(),
        };
        let directory = tempfile::tempdir().unwrap();
        let store = crate::state::StateStore::new(directory.path().join("state.json"));
        let mut state = crate::state::State::default();
        let transport = MemoryTransport::new(vec![
            json!({ "id": 1, "result": { "thread": { "status": { "type": "active" }, "turns": [{ "id": "turn-active", "status": "inProgress" }] } } }),
            json!({ "id": 2, "result": {} }),
        ]);
        let mut client = AppServerClient::new(transport);
        let context = super::DeliveryContext {
            source: &source,
            state_key: "agmsg:dev:codex",
            store: &store,
            mode: crate::cli::SendMode::Auto,
            thread: "thread-1",
        };

        let pass = super::deliver_polled_events(
            &context,
            vec![event],
            super::TargetGuard::Drifted {
                endpoint: crate::target::Endpoint::Explicit("ws://127.0.0.1:56473".into()),
                thread: "thread-1".into(),
            },
            &mut state,
            &mut client,
        )
        .await
        .unwrap();

        assert!(matches!(pass, super::DeliveryPass::TargetDrifted { .. }));
        assert_eq!(state.last_seen("agmsg:dev:codex"), 0);
        assert!(client.into_inner().sent.is_empty());
    }

    #[tokio::test]
    async fn failed_delivery_keeps_cursor_for_retry_until_acknowledged() {
        let event = BridgeEvent {
            source: "agmsg".into(),
            cursor: 42,
            event_id: "agmsg:dev:codex:42".into(),
            observed_at: "2026-07-10T00:00:00Z".into(),
            title: "agmsg from codex".into(),
            body: "retry me".into(),
            cwd_hint: None,
            reply_hint: None,
            metadata: BTreeMap::new(),
        };
        let source = FixedSource { event };
        let directory = tempfile::tempdir().unwrap();
        let store = crate::state::StateStore::new(directory.path().join("state.json"));
        let mut state = crate::state::State::default();

        let failed_transport = MemoryTransport::new(vec![
            json!({
                "id": 1,
                "result": {
                    "thread": {
                        "id": "thread-1",
                        "status": { "type": "active" },
                        "turns": [{ "id": "turn-active", "status": "inProgress" }]
                    }
                }
            }),
            json!({ "id": 2, "error": { "message": "connection reset" } }),
        ]);
        let mut failed_client = AppServerClient::new(failed_transport);
        let context = super::DeliveryContext {
            source: &source,
            state_key: "agmsg:dev:codex",
            store: &store,
            mode: crate::cli::SendMode::Auto,
            thread: "thread-1",
        };

        let first_events = source
            .poll_after(state.last_seen("agmsg:dev:codex"))
            .unwrap();
        let first = super::deliver_polled_events(
            &context,
            first_events,
            super::TargetGuard::Current,
            &mut state,
            &mut failed_client,
        )
        .await
        .unwrap();

        assert!(matches!(first, super::DeliveryPass::SessionFailed { .. }));
        assert_eq!(state.last_seen("agmsg:dev:codex"), 0);
        assert_eq!(store.load().await.unwrap().last_seen("agmsg:dev:codex"), 0);

        let successful_transport = MemoryTransport::new(vec![
            json!({
                "id": 1,
                "result": {
                    "thread": {
                        "id": "thread-1",
                        "status": { "type": "active" },
                        "turns": [{ "id": "turn-active", "status": "inProgress" }]
                    }
                }
            }),
            json!({ "id": 2, "result": {} }),
        ]);
        let mut successful_client = AppServerClient::new(successful_transport);

        let second_events = source
            .poll_after(state.last_seen("agmsg:dev:codex"))
            .unwrap();
        let second = super::deliver_polled_events(
            &context,
            second_events,
            super::TargetGuard::Current,
            &mut state,
            &mut successful_client,
        )
        .await
        .unwrap();

        assert!(matches!(second, super::DeliveryPass::Healthy));
        assert_eq!(state.last_seen("agmsg:dev:codex"), 42);
        assert_eq!(store.load().await.unwrap().last_seen("agmsg:dev:codex"), 42);
    }

    #[tokio::test]
    async fn shutdown_interrupts_reconnect_delay() {
        let interrupted = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            super::wait_for_delay_or_shutdown(std::time::Duration::from_secs(60), async { Ok(()) }),
        )
        .await
        .unwrap()
        .unwrap();

        assert!(interrupted);
    }
}
