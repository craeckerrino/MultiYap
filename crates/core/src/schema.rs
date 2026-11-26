//! Prüfung von Werten gegen ein Schema, mit derselben Wirkung wie die bisherigen zod-Schemas, damit gespeicherte Werte
//! gültig bleiben:
//! - Objekte: unbekannte Felder fallen weg, fehlende Felder mit Standard bekommen ihn, optionale bleiben weg.
//! - Texte: `trim` schneidet Leerraum am Rand ab, bevor die Länge geprüft wird; Längen zählen wie in JavaScript
//!   (UTF-16-Einheiten).
//! - Zahlen: endlich; `int` heißt ganze Zahl im sicheren Bereich von JavaScript.
//! - `Refine`: zusätzliche Prüfung des fertigen Werts mit eigener Meldung.
//!
//! Ergebnis ist der bereinigte Wert oder eine Liste von Fehlern (Pfad und Meldung auf Deutsch).

use crate::i18n::{t, tr, Text};
use serde::Serialize;
use serde_json::{Map, Value};

/// Ein Fehler mit Pfad (Feldnamen und Positionen) und Meldung.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Issue {
    pub path: Vec<Value>,
    pub message: String,
}

pub type Check<T> = fn(&T) -> bool;

pub enum Schema {
    Bool,
    Number { int: bool, min: Option<f64>, max: Option<f64>, gt: Option<f64>, lt: Option<f64> },
    Text { trim: bool, min: Option<usize>, max: Option<usize>, check: Option<(Check<str>, Text)> },
    /// Einer der festen Werte (Texte oder Zahlen).
    OneOf(Vec<Value>),
    Array { item: Box<Schema>, min: Option<usize>, max: Option<usize> },
    Object(Vec<Field>),
    /// Objekt mit beliebigen Schlüsseln (geprüft) und Werten nach Schema. `allowed`: nur diese Schlüssel.
    Record { key: Box<Schema>, value: Box<Schema> },
    Nullable(Box<Schema>),
    /// Erstes passendes Schema.
    Union(Vec<Schema>),
    Refine(Box<Schema>, Check<Value>, Text),
    /// Prüfung des fertigen Werts, die je Fehler eine eigene Meldung liefert (wie superRefine bei zod).
    Verify(Box<Schema>, fn(&Value) -> Option<&'static str>),
    /// Beliebiger Wert (unverändert).
    Any,
    /// Für verschachtelte Bäume: Schema wird bei Bedarf gebaut.
    Lazy(fn() -> Schema),
}

pub struct Field {
    pub name: &'static str,
    pub schema: Schema,
    pub optional: bool,
    pub default: Option<fn() -> Value>,
}

// ---- Bausteine ----

pub fn boolean() -> Schema {
    Schema::Bool
}

pub fn number() -> Schema {
    Schema::Number { int: false, min: None, max: None, gt: None, lt: None }
}

pub fn int(min: i64, max: i64) -> Schema {
    Schema::Number { int: true, min: Some(min as f64), max: Some(max as f64), gt: None, lt: None }
}

pub fn text() -> Schema {
    Schema::Text { trim: false, min: None, max: None, check: None }
}

/// Text mit Länge (nach JavaScript gezählt).
pub fn text_len(min: usize, max: usize) -> Schema {
    Schema::Text { trim: false, min: Some(min), max: Some(max), check: None }
}

/// Text ohne Leerraum am Rand, dann Länge.
pub fn trimmed(min: usize, max: usize) -> Schema {
    Schema::Text { trim: true, min: Some(min), max: Some(max), check: None }
}

pub fn non_empty() -> Schema {
    Schema::Text { trim: false, min: Some(1), max: None, check: None }
}

pub fn text_check(min: Option<usize>, max: Option<usize>, check: Check<str>, message: Text) -> Schema {
    Schema::Text { trim: false, min, max, check: Some((check, message)) }
}

pub fn one_of_texts(values: &[&str]) -> Schema {
    Schema::OneOf(values.iter().map(|v| Value::from(*v)).collect())
}

pub fn one_of_numbers(values: &[f64]) -> Schema {
    Schema::OneOf(values.iter().map(|v| number_value(*v)).collect())
}

pub fn array(item: Schema, max: usize) -> Schema {
    Schema::Array { item: Box::new(item), min: None, max: Some(max) }
}

pub fn object(fields: Vec<Field>) -> Schema {
    Schema::Object(fields)
}

pub fn record(key: Schema, value: Schema) -> Schema {
    Schema::Record { key: Box::new(key), value: Box::new(value) }
}

pub fn nullable(schema: Schema) -> Schema {
    Schema::Nullable(Box::new(schema))
}

pub fn refine(schema: Schema, check: Check<Value>, message: Text) -> Schema {
    Schema::Refine(Box::new(schema), check, message)
}

pub fn verify(schema: Schema, check: fn(&Value) -> Option<&'static str>) -> Schema {
    Schema::Verify(Box::new(schema), check)
}

pub fn any() -> Schema {
    Schema::Any
}

pub fn lazy(build: fn() -> Schema) -> Schema {
    Schema::Lazy(build)
}

/// Fester Text.
pub fn literal(value: &str) -> Schema {
    one_of_texts(&[value])
}

/// Liste ohne Höchstzahl.
pub fn list(item: Schema) -> Schema {
    Schema::Array { item: Box::new(item), min: None, max: None }
}

pub fn union(options: Vec<Schema>) -> Schema {
    Schema::Union(options)
}

pub fn field(name: &'static str, schema: Schema) -> Field {
    Field { name, schema, optional: false, default: None }
}

pub fn optional(name: &'static str, schema: Schema) -> Field {
    Field { name, schema, optional: true, default: None }
}

pub fn with_default(name: &'static str, schema: Schema, default: fn() -> Value) -> Field {
    Field { name, schema, optional: false, default: Some(default) }
}

/// Ganze Zahlen als Ganzzahl, sonst als Kommazahl (wie JavaScript ohne Unterschied).
pub fn number_value(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 { Value::from(value as i64) } else { Value::from(value) }
}

/// Länge wie `String.prototype.length` in JavaScript.
pub fn js_length(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Leerraum am Rand wie `String.prototype.trim` in JavaScript.
pub fn js_trim(text: &str) -> &str {
    let is_space = |c: char| c.is_whitespace() || c == '\u{feff}';
    text.trim_matches(is_space)
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => tr("Wahrheitswert", "boolean"),
        Value::Number(_) => tr("Zahl", "number"),
        Value::String(_) => tr("Text", "text"),
        Value::Array(_) => tr("Liste", "list"),
        Value::Object(_) => tr("Objekt", "object"),
    }
}

struct Context {
    path: Vec<Value>,
    issues: Vec<Issue>,
}

impl Context {
    fn fail(&mut self, message: impl Into<String>) {
        self.issues.push(Issue { path: self.path.clone(), message: message.into() });
    }
}

impl Schema {
    /// Prüft und bereinigt einen Wert. `None` (fehlender Wert) gilt als ungültig.
    pub fn parse(&self, value: &Value) -> Result<Value, Vec<Issue>> {
        let mut context = Context { path: Vec::new(), issues: Vec::new() };
        let result = self.run(value, &mut context);
        match result {
            Some(value) if context.issues.is_empty() => Ok(value),
            _ => {
                if context.issues.is_empty() {
                    context.fail(tr("Ungültiger Wert", "Invalid value"));
                }
                Err(context.issues)
            }
        }
    }

    pub fn is_valid(&self, value: &Value) -> bool {
        self.parse(value).is_ok()
    }

    fn run(&self, value: &Value, cx: &mut Context) -> Option<Value> {
        match self {
            Schema::Bool => match value {
                Value::Bool(_) => Some(value.clone()),
                other => {
                    cx.fail(crate::tf!("Erwartet Wahrheitswert, erhalten {}", "Expected boolean, got {}", kind(other)));
                    None
                }
            },
            Schema::Number { int, min, max, gt, lt } => {
                let Some(n) = value.as_f64().filter(|n| n.is_finite()) else {
                    cx.fail(crate::tf!("Erwartet Zahl, erhalten {}", "Expected number, got {}", kind(value)));
                    return None;
                };
                if *int && (n.fract() != 0.0 || n.abs() > 9_007_199_254_740_991.0) {
                    cx.fail(tr("Erwartet ganze Zahl", "Expected whole number"));
                    return None;
                }
                let ok = min.is_none_or(|m| n >= m) && max.is_none_or(|m| n <= m) && gt.is_none_or(|m| n > m) && lt.is_none_or(|m| n < m);
                if !ok {
                    cx.fail(tr("Zahl außerhalb des erlaubten Bereichs", "Number outside the allowed range"));
                    return None;
                }
                Some(number_value(n))
            }
            Schema::Text { trim, min, max, check } => {
                let Some(text) = value.as_str() else {
                    cx.fail(crate::tf!("Erwartet Text, erhalten {}", "Expected text, got {}", kind(value)));
                    return None;
                };
                let text = if *trim { js_trim(text) } else { text };
                let length = js_length(text);
                if min.is_some_and(|m| length < m) {
                    cx.fail(if *min == Some(1) { tr("Darf nicht leer sein", "Must not be empty").to_string() } else { crate::tf!("Mindestens {} Zeichen", "At least {} characters", min.unwrap_or(0)) });
                    return None;
                }
                if max.is_some_and(|m| length > m) {
                    cx.fail(crate::tf!("Höchstens {} Zeichen", "At most {} characters", max.unwrap_or(0)));
                    return None;
                }
                if let Some((check, message)) = check {
                    if !check(text) {
                        cx.fail(t(*message));
                        return None;
                    }
                }
                Some(Value::String(text.to_string()))
            }
            Schema::OneOf(values) => {
                let matches = values.iter().any(|allowed| match (allowed.as_f64(), value.as_f64()) {
                    (Some(a), Some(b)) => a == b,
                    _ => allowed == value,
                });
                if matches {
                    Some(value.as_f64().map_or_else(|| value.clone(), number_value))
                } else {
                    cx.fail(tr("Ungültige Auswahl", "Invalid choice"));
                    None
                }
            }
            Schema::Array { item, min, max } => {
                let Some(list) = value.as_array() else {
                    cx.fail(crate::tf!("Erwartet Liste, erhalten {}", "Expected list, got {}", kind(value)));
                    return None;
                };
                if min.is_some_and(|m| list.len() < m) || max.is_some_and(|m| list.len() > m) {
                    cx.fail(crate::tf!("Höchstens {} Einträge", "At most {} entries", max.unwrap_or(usize::MAX)));
                    return None;
                }
                let mut out = Vec::with_capacity(list.len());
                let mut ok = true;
                for (index, entry) in list.iter().enumerate() {
                    cx.path.push(Value::from(index));
                    match item.run(entry, cx) {
                        Some(parsed) => out.push(parsed),
                        None => ok = false,
                    }
                    cx.path.pop();
                }
                ok.then_some(Value::Array(out))
            }
            Schema::Object(fields) => {
                let Some(object) = value.as_object() else {
                    cx.fail(crate::tf!("Erwartet Objekt, erhalten {}", "Expected object, got {}", kind(value)));
                    return None;
                };
                let mut out = Map::new();
                let mut ok = true;
                for field in fields {
                    cx.path.push(Value::from(field.name));
                    match object.get(field.name) {
                        None => {
                            if let Some(default) = field.default {
                                out.insert(field.name.into(), default());
                            } else if !field.optional {
                                cx.fail(tr("Pflichtfeld fehlt", "Required field missing"));
                                ok = false;
                            }
                        }
                        Some(entry) => match field.schema.run(entry, cx) {
                            Some(parsed) => {
                                out.insert(field.name.into(), parsed);
                            }
                            None => ok = false,
                        },
                    }
                    cx.path.pop();
                }
                ok.then_some(Value::Object(out))
            }
            Schema::Record { key, value: value_schema } => {
                let Some(object) = value.as_object() else {
                    cx.fail(crate::tf!("Erwartet Objekt, erhalten {}", "Expected object, got {}", kind(value)));
                    return None;
                };
                let mut out = Map::new();
                let mut ok = true;
                for (name, entry) in object {
                    cx.path.push(Value::from(name.as_str()));
                    let parsed_key = key.run(&Value::from(name.as_str()), cx);
                    let parsed_value = value_schema.run(entry, cx);
                    match (parsed_key, parsed_value) {
                        (Some(Value::String(k)), Some(v)) => {
                            out.insert(k, v);
                        }
                        _ => ok = false,
                    }
                    cx.path.pop();
                }
                ok.then_some(Value::Object(out))
            }
            Schema::Nullable(inner) => {
                if value.is_null() {
                    Some(Value::Null)
                } else {
                    inner.run(value, cx)
                }
            }
            Schema::Union(options) => {
                for option in options {
                    let mut trial = Context { path: cx.path.clone(), issues: Vec::new() };
                    if let Some(parsed) = option.run(value, &mut trial) {
                        if trial.issues.is_empty() {
                            return Some(parsed);
                        }
                    }
                }
                cx.fail(tr("Ungültiger Wert", "Invalid value"));
                None
            }
            Schema::Verify(inner, check) => {
                let parsed = inner.run(value, cx)?;
                match check(&parsed) {
                    None => Some(parsed),
                    Some(message) => {
                        cx.fail(message);
                        None
                    }
                }
            }
            Schema::Any => Some(value.clone()),
            Schema::Lazy(build) => build().run(value, cx),
            Schema::Refine(inner, check, message) => {
                let parsed = inner.run(value, cx)?;
                if check(&parsed) {
                    Some(parsed)
                } else {
                    cx.fail(t(*message));
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn objekte_wie_zod() {
        let schema = object(vec![
            field("name", trimmed(1, 5)),
            optional("extra", int(1, 3)),
            with_default("regex", boolean(), || Value::Bool(false)),
            field("nick", nullable(text())),
        ]);
        assert_eq!(schema.parse(&json!({ "name": "  ab ", "nick": null, "fremd": 1 })), Ok(json!({ "name": "ab", "regex": false, "nick": null })));
        assert!(schema.parse(&json!({ "name": "   ", "nick": null })).is_err());
        assert!(schema.parse(&json!({ "name": "ab" })).is_err());
        let issues = schema.parse(&json!({ "name": "ab", "nick": null, "extra": 7 })).unwrap_err();
        assert_eq!(issues[0].path, vec![json!("extra")]);
    }

    #[test]
    fn laengen_wie_javascript() {
        // Ein Emoji zählt in JavaScript als zwei Zeichen.
        assert!(text_len(1, 1).parse(&json!("😀")).is_err());
        assert!(text_len(1, 2).parse(&json!("😀")).is_ok());
    }

    #[test]
    fn zahlen_listen_und_auswahl() {
        assert!(int(0, 100).parse(&json!(50.5)).is_err());
        assert_eq!(int(0, 100).parse(&json!(50.0)), Ok(json!(50)));
        assert!(one_of_numbers(&[0.0, 5.0]).parse(&json!(5)).is_ok());
        assert!(one_of_texts(&["a"]).parse(&json!("b")).is_err());
        assert!(array(boolean(), 1).parse(&json!([true, false])).is_err());
        let unique = refine(array(int(0, 9), 5), |v| v.as_array().is_some_and(|l| l.len() < 2), ("Zu viele", "Too many"));
        assert_eq!(unique.parse(&json!([1, 2])).unwrap_err()[0].message, "Zu viele");
    }
}
