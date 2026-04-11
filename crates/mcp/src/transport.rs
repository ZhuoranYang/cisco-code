//! MCP transport implementations.
//!
//! Two transport modes:
//! - Stdio: spawn child process, JSON-RPC over stdin/stdout (one JSON per line)
//! - HTTP: POST JSON-RPC to endpoint, SSE for server-initiated messages

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::jsonrpc::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use crate::types::McpTransport;

/// Trait for sending/receiving JSON-RPC messages.
#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    /// Send a request and wait for the corresponding response.
    async fn request(&self, req: JsonRpcRequest) -> Result<JsonRpcResponse>;

    /// Send a notification (no response expected).
    async fn notify(&self, notif: JsonRpcNotification) -> Result<()>;

    /// Shut down the transport.
    async fn close(&self) -> Result<()>;
}

/// Stdio transport — communicates with a child process via stdin/stdout.
pub struct StdioTransport {
    stdin_tx: mpsc::Sender<String>,
    response_rx: tokio::sync::Mutex<mpsc::Receiver<JsonRpcResponse>>,
    child: tokio::sync::Mutex<Option<tokio::process::Child>>,
}

impl StdioTransport {
    /// Spawn a child process and set up the transport.
    pub async fn spawn(
        command: &str,
        args: &[String],
        env: &std::collections::HashMap<String, String>,
    ) -> Result<Self> {
        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        for (k, v) in env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn()?;

        let child_stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("no stdin"))?;
        let child_stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("no stdout"))?;
        let child_stderr = child.stderr.take().ok_or_else(|| anyhow::anyhow!("no stderr"))?;

        // Channel for outgoing messages to stdin
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(64);
        // Channel for incoming responses from stdout
        let (response_tx, response_rx) = mpsc::channel::<JsonRpcResponse>(64);

        // Writer task: drain stdin_tx → child stdin
        tokio::spawn(async move {
            let mut writer = child_stdin;
            while let Some(line) = stdin_rx.recv().await {
                if writer.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if writer.write_all(b"\n").await.is_err() {
                    break;
                }
                if writer.flush().await.is_err() {
                    break;
                }
            }
        });

        // Reader task: child stdout → response_tx
        tokio::spawn(async move {
            let reader = BufReader::new(child_stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                match serde_json::from_str::<JsonRpcResponse>(&line) {
                    Ok(resp) => {
                        if response_tx.send(resp).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::debug!("Non-response line from MCP server: {e}: {line}");
                    }
                }
            }
        });

        // Stderr logger task
        tokio::spawn(async move {
            let reader = BufReader::new(child_stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!("MCP server stderr: {line}");
            }
        });

        Ok(Self {
            stdin_tx,
            response_rx: tokio::sync::Mutex::new(response_rx),
            child: tokio::sync::Mutex::new(Some(child)),
        })
    }
}

#[async_trait::async_trait]
impl Transport for StdioTransport {
    async fn request(&self, req: JsonRpcRequest) -> Result<JsonRpcResponse> {
        let json = serde_json::to_string(&req)?;
        self.stdin_tx
            .send(json)
            .await
            .map_err(|_| anyhow::anyhow!("MCP server stdin closed"))?;

        // Wait for response with timeout
        let mut rx = self.response_rx.lock().await;
        match tokio::time::timeout(std::time::Duration::from_secs(30), rx.recv()).await {
            Ok(Some(resp)) => Ok(resp),
            Ok(None) => anyhow::bail!("MCP server stdout closed"),
            Err(_) => anyhow::bail!("MCP server response timeout (30s)"),
        }
    }

    async fn notify(&self, notif: JsonRpcNotification) -> Result<()> {
        let json = serde_json::to_string(&notif)?;
        self.stdin_tx
            .send(json)
            .await
            .map_err(|_| anyhow::anyhow!("MCP server stdin closed"))?;
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        if let Some(mut child) = self.child.lock().await.take() {
            let _ = child.kill().await;
        }
        Ok(())
    }
}

/// HTTP transport — communicates via HTTP POST (streamable HTTP MCP).
pub struct HttpTransport {
    url: String,
    headers: std::collections::HashMap<String, String>,
    http: reqwest::Client,
}

impl HttpTransport {
    pub fn new(url: impl Into<String>, headers: std::collections::HashMap<String, String>) -> Self {
        Self {
            url: url.into(),
            headers,
            http: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl Transport for HttpTransport {
    async fn request(&self, req: JsonRpcRequest) -> Result<JsonRpcResponse> {
        let mut builder = self
            .http
            .post(&self.url)
            .header("content-type", "application/json");

        for (k, v) in &self.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }

        let resp = builder.json(&req).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("MCP HTTP error {status}: {text}");
        }

        let body = resp.text().await?;
        let rpc_resp: JsonRpcResponse = serde_json::from_str(&body)?;
        Ok(rpc_resp)
    }

    async fn notify(&self, notif: JsonRpcNotification) -> Result<()> {
        let mut builder = self
            .http
            .post(&self.url)
            .header("content-type", "application/json");

        for (k, v) in &self.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }

        let resp = builder.json(&notif).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            tracing::warn!("MCP notification failed: {status}");
        }
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        Ok(())
    }
}

/// Create a transport from an MCP server config.
pub async fn create_transport(
    transport_config: &McpTransport,
    env: &std::collections::HashMap<String, String>,
) -> Result<Box<dyn Transport>> {
    match transport_config {
        McpTransport::Stdio { command, args } => {
            let t = StdioTransport::spawn(command, args, env).await?;
            Ok(Box::new(t))
        }
        McpTransport::Http { url, headers } => {
            let t = HttpTransport::new(url.clone(), headers.clone());
            Ok(Box::new(t))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_transport_creation() {
        let t = HttpTransport::new("https://example.com/mcp", std::collections::HashMap::new());
        assert_eq!(t.url, "https://example.com/mcp");
    }

    #[tokio::test]
    async fn test_stdio_spawn_invalid_command() {
        let result = StdioTransport::spawn(
            "nonexistent-command-xyz",
            &[],
            &std::collections::HashMap::new(),
        )
        .await;
        assert!(result.is_err());
    }
}
