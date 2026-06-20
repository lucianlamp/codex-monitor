use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize, Eq, PartialEq)]
pub struct State {
    pub delivered: BTreeMap<String, u64>,
}

pub struct StateStore {
    path: PathBuf,
}

impl StateStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn load(&self) -> anyhow::Result<State> {
        match tokio::fs::read_to_string(&self.path).await {
            Ok(raw) => Ok(serde_json::from_str(&raw)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn save(&self, state: &State) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let raw = serde_json::to_string_pretty(state)?;
        tokio::fs::write(&tmp, raw).await?;
        tokio::fs::rename(&tmp, &self.path).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_file_loads_default_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path().join("state.json"));
        let loaded = store.load().await.unwrap();
        assert_eq!(loaded, State::default());
    }

    #[tokio::test]
    async fn saves_and_loads_state_json() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path().join("nested/state.json"));
        let mut state = State::default();
        state.delivered.insert("agmsg:dev:sally".into(), 42);
        store.save(&state).await.unwrap();
        let loaded = store.load().await.unwrap();
        assert_eq!(loaded, state);
        let raw = tokio::fs::read_to_string(store.path()).await.unwrap();
        assert!(raw.contains("agmsg:dev:sally"));
    }
}
