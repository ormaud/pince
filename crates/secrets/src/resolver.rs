//! `$secret:` reference resolution for tool-call arguments.
//!
//! The supervisor scans JSON arguments for string values matching
//! `$secret:<name>` and replaces each with the resolved secret value before
//! passing the arguments to a tool handler.
//!
//! The original (reference form) arguments are what get logged; callers must
//! never log the resolved values.

use anyhow::{Context, Result};

use crate::store::SecretStore;

const PREFIX: &str = "$secret:";

/// Walk a JSON value tree, replacing any string that is exactly
/// `$secret:<name>` with the resolved secret value.
///
/// Returns a new `serde_json::Value` with references replaced.
/// Errors if any referenced secret does not exist.
pub fn resolve_secret_refs(
    value: &serde_json::Value,
    store: &SecretStore,
) -> Result<serde_json::Value> {
    match value {
        serde_json::Value::String(s) => {
            if let Some(name) = s.strip_prefix(PREFIX) {
                let secret = store
                    .resolve(name)
                    .with_context(|| format!("resolving $secret:{name}"))?;
                let resolved_str = secret
                    .expose_str()
                    .with_context(|| format!("secret '{name}' is not valid UTF-8"))?;
                Ok(serde_json::Value::String(resolved_str.to_owned()))
            } else {
                Ok(value.clone())
            }
        }
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                out.insert(k.clone(), resolve_secret_refs(v, store)?);
            }
            Ok(serde_json::Value::Object(out))
        }
        serde_json::Value::Array(arr) => {
            let out: Result<Vec<_>> = arr.iter().map(|v| resolve_secret_refs(v, store)).collect();
            Ok(serde_json::Value::Array(out?))
        }
        // Null, Bool, Number — pass through unchanged.
        other => Ok(other.clone()),
    }
}

/// Return `true` if `value` (or any nested string) contains a `$secret:`
/// reference. Useful for deciding whether resolution is needed.
pub fn has_secret_refs(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(s) => s.starts_with(PREFIX),
        serde_json::Value::Object(map) => map.values().any(has_secret_refs),
        serde_json::Value::Array(arr) => arr.iter().any(has_secret_refs),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn store_with(secrets: &[(&str, &str)]) -> (TempDir, SecretStore) {
        let dir = TempDir::new().unwrap();
        let store = SecretStore::new(dir.path().join("secrets")).unwrap();
        for (name, val) in secrets {
            store.set(name, val.as_bytes()).unwrap();
        }
        (dir, store)
    }

    #[test]
    fn resolves_top_level_string() {
        let (_dir, store) = store_with(&[("api-key", "sk-12345")]);
        let input = json!("$secret:api-key");
        let out = resolve_secret_refs(&input, &store).unwrap();
        assert_eq!(out, json!("sk-12345"));
    }

    #[test]
    fn resolves_nested_in_object() {
        let (_dir, store) = store_with(&[("token", "tok-abc")]);
        let input = json!({ "auth": "$secret:token", "url": "https://example.com" });
        let out = resolve_secret_refs(&input, &store).unwrap();
        assert_eq!(out, json!({ "auth": "tok-abc", "url": "https://example.com" }));
    }

    #[test]
    fn resolves_multiple_refs_in_object() {
        let (_dir, store) = store_with(&[("k1", "v1"), ("k2", "v2")]);
        let input = json!({ "a": "$secret:k1", "b": "$secret:k2" });
        let out = resolve_secret_refs(&input, &store).unwrap();
        assert_eq!(out, json!({ "a": "v1", "b": "v2" }));
    }

    #[test]
    fn resolves_in_array() {
        let (_dir, store) = store_with(&[("x", "hello")]);
        let input = json!(["plain", "$secret:x"]);
        let out = resolve_secret_refs(&input, &store).unwrap();
        assert_eq!(out, json!(["plain", "hello"]));
    }

    #[test]
    fn non_secret_string_unchanged() {
        let (_dir, store) = store_with(&[]);
        let input = json!("no-secret-here");
        let out = resolve_secret_refs(&input, &store).unwrap();
        assert_eq!(out, json!("no-secret-here"));
    }

    #[test]
    fn missing_secret_errors() {
        let (_dir, store) = store_with(&[]);
        let input = json!("$secret:missing");
        let err = resolve_secret_refs(&input, &store).unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn invalid_secret_name_errors() {
        let (_dir, store) = store_with(&[]);
        let input = json!("$secret:../evil");
        let err = resolve_secret_refs(&input, &store).unwrap_err();
        assert!(err.to_string().contains("evil") || err.to_string().contains("invalid"));
    }

    #[test]
    fn has_secret_refs_detects_refs() {
        assert!(has_secret_refs(&json!("$secret:key")));
        assert!(has_secret_refs(&json!({ "a": "$secret:key" })));
        assert!(has_secret_refs(&json!(["plain", "$secret:key"])));
        assert!(!has_secret_refs(&json!("plain")));
        assert!(!has_secret_refs(&json!(42)));
        assert!(!has_secret_refs(&json!({ "a": "b" })));
    }

    #[test]
    fn non_string_values_pass_through() {
        let (_dir, store) = store_with(&[]);
        let input = json!({ "count": 42, "enabled": true, "nothing": null });
        let out = resolve_secret_refs(&input, &store).unwrap();
        assert_eq!(out, input);
    }
}
