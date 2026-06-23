use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
#[cfg(not(windows))]
use std::io::{Read, Write};
#[cfg(not(windows))]
use std::net::TcpListener as StdTcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use tokio::net::TcpListener;
#[cfg(unix)]
use tokio::net::UnixListener;
use tokio_tungstenite::{accept_async, tungstenite::protocol::Message};
#[cfg(not(windows))]
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::handshake::server::{Request as WsRequest, Response as WsResponse},
};

async fn start_fake_server() -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for _ in 0..2 {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let mut ws = accept_async(stream).await.unwrap();
            while let Some(message) = ws.next().await {
                let Ok(message) = message else {
                    break;
                };
                let Message::Text(text) = message else {
                    continue;
                };
                let request: Value = serde_json::from_str(&text).unwrap();
                match request["method"].as_str().unwrap() {
                    "initialize" => {
                        ws.send(Message::Text(
                            json!({ "id": request["id"], "result": {} })
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    "initialized" => {}
                    "thread/list" => {
                        ws.send(Message::Text(
                            json!({
                                "id": request["id"],
                                "result": {
                                    "data": [
                                        {
                                            "id": "thread-1",
                                            "name": "One",
                                            "preview": "First user message",
                                            "cwd": "/tmp/project"
                                        }
                                    ],
                                    "nextCursor": null
                                }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    "thread/loaded/list" => {
                        ws.send(Message::Text(
                            json!({
                                "id": request["id"],
                                "result": {
                                    "data": ["thread-1", "thread-2"],
                                    "nextCursor": null
                                }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    "thread/read" => {
                        assert_eq!(request["params"]["threadId"], "thread-1");
                        assert_eq!(request["params"]["includeTurns"], true);
                        ws.send(Message::Text(
                            json!({
                                "id": request["id"],
                                "result": {
                                    "thread": {
                                        "id": "thread-1",
                                        "status": { "type": "idle" },
                                        "turns": []
                                    }
                                }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    "remoteControl/status/read" => {
                        ws.send(Message::Text(
                            json!({
                                "id": request["id"],
                                "result": {
                                    "status": "connected",
                                    "serverName": "fake-mac.local",
                                    "installationId": "install-1",
                                    "environmentId": "env-1"
                                }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    "remoteControl/enable" => {
                        assert!(request["params"].is_null());
                        ws.send(Message::Text(
                            json!({
                                "id": request["id"],
                                "result": {
                                    "status": "connected",
                                    "serverName": "fake-mac.local",
                                    "installationId": "install-1",
                                    "environmentId": "env-1"
                                }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    "remoteControl/disable" => {
                        assert!(request["params"].is_null());
                        ws.send(Message::Text(
                            json!({
                                "id": request["id"],
                                "result": {
                                    "status": "disabled",
                                    "serverName": "fake-mac.local",
                                    "installationId": "install-1",
                                    "environmentId": null
                                }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    "remoteControl/client/list" => {
                        assert_eq!(request["params"]["environmentId"], "env-1");
                        ws.send(Message::Text(
                            json!({
                                "id": request["id"],
                                "result": {
                                    "data": [
                                        {
                                            "clientId": "client-1",
                                            "displayName": "Phone",
                                            "platform": "ios",
                                            "lastSeenAt": 1781840000000i64
                                        }
                                    ],
                                    "nextCursor": null
                                }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    "remoteControl/pairing/start" => {
                        assert_eq!(request["params"]["manualCode"], true);
                        ws.send(Message::Text(
                            json!({
                                "id": request["id"],
                                "result": {
                                    "environmentId": "env-1",
                                    "pairingCode": "pair-123",
                                    "manualPairingCode": "manual-456",
                                    "expiresAt": 1781840300000i64
                                }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    "account/read" => {
                        assert_eq!(request["params"]["refreshToken"], true);
                        ws.send(Message::Text(
                            json!({
                                "id": request["id"],
                                "result": {
                                    "account": {
                                        "type": "chatgpt",
                                        "email": "test@example.com",
                                        "planType": "pro"
                                    },
                                    "requiresOpenaiAuth": false
                                }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    "turn/start" => {
                        ws.send(Message::Text(
                            json!({
                                "id": request["id"],
                                "result": { "turn": { "id": "turn-1" } }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                        ws.send(Message::Text(
                            json!({
                                "method": "turn/completed",
                                "params": {
                                    "turn": { "id": "turn-1", "status": "completed" }
                                }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    other => panic!("unexpected method {other}"),
                }
            }
        }
    });
    format!("ws://{}", addr)
}

async fn start_fake_server_with_remote_client_list_error() -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for _ in 0..2 {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let mut ws = accept_async(stream).await.unwrap();
            while let Some(message) = ws.next().await {
                let Ok(message) = message else {
                    break;
                };
                let Message::Text(text) = message else {
                    continue;
                };
                let request: Value = serde_json::from_str(&text).unwrap();
                match request["method"].as_str().unwrap() {
                    "initialize" => {
                        ws.send(Message::Text(
                            json!({ "id": request["id"], "result": {} })
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    "initialized" => {}
                    "remoteControl/status/read" => {
                        ws.send(Message::Text(
                            json!({
                                "id": request["id"],
                                "result": {
                                    "status": "connected",
                                    "serverName": "fake-mac.local",
                                    "installationId": "install-1",
                                    "environmentId": "env-1"
                                }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    "remoteControl/client/list" => {
                        ws.send(Message::Text(
                            json!({
                                "id": request["id"],
                                "error": {
                                    "code": -32000,
                                    "message": "client list unavailable"
                                }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    other => panic!("unexpected method {other}"),
                }
            }
        }
    });
    format!("ws://{}", addr)
}

async fn start_fake_active_turn_server() -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for _ in 0..2 {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let mut ws = accept_async(stream).await.unwrap();
            while let Some(message) = ws.next().await {
                let Ok(message) = message else {
                    break;
                };
                let Message::Text(text) = message else {
                    continue;
                };
                let request: Value = serde_json::from_str(&text).unwrap();
                match request["method"].as_str().unwrap() {
                    "initialize" => {
                        ws.send(Message::Text(
                            json!({ "id": request["id"], "result": {} })
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    "initialized" => {}
                    "thread/loaded/list" => {
                        ws.send(Message::Text(
                            json!({
                                "id": request["id"],
                                "result": { "data": ["thread-1"], "nextCursor": null }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    "thread/read" => {
                        assert_eq!(request["params"]["threadId"], "thread-1");
                        assert_eq!(request["params"]["includeTurns"], true);
                        ws.send(Message::Text(
                            json!({
                                "id": request["id"],
                                "result": {
                                    "thread": {
                                        "id": "thread-1",
                                        "status": { "type": "active", "activeFlags": [] },
                                        "turns": [
                                            { "id": "turn-active", "status": "inProgress", "items": [] }
                                        ]
                                    }
                                }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    "turn/steer" => {
                        assert_eq!(request["params"]["threadId"], "thread-1");
                        assert_eq!(request["params"]["expectedTurnId"], "turn-active");
                        assert_eq!(request["params"]["input"][0]["text"], "hello");
                        ws.send(Message::Text(
                            json!({ "id": request["id"], "result": {} })
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                        break;
                    }
                    other => panic!("unexpected method {other}"),
                }
            }
        }
    });
    format!("ws://{}", addr)
}

async fn start_fake_idle_ack_only_server() -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for _ in 0..2 {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let mut ws = accept_async(stream).await.unwrap();
            while let Some(message) = ws.next().await {
                let Ok(message) = message else {
                    break;
                };
                let Message::Text(text) = message else {
                    continue;
                };
                let request: Value = serde_json::from_str(&text).unwrap();
                match request["method"].as_str().unwrap() {
                    "initialize" => {
                        ws.send(Message::Text(
                            json!({ "id": request["id"], "result": {} })
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    "initialized" => {}
                    "thread/loaded/list" => {
                        ws.send(Message::Text(
                            json!({
                                "id": request["id"],
                                "result": { "data": ["thread-1"], "nextCursor": null }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    "thread/list" => {
                        assert_eq!(request["params"]["cwd"], "/tmp/project");
                        ws.send(Message::Text(
                            json!({
                                "id": request["id"],
                                "result": {
                                    "data": [
                                        {
                                            "id": "thread-1",
                                            "name": "Project thread",
                                            "preview": "Existing thread",
                                            "cwd": "/tmp/project"
                                        }
                                    ],
                                    "nextCursor": null
                                }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    "thread/read" => {
                        assert_eq!(request["params"]["threadId"], "thread-1");
                        assert_eq!(request["params"]["includeTurns"], true);
                        ws.send(Message::Text(
                            json!({
                                "id": request["id"],
                                "result": {
                                    "thread": {
                                        "id": "thread-1",
                                        "status": { "type": "idle" },
                                        "turns": []
                                    }
                                }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    "turn/start" => {
                        assert_eq!(request["params"]["threadId"], "thread-1");
                        assert_eq!(request["params"]["input"][0]["text"], "hello");
                        ws.send(Message::Text(
                            json!({
                                "id": request["id"],
                                "result": { "turn": { "id": "turn-started" } }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                        break;
                    }
                    other => panic!("unexpected method {other}"),
                }
            }
        }
    });
    format!("ws://{}", addr)
}

async fn start_fake_unloaded_thread_listing_server() -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for _ in 0..2 {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let mut ws = accept_async(stream).await.unwrap();
            while let Some(message) = ws.next().await {
                let Ok(message) = message else {
                    break;
                };
                let Message::Text(text) = message else {
                    continue;
                };
                let request: Value = serde_json::from_str(&text).unwrap();
                match request["method"].as_str().unwrap() {
                    "initialize" => {
                        ws.send(Message::Text(
                            json!({ "id": request["id"], "result": {} })
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    "initialized" => {}
                    "thread/loaded/list" => {
                        ws.send(Message::Text(
                            json!({
                                "id": request["id"],
                                "result": { "data": [], "nextCursor": null }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    "thread/list" => {
                        ws.send(Message::Text(
                            json!({
                                "id": request["id"],
                                "result": {
                                    "data": [
                                        {
                                            "id": "thread-1",
                                            "name": "Project thread",
                                            "preview": "Existing but unloaded thread",
                                            "cwd": "/tmp/project"
                                        }
                                    ],
                                    "nextCursor": null
                                }
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .unwrap();
                    }
                    other => panic!("unexpected method {other}"),
                }
            }
        }
    });
    format!("ws://{}", addr)
}

#[cfg(unix)]
async fn start_fake_unix_server() -> String {
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("fake-app-server.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();
    tokio::spawn(async move {
        let _dir = dir;
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        while let Some(message) = ws.next().await {
            let Ok(message) = message else {
                break;
            };
            let Message::Text(text) = message else {
                continue;
            };
            let request: Value = serde_json::from_str(&text).unwrap();
            match request["method"].as_str().unwrap() {
                "initialize" => {
                    ws.send(Message::Text(
                        json!({ "id": request["id"], "result": {} })
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                }
                "initialized" => {}
                "thread/list" => {
                    ws.send(Message::Text(
                        json!({
                            "id": request["id"],
                            "result": {
                                "data": [
                                    {
                                        "id": "thread-1",
                                        "name": "One",
                                        "preview": "First user message",
                                        "cwd": "/tmp/project"
                                    }
                                ],
                                "nextCursor": null
                            }
                        })
                        .to_string()
                        .into(),
                    ))
                    .await
                    .unwrap();
                }
                other => panic!("unexpected method {other}"),
            }
        }
    });
    format!("unix://{}", socket_path.display())
}

fn fake_jwt(account_id: &str, account_user_id: &str) -> String {
    use base64::Engine;
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("{}");
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
        json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": account_id,
                "chatgpt_account_user_id": account_user_id
            }
        })
        .to_string(),
    );
    format!("{header}.{payload}.sig")
}

fn test_enrollment_record() -> codex_monitor::remote_control::RemoteControlClientEnrollmentRecord {
    codex_monitor::remote_control::RemoteControlClientEnrollmentRecord {
        account_user_id: "user-test".to_string(),
        client_id: "cli-test".to_string(),
        key_id: "key-test".to_string(),
        algorithm: "ES256".to_string(),
        protection_class: "allow_os_protected_nonextractable".to_string(),
        public_key_spki_der_base64: "public-key".to_string(),
    }
}

fn write_test_auth_and_state(
    dir: &tempfile::TempDir,
    enrollment: &codex_monitor::remote_control::RemoteControlClientEnrollmentRecord,
) -> (PathBuf, PathBuf) {
    let auth_file = dir.path().join("auth.json");
    let state_file = dir.path().join("state.json");
    std::fs::write(
        &auth_file,
        json!({
            "tokens": {
                "access_token": fake_jwt("acct-test", "user-test")
            }
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        &state_file,
        json!({
            "electron-remote-control-client-enrollments": {
                "Codex Desktop\nhttp://localhost:8000/api\nhttps://chatgpt.com/backend-api\nafter-first-unlock-device-key-v1\nuser-test": enrollment
            }
        })
        .to_string(),
    )
    .unwrap();
    (auth_file, state_file)
}

#[cfg(not(windows))]
fn write_device_key_module(path: &Path, missing: bool) {
    let script = if missing {
        r#"
exports.getDeviceKeyPublic = async function getDeviceKeyPublic() {
  throw new Error("device key not found");
};
"#
    } else {
        r#"
exports.getDeviceKeyPublic = async function getDeviceKeyPublic(keyId) {
  return {
    keyId,
    publicKeySpkiDerBase64: "public-key",
    algorithm: "ES256",
    protectionClass: "allow_os_protected_nonextractable"
  };
};
exports.signDeviceKey = async function signDeviceKey(keyId, payload) {
  if (!Buffer.isBuffer(payload)) throw new Error("expected payload buffer");
  return {
    signatureDerBase64: "signature",
    algorithm: "ES256"
  };
};
"#
    };
    std::fs::write(path, script).unwrap();
}

#[cfg(not(windows))]
fn start_fake_backend_clients_success() -> String {
    start_fake_backend_clients(
        "HTTP/1.1 200 OK",
        json!({
            "items": [
                {
                    "client_id": "backend-phone",
                    "display_name": "Backend Phone",
                    "device_type": "phone",
                    "platform": "ios",
                    "device_model": "iPhone",
                    "last_seen_at": "2099-01-01T00:00:00Z",
                    "enrollment_status": "enrolled"
                },
                {
                    "client_id": "pending-client",
                    "display_name": "Pending",
                    "platform": "ios",
                    "enrollment_status": "pending_enrollment"
                }
            ],
            "cursor": null
        }),
    )
}

#[cfg(not(windows))]
fn start_fake_backend_clients_failure() -> String {
    start_fake_backend_clients(
        "HTTP/1.1 401 Unauthorized",
        json!({
            "detail": "auth expired"
        }),
    )
}

#[cfg(not(windows))]
fn start_fake_backend_clients(status_line: &'static str, response: Value) -> String {
    let listener = StdTcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let (request, _body) = read_http_request(&mut stream);
        assert!(request.starts_with("GET /wham/remote/control/clients?limit=100 HTTP/1.1"));
        assert!(request.contains("authorization: Bearer "));
        assert!(request.contains("chatgpt-account-id: acct-test"));
        write_http_json_status(&mut stream, status_line, &response);
    });
    format!("http://{}", addr)
}

#[cfg(not(windows))]
fn start_fake_pair_backend(
    expected_client_id: &'static str,
    expected_code: &'static str,
) -> String {
    let listener = StdTcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = vec![0u8; 8192];
        let mut read = 0usize;
        loop {
            let n = stream.read(&mut buffer[read..]).unwrap();
            assert_ne!(n, 0, "http client closed before request body");
            read += n;
            let request = String::from_utf8_lossy(&buffer[..read]);
            let Some(header_end) = request.find("\r\n\r\n") else {
                continue;
            };
            let content_length = request
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .or_else(|| {
                    request
                        .lines()
                        .find_map(|line| line.strip_prefix("Content-Length: "))
                })
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if read < header_end + 4 + content_length {
                continue;
            }
            let body = &request[header_end + 4..header_end + 4 + content_length];
            assert!(request.starts_with("POST /wham/remote/control/client/pair HTTP/1.1"));
            assert!(request.contains("authorization: Bearer "));
            assert!(request.contains("chatgpt-account-id: acct-test"));
            let body: Value = serde_json::from_str(body).unwrap();
            assert_eq!(body["client_id"], expected_client_id);
            assert_eq!(body["manual_pairing_code"], expected_code);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 12\r\n\r\n{\"ok\":true}\n",
                )
                .unwrap();
            break;
        }
    });
    format!("http://{}", addr)
}

#[cfg(not(windows))]
fn start_fake_refresh_backend(
    expected_client_id: &'static str,
    expected_key_id: &'static str,
    device_identity_hash: String,
    remote_control_token: &'static str,
) -> String {
    let listener = StdTcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for request_index in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let (request, body) = read_http_request(&mut stream);
            if request_index == 0 {
                assert!(request
                    .starts_with("POST /api/codex/remote/control/client/refresh/start HTTP/1.1"));
                let body: Value = serde_json::from_str(&body).unwrap();
                assert_eq!(body["client_id"], expected_client_id);
                let response = json!({
                    "account_user_id": "user-test",
                    "client_id": expected_client_id,
                    "device_key_challenge": {
                        "challenge_id": "challenge-refresh",
                        "challenge_token": "challenge-token",
                        "nonce": "nonce-refresh",
                        "purpose": "remote_control_client_enrollment",
                        "audience": "remote_control_client_enrollment",
                        "account_user_id": "user-test",
                        "client_id": expected_client_id,
                        "target_origin": format!("http://{}", addr),
                        "target_path": "/api/codex/remote/control/client/refresh/finish",
                        "device_identity_hash": device_identity_hash.clone(),
                        "challenge_expires_at": "2099-01-01T00:00:00Z"
                    }
                });
                write_http_json(&mut stream, &response);
            } else {
                assert!(request
                    .starts_with("POST /api/codex/remote/control/client/refresh/finish HTTP/1.1"));
                let body: Value = serde_json::from_str(&body).unwrap();
                assert_eq!(body["client_id"], expected_client_id);
                assert_eq!(
                    body["device_key_proof"]["challenge_token"],
                    "challenge-token"
                );
                assert_eq!(body["device_key_proof"]["key_id"], expected_key_id);
                assert_eq!(
                    body["device_key_proof"]["signature_der_base64"],
                    "signature"
                );
                let response = json!({
                    "account_user_id": "user-test",
                    "client_id": expected_client_id,
                    "remote_control_token": remote_control_token,
                    "expires_at": "2099-01-01T00:00:00Z",
                    "scopes": ["remote_control_controller_websocket"]
                });
                write_http_json(&mut stream, &response);
            }
        }
    });
    format!("http://{addr}/api")
}

#[allow(clippy::result_large_err)]
#[cfg(not(windows))]
async fn start_fake_remote_control_websocket(
    expected_client_id: &'static str,
    expected_key_id: &'static str,
    remote_control_token: &'static str,
) -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let callback = |request: &WsRequest, response: WsResponse| {
            assert_eq!(request.uri().path(), "/remote-control-client");
            assert_eq!(
                request
                    .headers()
                    .get("x-codex-client-id")
                    .and_then(|value| value.to_str().ok()),
                Some(expected_client_id)
            );
            assert_eq!(
                request
                    .headers()
                    .get("x-codex-protocol-version")
                    .and_then(|value| value.to_str().ok()),
                Some("3")
            );
            assert!(request
                .headers()
                .get("x-codex-client-session-token")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value == format!("Bearer {remote_control_token}")));
            Ok(response)
        };
        let mut ws = accept_hdr_async(stream, callback).await.unwrap();
        let token_expires_at = 4_070_908_800i64;
        ws.send(Message::Text(
            json!({
                "type": "device_key_challenge",
                "nonce": "nonce-websocket",
                "purpose": "remote_control_client_websocket",
                "audience": "remote_control_client_websocket",
                "sessionId": "session-websocket",
                "targetOrigin": format!("http://{}", addr),
                "targetPath": "/remote-control-client",
                "accountUserId": "user-test",
                "clientId": expected_client_id,
                "tokenSha256Base64url": sha256_base64url(remote_control_token.as_bytes()),
                "tokenExpiresAt": token_expires_at,
                "scopes": ["remote_control_controller_websocket"]
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
        let proof = ws.next().await.unwrap().unwrap();
        let Message::Text(proof_text) = proof else {
            panic!("expected device key proof text message");
        };
        let proof: Value = serde_json::from_str(&proof_text).unwrap();
        assert_eq!(proof["type"], "device_key_proof");
        assert_eq!(proof["keyId"], expected_key_id);
        assert_eq!(proof["algorithm"], "ES256");
        assert_eq!(proof["signatureDerBase64"], "signature");
    });
    format!("ws://{addr}/remote-control-client")
}

#[cfg(not(windows))]
fn read_http_request(stream: &mut std::net::TcpStream) -> (String, String) {
    let mut buffer = vec![0u8; 8192];
    let mut read = 0usize;
    loop {
        let n = stream.read(&mut buffer[read..]).unwrap();
        assert_ne!(n, 0, "http client closed before request body");
        read += n;
        let request = String::from_utf8_lossy(&buffer[..read]).to_string();
        let Some(header_end) = request.find("\r\n\r\n") else {
            continue;
        };
        let content_length = request
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .or_else(|| {
                request
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
            })
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        if read < header_end + 4 + content_length {
            continue;
        }
        let body = request[header_end + 4..header_end + 4 + content_length].to_string();
        return (request, body);
    }
}

#[cfg(not(windows))]
fn write_http_json(stream: &mut std::net::TcpStream, value: &Value) {
    write_http_json_status(stream, "HTTP/1.1 200 OK", value);
}

#[cfg(not(windows))]
fn write_http_json_status(stream: &mut std::net::TcpStream, status_line: &str, value: &Value) {
    let body = value.to_string();
    write!(
        stream,
        "{}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
        status_line,
        body.len(),
        body
    )
    .unwrap();
}

#[cfg(not(windows))]
fn sha256_base64url(bytes: &[u8]) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(bytes))
}

fn write_agmsg_fixture_db(path: &Path, team: &str, name: &str) {
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
        "#,
    )
    .unwrap();
    conn.execute(
        "INSERT INTO messages (team, from_agent, to_agent, body, created_at, read_at) VALUES (?1, 'kimura', ?2, 'dry run body', '2026-06-20T00:00:01Z', NULL)",
        (team, name),
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn threads_command_lists_fake_thread() {
    let url = start_fake_server().await;
    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .args(["--endpoint", &url, "threads", "--cwd", "/tmp/project"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("thread-1"));
    assert!(stdout.contains("/tmp/project"));
}

#[tokio::test(flavor = "multi_thread")]
async fn loaded_command_lists_loaded_thread_ids() {
    let url = start_fake_server().await;
    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .args(["--endpoint", &url, "loaded"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("thread-1"));
    assert!(stdout.contains("thread-2"));
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_status_prints_current_remote_control_state() {
    let url = start_fake_server().await;
    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .args(["--endpoint", &url, "remote", "status"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("connected"));
    assert!(stdout.contains("fake-mac.local"));
    assert!(stdout.contains("env-1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_enable_uses_unit_params_for_current_daemon_contract() {
    let url = start_fake_server().await;
    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .args(["--endpoint", &url, "remote", "enable"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("connected"));
    assert!(stdout.contains("env-1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_clients_uses_status_environment_when_not_provided() {
    let url = start_fake_server().await;
    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .args(["--endpoint", &url, "remote", "clients"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("client-1"));
    assert!(stdout.contains("Phone"));
    assert!(stdout.contains("ios"));
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_monitor_prints_status_and_clients_once() {
    let url = start_fake_server().await;
    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .args([
            "--endpoint",
            &url,
            "remote",
            "monitor",
            "--count",
            "1",
            "--interval-ms",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("status\tconnected"));
    assert!(stdout.contains("client\tclient-1"));
    assert!(stdout.contains("Phone"));
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_pair_start_can_request_manual_pairing_code() {
    let url = start_fake_server().await;
    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .args(["--endpoint", &url, "remote", "pair-start", "--manual-code"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("env-1"));
    assert!(stdout.contains("pair-123"));
    assert!(stdout.contains("manual-456"));
}

#[cfg(not(windows))]
#[tokio::test(flavor = "multi_thread")]
async fn remote_claim_pairs_existing_enrolled_client_without_phone_input() {
    let app_server_url = start_fake_server().await;
    let backend_url = start_fake_pair_backend("cli-test", "MANUAL-123");
    let dir = tempfile::tempdir().unwrap();
    let auth_file = dir.path().join("auth.json");
    let state_file = dir.path().join("state.json");
    std::fs::write(
        &auth_file,
        json!({
            "tokens": {
                "access_token": fake_jwt("acct-test", "user-test")
            }
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        &state_file,
        json!({
            "electron-remote-control-client-enrollments": {
                "Codex Desktop\nhttp://localhost:8000/api\nhttps://chatgpt.com/backend-api\nafter-first-unlock-device-key-v1\nuser-test": {
                    "accountUserId": "user-test",
                    "clientId": "cli-test"
                }
            }
        })
        .to_string(),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .args([
            "--endpoint",
            &app_server_url,
            "remote",
            "claim",
            "--manual-pairing-code",
            "MANUAL-123",
            "--api-base-url",
            &backend_url,
            "--auth-file",
            auth_file.to_str().unwrap(),
            "--global-state-file",
            state_file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("claimed\tcli-test"));
}

#[tokio::test(flavor = "multi_thread")]
async fn send_defaults_to_steer_when_loaded_thread_has_active_turn() {
    let url = start_fake_active_turn_server().await;
    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .args([
            "--endpoint",
            &url,
            "send",
            "--thread",
            "thread-1",
            "--text",
            "hello",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn send_returns_after_turn_start_ack_by_default() {
    let url = start_fake_idle_ack_only_server().await;
    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .args([
            "--endpoint",
            &url,
            "send",
            "--thread",
            "thread-1",
            "--text",
            "hello",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn send_resolves_loaded_thread_from_cwd_when_thread_is_omitted() {
    let url = start_fake_idle_ack_only_server().await;
    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .args([
            "--endpoint",
            &url,
            "send",
            "--cwd",
            "/tmp/project",
            "--text",
            "hello",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn agmsg_watch_dry_run_prints_delivery_plan_without_sending_turn() {
    let url = start_fake_server().await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("messages.db");
    write_agmsg_fixture_db(&db_path, "dryrun-team", "target");

    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .env("HOME", dir.path())
        .args([
            "--endpoint",
            &url,
            "agmsg",
            "watch",
            "--team",
            "dryrun-team",
            "--name",
            "target",
            "--cwd",
            "/tmp/project",
            "--mode",
            "start",
            "--agmsg-db",
            db_path.to_str().unwrap(),
            "--dry-run",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("dry-run\ttarget"));
    assert!(stdout.contains("thread=thread-1"));
    assert!(stdout.contains("mode=start"));
    assert!(stdout.contains("dry-run\tdelivery\tsource=agmsg"));
    assert!(stdout.contains("agmsg_id=1"));
    assert!(stdout.contains("dry-run\tnote\tno state update, no app-server turn sent"));
}

#[tokio::test(flavor = "multi_thread")]
async fn monitor_watch_agmsg_dry_run_uses_agmsg_adapter() {
    let url = start_fake_server().await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("messages.db");
    write_agmsg_fixture_db(&db_path, "dryrun-team", "target");

    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .env("HOME", dir.path())
        .args([
            "--endpoint",
            &url,
            "monitor",
            "watch",
            "agmsg",
            "--team",
            "dryrun-team",
            "--name",
            "target",
            "--cwd",
            "/tmp/project",
            "--mode",
            "start",
            "--agmsg-db",
            db_path.to_str().unwrap(),
            "--dry-run",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("dry-run\ttarget"));
    assert!(stdout.contains("source=agmsg"));
    assert!(stdout.contains("thread=thread-1"));
    assert!(stdout.contains("mode=start"));
    assert!(stdout.contains("dry-run\tdelivery\tsource=agmsg"));
    assert!(stdout.contains("agmsg_id=1"));
    assert!(stdout.contains("cursor=1"));
    assert!(stdout.contains("dry-run\tnote\tno state update, no app-server turn sent"));
}

#[tokio::test(flavor = "multi_thread")]
async fn monitor_watch_dry_run_rejects_unloaded_explicit_thread() {
    let url = start_fake_unloaded_thread_listing_server().await;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("messages.db");
    write_agmsg_fixture_db(&db_path, "dryrun-team", "target");

    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .env("HOME", dir.path())
        .args([
            "--endpoint",
            &url,
            "monitor",
            "watch",
            "agmsg",
            "--team",
            "dryrun-team",
            "--name",
            "target",
            "--thread",
            "thread-1",
            "--agmsg-db",
            db_path.to_str().unwrap(),
            "--dry-run",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does not have loaded thread thread-1")
            || stderr.contains("no auto endpoint has loaded thread thread-1"),
        "stderr: {stderr}"
    );
}

#[cfg(not(windows))]
#[tokio::test(flavor = "multi_thread")]
async fn remote_doctor_reports_all_surfaces() {
    let app_server_url = start_fake_server().await;
    let backend_url = start_fake_backend_clients_success();
    let dir = tempfile::tempdir().unwrap();
    let enrollment = test_enrollment_record();
    let (auth_file, state_file) = write_test_auth_and_state(&dir, &enrollment);
    let signer_file = dir.path().join("fake-device-key.js");
    write_device_key_module(&signer_file, false);

    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .args([
            "--endpoint",
            &app_server_url,
            "remote",
            "doctor",
            "--api-base-url",
            &backend_url,
            "--auth-file",
            auth_file.to_str().unwrap(),
            "--global-state-file",
            state_file.to_str().unwrap(),
            "--device-key-module",
            signer_file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("doctor\tapp-server-status\tok\tconnected"));
    assert!(stdout.contains("doctor\tapp-server-clients\tok\t1"));
    assert!(stdout.contains("doctor\tbackend-clients\tok\t1"));
    assert!(stdout.contains("doctor\tbackend-client\tbackend-phone"));
    assert!(!stdout.contains("pending-client"));
    assert!(stdout.contains("doctor\tlocal-enrollment\tok\tcli-test\tkey-test"));
    assert!(stdout.contains("doctor\tdevice-key\tok\tcli-test\tkey-test\tavailable"));
}

#[cfg(not(windows))]
#[tokio::test(flavor = "multi_thread")]
async fn remote_doctor_keeps_going_when_device_key_missing() {
    let app_server_url = start_fake_server().await;
    let dir = tempfile::tempdir().unwrap();
    let enrollment = test_enrollment_record();
    let (auth_file, state_file) = write_test_auth_and_state(&dir, &enrollment);
    let signer_file = dir.path().join("missing-device-key.js");
    write_device_key_module(&signer_file, true);

    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .args([
            "--endpoint",
            &app_server_url,
            "remote",
            "doctor",
            "--skip-backend",
            "--auth-file",
            auth_file.to_str().unwrap(),
            "--global-state-file",
            state_file.to_str().unwrap(),
            "--device-key-module",
            signer_file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("doctor\tbackend-clients\tskipped\t--skip-backend"));
    assert!(stdout.contains("doctor\tlocal-enrollment\tok\tcli-test\tkey-test"));
    assert!(stdout.contains("doctor\tdevice-key\twarn\tcli-test\tkey-test\tunavailable"));
    assert!(stdout.contains("doctor\tdevice-key-next\trepair-local-controller-enrollment"));
    assert!(stdout.contains("device key not found"));
}

#[cfg(not(windows))]
#[tokio::test(flavor = "multi_thread")]
async fn remote_connect_explains_missing_device_key_repair() {
    let app_server_url = start_fake_server().await;
    let dir = tempfile::tempdir().unwrap();
    let enrollment = test_enrollment_record();
    let (auth_file, state_file) = write_test_auth_and_state(&dir, &enrollment);
    let signer_file = dir.path().join("missing-device-key.js");
    write_device_key_module(&signer_file, true);

    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .args([
            "--endpoint",
            &app_server_url,
            "remote",
            "connect",
            "--skip-refresh",
            "--auth-file",
            auth_file.to_str().unwrap(),
            "--global-state-file",
            state_file.to_str().unwrap(),
            "--device-key-module",
            signer_file.to_str().unwrap(),
            "--timeout-ms",
            "100",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("remote connect could not use the local controller enrollment"));
    assert!(stderr.contains("re-authorize remote control in Codex App settings"));
    assert!(stderr.contains("device key not found"));
}

#[cfg(not(windows))]
#[tokio::test(flavor = "multi_thread")]
async fn remote_doctor_reports_backend_failure_without_failing() {
    let app_server_url = start_fake_server().await;
    let backend_url = start_fake_backend_clients_failure();
    let dir = tempfile::tempdir().unwrap();
    let enrollment = test_enrollment_record();
    let (auth_file, state_file) = write_test_auth_and_state(&dir, &enrollment);

    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .args([
            "--endpoint",
            &app_server_url,
            "remote",
            "doctor",
            "--api-base-url",
            &backend_url,
            "--auth-file",
            auth_file.to_str().unwrap(),
            "--global-state-file",
            state_file.to_str().unwrap(),
            "--skip-device-key",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("doctor\tbackend-clients\terror"));
    assert!(stdout.contains("auth expired"));
    assert!(stdout.contains("doctor\tlocal-enrollment\tok\tcli-test\tkey-test"));
    assert!(stdout.contains("doctor\tdevice-key\tskipped\t--skip-device-key"));
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_doctor_reports_app_server_client_list_failure_without_failing() {
    let app_server_url = start_fake_server_with_remote_client_list_error().await;
    let dir = tempfile::tempdir().unwrap();
    let enrollment = test_enrollment_record();
    let (auth_file, state_file) = write_test_auth_and_state(&dir, &enrollment);

    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .args([
            "--endpoint",
            &app_server_url,
            "remote",
            "doctor",
            "--skip-refresh",
            "--skip-backend",
            "--skip-device-key",
            "--auth-file",
            auth_file.to_str().unwrap(),
            "--global-state-file",
            state_file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("doctor\tapp-server-status\tok\tconnected"));
    assert!(stdout.contains(
        "doctor\tapp-server-clients\terror\tremoteControl/client/list: client list unavailable"
    ));
    assert!(stdout.contains("doctor\tauth-refresh\tskipped\t--skip-refresh"));
    assert!(stdout.contains("doctor\tlocal-enrollment\tok\tcli-test\tkey-test"));
}

#[cfg(not(windows))]
#[tokio::test(flavor = "multi_thread")]
async fn remote_connect_completes_device_key_websocket_handshake() {
    let app_server_url = start_fake_server().await;
    let remote_control_token = "remote-token";
    let websocket_url =
        start_fake_remote_control_websocket("cli-test", "key-test", remote_control_token).await;
    let dir = tempfile::tempdir().unwrap();
    let auth_file = dir.path().join("auth.json");
    let state_file = dir.path().join("state.json");
    let signer_file = dir.path().join("fake-device-key.js");
    let enrollment = codex_monitor::remote_control::RemoteControlClientEnrollmentRecord {
        account_user_id: "user-test".to_string(),
        client_id: "cli-test".to_string(),
        key_id: "key-test".to_string(),
        algorithm: "ES256".to_string(),
        protection_class: "allow_os_protected_nonextractable".to_string(),
        public_key_spki_der_base64: "public-key".to_string(),
    };
    let backend_url = start_fake_refresh_backend(
        "cli-test",
        "key-test",
        codex_monitor::remote_control::device_identity_sha256_base64url(&enrollment),
        remote_control_token,
    );
    std::fs::write(
        &auth_file,
        json!({
            "tokens": {
                "access_token": fake_jwt("acct-test", "user-test")
            }
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        &state_file,
        json!({
            "electron-remote-control-client-enrollments": {
                "Codex Desktop\nhttp://localhost:8000/api\nhttps://chatgpt.com/backend-api\nafter-first-unlock-device-key-v1\nuser-test": enrollment
            }
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        &signer_file,
        r#"
exports.getDeviceKeyPublic = async function getDeviceKeyPublic(keyId) {
  return {
    keyId,
    publicKeySpkiDerBase64: "public-key",
    algorithm: "ES256",
    protectionClass: "allow_os_protected_nonextractable"
  };
};
exports.signDeviceKey = async function signDeviceKey(keyId, payload) {
  if (!Buffer.isBuffer(payload)) throw new Error("expected payload buffer");
  return {
    signatureDerBase64: "signature",
    algorithm: "ES256"
  };
};
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .args([
            "--endpoint",
            &app_server_url,
            "remote",
            "connect",
            "--api-base-url",
            &backend_url,
            "--websocket-url",
            &websocket_url,
            "--auth-file",
            auth_file.to_str().unwrap(),
            "--global-state-file",
            state_file.to_str().unwrap(),
            "--device-key-module",
            signer_file.to_str().unwrap(),
            "--timeout-ms",
            "2000",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("connected\tcli-test"));
    assert!(stdout.contains("device-key-proof\tES256"));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn threads_command_lists_fake_thread_over_explicit_unix_endpoint() {
    let url = start_fake_unix_server().await;
    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .args(["--endpoint", &url, "threads", "--cwd", "/tmp/project"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("thread-1"));
    assert!(stdout.contains("/tmp/project"));
}

#[tokio::test(flavor = "multi_thread")]
async fn send_command_waits_for_completion_when_wait_is_set() {
    let url = start_fake_server().await;
    let output = Command::new(env!("CARGO_BIN_EXE_cdxm"))
        .args([
            "--endpoint",
            &url,
            "send",
            "--thread",
            "thread-1",
            "--text",
            "hello",
            "--mode",
            "start",
            "--wait",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
}
