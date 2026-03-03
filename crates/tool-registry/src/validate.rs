//! Lightweight JSON Schema (draft-07 subset) validator.
//!
//! Supports the subset of JSON Schema features used by built-in tools:
//! - `type` (object, string, integer, number, boolean, array, null)
//! - `properties` with per-property schemas
//! - `required` (list of required property names)
//! - `additionalProperties: false`
//! - `minimum` / `maximum` for numbers/integers
//! - `enum` (allowed values)
//! - Nested schemas (for property validation)

use serde_json::Value;

use crate::ToolError;

/// Validate `value` against `schema`.
///
/// Returns `Ok(())` on success, or an `InvalidArguments` error describing
/// the first violation found.
pub fn validate(schema: &Value, value: &Value) -> Result<(), ToolError> {
    validate_inner(schema, value, "")
}

fn validate_inner(schema: &Value, value: &Value, path: &str) -> Result<(), ToolError> {
    let schema_obj = match schema.as_object() {
        Some(o) => o,
        None => return Ok(()), // permissive if schema isn't an object
    };

    // --- type check ---
    if let Some(type_val) = schema_obj.get("type") {
        let ok = match type_val.as_str().unwrap_or("") {
            "object" => value.is_object(),
            "string" => value.is_string(),
            "integer" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "array" => value.is_array(),
            "null" => value.is_null(),
            other => {
                return Err(ToolError::InvalidArguments(format!(
                    "{path}: unknown schema type '{other}'"
                )));
            }
        };
        if !ok {
            let got = json_type_name(value);
            let loc = if path.is_empty() { "root".to_string() } else { path.to_string() };
            return Err(ToolError::InvalidArguments(format!(
                "{loc}: expected {}, got {got}",
                type_val.as_str().unwrap_or("unknown")
            )));
        }
    }

    // --- enum check ---
    if let Some(enum_val) = schema_obj.get("enum") {
        if let Some(variants) = enum_val.as_array() {
            if !variants.contains(value) {
                let loc = if path.is_empty() { "root".to_string() } else { path.to_string() };
                return Err(ToolError::InvalidArguments(format!(
                    "{loc}: value not in enum {enum_val}"
                )));
            }
        }
    }

    // --- minimum / maximum (for numbers/integers) ---
    if let Some(min) = schema_obj.get("minimum").and_then(|v| v.as_f64()) {
        if let Some(n) = value.as_f64() {
            if n < min {
                let loc = if path.is_empty() { "root".to_string() } else { path.to_string() };
                return Err(ToolError::InvalidArguments(format!(
                    "{loc}: value {n} < minimum {min}"
                )));
            }
        }
    }
    if let Some(max) = schema_obj.get("maximum").and_then(|v| v.as_f64()) {
        if let Some(n) = value.as_f64() {
            if n > max {
                let loc = if path.is_empty() { "root".to_string() } else { path.to_string() };
                return Err(ToolError::InvalidArguments(format!(
                    "{loc}: value {n} > maximum {max}"
                )));
            }
        }
    }

    // --- object-specific checks ---
    if let Some(obj) = value.as_object() {
        // required fields
        if let Some(required) = schema_obj.get("required").and_then(|r| r.as_array()) {
            for req in required {
                if let Some(key) = req.as_str() {
                    if !obj.contains_key(key) {
                        let loc = if path.is_empty() { "root".to_string() } else { path.to_string() };
                        return Err(ToolError::InvalidArguments(format!(
                            "{loc}: missing required field '{key}'"
                        )));
                    }
                }
            }
        }

        // additionalProperties: false
        if schema_obj.get("additionalProperties") == Some(&Value::Bool(false)) {
            if let Some(props) = schema_obj.get("properties").and_then(|p| p.as_object()) {
                for key in obj.keys() {
                    if !props.contains_key(key) {
                        let loc = if path.is_empty() { "root".to_string() } else { path.to_string() };
                        return Err(ToolError::InvalidArguments(format!(
                            "{loc}: additional property '{key}' not allowed"
                        )));
                    }
                }
            } else {
                // no properties defined — all properties are additional
                if !obj.is_empty() {
                    let loc = if path.is_empty() { "root".to_string() } else { path.to_string() };
                    return Err(ToolError::InvalidArguments(format!(
                        "{loc}: additional properties not allowed"
                    )));
                }
            }
        }

        // validate each property's value against its schema
        if let Some(props) = schema_obj.get("properties").and_then(|p| p.as_object()) {
            for (key, prop_schema) in props {
                if let Some(val) = obj.get(key) {
                    let child_path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    validate_inner(prop_schema, val, &child_path)?;
                }
            }
        }
    }

    Ok(())
}

fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_valid_object() {
        let schema = json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"],
            "additionalProperties": false
        });
        assert!(validate(&schema, &json!({"path": "/tmp/foo"})).is_ok());
    }

    #[test]
    fn test_missing_required() {
        let schema = json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"],
            "additionalProperties": false
        });
        let err = validate(&schema, &json!({})).unwrap_err();
        assert!(err.to_string().contains("missing required field 'path'"));
    }

    #[test]
    fn test_wrong_type() {
        let schema = json!({
            "type": "object",
            "properties": {
                "timeout_secs": { "type": "integer", "minimum": 1, "maximum": 300 }
            },
            "required": ["timeout_secs"]
        });
        let err = validate(&schema, &json!({"timeout_secs": "hello"})).unwrap_err();
        assert!(err.to_string().contains("expected integer"));
    }

    #[test]
    fn test_minimum() {
        let schema = json!({
            "type": "object",
            "properties": {
                "n": { "type": "integer", "minimum": 1 }
            },
            "required": ["n"]
        });
        let err = validate(&schema, &json!({"n": 0})).unwrap_err();
        assert!(err.to_string().contains("minimum"));
    }

    #[test]
    fn test_additional_properties() {
        let schema = json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"],
            "additionalProperties": false
        });
        let err = validate(&schema, &json!({"path": "/x", "extra": "y"})).unwrap_err();
        assert!(err.to_string().contains("additional property"));
    }
}
