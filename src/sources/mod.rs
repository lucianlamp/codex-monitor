use std::collections::BTreeMap;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BridgeEvent {
    pub source: String,
    pub cursor: u64,
    pub event_id: String,
    pub observed_at: String,
    pub title: String,
    pub body: String,
    pub cwd_hint: Option<String>,
    pub reply_hint: Option<BTreeMap<String, String>>,
    pub metadata: BTreeMap<String, String>,
}

pub trait BridgeEventSource: Send + Sync {
    fn poll_after(&self, last_seen_id: u64) -> anyhow::Result<Vec<BridgeEvent>>;

    fn format_event_for_turn(&self, event: &BridgeEvent) -> String {
        format_generic_event_for_turn(event)
    }
}

pub fn format_generic_event_for_turn(event: &BridgeEvent) -> String {
    format!("{}\n\n{}", event.title, event.body)
}

pub mod agmsg;
