/// Frontend authentication helpers.
///
/// Frontends authenticate by reading a token file written by the supervisor at startup
/// (`$XDG_RUNTIME_DIR/pince/auth_token`, mode 0600) and sending it as the first protobuf
/// `Auth` message.  The supervisor responds with `AuthResult`.
use crate::codec::{read_message, write_message, CodecError};
use crate::generated::pince_frontend::{Auth, AuthResult, FrontendMessage, SupervisorFrontendMessage};
use crate::generated::pince_frontend::frontend_message::Msg;
use crate::generated::pince_frontend::supervisor_frontend_message::Msg as SMsg;
use std::path::PathBuf;
use tokio::io::{AsyncRead, AsyncWrite};

/// Default path for the supervisor auth token file.
///
/// Falls back to `/tmp/pince` when `XDG_RUNTIME_DIR` is not set.
pub fn default_auth_token_path() -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(base).join("pince").join("auth_token")
}

/// Read the token from the well-known token file on disk.
pub fn read_token_from_file(path: &std::path::Path) -> std::io::Result<String> {
    std::fs::read_to_string(path).map(|s| s.trim().to_string())
}

/// Send an `Auth` message as the first message from a frontend.
pub async fn send_auth<W>(writer: &mut W, token: String) -> Result<(), CodecError>
where
    W: AsyncWrite + Unpin,
{
    let msg = FrontendMessage {
        msg: Some(Msg::Auth(Auth { token })),
    };
    write_message(writer, &msg).await
}

/// Read the `AuthResult` response from the supervisor.
///
/// Returns `Ok(())` on success, `Err(FrontendAuthError::Rejected)` if the
/// supervisor rejected the token.
pub async fn recv_auth_result<R>(reader: &mut R) -> Result<(), FrontendAuthError>
where
    R: AsyncRead + Unpin,
{
    let msg: SupervisorFrontendMessage = read_message(reader).await.map_err(FrontendAuthError::Io)?;
    match msg.msg {
        Some(SMsg::AuthResult(AuthResult { success: true, .. })) => Ok(()),
        Some(SMsg::AuthResult(AuthResult { success: false, error })) => {
            Err(FrontendAuthError::Rejected(error))
        }
        _ => Err(FrontendAuthError::UnexpectedMessage),
    }
}

/// Validate an incoming `Auth` message on the supervisor side.
///
/// Returns the token string from the message, or `None` if the message is not
/// an `Auth` variant.
pub fn extract_auth_token(msg: &FrontendMessage) -> Option<&str> {
    match &msg.msg {
        Some(Msg::Auth(Auth { token })) => Some(token),
        _ => None,
    }
}

/// Build a successful `AuthResult` response.
pub fn auth_ok() -> SupervisorFrontendMessage {
    SupervisorFrontendMessage {
        msg: Some(SMsg::AuthResult(AuthResult {
            success: true,
            error: String::new(),
        })),
    }
}

/// Build a failed `AuthResult` response.
pub fn auth_err(reason: impl Into<String>) -> SupervisorFrontendMessage {
    SupervisorFrontendMessage {
        msg: Some(SMsg::AuthResult(AuthResult {
            success: false,
            error: reason.into(),
        })),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FrontendAuthError {
    #[error("auth rejected by supervisor: {0}")]
    Rejected(String),
    #[error("unexpected message during auth handshake")]
    UnexpectedMessage,
    #[error("I/O error: {0}")]
    Io(#[from] CodecError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{read_message, write_message};
    use tokio::net::UnixStream;

    #[tokio::test]
    async fn test_auth_handshake_success() {
        let (mut server, mut client) = UnixStream::pair().unwrap();

        // Client sends Auth
        send_auth(&mut client, "my-secret-token".to_string()).await.unwrap();

        // Server reads and validates
        let msg: FrontendMessage = read_message(&mut server).await.unwrap();
        let token = extract_auth_token(&msg).unwrap();
        assert_eq!(token, "my-secret-token");

        // Server sends AuthResult OK
        write_message(&mut server, &auth_ok()).await.unwrap();

        // Client reads result
        recv_auth_result(&mut client).await.unwrap();
    }

    #[tokio::test]
    async fn test_auth_handshake_rejected() {
        let (mut server, mut client) = UnixStream::pair().unwrap();

        send_auth(&mut client, "wrong-token".to_string()).await.unwrap();

        let msg: FrontendMessage = read_message(&mut server).await.unwrap();
        let _token = extract_auth_token(&msg).unwrap();

        // Server sends AuthResult Err
        write_message(&mut server, &auth_err("invalid token")).await.unwrap();

        let result = recv_auth_result(&mut client).await;
        assert!(matches!(result, Err(FrontendAuthError::Rejected(_))));
    }

    #[tokio::test]
    async fn test_extract_auth_token_wrong_variant() {
        use crate::generated::pince_frontend::{ListAgents};
        use crate::generated::pince_frontend::frontend_message::Msg;

        let msg = FrontendMessage {
            msg: Some(Msg::ListAgents(ListAgents {})),
        };
        assert!(extract_auth_token(&msg).is_none());
    }
}
