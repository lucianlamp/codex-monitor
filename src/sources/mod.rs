use std::collections::BTreeMap;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BridgeEvent {
    pub source: String,
    pub event_id: String,
    pub observed_at: String,
    pub title: String,
    pub body: String,
    pub cwd_hint: Option<String>,
    pub reply_hint: Option<BTreeMap<String, String>>,
    pub metadata: BTreeMap<String, String>,
}

pub mod agmsg;
