#[cfg(not(windows))]
use std::collections::BTreeMap;
use std::path::PathBuf;

#[cfg(not(windows))]
use rusqlite::Connection;

use crate::sources::BridgeEvent;

pub struct AgmsgSource {
    #[cfg(not(windows))]
    db_path: PathBuf,
    #[cfg(not(windows))]
    team: String,
    #[cfg(not(windows))]
    name: String,
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
            WHERE team = ?1 AND to_agent = ?2 AND id > ?3
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
}
