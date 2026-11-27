//! Datenspeicher: Migrationen als Folge einfacher Schritte, die die IndexedDB-Schicht in `crates/wasm/src/db.rs` in der
//! versionchange-Transaktion ausführt. Regeln wie bisher:
//! - Versionen lückenlos aufsteigend ab 1. Bestehende Migrationen nie ändern, nur neue anhängen.
//! - Alle Migrationen eines Starts laufen in EINER Transaktion. Schlägt eine fehl, bleibt der alte Stand unverändert.
//! - Unlesbare alte Werte bleiben unangetastet (Umwandlung liefert None).
//!
//! Die Umwandlungen sind bewusst ohne Abhängigkeit auf die Fachmodule, damit die Migrationen unverändert bleiben.

use std::collections::{BTreeMap, HashSet};

use serde_json::{json, Map, Value};

pub const SETTINGS: &str = "settings";
pub const AUTH: &str = "auth";
pub const WORKSPACE: &str = "workspace";
pub const BACKUPS: &str = "backups";
pub const SOUNDS: &str = "sounds";

/// Einstellungen, die es nicht mehr gibt. Sie werden per Migration entfernt und in älteren Sicherungen beim
/// Einspielen übersprungen.
pub const REMOVED_SETTING_KEYS: [&str; 6] = [
    // Ersetzt durch chat.pauseMaxSeconds (15.11.2026).
    "chat.pauseDelay",
    // Ersetzt durch highlights.scope, ein Schalter im Einstellungsfenster statt pro Tab (15.11.2026).
    "highlights.channels",
    // Ersetzt durch notifications.termCategories, Ton und Blinken pro Kategorie (15.11.2026).
    "notifications.terms",
    // Ersetzt durch moderation.shortcutList, Liste mit beliebigen Tasten, Dauern und Gründen (15.11.2026).
    "moderation.shortcuts",
    // Ersetzt durch display.inputSize: Eingabefeld immer einzeilig, nur kleiner einstellbar (15.11.2026).
    "display.inputRows",
    // Updates gibt es nur noch aus dem Versionsordner (27.11.2026).
    "updates.fromWebsite",
];

pub const LEGACY_SHORTCUTS_KEY: &str = "moderation.shortcuts";
pub const SHORTCUT_LIST_KEY: &str = "moderation.shortcutList";
const TERM_CATEGORIES_KEY: &str = "highlights.categories";
const COLORS_KEY: &str = "display.colors";
pub const LEGACY_TABS_KEY: &str = "tabs";
pub const LAYOUT_KEY: &str = "layout";
pub const MAIN_AREA_ID: &str = "main";
pub const MAIN_PAGE_ID: &str = "page-main";

/// Ein Schritt einer Migration.
#[derive(Debug, Clone, Copy)]
pub enum Step {
    CreateStore(&'static str),
    /// Löscht alle entfernten Einstellungen.
    DeleteRemovedSettings,
    /// Wandelt einen Wert um; None heißt: nichts ändern.
    Update { store: &'static str, key: &'static str, change: fn(&Value) -> Option<Value> },
    /// Schreibt die Umwandlung von `from` unter `to`, wenn es `to` noch nicht gibt (`from` bleibt).
    CopyIfMissing { store: &'static str, from: &'static str, to: &'static str, convert: fn(&Value) -> Option<Value> },
    /// Schreibt die Umwandlung von `from` unter `to` und löscht `from`; ist `from` unlesbar, bleibt alles liegen.
    Move { store: &'static str, from: &'static str, to: &'static str, convert: fn(&Value) -> Option<Value> },
}

pub struct Migration {
    pub version: u32,
    pub description: &'static str,
    pub steps: &'static [Step],
}

const fn migration(version: u32, description: &'static str, steps: &'static [Step]) -> Migration {
    Migration { version, description, steps }
}

pub const MIGRATIONS: [Migration; 20] = [
    migration(1, "Grundgerüst: Einstellungen als Schlüssel-Wert-Speicher", &[Step::CreateStore(SETTINGS)]),
    migration(2, "Anmeldedaten in eigenem Store (nie Teil der Sicherung)", &[Step::CreateStore(AUTH)]),
    migration(3, "Arbeitsfläche mit Tabs", &[Step::CreateStore(WORKSPACE)]),
    migration(4, "Automatische Sicherungen vor einem Import", &[Step::CreateStore(BACKUPS)]),
    migration(
        5,
        "Tab-Liste wird zum Layout mit Bereichen",
        // Unlesbare alte Daten bleiben unangetastet liegen; das Layout startet dann leer.
        &[Step::Move { store: WORKSPACE, from: LEGACY_TABS_KEY, to: LAYOUT_KEY, convert: workspace_from_legacy }],
    ),
    migration(6, "Entfernte Einstellungen löschen (Nachlaufzeit beim Pausieren)", &[Step::DeleteRemovedSettings]),
    migration(7, "Entfernte Einstellungen löschen (Markierungen pro Kanal)", &[Step::DeleteRemovedSettings]),
    migration(
        8,
        "Entfernte Einstellungen löschen (gemeinsamer Ton für Begriffe, jetzt pro Kategorie)",
        &[Step::DeleteRemovedSettings],
    ),
    migration(9, "Eigene Sounds in eigenem Store (Teil der Sicherung)", &[Step::CreateStore(SOUNDS)]),
    // Bestehende Daten bleiben gültig (nur Kanal-Tabs). Die neue Version sorgt dafür, dass ältere Programmstände ein
    // Layout mit Hervorhebungen-Tabs nicht falsch lesen, sondern „neuere Daten“ melden.
    migration(10, "Neue Tab-Art Hervorhebungen im Layout", &[]),
    migration(
        11,
        "Tastenkürzel als Liste (moderation.shortcuts wird zu moderation.shortcutList)",
        // Nur übernehmen, wenn es die neue Einstellung noch nicht gibt; unlesbare alte Werte entfallen (Standard gilt).
        &[
            Step::CopyIfMissing { store: SETTINGS, from: LEGACY_SHORTCUTS_KEY, to: SHORTCUT_LIST_KEY, convert: convert_legacy_shortcuts },
            Step::DeleteRemovedSettings,
        ],
    ),
    migration(
        12,
        "Tastenkürzel: jede Aktion nur eine Taste (doppelte verlieren ihre Taste)",
        &[Step::Update { store: SETTINGS, key: SHORTCUT_LIST_KEY, change: dedupe_shortcut_list }],
    ),
    // Wie Migration 10: ältere Programmstände melden bei Stream-Tabs „neuere Daten“.
    migration(13, "Neue Tab-Art Stream (Player) im Layout", &[]),
    migration(
        14,
        "Entfernte Einstellungen löschen (Eingabefeld-Höhe in Zeilen, jetzt nur kleiner einstellbar)",
        &[Step::DeleteRemovedSettings],
    ),
    migration(
        15,
        "Filter-Kategorien bekommen reguläre Ausdrücke (leere Liste für bestehende Kategorien)",
        &[Step::Update { store: SETTINGS, key: TERM_CATEGORIES_KEY, change: with_empty_patterns }],
    ),
    migration(
        16,
        "Filter-Kategorien: Schalter „Regex“ statt eigener regulärer Ausdrücke (Muster-Liste entfällt)",
        &[Step::Update { store: SETTINGS, key: TERM_CATEGORIES_KEY, change: with_regex_toggle }],
    ),
    // Wie Migration 10: ältere Programmstände verwerfen das Refresh-Token nicht, sondern melden „neuere Daten“.
    migration(17, "Anmeldung per Code: Konten können ein Refresh-Token haben", &[]),
    migration(
        18,
        "Filter haben nur noch ihre festen Farben (eigene Filterfarben unter Farben entfallen)",
        &[Step::Update { store: SETTINGS, key: COLORS_KEY, change: without_term_colors }],
    ),
    migration(
        19,
        "Arbeitsfläche mit Seiten oben, je Bereich genau ein Inhalt",
        &[Step::Update { store: WORKSPACE, key: LAYOUT_KEY, change: workspace_to_pages }],
    ),
    migration(20, "Entfernte Einstellungen löschen (Updates nur noch aus dem Versionsordner)", &[Step::DeleteRemovedSettings]),
];

/// Aktuelle Schema-Version.
pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

/// Prüft eine Liste von Versionen auf lückenlose, aufsteigende Nummern ab 1; Ergebnis ist die Zielversion.
pub fn check_versions(versions: &[u32]) -> Result<u32, String> {
    for (index, version) in versions.iter().enumerate() {
        if *version != index as u32 + 1 {
            return Err(format!("Migration an Position {index} hat Version {version}, erwartet {}.", index + 1));
        }
    }
    Ok(versions.len() as u32)
}

// ---- Tastenkürzel (Migrationen 11 und 12) ----

const LEGACY_ACTIONS: [&str; 4] = ["delete", "timeout", "ban", "unban"];
const MAX_TIMEOUT_SECONDS: f64 = 1_209_600.0;

/// Erste Fassung der Tastenkürzel (Version 0.53.0): feste Aktionen mit je einer Taste und einer Timeout-Dauer. Neue
/// Liste daraus; None, wenn der alte Wert fehlt oder unlesbar ist.
pub fn convert_legacy_shortcuts(value: &Value) -> Option<Value> {
    let enabled = value.get("enabled")?.as_bool()?;
    let timeout = value.get("timeoutSeconds")?.as_f64().filter(|t| t.fract() == 0.0 && (1.0..=MAX_TIMEOUT_SECONDS).contains(t))?;
    let keys = value.get("keys")?.as_object()?;
    let valid_key = |k: &Value| k.is_null() || k.as_str().is_some_and(|s| s.len() == 1 && s.chars().all(|c| c.is_ascii_digit() || c.is_ascii_lowercase()));
    if !LEGACY_ACTIONS.iter().all(|action| keys.get(*action).is_some_and(valid_key)) {
        return None;
    }
    let mut used = HashSet::new();
    let entries: Vec<Value> = LEGACY_ACTIONS
        .iter()
        .map(|action| {
            let raw = keys.get(*action).and_then(Value::as_str);
            // Doppelte Tasten (sollte es nicht geben) bleiben beim ersten Eintrag.
            let key = raw.filter(|k| used.insert(k.to_string()));
            if *action == "timeout" {
                json!({ "id": "timeout", "key": key, "action": action, "durationSeconds": number(timeout) })
            } else {
                json!({ "id": action, "key": key, "action": action })
            }
        })
        .collect();
    Some(json!({ "enabled": enabled, "entries": entries }))
}

/// Zahl als Text wie in JavaScript (ganze Zahlen ohne Nachkommastellen).
fn js_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e21 { format!("{value:.0}") } else { value.to_string() }
}

/// Zahl als JSON-Wert, ganze Zahlen als Ganzzahl (wie in JavaScript ohne Unterschied).
fn number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9e15 { json!(value as i64) } else { json!(value) }
}

/// Welche Tastenkürzel dieselbe Aktion sind (Jede Aktion nur eine Taste). Löschen, Ban und Unban je einmal, Timeouts
/// einmal je Dauer, Verwarnen einmal je Grund (ohne Leerraum am Rand, Groß- und Kleinschreibung egal).
pub fn shortcut_signature(action: &str, duration_seconds: Option<f64>, reason: Option<&str>) -> String {
    match action {
        "timeout" => format!("timeout:{}", js_number(duration_seconds.unwrap_or(0.0))),
        "warn" => format!("warn:{}", reason.unwrap_or("").trim().to_lowercase()),
        other => other.to_string(),
    }
}

/// Migration 12: Belegte Einträge mit derselben Aktion verlieren ab dem zweiten ihre Taste. Es geht kein Eintrag
/// verloren. Unlesbare Werte bleiben unverändert (None).
pub fn dedupe_shortcut_list(value: &Value) -> Option<Value> {
    let entries = value.as_object()?.get("entries")?.as_array()?;
    let mut seen = HashSet::new();
    let mut changed = false;
    let next: Vec<Value> = entries
        .iter()
        .map(|raw| {
            let Some(entry) = raw.as_object() else { return raw.clone() };
            let (Some(action), false) = (entry.get("action").and_then(Value::as_str), entry.get("key") == Some(&Value::Null))
            else {
                return raw.clone();
            };
            let signature = shortcut_signature(
                action,
                entry.get("durationSeconds").and_then(Value::as_f64),
                entry.get("reason").and_then(Value::as_str),
            );
            if seen.insert(signature) {
                return raw.clone();
            }
            changed = true;
            let mut entry = entry.clone();
            entry.insert("key".into(), Value::Null);
            Value::Object(entry)
        })
        .collect();
    if !changed {
        return None;
    }
    let mut list = value.as_object()?.clone();
    list.insert("entries".into(), Value::Array(next));
    Some(Value::Object(list))
}

// ---- Filter-Kategorien (Migrationen 15 und 16) ----

/// Ergänzt `patterns: []` bei jeder Kategorie ohne Muster-Liste. Unbekannte Formen bleiben unangetastet (None).
pub fn with_empty_patterns(raw: &Value) -> Option<Value> {
    let list = raw.as_array()?;
    let mut changed = false;
    let next = list
        .iter()
        .map(|entry| match entry.as_object() {
            Some(object) if !object.contains_key("patterns") => {
                changed = true;
                let mut object = object.clone();
                object.insert("patterns".into(), json!([]));
                Value::Object(object)
            }
            _ => entry.clone(),
        })
        .collect();
    changed.then_some(Value::Array(next))
}

/// Entfernt die Muster-Liste aus Migration 15 (Regex läuft automatisch, nur ein Schalter) und ergänzt `regex: false`.
/// Unbekannte Formen bleiben unangetastet (None).
pub fn with_regex_toggle(raw: &Value) -> Option<Value> {
    let list = raw.as_array()?;
    let mut changed = false;
    let next = list
        .iter()
        .map(|entry| match entry.as_object() {
            Some(object) if !(object.contains_key("regex") && !object.contains_key("patterns")) => {
                changed = true;
                let mut object = object.clone();
                object.remove("patterns");
                if !object.contains_key("regex") {
                    object.insert("regex".into(), Value::Bool(false));
                }
                Value::Object(object)
            }
            _ => entry.clone(),
        })
        .collect();
    changed.then_some(Value::Array(next))
}

/// Migration 18: entfernt eigene Farben der Filter (term-1 bis term-6) aus display.colors; Filter haben nur noch die
/// festen Farben aus theme.css.
pub fn without_term_colors(raw: &Value) -> Option<Value> {
    let object = raw.as_object()?;
    if !object.keys().any(|key| key.starts_with("term-")) {
        return None;
    }
    Some(Value::Object(object.iter().filter(|(key, _)| !key.starts_with("term-")).map(|(k, v)| (k.clone(), v.clone())).collect()))
}

// ---- Arbeitsfläche (Migration 5) ----

fn is_login(value: &Value) -> bool {
    value.as_str().is_some_and(|s| (1..=25).contains(&s.len()) && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
}

fn non_empty_string(value: Option<&Value>) -> Option<String> {
    value?.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

/// Eigener Name eines Tabs: Schlüssel muss da sein; null oder Text (ohne Leerraum am Rand 1 bis 50 Zeichen).
fn nickname(object: &Map<String, Value>) -> Option<Value> {
    match object.get("nickname")? {
        Value::Null => Some(Value::Null),
        Value::String(text) => {
            let trimmed = text.trim();
            let length = trimmed.encode_utf16().count();
            (1..=50).contains(&length).then(|| Value::String(trimmed.to_string()))
        }
        _ => None,
    }
}

fn all_or_list(value: Option<&Value>, max: usize, item: impl Fn(&Value) -> bool) -> Option<Value> {
    match value? {
        Value::String(s) if s == "all" => Some(json!("all")),
        Value::Array(list) if list.len() <= max && list.iter().all(item) => Some(Value::Array(list.clone())),
        _ => None,
    }
}

/// Prüft einen Tab und liefert ihn bereinigt (nur bekannte Felder, Name ohne Leerraum am Rand).
pub fn normalize_tab(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    let id = non_empty_string(object.get("id"))?;
    let kind = object.get("kind")?.as_str()?;
    let nickname = nickname(object)?;
    match kind {
        "channel" | "stream" => {
            if object.get("platform")?.as_str()? != "twitch" || !is_login(object.get("channel")?) {
                return None;
            }
            Some(json!({ "id": id, "kind": kind, "platform": "twitch", "channel": object["channel"], "nickname": nickname }))
        }
        "highlights" => {
            let show = object.get("show")?.as_object()?;
            let flag = |name: &str| show.get(name).and_then(Value::as_bool);
            let terms = all_or_list(show.get("terms"), 30, |v| v.as_str().is_some_and(|s| !s.is_empty()))?;
            let channels = all_or_list(show.get("channels"), 100, is_login)?;
            Some(json!({
                "id": id,
                "kind": "highlights",
                "nickname": nickname,
                "show": {
                    "terms": terms,
                    "mentions": flag("mentions")?,
                    "events": flag("events")?,
                    "modActions": flag("modActions")?,
                    "automod": flag("automod")?,
                    "channels": channels,
                },
                "counter": object.get("counter")?.as_bool()?,
            }))
        }
        _ => None,
    }
}

/// Wandelt die alte Tab-Liste (Schema-Version 3 und 4) in ein Layout mit einem Bereich um; None, wenn unlesbar.
pub fn workspace_from_legacy(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    let tabs: Vec<Value> = object.get("tabs")?.as_array()?.iter().map(normalize_tab).collect::<Option<_>>()?;
    let active = match object.get("activeTabId")? {
        Value::Null => None,
        Value::String(id) => Some(id.clone()),
        _ => return None,
    };
    let ids: Vec<&str> = tabs.iter().filter_map(|t| t["id"].as_str()).collect();
    let active = active.filter(|id| ids.contains(&id.as_str())).or_else(|| ids.first().map(|id| id.to_string()));
    Some(json!({
        "tabs": tabs,
        "root": { "type": "area", "id": MAIN_AREA_ID, "tabIds": ids, "activeTabId": active },
        "focusedAreaId": MAIN_AREA_ID,
    }))
}

/// Wandelt das Layout mit Tab-Gruppen je Bereich (bis Schema-Version 18) in Seiten um. Ein einzelner Bereich wird zu je
/// einer Seite pro Tab. Eine Teilung bleibt als erste Seite mit dem jeweils sichtbaren Tab je Bereich; die übrigen Tabs
/// bekommen eigene Seiten dahinter. None, wenn schon umgewandelt oder unlesbar.
pub fn workspace_to_pages(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    if object.contains_key("pages") {
        return None;
    }
    let tabs = object.get("tabs")?.as_array()?.clone();
    let root = object.get("root")?;
    let focused = object.get("focusedAreaId")?.as_str()?.to_string();
    fn kept(area: &Value) -> Option<String> {
        let ids: Vec<&str> = area["tabIds"].as_array()?.iter().filter_map(Value::as_str).collect();
        area["activeTabId"].as_str().filter(|a| ids.contains(a)).or(ids.first().copied()).map(str::to_string)
    }
    // Teilung: jeder Bereich behält den sichtbaren Tab, die anderen werden gesammelt.
    fn reduce(node: &Value, extra: &mut Vec<String>) -> Option<Value> {
        match node["type"].as_str()? {
            "area" => {
                let keep = kept(node);
                for id in node["tabIds"].as_array()?.iter().filter_map(Value::as_str) {
                    if Some(id) != keep.as_deref() {
                        extra.push(id.to_string());
                    }
                }
                let ids: Vec<&String> = keep.iter().collect();
                Some(json!({ "type": "area", "id": node["id"], "tabIds": ids, "activeTabId": keep }))
            }
            "split" => {
                let (first, second) = (reduce(&node["first"], extra)?, reduce(&node["second"], extra)?);
                let mut split = node.clone();
                split["first"] = first;
                split["second"] = second;
                Some(split)
            }
            _ => None,
        }
    }
    let single = |id: &str| json!({ "id": format!("page-{id}"), "name": null, "root": { "type": "area", "id": format!("area-{id}"), "tabIds": [id], "activeTabId": id } });
    let (pages, active_page, focused_area) = if root["type"] == "area" {
        let ids: Vec<&str> = root["tabIds"].as_array()?.iter().filter_map(Value::as_str).collect();
        if ids.is_empty() {
            let page = json!({ "id": MAIN_PAGE_ID, "name": null, "root": root });
            (vec![page], MAIN_PAGE_ID.to_string(), root["id"].as_str()?.to_string())
        } else {
            let active = kept(root)?;
            (ids.iter().map(|id| single(id)).collect(), format!("page-{active}"), format!("area-{active}"))
        }
    } else {
        let mut extra = Vec::new();
        let main = reduce(root, &mut extra)?;
        let mut pages = vec![json!({ "id": MAIN_PAGE_ID, "name": null, "root": main })];
        pages.extend(extra.iter().map(|id| single(id)));
        (pages, MAIN_PAGE_ID.to_string(), focused)
    };
    Some(json!({ "tabs": tabs, "pages": pages, "activePageId": active_page, "focusedAreaId": focused_area }))
}

// ---- Ausführen im Speicher (für Tests und als Vorlage der IndexedDB-Schicht) ----

/// Einfacher Speicher: Store → Schlüssel → Wert.
pub type MemoryDb = BTreeMap<String, BTreeMap<String, Value>>;

/// Führt die Migrationen nach `from` bis `to` im Speicher aus (gleiche Regeln wie in IndexedDB).
pub fn migrate_in_memory(db: &mut MemoryDb, from: u32, to: u32) -> Result<(), String> {
    for migration in MIGRATIONS.iter().filter(|m| m.version > from && m.version <= to) {
        for step in migration.steps {
            apply_in_memory(db, step)?;
        }
    }
    Ok(())
}

fn apply_in_memory(db: &mut MemoryDb, step: &Step) -> Result<(), String> {
    fn store<'a>(db: &'a mut MemoryDb, name: &str) -> Result<&'a mut BTreeMap<String, Value>, String> {
        db.get_mut(name).ok_or_else(|| format!("Store {name} fehlt"))
    }
    match *step {
        Step::CreateStore(name) => {
            if db.insert(name.to_string(), BTreeMap::new()).is_some() {
                return Err(format!("Store {name} gibt es schon"));
            }
        }
        Step::DeleteRemovedSettings => {
            let settings = store(db, SETTINGS)?;
            for key in REMOVED_SETTING_KEYS {
                settings.remove(key);
            }
        }
        Step::Update { store: name, key, change } => {
            let s = store(db, name)?;
            if let Some(next) = s.get(key).and_then(change) {
                s.insert(key.to_string(), next);
            }
        }
        Step::CopyIfMissing { store: name, from, to, convert } => {
            let s = store(db, name)?;
            if let Some(next) = s.get(from).and_then(convert) {
                s.entry(to.to_string()).or_insert(next);
            }
        }
        Step::Move { store: name, from, to, convert } => {
            let s = store(db, name)?;
            if let Some(next) = s.get(from).and_then(convert) {
                s.insert(to.to_string(), next);
                s.remove(from);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versionen_lueckenlos() {
        let versions: Vec<u32> = MIGRATIONS.iter().map(|m| m.version).collect();
        assert_eq!(check_versions(&versions), Ok(20));
        assert!(check_versions(&[2]).is_err());
        assert_eq!(SCHEMA_VERSION, 20);
    }

    #[test]
    fn neue_datenbank_hat_alle_stores() {
        let mut db = MemoryDb::new();
        migrate_in_memory(&mut db, 0, SCHEMA_VERSION).unwrap();
        assert_eq!(db.keys().collect::<Vec<_>>(), vec!["auth", "backups", "settings", "sounds", "workspace"]);
    }

    #[test]
    fn alte_tab_liste_wird_layout() {
        let mut db = MemoryDb::new();
        migrate_in_memory(&mut db, 0, 4).unwrap();
        db.get_mut(WORKSPACE).unwrap().insert(
            LEGACY_TABS_KEY.into(),
            json!({ "tabs": [
                { "id": "a", "kind": "channel", "platform": "twitch", "channel": "kanal", "nickname": "  Mein Kanal ", "alt": 1 },
                { "id": "b", "kind": "channel", "platform": "twitch", "channel": "zwei", "nickname": null },
            ], "activeTabId": "weg" }),
        );
        migrate_in_memory(&mut db, 4, SCHEMA_VERSION).unwrap();
        let workspace = &db[WORKSPACE];
        assert!(!workspace.contains_key(LEGACY_TABS_KEY));
        let layout = &workspace[LAYOUT_KEY];
        // Nach Migration 19: je Tab eine Seite, der aktive Tab zeigt.
        assert_eq!(layout["pages"][0]["root"]["tabIds"], json!(["a"]));
        assert_eq!(layout["pages"][1]["root"]["tabIds"], json!(["b"]));
        assert_eq!(layout["activePageId"], json!("page-a"));
        assert!(crate::stored::named("workspace").unwrap().parse(layout).is_ok());
        assert_eq!(layout["tabs"][0]["nickname"], json!("Mein Kanal"));
        assert!(layout["tabs"][0].get("alt").is_none());
    }

    #[test]
    fn teilung_wird_seiten() {
        let tab = |id: &str| json!({ "id": id, "kind": "channel", "platform": "twitch", "channel": id, "nickname": null });
        let old = json!({
            "tabs": [tab("a"), tab("b"), tab("c"), tab("d")],
            "root": { "type": "split", "id": "s", "direction": "row", "ratio": 0.4,
                "first": { "type": "area", "id": "main", "tabIds": ["a", "b"], "activeTabId": "b" },
                "second": { "type": "area", "id": "zwei", "tabIds": ["c", "d"], "activeTabId": null } },
            "focusedAreaId": "zwei",
        });
        let new = workspace_to_pages(&old).unwrap();
        assert!(crate::stored::named("workspace").unwrap().parse(&new).is_ok(), "{new}");
        assert_eq!(new["pages"][0]["root"]["first"]["tabIds"], json!(["b"]));
        assert_eq!(new["pages"][0]["root"]["second"]["tabIds"], json!(["c"]));
        assert_eq!(new["pages"][0]["root"]["ratio"], json!(0.4));
        assert_eq!(new["pages"][1]["id"], json!("page-a"));
        assert_eq!(new["pages"][2]["id"], json!("page-d"));
        assert_eq!(new["focusedAreaId"], json!("zwei"));
        assert!(workspace_to_pages(&new).is_none());
        let empty = json!({ "tabs": [], "root": { "type": "area", "id": "main", "tabIds": [], "activeTabId": null }, "focusedAreaId": "main" });
        assert!(crate::stored::named("workspace").unwrap().parse(&workspace_to_pages(&empty).unwrap()).is_ok());
    }

    #[test]
    fn unlesbare_tab_liste_bleibt_liegen() {
        let mut db = MemoryDb::new();
        migrate_in_memory(&mut db, 0, 4).unwrap();
        db.get_mut(WORKSPACE).unwrap().insert(LEGACY_TABS_KEY.into(), json!({ "tabs": [{ "id": "a" }], "activeTabId": null }));
        migrate_in_memory(&mut db, 4, SCHEMA_VERSION).unwrap();
        assert!(db[WORKSPACE].contains_key(LEGACY_TABS_KEY));
        assert!(!db[WORKSPACE].contains_key(LAYOUT_KEY));
    }

    #[test]
    fn entfernte_einstellungen_und_tastenkuerzel() {
        let mut db = MemoryDb::new();
        migrate_in_memory(&mut db, 0, 10).unwrap();
        let settings = db.get_mut(SETTINGS).unwrap();
        settings.insert("chat.pauseDelay".into(), json!(3));
        settings.insert("chat.timeFormat".into(), json!("HH:MM:SS"));
        settings.insert(
            LEGACY_SHORTCUTS_KEY.into(),
            json!({ "enabled": true, "timeoutSeconds": 600, "keys": { "delete": "d", "timeout": "t", "ban": "t", "unban": null } }),
        );
        migrate_in_memory(&mut db, 10, SCHEMA_VERSION).unwrap();
        let settings = &db[SETTINGS];
        assert_eq!(settings.keys().collect::<Vec<_>>(), vec!["chat.timeFormat", SHORTCUT_LIST_KEY]);
        let list = &settings[SHORTCUT_LIST_KEY];
        assert_eq!(list["entries"][1], json!({ "id": "timeout", "key": "t", "action": "timeout", "durationSeconds": 600 }));
        // Doppelte Taste bleibt beim ersten Eintrag.
        assert_eq!(list["entries"][2]["key"], Value::Null);
    }

    #[test]
    fn doppelte_tastenkuerzel_verlieren_ihre_taste() {
        let list = json!({ "enabled": true, "entries": [
            { "id": "1", "key": "a", "action": "timeout", "durationSeconds": 60 },
            { "id": "2", "key": "b", "action": "timeout", "durationSeconds": 60 },
            { "id": "3", "key": "c", "action": "warn", "reason": " Spam " },
            { "id": "4", "key": "d", "action": "warn", "reason": "spam" },
            { "id": "5", "key": null, "action": "ban" },
            { "id": "6", "key": "e", "action": "ban" },
        ] });
        let next = dedupe_shortcut_list(&list).unwrap();
        let keys: Vec<&Value> = next["entries"].as_array().unwrap().iter().map(|e| &e["key"]).collect();
        assert_eq!(keys, vec![&json!("a"), &Value::Null, &json!("c"), &Value::Null, &Value::Null, &json!("e")]);
        assert!(dedupe_shortcut_list(&json!({ "entries": [{ "key": "a", "action": "ban" }] })).is_none());
    }

    #[test]
    fn filter_kategorien() {
        let old = json!([{ "id": "a", "terms": [] }, { "id": "b", "patterns": ["x"] }, 5]);
        let with_patterns = with_empty_patterns(&old).unwrap();
        assert_eq!(with_patterns[0]["patterns"], json!([]));
        let toggled = with_regex_toggle(&with_patterns).unwrap();
        assert_eq!(toggled[0], json!({ "id": "a", "terms": [], "regex": false }));
        assert_eq!(toggled[2], json!(5));
        assert!(with_regex_toggle(&toggled).is_none());
        assert!(with_empty_patterns(&json!("x")).is_none());
    }

    #[test]
    fn filterfarben_entfallen_unter_farben() {
        let old = json!({ "term-1": { "color": "#ff0000", "enabled": true }, "link": { "color": null, "enabled": false } });
        assert_eq!(without_term_colors(&old).unwrap(), json!({ "link": { "color": null, "enabled": false } }));
        assert!(without_term_colors(&json!({ "link": { "color": null, "enabled": false } })).is_none());
        assert!(without_term_colors(&json!("x")).is_none());
    }
}
