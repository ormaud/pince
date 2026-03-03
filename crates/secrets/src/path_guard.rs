//! Hardcoded path-based deny rules for tool calls.
//!
//! This runs at the supervisor level and cannot be overridden by any policy.

use std::path::Path;

/// A guard that denies access to protected paths.
///
/// Instantiate with `PathGuard::new(secrets_dir)` and call
/// `check_path()` before executing any tool that accesses the filesystem.
pub struct PathGuard {
    protected: Vec<std::path::PathBuf>,
}

impl PathGuard {
    /// Create a guard that protects the given secrets directory.
    pub fn new(secrets_dir: std::path::PathBuf) -> Self {
        Self {
            protected: vec![secrets_dir],
        }
    }

    /// Return `Ok(())` if `path` is not under any protected directory,
    /// or `Err(denial_reason)` if access is denied.
    pub fn check_path(&self, path: &Path) -> Result<(), String> {
        // Canonicalize without requiring the path to exist (resolve `.` / `..`).
        let canonical = normalize_path(path);
        for protected in &self.protected {
            let canonical_protected = normalize_path(protected);
            if canonical.starts_with(&canonical_protected) {
                return Err(format!(
                    "access denied: '{}' is inside protected directory '{}'",
                    path.display(),
                    protected.display()
                ));
            }
        }
        Ok(())
    }

    /// Check multiple paths at once. Returns first denial error found.
    pub fn check_paths<'a, I>(&self, paths: I) -> Result<(), String>
    where
        I: IntoIterator<Item = &'a Path>,
    {
        for path in paths {
            self.check_path(path)?;
        }
        Ok(())
    }
}

/// Normalize a path without requiring it to exist (unlike `canonicalize`).
/// Resolves `.` and `..` components lexically.
fn normalize_path(path: &Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn guard() -> PathGuard {
        PathGuard::new(PathBuf::from("/home/user/.config/pince/secrets"))
    }

    #[test]
    fn allows_unrelated_path() {
        let g = guard();
        assert!(g.check_path(Path::new("/home/user/docs/file.txt")).is_ok());
    }

    #[test]
    fn denies_exact_secrets_dir() {
        let g = guard();
        let err = g.check_path(Path::new("/home/user/.config/pince/secrets")).unwrap_err();
        assert!(err.contains("protected"));
    }

    #[test]
    fn denies_file_inside_secrets_dir() {
        let g = guard();
        let err = g
            .check_path(Path::new("/home/user/.config/pince/secrets/api-key"))
            .unwrap_err();
        assert!(err.contains("protected"));
    }

    #[test]
    fn denies_traversal_into_secrets() {
        let g = guard();
        // path traversal attempt
        let err = g
            .check_path(Path::new(
                "/home/user/.config/pince/secrets/../secrets/api-key",
            ))
            .unwrap_err();
        assert!(err.contains("protected"));
    }

    #[test]
    fn allows_parent_of_secrets_dir() {
        let g = guard();
        assert!(g.check_path(Path::new("/home/user/.config/pince")).is_ok());
    }

    #[test]
    fn check_paths_denies_if_any_protected() {
        let g = guard();
        let err = g
            .check_paths([
                Path::new("/tmp/ok"),
                Path::new("/home/user/.config/pince/secrets/tok"),
            ])
            .unwrap_err();
        assert!(err.contains("protected"));
    }

    #[test]
    fn check_paths_allows_all_safe() {
        let g = guard();
        assert!(g
            .check_paths([Path::new("/tmp/a"), Path::new("/home/user/b")])
            .is_ok());
    }
}
