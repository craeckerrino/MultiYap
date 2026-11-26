//! Sicherung:
//! - Export als Datei mit Schema-Version und Prüfsumme (SHA-256 über die Daten mit sortierten Schlüsseln). Zugangsdaten
//!   sind nie enthalten.
//! - Import: Datei vollständig prüfen (Größe, JSON, Format, Prüfsumme, Schema-Version, Bereiche), bevor etwas geändert
//!   wird. Danach prüft das Schema der jeweiligen Funktion jeden einzelnen Wert (`validate_entries` auch das in Rust).
//!   Lesen und Einspielen in einer Transaktion: crates/web/src/backup.rs.
//!
//! Die Prüfsumme muss Byte für Byte so entstehen wie bisher in JavaScript, sonst ließen sich ältere Sicherungen nicht
//! mehr einspielen: Schlüssel nach UTF-16 sortiert, Zahlen und Texte wie `JSON.stringify`.

use crate::i18n::tr;
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::notify::MAX_CUSTOM_SOUNDS;
use crate::persistence::{self as p, workspace_from_legacy, workspace_to_pages};
use crate::{settings, stored};

pub const BACKUP_FORMAT: &str = "multiyap-backup";
/// Platz für die eigenen Sounds (höchstens 10 mal 1 MB, als Base64 etwa ein Drittel größer).
pub const MAX_FILE_BYTES: usize = 30 * 1024 * 1024;
pub const MAX_AUTO_BACKUPS: usize = 5;
/// Bereiche einer Sicherung (alles außer Zugangsdaten und den Sicherungen selbst).
pub const BACKUP_STORES: [&str; 3] = [p::SETTINGS, p::WORKSPACE, p::SOUNDS];

/// Zahl als Text wie `Number.prototype.toString` in JavaScript (kürzeste Ziffernfolge, Exponent ab 1e21 und unter 1e-6).
pub fn js_number(value: f64) -> String {
    if value.is_nan() || value.is_infinite() {
        // JSON.stringify macht daraus null.
        return "null".into();
    }
    if value == 0.0 {
        return "0".into();
    }
    // Kürzeste Darstellung, die beim Einlesen wieder genau diese Zahl ergibt (wie in JavaScript).
    let formatted = format!("{:e}", value.abs());
    let (mantissa, exponent) = formatted.split_once('e').unwrap_or((&formatted, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let k = digits.len() as i32;
    let n = exponent + 1;
    let sign = if value < 0.0 { "-" } else { "" };
    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e = n - 1;
        let e_text = if e >= 0 { format!("+{e}") } else { e.to_string() };
        if k == 1 { format!("{digits}e{e_text}") } else { format!("{}.{}e{e_text}", &digits[..1], &digits[1..]) }
    };
    format!("{sign}{body}")
}

/// Text wie `JSON.stringify` in JavaScript.
pub fn js_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// JSON mit sortierten Schlüsseln, damit die Prüfsumme unabhängig von der Reihenfolge ist.
pub fn stable_stringify(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => js_number(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => js_string(s),
        Value::Array(list) => format!("[{}]", list.iter().map(stable_stringify).collect::<Vec<_>>().join(",")),
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            // Wie Array.prototype.sort in JavaScript: nach UTF-16-Einheiten.
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            let entries: Vec<String> = keys.iter().map(|k| format!("{}:{}", js_string(k), stable_stringify(&map[*k]))).collect();
            format!("{{{}}}", entries.join(","))
        }
    }
}

/// SHA-256 über Schema-Version und Daten (Kleinbuchstaben, 64 Zeichen).
pub fn checksum(schema_version: u32, data: &Value) -> String {
    let mut wrapper = Map::new();
    wrapper.insert("schemaVersion".into(), Value::from(schema_version));
    wrapper.insert("data".into(), data.clone());
    let digest = Sha256::digest(stable_stringify(&Value::Object(wrapper)).as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Ein Wert aus der Sicherung, noch ohne Prüfung gegen sein Schema.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Entry {
    pub store: String,
    pub key: String,
    pub value: Value,
}

/// Vorgeprüfte Sicherung.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prepared {
    pub created_at: String,
    pub app_version: String,
    pub schema_version: u32,
    pub entries: Vec<Entry>,
}

/// Ältere Sicherungen enthalten eine einfache Tab-Liste (Schema-Version bis 4) oder ein Layout mit Tab-Gruppen je
/// Bereich (bis 18): in das Layout mit Seiten umwandeln.
fn upgrade_legacy_data(mut data: Map<String, Value>) -> Map<String, Value> {
    if let Some(Value::Object(workspace)) = data.get_mut(p::WORKSPACE) {
        if workspace.contains_key(p::LEGACY_TABS_KEY) && !workspace.contains_key(p::LAYOUT_KEY) {
            if let Some(layout) = workspace.get(p::LEGACY_TABS_KEY).and_then(workspace_from_legacy) {
                workspace.remove(p::LEGACY_TABS_KEY);
                workspace.insert(p::LAYOUT_KEY.into(), layout);
            }
        }
        if let Some(pages) = workspace.get(p::LAYOUT_KEY).and_then(workspace_to_pages) {
            workspace.insert(p::LAYOUT_KEY.into(), pages);
        }
    }
    data
}

fn is_positive_int(value: &Value) -> Option<u32> {
    let n = value.as_f64()?;
    (n.fract() == 0.0 && n > 0.0 && n <= f64::from(u32::MAX)).then_some(n as u32)
}

/// Bereiche und Einträge einer Sicherung vorprüfen (ändert nichts). Fehler auf Deutsch.
pub fn prepare_data(schema_version: u32, data: &Value, current_schema_version: u32) -> Result<Vec<Entry>, Vec<String>> {
    if schema_version > current_schema_version {
        return Err(vec![tr("Die Sicherung stammt von einer neueren Version von MultiYap.", "The backup comes from a newer version of MultiYap.").into()]);
    }
    let Some(data) = data.as_object() else { return Err(vec![tr("Die Datei ist keine MultiYap-Sicherung oder unvollständig.", "The file is not a MultiYap backup or is incomplete.").into()]) };
    let mut errors = Vec::new();
    let mut entries = Vec::new();
    for (store, values) in upgrade_legacy_data(data.clone()) {
        if !BACKUP_STORES.contains(&store.as_str()) {
            errors.push(crate::tf!("Unbekannter Bereich „{store}“.", "Unknown section “{store}”."));
            continue;
        }
        let Value::Object(values) = values else {
            errors.push(crate::tf!("Unbekannter Bereich „{store}“.", "Unknown section “{store}”."));
            continue;
        };
        for (key, value) in values {
            // Einstellungen, die es nicht mehr gibt, aus älteren Sicherungen überspringen.
            if store == p::SETTINGS && p::REMOVED_SETTING_KEYS.contains(&key.as_str()) {
                continue;
            }
            entries.push(Entry { store: store.clone(), key, value });
        }
    }
    if errors.is_empty() { Ok(entries) } else { Err(errors) }
}

/// Liest und prüft eine Sicherungsdatei: Größe, JSON, Format, Prüfsumme, Schema-Version, Bereiche.
pub fn prepare_file(text: &str, current_schema_version: u32) -> Result<Prepared, Vec<String>> {
    if text.len() > MAX_FILE_BYTES {
        return Err(vec![tr("Die Datei ist zu groß für eine MultiYap-Sicherung.", "The file is too large for a MultiYap backup.").into()]);
    }
    let Ok(json) = serde_json::from_str::<Value>(text) else {
        return Err(vec![tr("Die Datei ist keine gültige JSON-Datei.", "The file is not a valid JSON file.").into()]);
    };
    let incomplete = || vec![tr("Die Datei ist keine MultiYap-Sicherung oder unvollständig.", "The file is not a MultiYap backup or is incomplete.").to_string()];
    let file = json.as_object().ok_or_else(incomplete)?;
    let text_field = |name: &str| file.get(name).and_then(Value::as_str).map(str::to_string);
    let format_ok = file.get("format").and_then(Value::as_str) == Some(BACKUP_FORMAT)
        && file.get("formatVersion").and_then(Value::as_f64) == Some(1.0);
    let schema_version = file.get("schemaVersion").and_then(is_positive_int);
    let checksum_text = text_field("checksum").filter(|c| c.len() == 64 && c.chars().all(|ch| ch.is_ascii_digit() || ('a'..='f').contains(&ch)));
    let data = file.get("data").filter(|d| d.as_object().is_some_and(|m| m.values().all(Value::is_object)));
    let (Some(app_version), Some(created_at), Some(schema_version), Some(expected), Some(data), true) =
        (text_field("appVersion"), text_field("createdAt"), schema_version, checksum_text, data, format_ok)
    else {
        return Err(incomplete());
    };
    if checksum(schema_version, data) != expected {
        return Err(vec![tr("Die Prüfsumme stimmt nicht. Die Datei ist beschädigt oder wurde verändert.", "The checksum does not match. The file is damaged or was changed.").into()]);
    }
    let entries = prepare_data(schema_version, data, current_schema_version)?;
    Ok(Prepared { created_at, app_version, schema_version, entries })
}

/// Erste Meldung eines Schemas.
fn first_issue(issues: &[crate::schema::Issue]) -> String {
    issues.first().map_or_else(|| tr("ungültig", "invalid").to_string(), |issue| issue.message.clone())
}

/// Prüft jeden vorgeprüften Wert gegen das Schema seiner Funktion: Einstellungen, das Layout der Arbeitsfläche und eigene
/// Sounds (ID gleich Schlüssel, höchstens `MAX_CUSTOM_SOUNDS`). Gibt die geprüften (bereinigten) Werte zurück.
pub fn validate_entries(entries: Vec<Entry>) -> Result<Vec<Entry>, Vec<String>> {
    let mut errors = Vec::new();
    let mut values = Vec::with_capacity(entries.len());
    for Entry { store, key, value } in entries {
        let parsed = match store.as_str() {
            p::SETTINGS if settings::find(&key).is_some() => settings::parse(&key, &value),
            p::WORKSPACE if key == p::LAYOUT_KEY => stored::named("workspace").map(|schema| schema.parse(&value)).unwrap_or(Err(Vec::new())),
            p::SOUNDS => stored::named("customSound").map(|schema| schema.parse(&value)).unwrap_or(Err(Vec::new())),
            _ => {
                errors.push(crate::tf!("Unbekannter Eintrag „{store}/{key}“.", "Unknown entry “{store}/{key}”."));
                continue;
            }
        };
        match parsed {
            Err(issues) => errors.push(crate::tf!("Ungültiger Wert „{store}/{key}“: {}", "Invalid value “{store}/{key}”: {}", first_issue(&issues))),
            Ok(value) if store == p::SOUNDS && value.get("id").and_then(Value::as_str) != Some(key.as_str()) => {
                errors.push(crate::tf!("Ungültiger Wert „{store}/{key}“: ID passt nicht zum Schlüssel", "Invalid value “{store}/{key}”: ID does not match the key"));
            }
            Ok(value) => values.push(Entry { store, key, value }),
        }
    }
    if values.iter().filter(|entry| entry.store == p::SOUNDS).count() > MAX_CUSTOM_SOUNDS {
        errors.push(crate::tf!("Die Sicherung enthält mehr als {MAX_CUSTOM_SOUNDS} eigene Sounds.", "The backup contains more than {MAX_CUSTOM_SOUNDS} custom sounds."));
    }
    if errors.is_empty() { Ok(values) } else { Err(errors) }
}

/// Sicherungsdatei des aktuellen Stands (`data`: Bereich → Schlüssel → Wert), mit Prüfsumme.
pub fn backup_file(app_version: &str, schema_version: u32, created_at: &str, data: Value) -> Value {
    let checksum = checksum(schema_version, &data);
    serde_json::json!({
        "format": BACKUP_FORMAT,
        "formatVersion": 1,
        "appVersion": app_version,
        "schemaVersion": schema_version,
        "createdAt": created_at,
        "data": data,
        "checksum": checksum,
    })
}

/// Dateiname der Sicherung aus lokalem Datum und Uhrzeit, zum Beispiel „multiyap-sicherung-2026-11-22-1930.json“.
pub fn file_name(year: u32, month: u32, day: u32, hour: u32, minute: u32) -> String {
    format!("multiyap-sicherung-{year}-{month:02}-{day:02}-{hour:02}{minute:02}.json")
}

/// Schlüssel der automatischen Sicherungen, die über die Höchstzahl hinaus gelöscht werden (die ältesten).
pub fn surplus_auto_backups(mut keys: Vec<f64>) -> Vec<f64> {
    keys.sort_by(|a, b| b.total_cmp(a));
    keys.into_iter().skip(MAX_AUTO_BACKUPS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sound(id: &str) -> Value {
        json!({ "id": id, "name": "Pling", "mimeType": "audio/wav", "size": 3, "data": "AQID", "createdAt": "2026-11-15T07:00:00.000Z" })
    }

    fn entry(store: &str, key: &str, value: Value) -> Entry {
        Entry { store: store.into(), key: key.into(), value }
    }

    #[test]
    fn werte_werden_gegen_ihr_schema_geprueft() {
        let layout = json!({ "tabs": [], "pages": [{ "id": "p", "name": null, "root": { "type": "area", "id": "main", "tabIds": [], "activeTabId": null } }], "activePageId": "p", "focusedAreaId": "main" });
        let ok = validate_entries(vec![
            entry("settings", "chat.timeFormat", json!("HH:MM:SS")),
            entry("workspace", "layout", layout),
            entry("sounds", "ton-1", sound("ton-1")),
        ])
        .unwrap();
        assert_eq!(ok.len(), 3);
        let errors = validate_entries(vec![
            entry("settings", "chat.timeFormat", json!("falsch")),
            entry("settings", "gibt.es.nicht", json!(1)),
            entry("workspace", "anders", json!({})),
        ])
        .unwrap_err();
        assert_eq!(errors.len(), 3);
        assert!(errors[0].starts_with("Ungültiger Wert „settings/chat.timeFormat“"));
        assert_eq!(errors[1], "Unbekannter Eintrag „settings/gibt.es.nicht“.");
    }

    #[test]
    fn sounds_mit_falschem_schluessel_und_zu_viele_sounds() {
        assert!(validate_entries(vec![entry("sounds", "anders", sound("ton-1"))]).unwrap_err()[0].contains("ID passt nicht"));
        let many: Vec<Entry> = (0..=MAX_CUSTOM_SOUNDS).map(|i| entry("sounds", &format!("s{i}"), sound(&format!("s{i}")))).collect();
        assert!(validate_entries(many).unwrap_err()[0].contains("mehr als 10 eigene Sounds"));
    }

    #[test]
    fn datei_name_und_automatische_sicherungen() {
        assert_eq!(file_name(2026, 11, 2, 9, 5), "multiyap-sicherung-2026-11-02-0905.json");
        assert_eq!(surplus_auto_backups(vec![1.0, 7.0, 3.0, 5.0, 2.0, 6.0, 4.0]), vec![2.0, 1.0]);
        let file = backup_file("0.84.54", 17, "2026-11-22T00:00:00.000Z", json!({ "settings": {}, "workspace": {}, "sounds": {} }));
        assert!(prepare_file(&file.to_string(), 17).is_ok());
    }

    /// Prüfwerte, mit dem bisherigen JavaScript-Code berechnet (16.11.2026).
    #[test]
    fn pruefsumme_wie_in_javascript() {
        let data: Value = serde_json::from_str(
            r#"{"settings":{"chat.timeFormat":"HH:MM:SS","b":0.30000000000000004,"a":[1,2.5,0,1e+21,1e-7,123456789012,true,null],"ü":"Grüße \"x\"\n\t\u0001\u007f/😀","Z":{}},"workspace":{"layout":{"tabs":[],"root":{"type":"area","id":"main","tabIds":[],"activeTabId":null},"focusedAreaId":"main"}},"sounds":{}}"#,
        )
        .unwrap();
        assert_eq!(checksum(17, &data), "d739a4c69b927066d91ce589f8bba917116f52a00201a90100748481a4a7ee7f");
        let old = json!({ "settings": { "chat.pauseDelay": 3, "chat.pauseMaxSeconds": 30 }, "workspace": {} });
        assert_eq!(checksum(5, &old), "a5c738734eac0e75d38118d1fe4a10eebcbb4a29ae231959ba4a5c7de1ef2684");
    }

    #[test]
    fn zahlen_wie_in_javascript() {
        let cases = [
            (1.0, "1"),
            (-0.0, "0"),
            (2.5, "2.5"),
            (0.1 + 0.2, "0.30000000000000004"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (1e-7, "1e-7"),
            (0.000001, "0.000001"),
            (123456789012.0, "123456789012"),
            (-1.5e-9, "-1.5e-9"),
            (5e-324, "5e-324"),
        ];
        for (value, expected) in cases {
            assert_eq!(js_number(value), expected, "{value}");
        }
    }

    fn file(schema_version: u32, data: Value) -> String {
        json!({
            "format": BACKUP_FORMAT, "formatVersion": 1, "appVersion": "0.20.0", "schemaVersion": schema_version,
            "createdAt": "2026-11-16T00:00:00.000Z", "data": data, "checksum": checksum(schema_version, &data),
        })
        .to_string()
    }

    #[test]
    fn aeltere_sicherung_ohne_entfernte_einstellungen() {
        let prepared = prepare_file(&file(5, json!({ "settings": { "chat.pauseDelay": 3, "chat.pauseMaxSeconds": 30 }, "workspace": {} })), 17).unwrap();
        let keys: Vec<&str> = prepared.entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, vec!["chat.pauseMaxSeconds"]);
    }

    #[test]
    fn alte_tab_liste_wird_layout() {
        let data = json!({ "workspace": { "tabs": { "tabs": [{ "id": "a", "kind": "channel", "platform": "twitch", "channel": "kanal", "nickname": null }], "activeTabId": "a" } } });
        let prepared = prepare_file(&file(4, data), 17).unwrap();
        assert_eq!(prepared.entries[0].key, "layout");
    }

    #[test]
    fn fehler_werden_erkannt() {
        assert_eq!(prepare_file("kein json", 17).unwrap_err()[0], "Die Datei ist keine gültige JSON-Datei.");
        assert!(prepare_file("{}", 17).unwrap_err()[0].contains("unvollständig"));
        let mut changed: Value = serde_json::from_str(&file(17, json!({ "settings": { "a": 1 } }))).unwrap();
        changed["data"]["settings"]["a"] = json!(2);
        assert!(prepare_file(&changed.to_string(), 17).unwrap_err()[0].contains("Prüfsumme"));
        assert!(prepare_file(&file(18, json!({})), 17).unwrap_err()[0].contains("neueren Version"));
        assert!(prepare_file(&file(17, json!({ "auth": { "x": 1 } })), 17).unwrap_err()[0].contains("Unbekannter Bereich"));
    }
}
