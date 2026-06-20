use crate::sources::BridgeEvent;

pub fn format_event_for_turn(event: &BridgeEvent) -> String {
    if event.source == "agmsg" {
        let team = event
            .metadata
            .get("team")
            .map(String::as_str)
            .unwrap_or("-");
        let recipient = event
            .metadata
            .get("recipient")
            .map(String::as_str)
            .unwrap_or("-");
        let sender = event
            .metadata
            .get("sender")
            .map(String::as_str)
            .unwrap_or("-");
        return format!(
            "agmsg monitor event\n\nTeam: {team}\nRecipient: {recipient}\nSender: {sender}\n\n{}\n\nIf this requires a reply, use the agmsg scripts rather than answering only in chat.",
            event.body
        );
    }

    format!("{}\n\n{}", event.title, event.body)
}

pub async fn run_agmsg_watch(
    endpoint: crate::target::Endpoint,
    team: String,
    name: String,
    thread: String,
    agmsg_db: Option<String>,
) -> anyhow::Result<i32> {
    let db_path = agmsg_db
        .map(std::path::PathBuf::from)
        .unwrap_or_else(crate::sources::agmsg::AgmsgSource::default_db_path);
    let state_path = default_state_path()?;
    let store = crate::state::StateStore::new(state_path);
    let mut state = store.load().await?;
    let state_key = format!("agmsg:{team}:{name}");
    let source = crate::sources::agmsg::AgmsgSource::new(db_path, team, name);

    let transport = crate::transport::open_endpoint_transport(endpoint).await?;
    let mut client = crate::client::AppServerClient::new(transport);
    client.initialize().await?;

    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    loop {
        let last_seen = state.last_seen(&state_key);
        let events = match source.poll_after(last_seen) {
            Ok(events) => events,
            Err(error) => {
                let _ = client.close().await;
                return Err(error);
            }
        };
        for event in events {
            let text = format_event_for_turn(&event);
            if let Err(error) = client.turn_start_and_wait(&thread, &text).await {
                eprintln!("delivery failed for {}: {error:#}", event.event_id);
                let _ = client.close().await;
                return Err(error);
            }
            if let Some(raw_id) = event
                .metadata
                .get("agmsg_id")
                .and_then(|id| id.parse::<u64>().ok())
            {
                state.mark_seen(state_key.clone(), raw_id);
                store.save(&state).await?;
            }
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

fn default_state_path() -> anyhow::Result<std::path::PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "codex-control-bridge")
        .ok_or_else(|| anyhow::anyhow!("could not resolve local state directory"))?;
    Ok(dirs
        .state_dir()
        .unwrap_or_else(|| dirs.cache_dir())
        .join("state.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn formats_agmsg_event_with_reply_instruction() {
        let mut metadata = BTreeMap::new();
        metadata.insert("team".to_string(), "dev".to_string());
        metadata.insert("recipient".to_string(), "sally".to_string());
        metadata.insert("sender".to_string(), "kimura".to_string());
        let event = BridgeEvent {
            source: "agmsg".into(),
            event_id: "agmsg:dev:sally:1".into(),
            observed_at: "2026-06-20T00:00:00Z".into(),
            title: "agmsg from kimura".into(),
            body: "please check status".into(),
            cwd_hint: None,
            reply_hint: None,
            metadata,
        };
        let text = format_event_for_turn(&event);
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
            event_id: "other:1".into(),
            observed_at: "2026-06-20T00:00:00Z".into(),
            title: "External update".into(),
            body: "details".into(),
            cwd_hint: None,
            reply_hint: None,
            metadata: BTreeMap::new(),
        };
        let text = format_event_for_turn(&event);
        assert!(text.contains("External update"));
        assert!(text.contains("details"));
    }
}
