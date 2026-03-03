//! SecretStore: reads and writes secrets on disk.

use std::{
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};

/// Regex-like pattern for valid secret names: `[a-z0-9-]+`.
fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-'))
}

/// A secret value that intentionally avoids accidental exposure.
///
/// - No `Debug` or `Display` impl so it won't appear in logs.
/// - Call `.expose()` to get the inner bytes.
pub struct SecretValue(Vec<u8>);

impl SecretValue {
    /// Access the raw secret bytes. Use only where the value is actually needed.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    /// Convenience: expose as a UTF-8 string slice.
    pub fn expose_str(&self) -> Result<&str> {
        std::str::from_utf8(&self.0).context("secret is not valid UTF-8")
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        // Zero the memory on drop to reduce the window where the value lives.
        for byte in self.0.iter_mut() {
            *byte = 0;
        }
    }
}

/// Persistent store backed by files in a directory.
///
/// The directory is created with mode `0o700` on first access.
pub struct SecretStore {
    secrets_dir: PathBuf,
}

impl SecretStore {
    /// Create a new `SecretStore` rooted at `secrets_dir`.
    ///
    /// The directory is created (with mode `0o700`) if it does not exist.
    pub fn new(secrets_dir: PathBuf) -> Result<Self> {
        ensure_secrets_dir(&secrets_dir)?;
        Ok(Self { secrets_dir })
    }

    /// Return the secrets directory path.
    pub fn dir(&self) -> &Path {
        &self.secrets_dir
    }

    /// Read a secret by name. Reads from disk on every call.
    pub fn resolve(&self, name: &str) -> Result<SecretValue> {
        validate_name(name)?;
        let path = self.secrets_dir.join(name);
        let bytes = fs::read(&path)
            .with_context(|| format!("secret '{name}' not found"))?;
        Ok(SecretValue(bytes))
    }

    /// List secret names (never their values).
    pub fn list(&self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in fs::read_dir(&self.secrets_dir)
            .context("reading secrets directory")?
        {
            let entry = entry.context("reading directory entry")?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("secret filename is not valid UTF-8"))?;
            if is_valid_name(&name) {
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }

    /// Store a secret value. Creates or overwrites the file, mode `0o600`.
    pub fn set(&self, name: &str, value: &[u8]) -> Result<()> {
        validate_name(name)?;
        let path = self.secrets_dir.join(name);
        fs::write(&path, value)
            .with_context(|| format!("writing secret '{name}'"))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("chmod 0o600 on secret '{name}'"))?;
        Ok(())
    }

    /// Delete a secret. Returns an error if it does not exist.
    pub fn delete(&self, name: &str) -> Result<()> {
        validate_name(name)?;
        let path = self.secrets_dir.join(name);
        fs::remove_file(&path)
            .with_context(|| format!("deleting secret '{name}'"))?;
        Ok(())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn validate_name(name: &str) -> Result<()> {
    if !is_valid_name(name) {
        bail!(
            "invalid secret name '{name}': must be non-empty and contain only [a-z0-9-]"
        );
    }
    Ok(())
}

fn ensure_secrets_dir(dir: &Path) -> Result<()> {
    if dir.exists() {
        // Verify that it's a directory with the right permissions.
        let meta = fs::metadata(dir).context("stat secrets dir")?;
        if !meta.is_dir() {
            bail!("secrets path exists but is not a directory: {}", dir.display());
        }
        // Tighten permissions if they were widened somehow.
        let mode = meta.permissions().mode() & 0o777;
        if mode != 0o700 {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
                .context("chmod 700 on secrets dir")?;
        }
        return Ok(());
    }

    fs::DirBuilder::new()
        .mode(0o700)
        .recursive(true)
        .create(dir)
        .with_context(|| format!("creating secrets directory: {}", dir.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_store() -> (TempDir, SecretStore) {
        let dir = TempDir::new().unwrap();
        let store = SecretStore::new(dir.path().join("secrets")).unwrap();
        (dir, store)
    }

    #[test]
    fn set_and_resolve() {
        let (_dir, store) = temp_store();
        store.set("my-key", b"supersecret").unwrap();
        let val = store.resolve("my-key").unwrap();
        assert_eq!(val.expose(), b"supersecret");
    }

    #[test]
    fn resolve_missing_secret_errors() {
        let (_dir, store) = temp_store();
        match store.resolve("nonexistent") {
            Err(e) => assert!(e.to_string().contains("nonexistent"), "error: {e}"),
            Ok(_) => panic!("expected error for missing secret"),
        }
    }

    #[test]
    fn list_returns_sorted_names() {
        let (_dir, store) = temp_store();
        store.set("b-key", b"1").unwrap();
        store.set("a-key", b"2").unwrap();
        store.set("c-key", b"3").unwrap();
        let names = store.list().unwrap();
        assert_eq!(names, vec!["a-key", "b-key", "c-key"]);
    }

    #[test]
    fn delete_removes_secret() {
        let (_dir, store) = temp_store();
        store.set("to-delete", b"gone").unwrap();
        store.delete("to-delete").unwrap();
        assert!(store.resolve("to-delete").is_err());
    }

    #[test]
    fn delete_nonexistent_errors() {
        let (_dir, store) = temp_store();
        assert!(store.delete("nope").is_err());
    }

    #[test]
    fn invalid_name_rejected() {
        let (_dir, store) = temp_store();
        assert!(store.set("../evil", b"bad").is_err());
        assert!(store.set("Has_Upper", b"bad").is_err());
        assert!(store.set("has space", b"bad").is_err());
        assert!(store.set("", b"bad").is_err());
    }

    #[test]
    fn secrets_dir_created_with_mode_700() {
        let dir = TempDir::new().unwrap();
        let secrets_path = dir.path().join("secrets");
        SecretStore::new(secrets_path.clone()).unwrap();
        let meta = fs::metadata(&secrets_path).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "secrets dir should be mode 0o700");
    }

    #[test]
    fn secret_file_created_with_mode_600() {
        let (_dir, store) = temp_store();
        store.set("api-key", b"val").unwrap();
        let path = store.dir().join("api-key");
        let meta = fs::metadata(&path).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "secret file should be mode 0o600");
    }

    #[test]
    fn expose_str_valid_utf8() {
        let (_dir, store) = temp_store();
        store.set("tok", b"hello-world").unwrap();
        let val = store.resolve("tok").unwrap();
        assert_eq!(val.expose_str().unwrap(), "hello-world");
    }
}
