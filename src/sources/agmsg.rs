#[cfg(not(windows))]
use std::collections::BTreeMap;
use std::path::PathBuf;

#[cfg(not(windows))]
use rusqlite::Connection;

use crate::sources::{BridgeEvent, BridgeEventSource};

pub struct AgmsgSource {
    #[cfg(not(windows))]
    db_path: PathBuf,
    #[cfg(not(windows))]
    team: String,
    #[cfg(not(windows))]
    name: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AgmsgInboxStats {
    pub latest_id: Option<u64>,
    pub latest_unread_id: Option<u64>,
    pub next_pending_after_state_id: Option<u64>,
    pub pending_after_state_count: u64,
    pub unread_count: u64,
}

impl AgmsgSource {
    pub fn new(db_path: PathBuf, team: String, name: String) -> Self {
        #[cfg(windows)]
        {
            let _ = (db_path, team, name);
            Self {}
        }

        #[cfg(not(windows))]
        {
            Self {
                db_path,
                team,
                name,
            }
        }
    }

    pub fn default_db_path() -> PathBuf {
        if let Ok(root) = std::env::var("AGMSG_STORAGE_PATH") {
            return PathBuf::from(root).join("messages.db");
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".agents/skills/agmsg/db/messages.db")
    }

    pub fn poll_after(&self, last_seen_id: u64) -> anyhow::Result<Vec<BridgeEvent>> {
        #[cfg(windows)]
        {
            let _ = last_seen_id;
            anyhow::bail!("agmsg SQLite adapter is not available on Windows builds");
        }

        #[cfg(not(windows))]
        {
            let conn = Connection::open(&self.db_path)?;
            let last_seen_id = i64::try_from(last_seen_id)?;
            let mut statement = conn.prepare(
                r#"
            SELECT id, created_at, from_agent, body
            FROM messages
            WHERE team = ?1 AND to_agent = ?2 AND id > ?3 AND read_at IS NULL
            ORDER BY id ASC
            "#,
            )?;
            let rows = statement.query_map((&self.team, &self.name, last_seen_id), |row| {
                let id: i64 = row.get(0)?;
                let observed_at: String = row.get(1)?;
                let sender: String = row.get(2)?;
                let body: String = row.get(3)?;
                let mut metadata = BTreeMap::new();
                metadata.insert("team".to_string(), self.team.clone());
                metadata.insert("recipient".to_string(), self.name.clone());
                metadata.insert("sender".to_string(), sender.clone());
                metadata.insert("agmsg_id".to_string(), id.to_string());
                Ok(BridgeEvent {
                    source: "agmsg".to_string(),
                    cursor: u64::try_from(id).unwrap_or(0),
                    event_id: format!("agmsg:{}:{}:{id}", self.team, self.name),
                    observed_at,
                    title: format!("agmsg from {sender}"),
                    body,
                    cwd_hint: None,
                    reply_hint: None,
                    metadata,
                })
            })?;

            let mut events = Vec::new();
            for row in rows {
                events.push(row?);
            }
            Ok(events)
        }
    }

    pub fn inbox_stats(&self, last_seen_id: u64) -> anyhow::Result<AgmsgInboxStats> {
        #[cfg(windows)]
        {
            let _ = last_seen_id;
            anyhow::bail!("agmsg SQLite adapter is not available on Windows builds");
        }

        #[cfg(not(windows))]
        {
            let conn = Connection::open(&self.db_path)?;
            let last_seen_id = i64::try_from(last_seen_id)?;
            let mut statement = conn.prepare(
                r#"
            SELECT
                MAX(id),
                MAX(CASE WHEN read_at IS NULL THEN id END),
                MIN(CASE WHEN id > ?3 AND read_at IS NULL THEN id END),
                COUNT(CASE WHEN id > ?3 AND read_at IS NULL THEN 1 END),
                COUNT(CASE WHEN read_at IS NULL THEN 1 END)
            FROM messages
            WHERE team = ?1 AND to_agent = ?2
            "#,
            )?;
            let stats = statement.query_row((&self.team, &self.name, last_seen_id), |row| {
                Ok(AgmsgInboxStats {
                    latest_id: optional_i64_to_u64(row.get(0)?),
                    latest_unread_id: optional_i64_to_u64(row.get(1)?),
                    next_pending_after_state_id: optional_i64_to_u64(row.get(2)?),
                    pending_after_state_count: i64_to_u64(row.get(3)?),
                    unread_count: i64_to_u64(row.get(4)?),
                })
            })?;
            Ok(stats)
        }
    }
}

impl BridgeEventSource for AgmsgSource {
    fn poll_after(&self, last_seen_id: u64) -> anyhow::Result<Vec<BridgeEvent>> {
        self.poll_after(last_seen_id)
    }

    fn format_event_for_turn(&self, event: &BridgeEvent) -> String {
        format_agmsg_event_for_turn(event)
    }
}

pub fn format_agmsg_event_for_turn(event: &BridgeEvent) -> String {
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
    format!(
        "agmsg monitor event\n\nTeam: {team}\nRecipient: {recipient}\nSender: {sender}\n\n{}\n\nIf this requires a reply, use the agmsg scripts rather than answering only in chat.",
        event.body
    )
}

#[cfg(not(windows))]
fn optional_i64_to_u64(value: Option<i64>) -> Option<u64> {
    value.and_then(|value| u64::try_from(value).ok())
}

#[cfg(not(windows))]
fn i64_to_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}
