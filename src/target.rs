use anyhow::{anyhow, bail};
use serde_json::Value;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Endpoint {
    Managed,
    App,
    Explicit(String),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ThreadSummary {
    pub id: String,
    pub title: Option<String>,
    pub cwd: Option<String>,
}

pub fn endpoint_from_options(endpoint: Option<String>, target: crate::cli::TargetKind) -> Endpoint {
    match endpoint {
        Some(url) => Endpoint::Explicit(url),
        None if target == crate::cli::TargetKind::App => Endpoint::App,
        None => Endpoint::Managed,
    }
}

pub fn parse_thread_list(value: &Value) -> anyhow::Result<Vec<ThreadSummary>> {
    let raw_threads = value
        .get("threads")
        .or_else(|| value.get("items"))
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("thread/list response missing threads array"))?;

    let mut threads = Vec::new();
    for raw in raw_threads {
        let thread = raw.get("thread").unwrap_or(raw);
        let Some(id) = thread.get("id").and_then(Value::as_str) else {
            continue;
        };
        threads.push(ThreadSummary {
            id: id.to_string(),
            title: thread
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_string),
            cwd: thread
                .get("cwd")
                .or_else(|| thread.get("session").and_then(|session| session.get("cwd")))
                .and_then(Value::as_str)
                .map(str::to_string),
        });
    }
    Ok(threads)
}

pub fn resolve_single_thread(threads: &[ThreadSummary]) -> anyhow::Result<String> {
    match threads {
        [thread] => Ok(thread.id.clone()),
        [] => bail!("no matching threads"),
        many => {
            let ids = many
                .iter()
                .map(|thread| thread.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!("multiple matching threads; pass --thread explicitly: {ids}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn endpoint_explicit_wins() {
        assert_eq!(
            endpoint_from_options(
                Some("ws://127.0.0.1:7777".into()),
                crate::cli::TargetKind::App
            ),
            Endpoint::Explicit("ws://127.0.0.1:7777".into())
        );
        assert_eq!(
            endpoint_from_options(None, crate::cli::TargetKind::App),
            Endpoint::App
        );
        assert_eq!(
            endpoint_from_options(None, crate::cli::TargetKind::Managed),
            Endpoint::Managed
        );
    }

    #[test]
    fn parses_thread_list_shapes() {
        let value = json!({
            "threads": [
                { "id": "t1", "title": "One", "cwd": "/tmp/a" },
                { "thread": { "id": "t2", "title": "Two", "cwd": "/tmp/a" } }
            ]
        });
        let parsed = parse_thread_list(&value).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].id, "t1");
        assert_eq!(parsed[1].id, "t2");
    }

    #[test]
    fn parses_items_and_nested_session_cwd() {
        let value = json!({
            "items": [
                { "thread": { "id": "t1", "title": "One", "session": { "cwd": "/tmp/a" } } }
            ]
        });
        let parsed = parse_thread_list(&value).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].cwd.as_deref(), Some("/tmp/a"));
    }

    #[test]
    fn rejects_missing_thread_array() {
        let error = parse_thread_list(&json!({ "result": [] })).unwrap_err();
        assert!(error
            .to_string()
            .contains("thread/list response missing threads array"));
    }

    #[test]
    fn single_thread_resolution_rejects_ambiguous_matches() {
        let one = vec![ThreadSummary {
            id: "t1".into(),
            title: None,
            cwd: None,
        }];
        assert_eq!(resolve_single_thread(&one).unwrap(), "t1");

        let two = vec![
            ThreadSummary {
                id: "t1".into(),
                title: None,
                cwd: None,
            },
            ThreadSummary {
                id: "t2".into(),
                title: None,
                cwd: None,
            },
        ];
        let error = resolve_single_thread(&two).unwrap_err();
        assert!(error.to_string().contains("multiple matching threads"));
    }
}
