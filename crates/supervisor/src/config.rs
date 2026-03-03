//! Supervisor configuration loaded from environment / defaults.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    /// Path to the Unix socket for frontend connections.
    pub frontend_socket: PathBuf,
    /// Directory where per-agent sockets will be created.
    pub agent_socket_dir: PathBuf,
    /// Path to the file-based auth token for frontends.
    pub auth_token_file: PathBuf,
    /// Path to the audit log (JSONL).
    pub audit_log: PathBuf,
    /// Seconds without a heartbeat before an agent is considered dead.
    pub heartbeat_timeout_secs: u64,
}

impl Config {
    /// Build config from environment variables, falling back to XDG defaults.
    pub fn from_env() -> Self {
        let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"));
        let data_dir = std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                dirs_home().join(".local/share")
            });

        let pince_runtime = runtime_dir.join("pince");
        let pince_data = data_dir.join("pince");

        Self {
            frontend_socket: std::env::var("PINCE_FRONTEND_SOCKET")
                .map(PathBuf::from)
                .unwrap_or_else(|_| pince_runtime.join("supervisor.sock")),
            agent_socket_dir: std::env::var("PINCE_AGENT_SOCKET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| pince_runtime.join("agents")),
            auth_token_file: std::env::var("PINCE_AUTH_TOKEN_FILE")
                .map(PathBuf::from)
                .unwrap_or_else(|_| pince_runtime.join("auth_token")),
            audit_log: std::env::var("PINCE_AUDIT_LOG")
                .map(PathBuf::from)
                .unwrap_or_else(|_| pince_data.join("audit.jsonl")),
            heartbeat_timeout_secs: std::env::var("PINCE_HEARTBEAT_TIMEOUT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
        }
    }
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}
