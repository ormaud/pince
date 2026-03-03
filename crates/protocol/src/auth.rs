/// One-time auth token protocol.
///
/// The supervisor generates a 32-byte random token per agent at spawn time.
/// The token is passed via `PINCE_AUTH_TOKEN` env variable (hex-encoded).
/// The agent sends the token as its first raw frame before any protobuf messages.
/// The supervisor validates and rejects invalid tokens. Single-use only.
use crate::codec::{read_raw_frame, write_raw_frame, CodecError};
use tokio::io::{AsyncRead, AsyncWrite};

/// Environment variable name for the auth token.
pub const AUTH_TOKEN_ENV: &str = "PINCE_AUTH_TOKEN";

/// Length of the auth token in bytes.
pub const AUTH_TOKEN_LEN: usize = 32;

/// Send the auth token as the first raw frame.
pub async fn send_auth_token<W>(writer: &mut W, token: &[u8; AUTH_TOKEN_LEN]) -> Result<(), CodecError>
where
    W: AsyncWrite + Unpin,
{
    write_raw_frame(writer, token).await
}

/// Read and validate the auth token from the first raw frame.
///
/// Returns `Ok(())` if the token matches, `Err` otherwise.
pub async fn recv_auth_token<R>(
    reader: &mut R,
    expected: &[u8; AUTH_TOKEN_LEN],
) -> Result<(), AuthError>
where
    R: AsyncRead + Unpin,
{
    let frame = read_raw_frame(reader).await.map_err(AuthError::Io)?;
    if frame.len() != AUTH_TOKEN_LEN {
        return Err(AuthError::InvalidToken);
    }
    // Constant-time comparison to prevent timing attacks
    let mut diff = 0u8;
    for (a, b) in frame.iter().zip(expected.iter()) {
        diff |= a ^ b;
    }
    if diff != 0 {
        return Err(AuthError::InvalidToken);
    }
    Ok(())
}

/// Parse a hex-encoded auth token from a string.
pub fn parse_token(hex: &str) -> Result<[u8; AUTH_TOKEN_LEN], AuthError> {
    if hex.len() != AUTH_TOKEN_LEN * 2 {
        return Err(AuthError::InvalidToken);
    }
    let mut bytes = [0u8; AUTH_TOKEN_LEN];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let hi = hex_digit(chunk[0])?;
        let lo = hex_digit(chunk[1])?;
        bytes[i] = (hi << 4) | lo;
    }
    Ok(bytes)
}

fn hex_digit(b: u8) -> Result<u8, AuthError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(AuthError::InvalidToken),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid or missing auth token")]
    InvalidToken,
    #[error("I/O error during auth: {0}")]
    Io(CodecError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_token_valid() {
        let token = "aabbccddeeff00112233445566778899aabbccddeeff001122334455667788aa";
        let parsed = parse_token(token).unwrap();
        assert_eq!(parsed[0], 0xaa);
        assert_eq!(parsed[1], 0xbb);
        assert_eq!(parsed[31], 0xaa);
    }

    #[test]
    fn test_parse_token_wrong_length() {
        assert!(parse_token("aabb").is_err());
        assert!(parse_token("").is_err());
    }

    #[test]
    fn test_parse_token_invalid_hex() {
        let bad = "zz000000000000000000000000000000000000000000000000000000000000000000";
        assert!(parse_token(bad).is_err());
    }

    #[tokio::test]
    async fn test_auth_round_trip() {
        use tokio::net::UnixStream;

        let token: [u8; AUTH_TOKEN_LEN] = [0xde; AUTH_TOKEN_LEN];
        let (mut server, mut client) = UnixStream::pair().unwrap();

        send_auth_token(&mut client, &token).await.unwrap();
        recv_auth_token(&mut server, &token).await.unwrap();
    }

    #[tokio::test]
    async fn test_auth_wrong_token_rejected() {
        use tokio::net::UnixStream;

        let token: [u8; AUTH_TOKEN_LEN] = [0xde; AUTH_TOKEN_LEN];
        let wrong: [u8; AUTH_TOKEN_LEN] = [0xad; AUTH_TOKEN_LEN];
        let (mut server, mut client) = UnixStream::pair().unwrap();

        send_auth_token(&mut client, &token).await.unwrap();
        let result = recv_auth_token(&mut server, &wrong).await;
        assert!(matches!(result, Err(AuthError::InvalidToken)));
    }

    #[tokio::test]
    async fn test_auth_short_frame_rejected() {
        use tokio::net::UnixStream;
        use crate::codec::write_raw_frame;

        let expected: [u8; AUTH_TOKEN_LEN] = [0xde; AUTH_TOKEN_LEN];
        let (mut server, mut client) = UnixStream::pair().unwrap();

        // Send only 16 bytes (too short)
        write_raw_frame(&mut client, &[0u8; 16]).await.unwrap();
        let result = recv_auth_token(&mut server, &expected).await;
        assert!(matches!(result, Err(AuthError::InvalidToken)));
    }
}
