//! Agent process spawning: socketpair creation, token generation, child exec.

use std::os::unix::io::AsRawFd;

use anyhow::{Context, Result};
use tokio::net::UnixStream;
use tokio::process::{Child, Command};

use pince_protocol::auth::AUTH_TOKEN_LEN;

/// A spawned agent process with its supervisor-side socket and auth token.
pub struct SpawnedAgent {
    pub agent_id: String,
    pub auth_token: [u8; AUTH_TOKEN_LEN],
    pub stream: UnixStream,
    pub child: Child,
}

/// Generate a random 32-byte auth token.
fn generate_token() -> [u8; AUTH_TOKEN_LEN] {
    let mut token = [0u8; AUTH_TOKEN_LEN];
    getrandom::getrandom(&mut token).expect("getrandom failed");
    token
}

/// Hex-encode a byte slice.
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Spawn a Python sub-agent process.
///
/// Creates a Unix socketpair, generates an auth token, and spawns
/// `python -m pince_agent` with `PINCE_AUTH_TOKEN` and `PINCE_SOCKET_FD`
/// environment variables.
///
/// Returns the supervisor's end of the socket, the token, and the child handle.
pub fn spawn_agent(agent_id: &str) -> Result<SpawnedAgent> {
    let token = generate_token();

    // Create Unix socket pair (both ends start with CLOEXEC).
    let (parent_stream, child_stream) =
        std::os::unix::net::UnixStream::pair().context("creating socket pair")?;

    let child_fd = child_stream.as_raw_fd();

    // SAFETY: pre_exec runs after fork, before exec in the child.
    // We clear CLOEXEC on the child's FD so the agent inherits it.
    let child = unsafe {
        Command::new("python")
            .arg("-m")
            .arg("pince_agent")
            .env("PINCE_AUTH_TOKEN", hex_encode(&token))
            .env("PINCE_SOCKET_FD", child_fd.to_string())
            .kill_on_drop(true)
            .pre_exec(move || {
                let flags = libc::fcntl(child_fd, libc::F_GETFD);
                if flags < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::fcntl(child_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            })
            .spawn()
            .context("spawning agent process")?
    };

    // Close the child's end in the parent.
    drop(child_stream);

    // Convert parent's end to async tokio stream.
    parent_stream
        .set_nonblocking(true)
        .context("set_nonblocking")?;
    let stream = UnixStream::from_std(parent_stream).context("UnixStream::from_std")?;

    Ok(SpawnedAgent {
        agent_id: agent_id.to_string(),
        auth_token: token,
        stream,
        child,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_token() {
        let t1 = generate_token();
        let t2 = generate_token();
        assert_ne!(t1, t2);
        assert_eq!(t1.len(), AUTH_TOKEN_LEN);
    }

    #[test]
    fn test_hex_encode() {
        assert_eq!(hex_encode(&[0xab, 0xcd, 0x01]), "abcd01");
        assert_eq!(hex_encode(&[0x00, 0xff]), "00ff");
    }
}
