use async_trait::async_trait;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use super::AppServerTransport;

pub struct StdioTransport {
    child: Child,
    stdin: ChildStdin,
    stdout: tokio::io::Lines<BufReader<ChildStdout>>,
}

impl StdioTransport {
    pub async fn spawn() -> anyhow::Result<Self> {
        let mut child = Command::new("codex")
            .arg("app-server")
            .arg("--listen")
            .arg("stdio://")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("missing app-server stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("missing app-server stdout"))?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout).lines(),
        })
    }
}

#[async_trait]
impl AppServerTransport for StdioTransport {
    async fn send(&mut self, message: Value) -> anyhow::Result<()> {
        self.stdin.write_all(message.to_string().as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn recv(&mut self) -> anyhow::Result<Option<Value>> {
        match self.stdout.next_line().await? {
            Some(line) => Ok(Some(serde_json::from_str(&line)?)),
            None => Ok(None),
        }
    }

    async fn close(&mut self) -> anyhow::Result<()> {
        let _ = self.stdin.shutdown().await;
        let _ = self.child.kill().await;
        Ok(())
    }
}
