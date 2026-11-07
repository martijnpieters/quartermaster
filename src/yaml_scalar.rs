use serde_yaml::Value;

/// A YAML node as the text Ruby's `to_s` would give it.
pub fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        other => serde_yaml::to_string(other).unwrap_or_default().trim().to_string(),
    }
}

/// `Array(value)`: a sequence's items, a scalar as one item, null as none.
pub fn array(v: Option<&Value>) -> Vec<&Value> {
    match v {
        Some(Value::Sequence(items)) => items.iter().collect(),
        Some(Value::Null) | None => vec![],
        Some(other) => vec![other],
    }
}

pub fn get<'a>(map: &'a serde_yaml::Mapping, key: &str) -> Option<&'a Value> {
    match map.get(key) {
        Some(Value::Null) | None => None,
        Some(v) => Some(v),
    }
}

/// A non-blank string value, stripped.
pub fn string(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) if !crate::ruby_text::strip(s).is_empty() => Some(crate::ruby_text::strip(s).to_string()),
        _ => None,
    }
}
