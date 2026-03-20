//! Supervisor configuration loaded from TOML file and environment variables.
//!
//! Load order:
//! 1. Defaults (XDG paths)
//! 2. TOML config file (`~/.config/pince/supervisor.toml`) — if it exists
//! 3. Environment variables — override individual fields

use std::path::PathBuf;

use serde::Deserialize;

use pince_scheduler::CronJob;

/// Top-level TOML config structure.
#[derive(Debug, Default, Deserialize)]
struct TomlConfig {
    heartbeat_timeout_secs: Option<u64>,
    #[serde(default)]
    cron_jobs: Vec<CronJob>,
    #[serde(default)]
    permissions: TomlPermissionsConfig,
}

/// TOML permissions section.
#[derive(Debug, Deserialize)]
struct TomlPermissionsConfig {
    global_policy: Option<String>,
    project_policy: Option<String>,
    hot_reload: Option<bool>,
}

impl Default for TomlPermissionsConfig {
    fn default() -> Self {
        Self {
            global_policy: None,
            project_policy: None,
            hot_reload: Some(true),
        }
    }
}

/// Permissions configuration resolved from TOML and env.
#[derive(Debug, Clone)]
pub struct PermissionsConfig {
    /// Path to the global policy TOML file.
    pub global_policy: PathBuf,
    /// Optional path to the project-local policy TOML file.
    pub project_policy: Option<PathBuf>,
    /// Whether to watch policy files for hot-reload.
    pub hot_reload: bool,
}

/// Supervisor runtime configuration.
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
    /// Path to the TOML config file.
    pub config_file: PathBuf,
    /// Cron jobs loaded from the TOML config.
    pub cron_jobs: Vec<CronJob>,
    /// Permission engine configuration.
    pub permissions: PermissionsConfig,
}

impl Config {
    /// Build config from the TOML file (if it exists) and environment variables.
    pub fn from_env() -> Self {
        let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"));
        let data_dir = std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| dirs_home().join(".local/share"));
        let config_dir = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| dirs_home().join(".config"));

        let pince_runtime = runtime_dir.join("pince");
        let pince_data = data_dir.join("pince");
        let config_file = config_dir.join("pince").join("supervisor.toml");

        // Load TOML config if present.
        let toml_cfg = load_toml(&config_file);

        let global_policy = std::env::var("PINCE_GLOBAL_POLICY")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                toml_cfg
                    .permissions
                    .global_policy
                    .as_deref()
                    .map(expand_tilde)
                    .unwrap_or_else(|| config_dir.join("pince").join("policy.toml"))
            });

        let project_policy = std::env::var("PINCE_PROJECT_POLICY")
            .map(|s| Some(PathBuf::from(s)))
            .unwrap_or_else(|_| {
                toml_cfg
                    .permissions
                    .project_policy
                    .as_deref()
                    .map(expand_tilde)
                    .or_else(|| {
                        // Default: .pince/policy.toml relative to CWD.
                        Some(PathBuf::from(".pince/policy.toml"))
                    })
            });

        let hot_reload = toml_cfg.permissions.hot_reload.unwrap_or(true);

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
                .or(toml_cfg.heartbeat_timeout_secs)
                .unwrap_or(30),
            cron_jobs: toml_cfg.cron_jobs,
            config_file,
            permissions: PermissionsConfig {
                global_policy,
                project_policy,
                hot_reload,
            },
        }
    }
}

/// Try to load and parse the TOML config file. Returns defaults if missing or invalid.
fn load_toml(path: &std::path::Path) -> TomlConfig {
    match std::fs::read_to_string(path) {
        Ok(content) => match toml::from_str::<TomlConfig>(&content) {
            Ok(cfg) => {
                tracing::info!(path = ?path, cron_jobs = cfg.cron_jobs.len(), "loaded supervisor config");
                cfg
            }
            Err(e) => {
                tracing::warn!(path = ?path, "failed to parse supervisor.toml: {e}");
                TomlConfig::default()
            }
        },
        Err(_) => TomlConfig::default(),
    }
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

/// Expand a leading `~/` to the user's home directory.
fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        dirs_home().join(rest)
    } else {
        PathBuf::from(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_toml_with_cron_jobs() {
        let toml_str = r#"
heartbeat_timeout_secs = 60

[[cron_jobs]]
name = "daily-cleanup"
schedule = "0 0 3 * * * *"
agent = "default"
prompt = "Clean up stale files."
timeout_secs = 300
enabled = true

[[cron_jobs]]
name = "disabled-job"
schedule = "0 * * * * * *"
agent = "default"
prompt = "This is disabled."
enabled = false
"#;
        let cfg: TomlConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.heartbeat_timeout_secs, Some(60));
        assert_eq!(cfg.cron_jobs.len(), 2);
        assert_eq!(cfg.cron_jobs[0].name, "daily-cleanup");
        assert!(!cfg.cron_jobs[1].enabled);
    }

    #[test]
    fn empty_toml_gives_defaults() {
        let cfg: TomlConfig = toml::from_str("").unwrap();
        assert!(cfg.heartbeat_timeout_secs.is_none());
        assert!(cfg.cron_jobs.is_empty());
    }
}
