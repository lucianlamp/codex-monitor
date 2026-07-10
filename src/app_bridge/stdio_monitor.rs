use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use anyhow::Context;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, watch, Mutex},
    task::JoinSet,
};
use tokio_tungstenite::{accept_async, tungstenite::Message};

use super::monitor_router::{ChildOutput, MonitorInput, MonitorRouter};

const APP_WRITE_CAPACITY: usize = 32;
const MONITOR_WRITE_CAPACITY: usize = 64;
const CLIENT_QUEUE_CAPACITY: usize = 32;

enum ChildWrite {
    Raw(String),
    Json(Value),
}

type ClientSender = mpsc::Sender<Message>;
type Clients = Arc<Mutex<HashMap<u64, ClientSender>>>;
type SharedRouter = Arc<Mutex<MonitorRouter>>;

pub(super) async fn proxy_stdio_monitor_io<AR, AW, CR, CW>(
    app_input: AR,
    app_output: AW,
    child_output: CR,
    child_input: CW,
    listener: TcpListener,
    nonce: String,
    ready_tx: watch::Sender<bool>,
) -> anyhow::Result<()>
where
    AR: AsyncBufRead + Unpin + Send + 'static,
    AW: AsyncWrite + Unpin + Send + 'static,
    CR: AsyncBufRead + Unpin + Send + 'static,
    CW: AsyncWrite + Unpin + Send + 'static,
{
    let router = Arc::new(Mutex::new(MonitorRouter::new(nonce)));
    let clients: Clients = Arc::new(Mutex::new(HashMap::new()));
    let (app_write_tx, app_write_rx) = mpsc::channel(APP_WRITE_CAPACITY);
    let (monitor_write_tx, monitor_write_rx) = mpsc::channel(MONITOR_WRITE_CAPACITY);

    let mut child_writer = tokio::spawn(write_child(child_input, app_write_rx, monitor_write_rx));
    let mut app_pump = tokio::spawn(pump_app_input(
        app_input,
        app_write_tx,
        Arc::clone(&router),
        ready_tx,
    ));
    let mut child_pump = tokio::spawn(pump_child_output(
        child_output,
        app_output,
        Arc::clone(&router),
        Arc::clone(&clients),
    ));
    let mut listener_task = tokio::spawn(accept_monitors(
        listener,
        Arc::clone(&router),
        Arc::clone(&clients),
        monitor_write_tx,
    ));

    let (finished, result) = tokio::select! {
        result = &mut app_pump => (FinishedTask::AppInput, join_result(result, "App input pump")),
        result = &mut child_pump => (FinishedTask::ChildOutput, join_result(result, "child output pump")),
        result = &mut child_writer => (FinishedTask::ChildInput, join_result(result, "child input writer")),
        result = &mut listener_task => (FinishedTask::MonitorListener, join_result(result, "monitor listener")),
    };

    if finished != FinishedTask::AppInput {
        app_pump.abort();
        let _ = app_pump.await;
    }
    if finished != FinishedTask::ChildOutput {
        child_pump.abort();
        let _ = child_pump.await;
    }
    if finished != FinishedTask::ChildInput {
        child_writer.abort();
        let _ = child_writer.await;
    }
    if finished != FinishedTask::MonitorListener {
        listener_task.abort();
        let _ = listener_task.await;
    }
    result
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum FinishedTask {
    AppInput,
    ChildOutput,
    ChildInput,
    MonitorListener,
}

fn join_result(
    result: Result<anyhow::Result<()>, tokio::task::JoinError>,
    task: &str,
) -> anyhow::Result<()> {
    result
        .with_context(|| format!("{task} task failed"))?
        .with_context(|| format!("{task} failed"))
}

async fn pump_app_input<R>(
    mut input: R,
    child_tx: mpsc::Sender<ChildWrite>,
    router: SharedRouter,
    ready_tx: watch::Sender<bool>,
) -> anyhow::Result<()>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        child_tx
            .send(ChildWrite::Raw(line.clone()))
            .await
            .context("child input closed while forwarding App input")?;
        if let Ok(message) = serde_json::from_str::<Value>(line.trim()) {
            if router.lock().await.observe_app(&message) {
                ready_tx.send_replace(true);
            }
        }
    }
}

async fn write_child<W>(
    mut child_input: W,
    mut app_rx: mpsc::Receiver<ChildWrite>,
    mut monitor_rx: mpsc::Receiver<ChildWrite>,
) -> anyhow::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mut app_open = true;
    let mut monitor_open = true;
    while app_open || monitor_open {
        let write = tokio::select! {
            biased;
            write = app_rx.recv(), if app_open => {
                match write {
                    Some(write) => Some(write),
                    None => { app_open = false; None }
                }
            }
            write = monitor_rx.recv(), if monitor_open => {
                match write {
                    Some(write) => Some(write),
                    None => { monitor_open = false; None }
                }
            }
        };
        let Some(write) = write else {
            continue;
        };
        match write {
            ChildWrite::Raw(line) => child_input.write_all(line.as_bytes()).await?,
            ChildWrite::Json(message) => {
                child_input
                    .write_all(message.to_string().as_bytes())
                    .await?;
                child_input.write_all(b"\n").await?;
            }
        }
        child_input.flush().await?;
    }
    child_input.shutdown().await?;
    Ok(())
}

async fn pump_child_output<R, W>(
    mut child_output: R,
    mut app_output: W,
    router: SharedRouter,
    clients: Clients,
) -> anyhow::Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut line = String::new();
    loop {
        line.clear();
        if child_output.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        let Ok(message) = serde_json::from_str::<Value>(line.trim()) else {
            write_app_line(&mut app_output, &line).await?;
            continue;
        };
        let route = router.lock().await.route_child(&message);
        match route {
            ChildOutput::AppOnly => write_app_line(&mut app_output, &line).await?,
            ChildOutput::AppAndBroadcast(message) => {
                write_app_line(&mut app_output, &line).await?;
                let retired =
                    broadcast_to_clients(&clients, Message::Text(message.to_string().into())).await;
                retire_connections(&router, retired).await;
            }
            ChildOutput::Monitor {
                connection_id,
                message,
            } => {
                if !send_to_client(
                    &clients,
                    connection_id,
                    Message::Text(message.to_string().into()),
                )
                .await
                {
                    router.lock().await.retire_connection(connection_id);
                }
            }
            ChildOutput::Drop => {}
        }
    }
}

async fn write_app_line<W>(output: &mut W, line: &str) -> anyhow::Result<()>
where
    W: AsyncWrite + Unpin,
{
    output.write_all(line.as_bytes()).await?;
    output.flush().await?;
    Ok(())
}

async fn accept_monitors(
    listener: TcpListener,
    router: SharedRouter,
    clients: Clients,
    child_tx: mpsc::Sender<ChildWrite>,
) -> anyhow::Result<()> {
    let next_connection = Arc::new(AtomicU64::new(1));
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, address) = accepted?;
                if !address.ip().is_loopback() {
                    continue;
                }
                let connection_id = next_connection.fetch_add(1, Ordering::Relaxed);
                connections.spawn(handle_monitor(
                    stream,
                    connection_id,
                    Arc::clone(&router),
                    Arc::clone(&clients),
                    child_tx.clone(),
                ));
            }
            completed = connections.join_next(), if !connections.is_empty() => {
                if let Some(Err(error)) = completed {
                    eprintln!("codex-monitor app bridge: monitor connection task failed: {error}");
                }
            }
        }
    }
}

async fn handle_monitor(
    stream: TcpStream,
    connection_id: u64,
    router: SharedRouter,
    clients: Clients,
    child_tx: mpsc::Sender<ChildWrite>,
) -> anyhow::Result<()> {
    let websocket = accept_async(stream)
        .await
        .context("monitor WebSocket handshake failed")?;
    let (mut sink, mut source) = websocket.split();
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel(CLIENT_QUEUE_CAPACITY);
    clients.lock().await.insert(connection_id, outgoing_tx);

    let reader = async {
        while let Some(message) = source.next().await.transpose()? {
            match message {
                Message::Text(text) => {
                    let parsed =
                        serde_json::from_str::<Value>(text.as_str()).unwrap_or(Value::Null);
                    match router.lock().await.handle_monitor(connection_id, parsed) {
                        MonitorInput::Reply(reply) => {
                            if !send_to_client(
                                &clients,
                                connection_id,
                                Message::Text(reply.to_string().into()),
                            )
                            .await
                            {
                                break;
                            }
                        }
                        MonitorInput::Forward(forwarded) => {
                            if child_tx
                                .try_send(ChildWrite::Json(forwarded.clone()))
                                .is_err()
                            {
                                router.lock().await.cancel_forward(&forwarded);
                                let error = json!({
                                    "id": Value::Null,
                                    "error": {"code": -32003, "message": "monitor request queue is full"}
                                });
                                if !send_to_client(
                                    &clients,
                                    connection_id,
                                    Message::Text(error.to_string().into()),
                                )
                                .await
                                {
                                    break;
                                }
                            }
                        }
                        MonitorInput::Ignore => {}
                    }
                }
                Message::Ping(payload) => {
                    if !send_to_client(&clients, connection_id, Message::Pong(payload)).await {
                        break;
                    }
                }
                Message::Close(_) => break,
                Message::Binary(_) => {
                    let error = json!({
                        "id": Value::Null,
                        "error": {"code": -32600, "message": "monitor messages must be JSON text"}
                    });
                    if !send_to_client(
                        &clients,
                        connection_id,
                        Message::Text(error.to_string().into()),
                    )
                    .await
                    {
                        break;
                    }
                }
                Message::Pong(_) | Message::Frame(_) => {}
            }
        }
        Ok::<(), tokio_tungstenite::tungstenite::Error>(())
    };

    let writer = async {
        while let Some(message) = outgoing_rx.recv().await {
            sink.send(message).await?;
        }
        Ok::<(), tokio_tungstenite::tungstenite::Error>(())
    };

    tokio::select! {
        result = reader => result?,
        result = writer => result?,
    }
    clients.lock().await.remove(&connection_id);
    router.lock().await.retire_connection(connection_id);
    Ok(())
}

async fn send_to_client(clients: &Clients, connection_id: u64, message: Message) -> bool {
    let mut clients = clients.lock().await;
    let Some(sender) = clients.get(&connection_id) else {
        return false;
    };
    if sender.try_send(message).is_ok() {
        true
    } else {
        clients.remove(&connection_id);
        false
    }
}

async fn broadcast_to_clients(clients: &Clients, message: Message) -> Vec<u64> {
    let mut retired = Vec::new();
    clients.lock().await.retain(|connection_id, sender| {
        if sender.try_send(message.clone()).is_ok() {
            true
        } else {
            retired.push(*connection_id);
            false
        }
    });
    retired
}

async fn retire_connections(router: &SharedRouter, connection_ids: Vec<u64>) {
    let mut router = router.lock().await;
    for connection_id in connection_ids {
        router.retire_connection(connection_id);
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use futures_util::{SinkExt, StreamExt};
    use serde_json::{json, Value};
    use tokio::{
        io::{duplex, AsyncBufReadExt, AsyncWriteExt, BufReader},
        net::TcpListener,
        sync::{mpsc, watch, Mutex},
    };
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    use super::*;

    #[tokio::test]
    async fn app_stdio_is_unchanged_and_monitor_requests_share_the_initialized_child() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (test_app, bridge_app) = duplex(64 * 1024);
        let (test_child, bridge_child) = duplex(64 * 1024);
        let (test_app_read, mut test_app_write) = tokio::io::split(test_app);
        let (bridge_app_read, bridge_app_write) = tokio::io::split(bridge_app);
        let (test_child_read, mut test_child_write) = tokio::io::split(test_child);
        let (bridge_child_read, bridge_child_write) = tokio::io::split(bridge_child);
        let (ready_tx, mut ready_rx) = watch::channel(false);

        let proxy = tokio::spawn(proxy_stdio_monitor_io(
            BufReader::new(bridge_app_read),
            bridge_app_write,
            BufReader::new(bridge_child_read),
            bridge_child_write,
            listener,
            "test-nonce".to_string(),
            ready_tx,
        ));

        let mut app_output = BufReader::new(test_app_read);
        let mut child_input = BufReader::new(test_child_read);
        let app_initialize = "{\"id\":10,\"method\":\"initialize\",\"params\":{}}\n";
        test_app_write
            .write_all(app_initialize.as_bytes())
            .await
            .unwrap();
        let mut line = String::new();
        child_input.read_line(&mut line).await.unwrap();
        assert_eq!(line, app_initialize);

        let child_initialize = "{\"id\":10,\"result\":{\"serverInfo\":{\"name\":\"codex\"}}}\n";
        test_child_write
            .write_all(child_initialize.as_bytes())
            .await
            .unwrap();
        line.clear();
        app_output.read_line(&mut line).await.unwrap();
        assert_eq!(line, child_initialize);

        let app_initialized = "{\"method\":\"initialized\",\"params\":{}}\n";
        test_app_write
            .write_all(app_initialized.as_bytes())
            .await
            .unwrap();
        line.clear();
        child_input.read_line(&mut line).await.unwrap();
        assert_eq!(line, app_initialized);
        ready_rx.changed().await.unwrap();
        assert!(*ready_rx.borrow());

        let (mut monitor, _) = connect_async(format!("ws://{address}")).await.unwrap();
        monitor
            .send(Message::Text(
                json!({"id":1,"method":"initialize","params":{}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let init_reply: Value =
            serde_json::from_str(monitor.next().await.unwrap().unwrap().to_text().unwrap())
                .unwrap();
        assert_eq!(init_reply["id"], 1);

        monitor
            .send(Message::Text(
                json!({"id":2,"method":"thread/loaded/list","params":{"limit":100}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        line.clear();
        child_input.read_line(&mut line).await.unwrap();
        let forwarded: Value = serde_json::from_str(line.trim()).unwrap();
        assert!(forwarded["id"]
            .as_str()
            .unwrap()
            .starts_with("cdxm:test-nonce:"));

        test_child_write
            .write_all(
                format!(
                    "{}\n",
                    json!({"id":forwarded["id"].clone(),"result":{"data":["thread-1"]}})
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let monitor_reply: Value =
            serde_json::from_str(monitor.next().await.unwrap().unwrap().to_text().unwrap())
                .unwrap();
        assert_eq!(
            monitor_reply,
            json!({"id":2,"result":{"data":["thread-1"]}})
        );

        let mut unexpected = String::new();
        assert!(tokio::time::timeout(
            Duration::from_millis(100),
            app_output.read_line(&mut unexpected)
        )
        .await
        .is_err());

        test_app_write.shutdown().await.unwrap();
        proxy.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn full_client_queue_is_removed_without_blocking_healthy_clients() {
        let clients: Clients = Arc::new(Mutex::new(Default::default()));
        let (slow_tx, mut slow_rx) = mpsc::channel(1);
        slow_tx
            .send(Message::Text("occupied".into()))
            .await
            .unwrap();
        let (healthy_tx, mut healthy_rx) = mpsc::channel(2);
        clients.lock().await.insert(1, slow_tx);
        clients.lock().await.insert(2, healthy_tx);

        let retired = broadcast_to_clients(&clients, Message::Text("turn".into())).await;

        assert_eq!(retired, vec![1]);
        assert!(!clients.lock().await.contains_key(&1));
        assert!(clients.lock().await.contains_key(&2));
        assert_eq!(
            healthy_rx.recv().await.unwrap().into_text().unwrap(),
            "turn"
        );
        assert_eq!(
            slow_rx.recv().await.unwrap().into_text().unwrap(),
            "occupied"
        );
    }
}
