//! Host-side vsock communication with the guest agent.
//!
//! Firecracker implements vsock via a Unix socket proxy:
//!   - The configured `uds_path` (e.g. `/tmp/agent.vsock`) is managed by Firecracker.
//!   - To connect from host to guest port P, the host connects to `{uds_path}_{P}`.
//!   - Firecracker proxies the connection to the guest's vsock listener on port P.
//!
//! Wire format: 4-byte LE length prefix + JSON payload.

use std::path::Path;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::error::SandboxError;
use crate::protocol::{ShutdownRequest, ToolRequest, ToolResponse, MAX_PAYLOAD_BYTES};

/// An active vsock connection to a guest agent.
pub struct VsockConnection {
    stream: UnixStream,
}

impl VsockConnection {
    /// Connect to the guest agent via Firecracker's vsock proxy.
    ///
    /// The socket path is `{uds_path}_{port}`, created by Firecracker once the
    /// guest is listening on `port`.
    pub async fn connect(uds_path: &Path, port: u32) -> Result<Self, SandboxError> {
        let socket_path = format!("{}_{port}", uds_path.display());
        let stream = UnixStream::connect(&socket_path).await.map_err(|e| {
            SandboxError::Communication(format!("connect to vsock proxy {socket_path}: {e}"))
        })?;
        Ok(Self { stream })
    }

    /// Wait for the guest to become ready, retrying until `timeout` expires.
    pub async fn connect_with_retry(
        uds_path: &Path,
        port: u32,
        timeout: Duration,
    ) -> Result<Self, SandboxError> {
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(SandboxError::Timeout);
            }

            match Self::connect(uds_path, port).await {
                Ok(conn) => return Ok(conn),
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    /// Execute a tool call: send the request and receive the response.
    pub async fn execute(
        &mut self,
        request: &ToolRequest,
    ) -> Result<ToolResponse, SandboxError> {
        self.send_frame(request).await?;
        self.recv_frame().await
    }

    /// Send a graceful shutdown command to the guest.
    pub async fn send_shutdown(&mut self) -> Result<(), SandboxError> {
        self.send_frame(&ShutdownRequest { shutdown: true }).await
    }

    async fn send_frame<T: serde::Serialize>(&mut self, payload: &T) -> Result<(), SandboxError> {
        let json = serde_json::to_vec(payload).map_err(|e| {
            SandboxError::Communication(format!("serialize frame: {e}"))
        })?;

        let len = json.len() as u32;
        self.stream.write_all(&len.to_le_bytes()).await.map_err(|e| {
            SandboxError::Communication(format!("write length prefix: {e}"))
        })?;
        self.stream.write_all(&json).await.map_err(|e| {
            SandboxError::Communication(format!("write payload: {e}"))
        })?;
        self.stream.flush().await.map_err(|e| {
            SandboxError::Communication(format!("flush: {e}"))
        })?;
        Ok(())
    }

    async fn recv_frame<T: serde::de::DeserializeOwned>(&mut self) -> Result<T, SandboxError> {
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await.map_err(|e| {
            SandboxError::Communication(format!("read length prefix: {e}"))
        })?;

        let len = u32::from_le_bytes(len_buf) as usize;
        if len > MAX_PAYLOAD_BYTES {
            return Err(SandboxError::Communication(format!(
                "payload too large: {len} bytes (max {MAX_PAYLOAD_BYTES})"
            )));
        }

        let mut buf = vec![0u8; len];
        self.stream.read_exact(&mut buf).await.map_err(|e| {
            SandboxError::Communication(format!("read payload: {e}"))
        })?;

        serde_json::from_slice(&buf).map_err(|e| {
            SandboxError::Communication(format!("deserialize frame: {e}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::UnixListener;

    // Simulate a guest agent that handles one request then closes.
    async fn mock_guest_server(listener: UnixListener) {
        let (mut stream, _) = listener.accept().await.unwrap();

        // Read one frame
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await.unwrap();
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf).await.unwrap();

        // Decode and respond
        let req: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        let resp = if req.get("shutdown").is_some() {
            serde_json::json!({"ok": true, "result": null})
        } else {
            serde_json::json!({"ok": true, "result": {"echo": req["tool"]}})
        };
        let resp_bytes = serde_json::to_vec(&resp).unwrap();
        let resp_len = resp_bytes.len() as u32;
        stream.write_all(&resp_len.to_le_bytes()).await.unwrap();
        stream.write_all(&resp_bytes).await.unwrap();
        stream.flush().await.unwrap();
    }

    #[tokio::test]
    async fn framing_roundtrip_via_uds() {
        let dir = tempfile::tempdir().unwrap();
        // Simulate the Firecracker vsock proxy naming: {uds_path}_{port}
        let uds_path = dir.path().join("agent.vsock");
        let port: u32 = 52000;
        let socket_path = format!("{}_{port}", uds_path.display());

        let listener = UnixListener::bind(&socket_path).unwrap();

        tokio::spawn(mock_guest_server(listener));

        // Give the server a moment to be ready
        tokio::time::sleep(Duration::from_millis(10)).await;

        let mut conn = VsockConnection::connect(&uds_path, port).await.unwrap();
        let req = ToolRequest {
            tool: "read_file".to_string(),
            args: serde_json::json!({"path": "foo.txt"}),
        };
        let resp = conn.execute(&req).await.unwrap();
        assert!(resp.ok);
        assert_eq!(resp.result.unwrap()["echo"], "read_file");
    }

    #[tokio::test]
    async fn connect_with_retry_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let uds_path = dir.path().join("no_server.vsock");

        let result = VsockConnection::connect_with_retry(
            &uds_path,
            52000,
            Duration::from_millis(200),
        )
        .await;
        assert!(matches!(result, Err(SandboxError::Timeout)));
    }

    #[tokio::test]
    async fn rejects_oversized_payload() {
        let dir = tempfile::tempdir().unwrap();
        let uds_path = dir.path().join("big.vsock");
        let port: u32 = 52001;
        let socket_path = format!("{}_{port}", uds_path.display());

        let listener = UnixListener::bind(&socket_path).unwrap();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            // Send a frame claiming to be 128 MiB (way over limit)
            let fake_len: u32 = 128 * 1024 * 1024;
            stream.write_all(&fake_len.to_le_bytes()).await.unwrap();
            stream.flush().await.unwrap();
            // Keep the stream open so the client can read
            tokio::time::sleep(Duration::from_secs(1)).await;
        });

        tokio::time::sleep(Duration::from_millis(10)).await;

        let mut conn = VsockConnection::connect(&uds_path, port).await.unwrap();
        let req = ToolRequest {
            tool: "dummy".to_string(),
            args: serde_json::Value::Null,
        };
        // We can't use execute() here because it sends first, then reads.
        // Instead, manually trigger a read by calling recv_frame directly through execute.
        // The server sends an oversized frame header as the "response".
        // First we need to send the request so the server (fake) doesn't block.
        // Actually our fake server sends the large header immediately after accept.
        // The client will send its request then try to read the response → gets the large header.
        let result = conn.execute(&req).await;
        assert!(matches!(result, Err(SandboxError::Communication(_))));
    }
}
