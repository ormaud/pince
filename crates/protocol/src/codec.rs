/// Framing layer for the pince protocol.
///
/// Frame format: `[4 bytes: u32 BE length][N bytes: protobuf-encoded message]`
/// Max message size: 16 MiB.
use bytes::{BufMut, Bytes, BytesMut};
use prost::Message;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Maximum allowed message size (16 MiB).
pub const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("message too large: {size} bytes (max {MAX_MESSAGE_SIZE})")]
    MessageTooLarge { size: usize },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("decode error: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("encode error: {0}")]
    Encode(#[from] prost::EncodeError),
}

/// Read a length-prefixed protobuf message from an async reader.
pub async fn read_message<T, R>(reader: &mut R) -> Result<T, CodecError>
where
    T: Message + Default,
    R: AsyncRead + Unpin,
{
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;

    if len > MAX_MESSAGE_SIZE {
        return Err(CodecError::MessageTooLarge { size: len });
    }

    let mut buf = BytesMut::with_capacity(len);
    buf.resize(len, 0);
    reader.read_exact(&mut buf).await?;

    let msg = T::decode(buf.freeze())?;
    Ok(msg)
}

/// Write a length-prefixed protobuf message to an async writer.
pub async fn write_message<T, W>(writer: &mut W, msg: &T) -> Result<(), CodecError>
where
    T: Message,
    W: AsyncWrite + Unpin,
{
    let len = msg.encoded_len();
    if len > MAX_MESSAGE_SIZE {
        return Err(CodecError::MessageTooLarge { size: len });
    }

    let mut buf = BytesMut::with_capacity(4 + len);
    buf.put_u32(len as u32);
    msg.encode(&mut buf)?;

    writer.write_all(&buf).await?;
    writer.flush().await?;
    Ok(())
}

/// Read a raw length-prefixed byte frame (used for auth token exchange).
pub async fn read_raw_frame<R>(reader: &mut R) -> Result<Bytes, CodecError>
where
    R: AsyncRead + Unpin,
{
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;

    if len > MAX_MESSAGE_SIZE {
        return Err(CodecError::MessageTooLarge { size: len });
    }

    let mut buf = BytesMut::with_capacity(len);
    buf.resize(len, 0);
    reader.read_exact(&mut buf).await?;
    Ok(buf.freeze())
}

/// Write a raw length-prefixed byte frame (used for auth token exchange).
pub async fn write_raw_frame<W>(writer: &mut W, data: &[u8]) -> Result<(), CodecError>
where
    W: AsyncWrite + Unpin,
{
    let len = data.len();
    if len > MAX_MESSAGE_SIZE {
        return Err(CodecError::MessageTooLarge { size: len });
    }

    let mut buf = BytesMut::with_capacity(4 + len);
    buf.put_u32(len as u32);
    buf.extend_from_slice(data);
    writer.write_all(&buf).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::pince_agent::{AgentMessage, Heartbeat, Ready, ResponseChunk};
    use crate::generated::pince_agent::agent_message::Msg;

    #[tokio::test]
    async fn test_round_trip_ready() {
        let msg = AgentMessage {
            msg: Some(Msg::Ready(Ready {})),
        };

        let mut buf = Vec::new();
        write_message(&mut buf, &msg).await.unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let decoded: AgentMessage = read_message(&mut cursor).await.unwrap();

        assert!(matches!(decoded.msg, Some(Msg::Ready(_))));
    }

    #[tokio::test]
    async fn test_round_trip_response_chunk() {
        let msg = AgentMessage {
            msg: Some(Msg::Response(ResponseChunk {
                content: "Hello, world!".to_string(),
            })),
        };

        let mut buf = Vec::new();
        write_message(&mut buf, &msg).await.unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let decoded: AgentMessage = read_message(&mut cursor).await.unwrap();

        match decoded.msg {
            Some(Msg::Response(chunk)) => assert_eq!(chunk.content, "Hello, world!"),
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_round_trip_heartbeat() {
        let msg = AgentMessage {
            msg: Some(Msg::Heartbeat(Heartbeat {})),
        };

        let mut buf = Vec::new();
        write_message(&mut buf, &msg).await.unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let decoded: AgentMessage = read_message(&mut cursor).await.unwrap();

        assert!(matches!(decoded.msg, Some(Msg::Heartbeat(_))));
    }

    #[tokio::test]
    async fn test_raw_frame_round_trip() {
        let token = b"my-secret-auth-token-32bytes-xxxx";

        let mut buf = Vec::new();
        write_raw_frame(&mut buf, token).await.unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let decoded = read_raw_frame(&mut cursor).await.unwrap();

        assert_eq!(decoded.as_ref(), token);
    }

    #[tokio::test]
    async fn test_oversized_message_rejected() {
        // Write a fake frame with oversized length
        let mut buf = Vec::new();
        let fake_len: u32 = (MAX_MESSAGE_SIZE + 1) as u32;
        buf.extend_from_slice(&fake_len.to_be_bytes());

        let mut cursor = std::io::Cursor::new(buf);
        let result: Result<AgentMessage, _> = read_message(&mut cursor).await;

        assert!(matches!(result, Err(CodecError::MessageTooLarge { .. })));
    }

    #[tokio::test]
    async fn test_truncated_message_error() {
        // Write a frame that claims length=100 but has no data
        let mut buf = Vec::new();
        buf.extend_from_slice(&100u32.to_be_bytes());
        // No actual data

        let mut cursor = std::io::Cursor::new(buf);
        let result: Result<AgentMessage, _> = read_message(&mut cursor).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_codec_over_unix_socket() {
        use tokio::net::UnixStream;

        let (mut server, mut client) = UnixStream::pair().unwrap();

        let send_msg = AgentMessage {
            msg: Some(Msg::Ready(Ready {})),
        };

        // Write from client
        write_message(&mut client, &send_msg).await.unwrap();

        // Read from server
        let recv_msg: AgentMessage = read_message(&mut server).await.unwrap();
        assert!(matches!(recv_msg.msg, Some(Msg::Ready(_))));
    }
}
