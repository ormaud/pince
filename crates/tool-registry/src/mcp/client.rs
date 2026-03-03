//! MCP stdio client — manages one child process per MCP server.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, oneshot};

use super::{JsonRpcRequest, JsonRpcResponse, McpTool};

type PendingMap = HashMap<u64, oneshot::Sender<JsonRpcResponse>>;

/// A client connected to a single MCP server over stdio.
///
/// Spawns the server as a child process, handles JSON-RPC 2.0 framing
/// (newline-delimited), and dispatches responses to waiting callers.
pub struct McpClient {
    stdin: Arc<Mutex<ChildStdin>>,
    next_id: AtomicU64,
    pending: Arc<Mutex<PendingMap>>,
    // Keep the child alive as long as the client lives.
    _child: Child,
}

impl McpClient {
    /// Spawn an MCP server and perform the `initialize` handshake.
    pub async fn spawn(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<Self> {
        let mut child = Command::new(command)
            .args(args)
            .envs(env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .with_context(|| format!("spawn MCP server '{command}'"))?;

        let stdin = child.stdin.take().context("MCP child stdin")?;
        let stdout = child.stdout.take().context("MCP child stdout")?;

        let stdin = Arc::new(Mutex::new(stdin));
        let pending: Arc<Mutex<PendingMap>> = Arc::new(Mutex::new(HashMap::new()));

        // Background reader task: read newline-delimited JSON responses and
        // dispatch them to the matching oneshot channel.
        {
            let pending2 = pending.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stdout);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) => {
                            tracing::debug!("MCP server stdout closed");
                            break;
                        }
                        Ok(_) => {
                            let trimmed = line.trim();
                            if trimmed.is_empty() {
                                continue;
                            }
                            match serde_json::from_str::<JsonRpcResponse>(trimmed) {
                                Ok(resp) => {
                                    if let Some(id) = resp.id.as_u64() {
                                        let mut map = pending2.lock().await;
                                        if let Some(tx) = map.remove(&id) {
                                            let _ = tx.send(resp);
                                        }
                                    }
                                    // Ignore notifications (no id or non-numeric id).
                                }
                                Err(e) => {
                                    tracing::warn!("MCP: bad JSON response: {e}: {trimmed}");
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!("MCP reader error: {e}");
                            break;
                        }
                    }
                }
            });
        }

        let client = Self {
            stdin,
            next_id: AtomicU64::new(1),
            pending,
            _child: child,
        };

        // Perform the MCP initialize handshake.
        client.initialize().await.context("MCP initialize")?;

        Ok(client)
    }

    /// Send `initialize` and wait for the response.
    async fn initialize(&self) -> Result<()> {
        let _resp = self
            .call(
                "initialize",
                Some(json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "pince-supervisor",
                        "version": "0.1.0"
                    }
                })),
            )
            .await?;

        // Send initialized notification (no response expected).
        self.notify("notifications/initialized", None).await?;

        Ok(())
    }

    /// List all tools exposed by this MCP server.
    pub async fn list_tools(&self) -> Result<Vec<McpTool>> {
        let resp = self.call("tools/list", None).await?;
        let tools_json = resp
            .get("tools")
            .context("MCP tools/list: missing 'tools' field")?;
        let tools: Vec<McpTool> = serde_json::from_value(tools_json.clone())
            .context("MCP tools/list: deserialize tools")?;
        Ok(tools)
    }

    /// Call a tool on this MCP server.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value> {
        let resp = self
            .call(
                "tools/call",
                Some(json!({
                    "name": name,
                    "arguments": arguments
                })),
            )
            .await?;
        Ok(resp)
    }

    // ── Internal RPC helpers ─────────────────────────────────────────────────

    /// Send a JSON-RPC request and wait for the response result.
    async fn call(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = JsonRpcRequest::new(id, method, params);

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        self.send_raw(&req).await?;

        let resp = rx.await.context("MCP: response channel closed")?;

        if let Some(err) = resp.error {
            bail!("MCP error {}: {}", err.code, err.message);
        }

        Ok(resp.result.unwrap_or(Value::Null))
    }

    /// Send a JSON-RPC notification (no response expected).
    async fn notify(&self, method: &str, params: Option<Value>) -> Result<()> {
        #[derive(serde::Serialize)]
        struct Notification {
            jsonrpc: &'static str,
            method: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            params: Option<Value>,
        }
        let n = Notification {
            jsonrpc: "2.0",
            method: method.into(),
            params,
        };
        let mut json = serde_json::to_string(&n)?;
        json.push('\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(json.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Serialize a request to JSON and write it as a newline-delimited frame.
    async fn send_raw(&self, req: &JsonRpcRequest) -> Result<()> {
        let mut json = serde_json::to_string(req)?;
        json.push('\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(json.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }
}
