//! Deserialisation helpers for Nextcloud's loose JSON typing.
//!
//! One field can arrive as `12`, `"12"`, `null` or `""`, depending on endpoint,
//! server version, and whether it passed through a database driver.

use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// Deserialise an integer that may arrive as a number or a string.
pub(crate) fn flexible_i64<'de, D>(d: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(opt_flexible_i64(d)?.unwrap_or(0))
}

/// Deserialise an optional integer from a number, string, `""` or null.
pub(crate) fn opt_flexible_i64<'de, D>(d: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    let v = Option::<Value>::deserialize(d)?;
    Ok(match v {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Some(Value::String(s)) if !s.is_empty() => s.parse().ok(),
        _ => None,
    })
}

/// Deserialise a string field that may arrive as a number.
pub(crate) fn flexible_string<'de, D>(d: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(opt_flexible_string(d)?.unwrap_or_default())
}

/// Deserialise an optional string from a number, `""` or null. An empty string
/// means unset.
pub(crate) fn opt_flexible_string<'de, D>(d: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let v = Option::<Value>::deserialize(d)?;
    Ok(match v {
        Some(Value::String(s)) if !s.is_empty() => Some(s),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::Bool(b)) => Some(b.to_string()),
        _ => None,
    })
}

/// Deserialise a boolean that may arrive as `true`, `"true"`, `1`, or `"1"`.
pub(crate) fn flexible_bool<'de, D>(d: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let v = Option::<Value>::deserialize(d)?;
    Ok(match v {
        Some(Value::Bool(b)) => b,
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0) != 0,
        Some(Value::String(s)) => matches!(s.as_str(), "1" | "true" | "yes"),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct T {
        #[serde(deserialize_with = "flexible_i64")]
        n: i64,
        #[serde(default, deserialize_with = "opt_flexible_i64")]
        o: Option<i64>,
        #[serde(default, deserialize_with = "opt_flexible_string")]
        s: Option<String>,
        #[serde(default, deserialize_with = "flexible_bool")]
        b: bool,
    }

    fn parse(j: serde_json::Value) -> T {
        serde_json::from_value(j).unwrap()
    }

    #[test]
    fn flexible_ints() {
        assert_eq!(parse(serde_json::json!({"n": 12})).n, 12);
        assert_eq!(parse(serde_json::json!({"n": "12"})).n, 12);
        assert_eq!(parse(serde_json::json!({"n": null})).n, 0);
        assert_eq!(parse(serde_json::json!({"n": ""})).n, 0);
    }

    #[test]
    fn optional_ints() {
        assert_eq!(parse(serde_json::json!({"n": 0, "o": "7"})).o, Some(7));
        assert_eq!(parse(serde_json::json!({"n": 0, "o": ""})).o, None);
        assert_eq!(parse(serde_json::json!({"n": 0})).o, None);
    }

    #[test]
    fn empty_string_is_none() {
        assert_eq!(parse(serde_json::json!({"n": 0, "s": ""})).s, None);
        assert_eq!(
            parse(serde_json::json!({"n": 0, "s": "x"})).s,
            Some("x".into())
        );
        assert_eq!(
            parse(serde_json::json!({"n": 0, "s": 42})).s,
            Some("42".into())
        );
    }

    #[test]
    fn php_bools() {
        assert!(parse(serde_json::json!({"n": 0, "b": true})).b);
        assert!(parse(serde_json::json!({"n": 0, "b": 1})).b);
        assert!(parse(serde_json::json!({"n": 0, "b": "1"})).b);
        assert!(parse(serde_json::json!({"n": 0, "b": "true"})).b);
        assert!(!parse(serde_json::json!({"n": 0, "b": 0})).b);
        assert!(!parse(serde_json::json!({"n": 0, "b": null})).b);
    }
}
