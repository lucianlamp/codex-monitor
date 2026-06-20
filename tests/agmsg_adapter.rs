use codex_control_bridge::delivery::format_event_for_turn;
use codex_control_bridge::sources::agmsg::AgmsgSource;

fn create_fixture_db(path: &std::path::Path) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            team TEXT NOT NULL,
            from_agent TEXT NOT NULL,
            to_agent TEXT NOT NULL,
            body TEXT NOT NULL,
            created_at TEXT NOT NULL,
            read_at TEXT
        );
        INSERT INTO messages (team, from_agent, to_agent, body, created_at, read_at)
        VALUES
            ('dev', 'kimura', 'sally', 'first', '2026-06-20T00:00:01Z', NULL),
            ('dev', 'nakai', 'other', 'skip me', '2026-06-20T00:00:02Z', NULL),
            ('dev', 'kimura', 'sally', 'second', '2026-06-20T00:00:03Z', NULL);
        "#,
    )
    .unwrap();
}

#[test]
fn polls_matching_messages_after_last_seen() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("messages.db");
    create_fixture_db(&db_path);

    let source = AgmsgSource::new(db_path, "dev".into(), "sally".into());
    let events = source.poll_after(1).unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_id, "agmsg:dev:sally:3");
    assert_eq!(events[0].body, "second");
    assert_eq!(events[0].observed_at, "2026-06-20T00:00:03Z");
    assert_eq!(events[0].metadata.get("team").unwrap(), "dev");
    assert_eq!(events[0].metadata.get("recipient").unwrap(), "sally");
    assert_eq!(events[0].metadata.get("sender").unwrap(), "kimura");
    assert_eq!(events[0].metadata.get("agmsg_id").unwrap(), "3");
}

#[test]
fn polls_matching_messages_in_ascending_order() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("messages.db");
    create_fixture_db(&db_path);

    let source = AgmsgSource::new(db_path, "dev".into(), "sally".into());
    let events = source.poll_after(0).unwrap();

    let ids = events
        .iter()
        .map(|event| event.metadata.get("agmsg_id").unwrap().as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["1", "3"]);
}

#[test]
fn agmsg_event_formats_for_delivery() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("messages.db");
    create_fixture_db(&db_path);

    let source = AgmsgSource::new(db_path, "dev".into(), "sally".into());
    let events = source.poll_after(0).unwrap();
    let text = format_event_for_turn(&events[0]);

    assert!(text.contains("Team: dev"));
    assert!(text.contains("Recipient: sally"));
    assert!(text.contains("first"));
}
