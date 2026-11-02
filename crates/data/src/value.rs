//! Attribute values and field definitions.

use crate::date::{Date, DateTime};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldType {
    Integer,
    Double,
    Text,
    Boolean,
    /// A calendar day.
    Date,
    /// A moment: a date and time, with or without a zone.
    DateTime,
}

impl FieldType {
    pub fn label(self) -> &'static str {
        match self {
            FieldType::Integer => "Long",
            FieldType::Double => "Double",
            FieldType::Text => "Text",
            FieldType::Boolean => "Boolean",
            FieldType::Date => "Date",
            FieldType::DateTime => "DateTime",
        }
    }
    pub fn is_numeric(self) -> bool {
        matches!(self, FieldType::Integer | FieldType::Double)
    }
    /// Whether it holds dates or date-times.
    pub fn is_temporal(self) -> bool {
        matches!(self, FieldType::Date | FieldType::DateTime)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub ty: FieldType,
    /// Display name (the field alias).
    #[serde(default)]
    pub alias: Option<String>,
}

impl Field {
    pub fn new(name: impl Into<String>, ty: FieldType) -> Self {
        Field { name: name.into(), ty, alias: None }
    }
    pub fn display(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Value {
    #[default]
    Null,
    Int(i64),
    Float(f64),
    Text(String),
    Bool(bool),
    Date(Date),
    DateTime(DateTime),
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) if f.is_finite() => Some(*f),
            Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Value::Text(s) => s.trim().parse::<f64>().ok().filter(|f| f.is_finite()),
            _ => None,
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
    /// How a date or date-time compares: microseconds since 1970, a date at its midnight and a
    /// time without a zone as if it were UTC. `None` for other values.
    pub fn temporal_key(&self) -> Option<i64> {
        match self {
            Value::Date(d) => Some(d.sort_key()),
            Value::DateTime(t) => Some(t.sort_key()),
            _ => None,
        }
    }
    /// The comparison keys of two values when one is a date or date-time and the other is one
    /// too, or text that reads as one (`'2026-01-01'`, `'2026-01-01T12:00Z'`): how
    /// `issued >= '2026-01-01'` compares dates. `None` otherwise, so text that isn't a full date
    /// (`'2026'`, `''`) compares as text.
    pub fn temporal_keys(&self, other: &Value) -> Option<(i64, i64)> {
        let key = |v: &Value| match v {
            Value::Text(s) => DateTime::parse(s).map(DateTime::sort_key),
            v => v.temporal_key(),
        };
        if self.temporal_key().is_none() && other.temporal_key().is_none() {
            return None;
        }
        Some((key(self)?, key(other)?))
    }
    /// Converts a JSON value to an attribute value.
    pub fn from_json(v: &serde_json::Value) -> Value {
        match v {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::Bool(*b),
            serde_json::Value::Number(n) => match n.as_i64() {
                Some(i) => Value::Int(i),
                None => n.as_f64().map(Value::Float).unwrap_or(Value::Null),
            },
            serde_json::Value::String(s) => Value::Text(s.clone()),
            other => Value::Text(other.to_string()),
        }
    }
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Value::Null => serde_json::Value::Null,
            Value::Int(i) => (*i).into(),
            Value::Float(f) => serde_json::Number::from_f64(*f).map(serde_json::Value::Number).unwrap_or(serde_json::Value::Null),
            Value::Text(s) => s.clone().into(),
            Value::Bool(b) => (*b).into(),
            Value::Date(_) | Value::DateTime(_) => self.to_string().into(),
        }
    }
    /// Parses user text into a value of `ty` (empty → Null). Dates are ISO 8601: `YYYY-MM-DD`
    /// for a date; that, a time and an optional zone for a date-time. Text that isn't one is Null.
    pub fn parse(text: &str, ty: FieldType) -> Value {
        let t = text.trim();
        if t.is_empty() || t.eq_ignore_ascii_case("<null>") {
            return Value::Null;
        }
        match ty {
            FieldType::Integer => {
                t.parse::<i64>().map(Value::Int).or_else(|_| t.parse::<f64>().map(|f| Value::Int(f as i64))).unwrap_or(Value::Null)
            }
            FieldType::Double => t.parse::<f64>().map(Value::Float).unwrap_or(Value::Null),
            FieldType::Boolean => Value::Bool(matches!(t.to_ascii_lowercase().as_str(), "true" | "yes" | "1" | "y")),
            FieldType::Text => Value::Text(t.to_string()),
            FieldType::Date => Date::parse(t).map(Value::Date).unwrap_or(Value::Null),
            FieldType::DateTime => DateTime::parse(t).map(Value::DateTime).unwrap_or(Value::Null),
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Null => write!(f, "<Null>"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(v) => {
                if v.fract() == 0.0 && v.abs() < 1e15 {
                    write!(f, "{v:.0}")
                } else {
                    write!(f, "{}", (v * 1e6).round() / 1e6)
                }
            }
            Value::Text(s) => write!(f, "{s}"),
            Value::Bool(b) => write!(f, "{}", if *b { "True" } else { "False" }),
            Value::Date(d) => write!(f, "{d}"),
            Value::DateTime(t) => write!(f, "{t}"),
        }
    }
}

impl PartialOrd for Value {
    /// Dates by instant (also against date text), numbers as numbers, nulls first, then text
    /// ignoring case.
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        use std::cmp::Ordering;
        if let Some((a, b)) = self.temporal_keys(other) {
            return Some(a.cmp(&b));
        }
        match (self.as_f64(), other.as_f64()) {
            (Some(a), Some(b)) if !matches!(self, Value::Text(_)) || !matches!(other, Value::Text(_)) => a.partial_cmp(&b),
            _ => match (self, other) {
                (Value::Null, Value::Null) => Some(Ordering::Equal),
                (Value::Null, _) => Some(Ordering::Less),
                (_, Value::Null) => Some(Ordering::Greater),
                _ => Some(self.to_string().to_lowercase().cmp(&other.to_string().to_lowercase())),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_stay_small() {
        assert_eq!(std::mem::size_of::<Value>(), 24);
    }

    #[test]
    fn dates_compare_with_dates_and_date_text() {
        let d = Value::parse("2026-03-05", FieldType::Date);
        let t = Value::parse("2026-03-05T00:00:00+01:00", FieldType::DateTime);
        assert_eq!(d.to_string(), "2026-03-05");
        assert!(t < d, "an hour before UTC midnight");
        assert!(d > Value::Text("2026-01-01".into()));
        assert!(d < Value::Text("2026-03-05T00:00:01".into()));
        // Text that isn't a full date compares as text.
        assert_eq!(d.temporal_keys(&Value::Text("2026".into())), None);
        assert!(d > Value::Text("2026".into()));
        assert!(d > Value::Null);
        assert_eq!(Value::parse("2026-02-30", FieldType::Date), Value::Null);
        assert_eq!(d.to_json(), serde_json::json!("2026-03-05"));
    }

    #[test]
    fn project_files_hold_dates_as_iso_text() {
        let vals = vec![Value::parse("2026-03-05", FieldType::Date), Value::parse("2026-03-05T14:30:00+02:00", FieldType::DateTime)];
        let json = serde_json::to_string(&vals).unwrap_or_default();
        assert_eq!(json, r#"[{"Date":"2026-03-05"},{"DateTime":"2026-03-05T14:30:00+02:00"}]"#);
        assert_eq!(serde_json::from_str::<Vec<Value>>(&json).ok(), Some(vals));
        assert!(serde_json::from_str::<Value>(r#"{"Date":"2026-02-30"}"#).is_err());
    }
}
