//! Length-prefixed JSON framing codec for Unix socket communication.
//!
//! Wire format: `[u32 big-endian length][JSON bytes]`

use anyhow::{Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub struct Codec;

impl Codec {
    /// Write a single message as a length-prefixed JSON frame.
    pub async fn write_message<W, M>(writer: &mut W, msg: &M) -> Result<()>
    where
        W: AsyncWrite + Unpin,
        M: Serialize,
    {
        let payload = serde_json::to_vec(msg).context("serializing message")?;
        let len = payload.len() as u32;
        writer
            .write_all(&len.to_be_bytes())
            .await
            .context("writing length prefix")?;
        writer
            .write_all(&payload)
            .await
            .context("writing payload")?;
        Ok(())
    }

    /// Read a single length-prefixed JSON frame and deserialize it.
    pub async fn read_message<R, M>(reader: &mut R) -> Result<M>
    where
        R: AsyncRead + Unpin,
        M: DeserializeOwned,
    {
        let mut len_buf = [0u8; 4];
        reader
            .read_exact(&mut len_buf)
            .await
            .context("reading length prefix")?;
        let len = u32::from_be_bytes(len_buf) as usize;

        let mut payload = vec![0u8; len];
        reader
            .read_exact(&mut payload)
            .await
            .context("reading payload")?;

        serde_json::from_slice(&payload).context("deserializing message")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentToSupervisor, SupervisorToAgent};
    use uuid::Uuid;

    #[tokio::test]
    async fn round_trip_agent_ready() {
        let agent_id = Uuid::new_v4();
        let msg = AgentToSupervisor::Ready { agent_id };

        let mut buf = Vec::new();
        Codec::write_message(&mut buf, &msg).await.unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let decoded: AgentToSupervisor = Codec::read_message(&mut cursor).await.unwrap();

        match decoded {
            AgentToSupervisor::Ready { agent_id: id } => assert_eq!(id, agent_id),
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[tokio::test]
    async fn round_trip_supervisor_shutdown() {
        let msg = SupervisorToAgent::Shutdown;

        let mut buf = Vec::new();
        Codec::write_message(&mut buf, &msg).await.unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let decoded: SupervisorToAgent = Codec::read_message(&mut cursor).await.unwrap();

        assert!(matches!(decoded, SupervisorToAgent::Shutdown));
    }

    #[tokio::test]
    async fn multiple_messages_in_stream() {
        let mut buf = Vec::new();
        for i in 0..5u32 {
            let msg = SupervisorToAgent::Init {
                agent_id: Uuid::new_v4(),
            };
            Codec::write_message(&mut buf, &msg).await.unwrap();
            let _ = i;
        }

        let mut cursor = std::io::Cursor::new(buf);
        for _ in 0..5 {
            let decoded: SupervisorToAgent = Codec::read_message(&mut cursor).await.unwrap();
            assert!(matches!(decoded, SupervisorToAgent::Init { .. }));
        }
    }

    #[tokio::test]
    async fn large_payload() {
        // Ensure we handle payloads larger than typical buffer sizes.
        let large_text = "x".repeat(1_000_000);
        let msg = crate::agent::AgentToSupervisor::Response {
            request_id: Uuid::new_v4(),
            text: large_text.clone(),
        };

        let mut buf = Vec::new();
        Codec::write_message(&mut buf, &msg).await.unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let decoded: crate::agent::AgentToSupervisor = Codec::read_message(&mut cursor).await.unwrap();

        match decoded {
            crate::agent::AgentToSupervisor::Response { text, .. } => {
                assert_eq!(text.len(), 1_000_000)
            }
            other => panic!("unexpected: {:?}", other),
        }
    }
}
