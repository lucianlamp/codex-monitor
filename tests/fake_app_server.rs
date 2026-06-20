use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::process::Command;
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_async, tungstenite::protocol::Message};

async fn start_fake_server() -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        while let Some(message) = ws.next().await {
            let Message::Text(text) = message.unwrap() else {
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
                                "threads": [
                                    { "id": "thread-1", "title": "One", "cwd": "/tmp/project" }
                                ]
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
    });
    format!("ws://{}", addr)
}

#[tokio::test(flavor = "multi_thread")]
async fn threads_command_lists_fake_thread() {
    let url = start_fake_server().await;
    let output = Command::new(env!("CARGO_BIN_EXE_ccb"))
        .args(["--endpoint", &url, "threads", "--cwd", "/tmp/project"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("thread-1"));
    assert!(stdout.contains("/tmp/project"));
}

#[tokio::test(flavor = "multi_thread")]
async fn send_command_waits_for_completion() {
    let url = start_fake_server().await;
    let output = Command::new(env!("CARGO_BIN_EXE_ccb"))
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
    assert!(output.status.success());
}
