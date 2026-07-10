use std::collections::HashMap;

use serde_json::{json, Value};

const ALLOWED_REQUESTS: &[&str] = &[
    "thread/list",
    "thread/read",
    "thread/loaded/list",
    "turn/start",
    "turn/steer",
];

const BROADCAST_NOTIFICATIONS: &[&str] = &["turn/started", "turn/completed", "error"];

#[derive(Debug, PartialEq)]
pub(super) enum MonitorInput {
    Reply(Value),
    Forward(Value),
    Ignore,
}

#[derive(Debug, PartialEq)]
pub(super) enum ChildOutput {
    AppOnly,
    AppAndBroadcast(Value),
    Monitor { connection_id: u64, message: Value },
    Drop,
}

pub(super) struct MonitorRouter {
    nonce: String,
    internal_prefix: String,
    ready: bool,
    next_sequence: u64,
    pending: HashMap<String, Pending>,
}

struct Pending {
    connection_id: u64,
    original_id: Value,
}

impl MonitorRouter {
    pub(super) fn new(nonce: impl Into<String>) -> Self {
        let nonce = nonce.into();
        Self {
            internal_prefix: format!("cdxm:{nonce}:"),
            nonce,
            ready: false,
            next_sequence: 1,
            pending: HashMap::new(),
        }
    }

    pub(super) fn observe_app(&mut self, message: &Value) -> bool {
        if !self.ready && message.get("method").and_then(Value::as_str) == Some("initialized") {
            self.ready = true;
            return true;
        }
        false
    }

    pub(super) fn handle_monitor(
        &mut self,
        connection_id: u64,
        mut message: Value,
    ) -> MonitorInput {
        let Some(method) = message
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return MonitorInput::Reply(error_response(
                request_id(&message),
                -32600,
                "invalid monitor request",
            ));
        };

        if method == "initialized" {
            return if message.get("id").is_none() {
                MonitorInput::Ignore
            } else {
                MonitorInput::Reply(error_response(
                    request_id(&message),
                    -32600,
                    "initialized must be a notification",
                ))
            };
        }

        let Some(original_id) = numeric_request_id(&message) else {
            return MonitorInput::Reply(error_response(
                request_id(&message),
                -32600,
                "monitor request id must be numeric",
            ));
        };

        if method == "initialize" {
            return MonitorInput::Reply(json!({
                "id": original_id,
                "result": {
                    "serverInfo": {
                        "name": "codex-monitor-stdio-bridge",
                        "version": crate::VERSION,
                    }
                }
            }));
        }

        if !ALLOWED_REQUESTS.contains(&method.as_str()) {
            return MonitorInput::Reply(error_response(
                original_id,
                -32601,
                "method is not available on the monitor endpoint",
            ));
        }

        if !self.ready {
            return MonitorInput::Reply(error_response(
                original_id,
                -32002,
                "Codex App app-server is not initialized yet",
            ));
        }

        let internal_id = format!("cdxm:{}:{connection_id}:{}", self.nonce, self.next_sequence);
        self.next_sequence = self.next_sequence.wrapping_add(1).max(1);
        self.pending.insert(
            internal_id.clone(),
            Pending {
                connection_id,
                original_id,
            },
        );
        message["id"] = Value::String(internal_id);
        MonitorInput::Forward(message)
    }

    pub(super) fn route_child(&mut self, message: &Value) -> ChildOutput {
        if let Some(internal_id) = message.get("id").and_then(Value::as_str) {
            if let Some(pending) = self.pending.remove(internal_id) {
                let mut restored = message.clone();
                restored["id"] = pending.original_id;
                return ChildOutput::Monitor {
                    connection_id: pending.connection_id,
                    message: restored,
                };
            }
            if internal_id.starts_with(&self.internal_prefix) {
                return ChildOutput::Drop;
            }
        }

        let method = message.get("method").and_then(Value::as_str);
        if message.get("id").is_none()
            && method.is_some_and(|method| BROADCAST_NOTIFICATIONS.contains(&method))
        {
            return ChildOutput::AppAndBroadcast(message.clone());
        }
        ChildOutput::AppOnly
    }

    pub(super) fn retire_connection(&mut self, connection_id: u64) {
        self.pending
            .retain(|_, pending| pending.connection_id != connection_id);
    }

    pub(super) fn cancel_forward(&mut self, message: &Value) {
        if let Some(internal_id) = message.get("id").and_then(Value::as_str) {
            self.pending.remove(internal_id);
        }
    }
}

fn numeric_request_id(message: &Value) -> Option<Value> {
    message.get("id").filter(|id| id.is_number()).cloned()
}

fn request_id(message: &Value) -> Value {
    message.get("id").cloned().unwrap_or(Value::Null)
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "id": id,
        "error": {
            "code": code,
            "message": message,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn initialize_is_local_and_allowlisted_request_waits_for_app_readiness() {
        let mut router = MonitorRouter::new("bridge-nonce");
        let init = router.handle_monitor(7, json!({"id":1,"method":"initialize","params":{}}));
        assert!(matches!(init, MonitorInput::Reply(value) if value["id"] == 1));

        let early = router.handle_monitor(7, json!({"id":2,"method":"thread/list","params":{}}));
        assert!(matches!(early, MonitorInput::Reply(value) if value["error"]["code"] == -32002));

        assert!(router.observe_app(&json!({"method":"initialized","params":{}})));
        let ready = router.handle_monitor(7, json!({"id":3,"method":"thread/list","params":{}}));
        assert!(matches!(ready, MonitorInput::Forward(value)
            if value["id"].as_str().unwrap().starts_with("cdxm:bridge-nonce:7:")));
    }

    #[test]
    fn responses_return_only_to_the_owning_monitor_and_restore_ids() {
        let mut router = MonitorRouter::new("bridge-nonce");
        router.observe_app(&json!({"method":"initialized","params":{}}));
        let MonitorInput::Forward(forwarded) = router.handle_monitor(
            9,
            json!({"id":41,"method":"thread/read","params":{"threadId":"t"}}),
        ) else {
            panic!("request was not forwarded")
        };
        let internal = forwarded["id"].clone();
        let routed = router.route_child(&json!({"id":internal,"result":{"thread":{"id":"t"}}}));
        assert_eq!(
            routed,
            ChildOutput::Monitor {
                connection_id: 9,
                message: json!({"id":41,"result":{"thread":{"id":"t"}}}),
            }
        );
    }

    #[test]
    fn disallowed_methods_server_requests_and_notifications_stay_bounded() {
        let mut router = MonitorRouter::new("bridge-nonce");
        router.observe_app(&json!({"method":"initialized","params":{}}));
        let denied = router.handle_monitor(1, json!({"id":1,"method":"account/read","params":{}}));
        assert!(matches!(denied, MonitorInput::Reply(value) if value["error"]["code"] == -32601));
        assert_eq!(
            router.route_child(&json!({"id":8,"method":"item/tool/call","params":{}})),
            ChildOutput::AppOnly
        );
        assert!(matches!(
            router.route_child(&json!({"method":"turn/completed","params":{"turn":{"id":"x"}}})),
            ChildOutput::AppAndBroadcast(_)
        ));
        assert_eq!(
            router.route_child(&json!({"method":"thread/started","params":{}})),
            ChildOutput::AppOnly
        );
    }

    #[test]
    fn retiring_connection_discards_late_internal_responses() {
        let mut router = MonitorRouter::new("bridge-nonce");
        router.observe_app(&json!({"method":"initialized","params":{}}));
        let MonitorInput::Forward(forwarded) = router.handle_monitor(
            11,
            json!({"id":5,"method":"thread/loaded/list","params":{"limit":100}}),
        ) else {
            panic!("request was not forwarded")
        };
        router.retire_connection(11);
        assert_eq!(
            router.route_child(&json!({"id":forwarded["id"].clone(),"result":{"data":[]}})),
            ChildOutput::Drop
        );
    }
}
