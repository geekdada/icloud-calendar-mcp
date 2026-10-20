//! Reading loosely typed JSON-RPC tool arguments.
//!
//! Scalars are read the way the Kotlin server read them: a string is taken
//! as-is, and a number or boolean by its JSON text. `null` counts as absent.
//! A value of the wrong shape (an object where a string belongs) is reported
//! as a validation error rather than silently ignored.

use serde_json::{Map, Value};

use crate::ics::AlarmSpec;
use crate::validation::AlarmInput;

pub type Args = Map<String, Value>;

/// A scalar argument as text.
pub fn string(args: &Args, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => scalar_text(value).map(Some).ok_or_else(|| format!("{key} must be a string")),
    }
}

/// A boolean argument; only `true`/`false` (or those strings) count.
pub fn boolean(args: &Args, key: &str) -> Option<bool> {
    match args.get(key)? {
        Value::Bool(b) => Some(*b),
        Value::String(s) if s == "true" => Some(true),
        Value::String(s) if s == "false" => Some(false),
        _ => None,
    }
}

/// An array of scalars; `null` elements are dropped.
pub fn string_list(args: &Args, key: &str) -> Result<Option<Vec<String>>, String> {
    let items = match args.get(key) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Array(items)) => items,
        Some(_) => return Err(format!("{key} must be an array of strings")),
    };
    items
        .iter()
        .filter(|item| !item.is_null())
        .map(|item| scalar_text(item).ok_or_else(|| format!("{key} must be an array of strings")))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// The `alarms` array. `None` when absent (keep existing alarms on update);
/// `Some(vec![])` when given as `[]` (clear them). Non-object entries are skipped.
pub fn alarms(args: &Args, key: &str) -> Result<Option<Vec<AlarmInput>>, String> {
    let items = match args.get(key) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Array(items)) => items,
        Some(_) => return Err(format!("{key} must be an array of alarm objects")),
    };
    items
        .iter()
        .filter_map(Value::as_object)
        .enumerate()
        .map(|(i, entry)| {
            let field = |name: &str| -> Result<Option<String>, String> {
                match entry.get(name) {
                    None | Some(Value::Null) => Ok(None),
                    Some(value) => scalar_text(value).map(Some).ok_or_else(|| format!("{key}[{i}].{name} must be a string")),
                }
            };
            Ok(AlarmInput {
                trigger: field("trigger")?,
                action: field("action")?,
                description: field("description")?,
                summary: field("summary")?,
                repeat_count: entry.get("repeat_count").and_then(integer),
                repeat_duration: field("repeat_duration")?,
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// Validated alarm inputs as builder specs. Callers validate first, so a
/// missing trigger cannot occur here; it maps to an empty trigger defensively.
pub fn alarm_specs(inputs: Option<&[AlarmInput]>) -> Option<Vec<AlarmSpec>> {
    inputs.map(|list| {
        list.iter()
            .map(|a| AlarmSpec {
                trigger: a.trigger.clone().unwrap_or_default(),
                action: a.action.clone(),
                description: a.description.clone(),
                summary: a.summary.clone(),
                repeat_count: a.repeat_count.and_then(|c| i32::try_from(c).ok()),
                repeat_duration: a.repeat_duration.clone(),
            })
            .collect()
    })
}

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn integer(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(value: Value) -> Args {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn scalars_read_as_text() {
        let a = args(json!({"s": "x", "n": 5, "b": true, "z": null, "o": {}}));
        assert_eq!(string(&a, "s").unwrap().as_deref(), Some("x"));
        assert_eq!(string(&a, "n").unwrap().as_deref(), Some("5"));
        assert_eq!(string(&a, "b").unwrap().as_deref(), Some("true"));
        assert_eq!(string(&a, "z").unwrap(), None);
        assert_eq!(string(&a, "missing").unwrap(), None);
        assert!(string(&a, "o").is_err());
    }

    #[test]
    fn booleans_are_strict() {
        let a = args(json!({"t": true, "s": "true", "y": "yes", "n": 1}));
        assert_eq!(boolean(&a, "t"), Some(true));
        assert_eq!(boolean(&a, "s"), Some(true));
        assert_eq!(boolean(&a, "y"), None);
        assert_eq!(boolean(&a, "n"), None);
    }

    #[test]
    fn lists_and_alarms() {
        let a = args(json!({
            "l": ["a", null, 3],
            "bad": "x",
            "alarms": [{"trigger": "-PT15M", "repeat_count": "2"}, "skip", {"trigger": "-PT5M", "repeat_count": 3}],
            "empty": []
        }));
        assert_eq!(string_list(&a, "l").unwrap(), Some(vec!["a".into(), "3".into()]));
        assert!(string_list(&a, "bad").is_err());
        let alarms = alarms(&a, "alarms").unwrap().unwrap();
        assert_eq!(alarms.len(), 2);
        assert_eq!(alarms[0].repeat_count, Some(2));
        assert_eq!(alarms[1].repeat_count, Some(3));
        assert_eq!(super::alarms(&a, "empty").unwrap(), Some(vec![]));
        assert_eq!(super::alarms(&a, "missing").unwrap(), None);
    }
}
