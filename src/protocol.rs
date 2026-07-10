use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Response {
        id: u64,
        result: Option<Value>,
        error: Option<Value>,
    },
    Notification {
        method: String,
    },
    ServerRequest {
        id: u64,
        method: String,
    },
    Unknown,
}

pub fn initialize(id: u64) -> Value {
    json!({
        "method": "initialize",
        "id": id,
        "params": {
            "clientInfo": {
                "name": crate::CLIENT_INFO_NAME,
                "title": crate::CLIENT_INFO_TITLE,
                "version": crate::VERSION
            },
            "capabilities": { "experimentalApi": true }
        }
    })
}

pub fn initialized() -> Value {
    json!({ "method": "initialized", "params": {} })
}

pub fn thread_list_by_cwd(id: u64, cwd: &str, limit: u32) -> Value {
    json!({
        "method": "thread/list",
        "id": id,
        "params": {
            "cwd": cwd,
            "limit": limit,
            "sortDirection": "desc"
        }
    })
}

pub fn thread_read(id: u64, thread_id: &str, include_turns: bool) -> Value {
    json!({
        "method": "thread/read",
        "id": id,
        "params": {
            "threadId": thread_id,
            "includeTurns": include_turns
        }
    })
}

pub fn thread_loaded_list(id: u64, limit: u32) -> Value {
    json!({
        "method": "thread/loaded/list",
        "id": id,
        "params": {
            "limit": limit
        }
    })
}

pub fn remote_control_status_read(id: u64) -> Value {
    json!({
        "method": "remoteControl/status/read",
        "id": id,
        "params": null
    })
}

pub fn remote_control_enable(id: u64) -> Value {
    json!({
        "method": "remoteControl/enable",
        "id": id,
        "params": null
    })
}

pub fn remote_control_disable(id: u64) -> Value {
    json!({
        "method": "remoteControl/disable",
        "id": id,
        "params": null
    })
}

pub fn remote_control_pairing_start(id: u64, manual_code: bool) -> Value {
    json!({
        "method": "remoteControl/pairing/start",
        "id": id,
        "params": {
            "manualCode": manual_code
        }
    })
}

pub fn remote_control_pairing_status(
    id: u64,
    pairing_code: Option<&str>,
    manual_pairing_code: Option<&str>,
) -> Value {
    json!({
        "method": "remoteControl/pairing/status",
        "id": id,
        "params": {
            "pairingCode": pairing_code,
            "manualPairingCode": manual_pairing_code
        }
    })
}

pub fn remote_control_clients_list(id: u64, environment_id: &str, limit: u32) -> Value {
    json!({
        "method": "remoteControl/client/list",
        "id": id,
        "params": {
            "environmentId": environment_id,
            "limit": limit,
            "cursor": null,
            "order": "desc"
        }
    })
}

pub fn account_read(id: u64, refresh_token: bool) -> Value {
    json!({
        "method": "account/read",
        "id": id,
        "params": {
            "refreshToken": refresh_token
        }
    })
}

pub fn turn_start(id: u64, thread_id: &str, text: &str) -> Value {
    json!({
        "method": "turn/start",
        "id": id,
        "params": {
            "threadId": thread_id,
            "input": [{ "type": "text", "text": text }]
        }
    })
}

pub fn turn_steer(id: u64, thread_id: &str, expected_turn_id: &str, text: &str) -> Value {
    json!({
        "method": "turn/steer",
        "id": id,
        "params": {
            "threadId": thread_id,
            "expectedTurnId": expected_turn_id,
            "input": [{ "type": "text", "text": text }]
        }
    })
}

pub fn thread_inject_items(id: u64, thread_id: &str, items: Vec<Value>) -> Value {
    json!({
        "method": "thread/inject_items",
        "id": id,
        "params": {
            "threadId": thread_id,
            "items": items
        }
    })
}

pub fn classify(value: &Value) -> Incoming {
    let id = value.get("id").and_then(Value::as_u64);
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_owned);
    match (id, method) {
        (Some(id), Some(method)) => Incoming::ServerRequest { id, method },
        (Some(id), None) => Incoming::Response {
            id,
            result: value.get("result").cloned(),
            error: value.get("error").cloned(),
        },
        (None, Some(method)) => Incoming::Notification { method },
        (None, None) => Incoming::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_has_fixed_client_info() {
        let v = initialize(1);
        assert_eq!(v["method"], "initialize");
        assert_eq!(v["id"], 1);
        assert_eq!(v["params"]["clientInfo"]["name"], "codex-monitor");
        assert_eq!(v["params"]["clientInfo"]["title"], "Codex Monitor");
        assert_eq!(v["params"]["clientInfo"]["version"], "0.2.0");
        assert_eq!(v["params"]["capabilities"]["experimentalApi"], true);
    }

    #[test]
    fn initialized_is_notification() {
        let v = initialized();
        assert_eq!(v, json!({ "method": "initialized", "params": {} }));
    }

    #[test]
    fn thread_list_uses_cwd_filter_and_limit() {
        let v = thread_list_by_cwd(2, "/tmp/project", 20);
        assert_eq!(v["method"], "thread/list");
        assert_eq!(v["params"]["cwd"], "/tmp/project");
        assert_eq!(v["params"]["limit"], 20);
        assert_eq!(v["params"]["sortDirection"], "desc");
    }

    #[test]
    fn thread_read_can_include_turns() {
        let v = thread_read(3, "thread-1", true);
        assert_eq!(v["method"], "thread/read");
        assert_eq!(v["params"]["threadId"], "thread-1");
        assert_eq!(v["params"]["includeTurns"], true);
    }

    #[test]
    fn thread_loaded_list_requests_loaded_thread_ids() {
        let v = thread_loaded_list(4, 100);
        assert_eq!(v["method"], "thread/loaded/list");
        assert_eq!(v["params"]["limit"], 100);
    }

    #[test]
    fn remote_control_status_read_uses_null_params() {
        let v = remote_control_status_read(5);
        assert_eq!(v["method"], "remoteControl/status/read");
        assert!(v["params"].is_null());
    }

    #[test]
    fn remote_control_enable_uses_null_params_for_current_daemon() {
        let v = remote_control_enable(6);
        assert_eq!(v["method"], "remoteControl/enable");
        assert!(v["params"].is_null());
    }

    #[test]
    fn remote_control_clients_list_uses_environment_id() {
        let v = remote_control_clients_list(6, "env-1", 100);
        assert_eq!(v["method"], "remoteControl/client/list");
        assert_eq!(v["params"]["environmentId"], "env-1");
        assert_eq!(v["params"]["order"], "desc");
    }

    #[test]
    fn account_read_can_request_token_refresh() {
        let v = account_read(7, true);
        assert_eq!(v["method"], "account/read");
        assert_eq!(v["params"]["refreshToken"], true);
    }

    #[test]
    fn turn_start_wraps_text_input() {
        let v = turn_start(4, "thread-1", "hello");
        assert_eq!(v["method"], "turn/start");
        assert_eq!(v["params"]["threadId"], "thread-1");
        assert_eq!(v["params"]["input"][0]["type"], "text");
        assert_eq!(v["params"]["input"][0]["text"], "hello");
    }

    #[test]
    fn turn_steer_wraps_text_input_with_expected_turn_id() {
        let v = turn_steer(5, "thread-1", "turn-1", "hello");
        assert_eq!(v["method"], "turn/steer");
        assert_eq!(v["params"]["threadId"], "thread-1");
        assert_eq!(v["params"]["expectedTurnId"], "turn-1");
        assert_eq!(v["params"]["input"][0]["type"], "text");
        assert_eq!(v["params"]["input"][0]["text"], "hello");
    }

    #[test]
    fn inject_items_is_explicit_raw_append() {
        let item = json!({ "type": "message", "role": "user", "content": "x" });
        let v = thread_inject_items(5, "thread-1", vec![item.clone()]);
        assert_eq!(v["method"], "thread/inject_items");
        assert_eq!(v["params"]["threadId"], "thread-1");
        assert_eq!(v["params"]["items"], json!([item]));
    }

    #[test]
    fn classify_response_notification_and_server_request() {
        assert_eq!(
            classify(&json!({ "id": 1, "result": { "ok": true } })),
            Incoming::Response {
                id: 1,
                result: Some(json!({ "ok": true })),
                error: None
            }
        );
        assert_eq!(
            classify(&json!({ "method": "turn/completed", "params": {} })),
            Incoming::Notification {
                method: "turn/completed".to_string()
            }
        );
        assert_eq!(
            classify(&json!({ "id": 7, "method": "approval/request", "params": {} })),
            Incoming::ServerRequest {
                id: 7,
                method: "approval/request".to_string()
            }
        );
    }
}
