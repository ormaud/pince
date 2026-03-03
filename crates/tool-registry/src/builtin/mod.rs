//! Built-in tool implementations.

pub mod feedback;
pub mod list_files;
pub mod read_file;
pub mod shell_exec;
pub mod write_file;

pub use feedback::FeedbackConfig;

use std::path::{Path, PathBuf};

use crate::ToolError;

/// Paths that are always blocked from access by built-in tools.
/// These typically contain secrets (API keys, tokens, credentials).
static PROTECTED_PATH_PREFIXES: &[&str] = &[
    // Default secrets directory.
    "/run/pince/secrets",
    "/tmp/pince/secrets",
];

/// Additional protected paths configured at runtime.
pub struct ProtectedPaths {
    pub(crate) paths: Vec<PathBuf>,
}

impl ProtectedPaths {
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self { paths }
    }

    pub fn default_protected() -> Self {
        let mut paths: Vec<PathBuf> = PROTECTED_PATH_PREFIXES
            .iter()
            .map(PathBuf::from)
            .collect();

        // Also block the XDG_RUNTIME_DIR/pince/secrets path.
        if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
            paths.push(PathBuf::from(runtime_dir).join("pince").join("secrets"));
        }

        Self { paths }
    }

    /// Check whether `path` falls under any protected prefix.
    pub fn is_protected(&self, path: &Path) -> bool {
        // Resolve to canonical form if possible; otherwise use as-is.
        let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        for prefix in &self.paths {
            let resolved_prefix = prefix.canonicalize().unwrap_or_else(|_| prefix.to_path_buf());
            if resolved.starts_with(&resolved_prefix) {
                return true;
            }
        }
        false
    }

    /// Return an access-denied error for a protected path.
    pub fn deny(&self, path: &Path) -> ToolError {
        ToolError::AccessDenied(format!(
            "access to '{}' is not permitted",
            path.display()
        ))
    }
}

/// Helper: register all built-in tools (except optional ones) into a registry.
pub fn register_all(registry: &mut crate::ToolRegistry, protected: ProtectedPaths) {
    use std::sync::Arc;

    let protected = Arc::new(protected);

    registry.register(
        read_file::schema(),
        Box::new(read_file::ReadFileHandler { protected: protected.clone() }),
    );
    registry.register(
        write_file::schema(),
        Box::new(write_file::WriteFileHandler { protected: protected.clone() }),
    );
    registry.register(
        list_files::schema(),
        Box::new(list_files::ListFilesHandler { protected: protected.clone() }),
    );
    registry.register(
        shell_exec::schema(),
        Box::new(shell_exec::ShellExecHandler { protected }),
    );
}

/// Register the optional `feedback` tool into the registry.
///
/// This is separate from `register_all` because it requires external
/// configuration (Trame API URL and secret store).
pub fn register_feedback(registry: &mut crate::ToolRegistry, config: FeedbackConfig) {
    registry.register(
        feedback::schema(),
        Box::new(feedback::FeedbackHandler { config }),
    );
}
