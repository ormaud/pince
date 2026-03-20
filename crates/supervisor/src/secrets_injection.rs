//! Secret injection stub.
//!
//! In the future, this module will resolve `$secret:xxx` references in tool
//! arguments by looking up secrets from the secure secrets store and
//! substituting them before the tool call reaches the sandbox.
//!
//! For the MVP, this is a no-op pass-through.

use serde_json::Value;

/// Resolve `$secret:xxx` placeholders in tool arguments.
///
/// Currently a no-op — returns the arguments unchanged.
/// Future: walk the JSON value tree and replace `$secret:NAME` strings
/// with the corresponding secret fetched from the secrets store.
pub fn inject_secrets(args: Value) -> Value {
    // TODO: resolve $secret:xxx references from the secrets store
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_unchanged() {
        let args = serde_json::json!({"command": "echo hello", "timeout_secs": 30});
        let out = inject_secrets(args.clone());
        assert_eq!(out, args);
    }
}
