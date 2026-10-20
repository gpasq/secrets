//! Pure helpers (formatting, parsing, filtering) shared by the CLI and TUI.

use anyhow::{Result, bail};
use base64::Engine;
use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

use crate::aws::{SecretDetail, SecretSummary, SecretValue};

pub fn fmt_time(t: Option<DateTime<Utc>>) -> String {
    t.map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "-".into())
}

/// Pretty-print JSON objects/arrays; return anything else unchanged.
pub fn pretty_value(s: &str) -> String {
    match serde_json::from_str::<Value>(s) {
        Ok(v @ (Value::Object(_) | Value::Array(_))) => {
            serde_json::to_string_pretty(&v).unwrap_or_else(|_| s.to_string())
        }
        _ => s.to_string(),
    }
}

/// Pretty-printed JSON if `s` is a valid JSON object or array.
pub fn format_json(s: &str) -> Option<String> {
    match serde_json::from_str::<Value>(s) {
        Ok(v @ (Value::Object(_) | Value::Array(_))) => serde_json::to_string_pretty(&v).ok(),
        _ => None,
    }
}

/// Compact JSON (as the AWS console stores it) if `s` is a JSON object or array;
/// anything else is returned unchanged.
pub fn compact_if_json(s: &str) -> String {
    match serde_json::from_str::<Value>(s) {
        Ok(v @ (Value::Object(_) | Value::Array(_))) => {
            serde_json::to_string(&v).unwrap_or_else(|_| s.to_string())
        }
        _ => s.to_string(),
    }
}

/// The secret as a JSON key/value object, if it is one.
pub fn json_object(s: &str) -> Option<Map<String, Value>> {
    match serde_json::from_str::<Value>(s) {
        Ok(Value::Object(m)) => Some(m),
        _ => None,
    }
}

/// Render a JSON value for display in a key/value table (strings without quotes).
pub fn scalar_display(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Human-readable rendering of a revealed value.
pub fn display_value(v: &SecretValue) -> String {
    match v {
        SecretValue::Text(s) => pretty_value(s),
        SecretValue::Binary(b) => format!("<binary, {} bytes, base64>\n{}", b.len(), b64(b)),
    }
}

/// Case-insensitive match of every whitespace separated term against a secret's name,
/// description, and tags.
pub fn matches_query(s: &SecretSummary, query: &str) -> bool {
    let hay = {
        let mut h = s.name.to_lowercase();
        if let Some(d) = &s.description {
            h.push('\n');
            h.push_str(&d.to_lowercase());
        }
        for (k, v) in &s.tags {
            h.push('\n');
            h.push_str(&k.to_lowercase());
            h.push('=');
            h.push_str(&v.to_lowercase());
        }
        h
    };
    query
        .split_whitespace()
        .all(|term| hay.contains(&term.to_lowercase()))
}

pub fn validate_recovery_days(days: i64) -> Result<i64> {
    if !(7..=30).contains(&days) {
        bail!("recovery window must be between 7 and 30 days (got {days})");
    }
    Ok(days)
}

/// Label/value rows describing a secret's metadata (never its value).
pub fn detail_lines(d: &SecretDetail) -> Vec<(String, String)> {
    let mut out = vec![
        ("Name".into(), d.name.clone()),
        ("ARN".into(), d.arn.clone()),
        (
            "Description".into(),
            d.description.clone().unwrap_or_else(|| "-".into()),
        ),
        (
            "KMS key".into(),
            d.kms_key_id
                .clone()
                .unwrap_or_else(|| "aws/secretsmanager (default)".into()),
        ),
        ("Created".into(), fmt_time(d.created)),
        ("Last changed".into(), fmt_time(d.last_changed)),
        ("Last accessed".into(), fmt_time(d.last_accessed)),
    ];
    if let Some(del) = d.deleted {
        out.push(("Deletion date".into(), fmt_time(Some(del))));
    }
    out.push((
        "Rotation".into(),
        if d.rotation_enabled {
            let every = match (&d.rotation_schedule, d.rotation_days) {
                (Some(s), _) => s.clone(),
                (None, Some(n)) => format!("every {n} days"),
                _ => "enabled".into(),
            };
            match &d.rotation_window {
                Some(w) => format!("{every} (window {w})"),
                None => every,
            }
        } else {
            "disabled".into()
        },
    ));
    if d.rotation_enabled || d.rotation_lambda.is_some() {
        if let Some(l) = &d.rotation_lambda {
            out.push(("Rotation Lambda".into(), l.clone()));
        }
        out.push(("Last rotated".into(), fmt_time(d.last_rotated)));
        out.push(("Next rotation".into(), fmt_time(d.next_rotation)));
    }
    if let Some(s) = &d.owning_service {
        out.push(("Managed by".into(), s.clone()));
    }
    if let Some(r) = &d.primary_region {
        out.push(("Primary region".into(), r.clone()));
    }
    out
}

/// Compute the tag changes needed to go from `old` to `new`: (tags to set, keys to remove).
pub fn tag_diff(
    old: &[(String, String)],
    new: &[(String, String)],
) -> (Vec<(String, String)>, Vec<String>) {
    let remove = old
        .iter()
        .filter(|(k, _)| !new.iter().any(|(nk, _)| nk == k))
        .map(|(k, _)| k.clone())
        .collect();
    let set = new.iter().filter(|t| !old.contains(t)).cloned().collect();
    (set, remove)
}

/// Validate edited tag rows: keys are trimmed, blank rows dropped, duplicates rejected.
pub fn clean_tags(rows: &[(String, String)]) -> Result<Vec<(String, String)>> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (k, v) in rows {
        let k = k.trim();
        if k.is_empty() {
            if v.trim().is_empty() {
                continue;
            }
            bail!("a tag has a value but no key");
        }
        if out.iter().any(|(ok, _)| ok == k) {
            bail!("duplicate tag key {k:?}");
        }
        out.push((k.to_string(), v.trim().to_string()));
    }
    Ok(out)
}

/// One editable row of a JSON key/value secret. `original` keeps the original JSON value so
/// untouched numbers/booleans/objects are written back unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct KvRow {
    pub key: String,
    pub text: String,
    pub original: Option<Value>,
}

pub fn kv_rows(map: &Map<String, Value>) -> Vec<KvRow> {
    map.iter()
        .map(|(k, v)| KvRow {
            key: k.clone(),
            text: scalar_display(v),
            original: Some(v.clone()),
        })
        .collect()
}

/// Serialize edited rows back to a compact JSON object.
pub fn kv_to_json(rows: &[KvRow]) -> Result<String> {
    let mut map = Map::new();
    for r in rows {
        let key = r.key.trim();
        if key.is_empty() {
            if r.text.is_empty() {
                continue;
            }
            bail!("a value has no key");
        }
        if map.contains_key(key) {
            bail!("duplicate key {key:?}");
        }
        let value = match &r.original {
            Some(orig) if scalar_display(orig) == r.text => orig.clone(),
            _ => Value::String(r.text.clone()),
        };
        map.insert(key.to_string(), value);
    }
    Ok(serde_json::to_string(&Value::Object(map))?)
}

/// True if two secret strings are the same value (JSON compared structurally).
pub fn same_value(a: &str, b: &str) -> bool {
    match (
        serde_json::from_str::<Value>(a),
        serde_json::from_str::<Value>(b),
    ) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(name: &str, desc: Option<&str>, tags: &[(&str, &str)]) -> SecretSummary {
        SecretSummary {
            name: name.into(),
            description: desc.map(Into::into),
            last_changed: None,
            deleted: None,
            rotation_enabled: false,
            tags: tags
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn pretty_prints_only_json_containers() {
        assert_eq!(pretty_value(r#"{"a":"b"}"#), "{\n  \"a\": \"b\"\n}");
        assert_eq!(pretty_value("plain"), "plain");
        assert_eq!(pretty_value("42"), "42");
        assert!(json_object(r#"{"a":1}"#).is_some());
        assert!(json_object("[1]").is_none());
    }

    #[test]
    fn query_matching() {
        let s = summary(
            "prod/db/password",
            Some("Primary DB"),
            &[("team", "Payments")],
        );
        assert!(matches_query(&s, ""));
        assert!(matches_query(&s, "DB"));
        assert!(matches_query(&s, "prod primary"));
        assert!(matches_query(&s, "team=payments"));
        assert!(!matches_query(&s, "prod staging"));
    }

    #[test]
    fn display_binary() {
        let v = SecretValue::Binary(vec![0, 1, 2]);
        assert_eq!(display_value(&v), "<binary, 3 bytes, base64>\nAAEC");
    }

    #[test]
    fn recovery_window_bounds() {
        assert!(validate_recovery_days(7).is_ok());
        assert!(validate_recovery_days(30).is_ok());
        assert!(validate_recovery_days(6).is_err());
        assert!(validate_recovery_days(31).is_err());
    }
    #[test]
    fn tag_diffs() {
        let old = vec![("team".into(), "core".into()), ("old".into(), "x".into())];
        let new = vec![
            ("team".into(), "platform".into()),
            ("env".into(), "prod".into()),
        ];
        let (set, remove) = tag_diff(&old, &new);
        assert_eq!(remove, vec!["old".to_string()]);
        assert_eq!(set, new);
        assert_eq!(tag_diff(&old, &old), (vec![], vec![]));
    }

    #[test]
    fn tag_cleaning() {
        let rows = vec![(" a ".into(), " 1 ".into()), ("".into(), "".into())];
        assert_eq!(clean_tags(&rows).unwrap(), vec![("a".into(), "1".into())]);
        assert!(clean_tags(&[("".into(), "v".into())]).is_err());
        assert!(clean_tags(&[("a".into(), "1".into()), ("a".into(), "2".into())]).is_err());
    }

    #[test]
    fn kv_round_trip_preserves_types() {
        let map = json_object(r#"{"user":"u","port":5432,"tls":true}"#).unwrap();
        let mut rows = kv_rows(&map);
        assert_eq!(
            kv_to_json(&rows).unwrap(),
            r#"{"user":"u","port":5432,"tls":true}"#
        );
        rows[0].text = "admin".into();
        rows[1].text = "6543".into();
        rows.push(KvRow {
            key: "host".into(),
            text: "db".into(),
            original: None,
        });
        assert_eq!(
            kv_to_json(&rows).unwrap(),
            r#"{"user":"admin","port":"6543","tls":true,"host":"db"}"#
        );
        rows.push(KvRow {
            key: "user".into(),
            text: "x".into(),
            original: None,
        });
        assert!(kv_to_json(&rows).is_err());
    }

    #[test]
    fn json_formatting() {
        let src = r#"{"b":1,"a":[true,null]}"#;
        let pretty = format_json(src).unwrap();
        assert_eq!(
            pretty,
            "{\n  \"b\": 1,\n  \"a\": [\n    true,\n    null\n  ]\n}"
        );
        assert_eq!(compact_if_json(&pretty), src, "key order is preserved");
        assert_eq!(format_json("plain"), None);
        assert_eq!(format_json("42"), None);
        assert_eq!(compact_if_json("  not json "), "  not json ");
        assert_eq!(compact_if_json("{broken"), "{broken");
    }

    #[test]
    fn value_comparison() {
        assert!(same_value(r#"{"a": 1}"#, r#"{"a":1}"#));
        assert!(!same_value(r#"{"a":1}"#, r#"{"a":2}"#));
        assert!(same_value("x", "x"));
        assert!(!same_value("x", "x "));
    }
}
