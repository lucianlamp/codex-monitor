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
