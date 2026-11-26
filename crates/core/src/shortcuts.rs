//! Mod-Tastenkürzel und Schnellaktionen in den Einstellungen, übertragen aus src/features/moderation/settings.ts,
//! src/features/hotkeys/settings.ts und src/features/settings/duration-input.tsx.

use crate::i18n::tr;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::hotkeys::{hotkey_actions, mod_key_of};
use crate::persistence::shortcut_signature;
use crate::settings::MAX_TIMEOUT_SECONDS;

/// Ein Mod-Tastenkürzel (Einstellung moderation.shortcutList).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutEntry {
    pub id: String,
    pub key: Option<String>,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ShortcutEntry {
    pub fn signature(&self) -> String {
        shortcut_signature(&self.action, self.duration_seconds.map(|d| d as f64), self.reason.as_deref())
    }
}

fn action_order(action: &str) -> i32 {
    match action {
        "delete" => 0,
        "warn" => 1,
        "timeout" => 2,
        "ban" => 3,
        "unban" => 4,
        _ => 5,
    }
}

/// Feste Reihenfolge: Löschen, Verwarnen, Timeouts nach Dauer aufsteigend, Ban, Unban; Einträge ohne Taste ganz am
/// Ende. Sonst bleibt die bisherige Reihenfolge (stabil).
pub fn sort_entries(entries: &[ShortcutEntry]) -> Vec<ShortcutEntry> {
    let mut sorted = entries.to_vec();
    sorted.sort_by_key(|e| match e.key {
        None => (1, 0, 0),
        Some(_) => (0, action_order(&e.action), e.duration_seconds.unwrap_or(0)),
    });
    sorted
}

/// Gruppen in fester Reihenfolge: Verwarnungen, Timeouts, Weiteres, noch ohne Taste. Leere Gruppen entfallen.
pub fn group_entries(entries: &[ShortcutEntry]) -> Vec<(&'static str, &'static str, Vec<ShortcutEntry>)> {
    let groups = [("warn", tr("Verwarnungen", "Warnings")), ("timeout", "Timeouts"), ("other", tr("Weiteres", "More")), ("unassigned", tr("Neu, noch ohne Taste", "New, no key yet"))];
    let group_of = |e: &ShortcutEntry| match (&e.key, e.action.as_str()) {
        (None, _) => "unassigned",
        (_, "warn") => "warn",
        (_, "timeout") => "timeout",
        _ => "other",
    };
    let sorted = sort_entries(entries);
    groups
        .into_iter()
        .map(|(id, title)| (id, title, sorted.iter().filter(|e| group_of(e) == id).cloned().collect::<Vec<_>>()))
        .filter(|(_, _, list)| !list.is_empty())
        .collect()
}

/// Konflikt mit einem anderen belegten Eintrag: dieselbe Taste ("key") oder dieselbe Aktion ("action").
pub fn find_clash<'a>(entries: &'a [ShortcutEntry], candidate: &ShortcutEntry) -> Option<(&'static str, &'a ShortcutEntry)> {
    let key = candidate.key.as_ref()?;
    let others: Vec<&ShortcutEntry> = entries.iter().filter(|e| e.id != candidate.id && e.key.is_some()).collect();
    if let Some(other) = others.iter().find(|e| e.key.as_ref() == Some(key)) {
        return Some(("key", other));
    }
    let signature = candidate.signature();
    others.into_iter().find(|e| e.signature() == signature).map(|other| ("action", other))
}

/// Allgemeines Tastenkürzel, das dieselbe Taste wie ein Mod-Tastenkürzel nutzt.
pub fn hotkey_using_mod_key(hotkeys: &BTreeMap<String, Option<String>>, key: &str) -> Option<&'static str> {
    hotkey_actions()
        .iter()
        .find(|(id, _)| hotkeys.get(*id).and_then(Option::as_deref).and_then(mod_key_of).as_deref() == Some(key))
        .map(|(id, _)| *id)
}

/// Nächster freier Standard-Hotkey 1 bis 9 für eine neue Vorlage.
pub fn next_free_hotkey<'a>(used: impl IntoIterator<Item = Option<&'a str>>, no_reason_hotkey: &str) -> Option<String> {
    let used: Vec<Option<&str>> = used.into_iter().collect();
    (1..=9).map(|d| d.to_string()).find(|d| d != no_reason_hotkey && !used.contains(&Some(d.as_str())))
}

/// Einheiten für Timeout-Dauern.
pub fn duration_units() -> [(&'static str, &'static str, i64); 5] {
    [("s", tr("Sekunden", "Seconds"), 1), ("m", tr("Minuten", "Minutes"), 60), ("h", tr("Stunden", "Hours"), 3_600), ("d", tr("Tage", "Days"), 86_400), ("w", tr("Wochen", "Weeks"), 604_800)]
}

/// Zerlegt Sekunden in die größte glatt passende Einheit.
pub fn split_duration(seconds: i64) -> (i64, &'static str) {
    duration_units().iter().rev().find(|(_, _, size)| seconds % size == 0).map_or((seconds, "s"), |(id, _, size)| (seconds / size, *id))
}

/// Rechnet in Sekunden um; Fehlertext, wenn der Wert außerhalb 1 Sekunde bis 2 Wochen liegt.
pub fn to_seconds(amount: f64, unit: &str) -> Result<i64, &'static str> {
    let factor = duration_units().iter().find(|(id, _, _)| *id == unit).map_or(1, |(_, _, s)| *s);
    if amount.fract() != 0.0 || !amount.is_finite() || amount < 1.0 {
        return Err(tr("Bitte eine ganze Zahl ab 1 eingeben.", "Please enter a whole number from 1."));
    }
    let seconds = amount * factor as f64;
    if seconds > MAX_TIMEOUT_SECONDS as f64 {
        return Err(tr("Höchstens 2 Wochen.", "At most 2 weeks."));
    }
    Ok(seconds as i64)
}

/// Eintrag zur gedrückten Taste (KeyboardEvent.key); Einträge ohne Taste nie, reservierte Tasten nie.
pub fn entry_for_key<'a>(entries: &'a [ShortcutEntry], key: &str) -> Option<&'a ShortcutEntry> {
    let pressed = crate::hotkeys::normalize_shortcut_key(key)?;
    entries.iter().find(|entry| entry.key.as_deref() == Some(pressed.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, key: Option<&str>, action: &str, duration: Option<i64>) -> ShortcutEntry {
        ShortcutEntry { id: id.into(), key: key.map(Into::into), action: action.into(), duration_seconds: duration, reason: None }
    }

    #[test]
    fn reihenfolge_und_gruppen() {
        let list = vec![
            entry("u", Some("u"), "unban", None),
            entry("n", None, "ban", None),
            entry("t2", Some("x"), "timeout", Some(3600)),
            entry("t1", Some("t"), "timeout", Some(600)),
            entry("d", Some("d"), "delete", None),
        ];
        let ids: Vec<String> = sort_entries(&list).into_iter().map(|e| e.id).collect();
        assert_eq!(ids, vec!["d", "t1", "t2", "u", "n"]);
        let groups: Vec<&str> = group_entries(&list).iter().map(|(id, _, _)| *id).collect();
        assert_eq!(groups, vec!["timeout", "other", "unassigned"]);
    }

    #[test]
    fn konflikte_und_dauern() {
        let list = vec![entry("a", Some("t"), "timeout", Some(600)), entry("b", Some("b"), "ban", None)];
        assert_eq!(find_clash(&list, &entry("c", Some("t"), "ban", None)).map(|(k, e)| (k, e.id.clone())), Some(("key", "a".into())));
        assert_eq!(find_clash(&list, &entry("c", Some("x"), "ban", None)).map(|(k, _)| k), Some("action"));
        assert!(find_clash(&list, &entry("c", None, "ban", None)).is_none());
        assert_eq!(split_duration(7200), (2, "h"));
        assert_eq!(split_duration(90), (90, "s"));
        assert_eq!(to_seconds(2.0, "w"), Ok(1_209_600));
        assert_eq!(to_seconds(3.0, "w"), Err("Höchstens 2 Wochen."));
        assert_eq!(to_seconds(0.5, "m"), Err("Bitte eine ganze Zahl ab 1 eingeben."));
        assert_eq!(next_free_hotkey([Some("1"), None], "2").as_deref(), Some("3"));
    }

    #[test]
    fn entry_for_pressed_key() {
        let entry = |id: &str, key: Option<&str>, action: &str, duration: Option<i64>, reason: Option<&str>| ShortcutEntry {
            id: id.into(),
            key: key.map(str::to_string),
            action: action.into(),
            duration_seconds: duration,
            reason: reason.map(str::to_string),
        };
        let list = vec![
            entry("a", Some("d"), "delete", None, None),
            entry("b", Some("t"), "timeout", Some(600), None),
            entry("c", Some("F2"), "timeout", Some(60), None),
            entry("d", Some("Space"), "warn", None, Some("Bitte Deutsch")),
            entry("e", None, "ban", None, None),
        ];
        assert_eq!(entry_for_key(&list, "T").map(|e| e.id.as_str()), Some("b"));
        assert_eq!(entry_for_key(&list, "F2").and_then(|e| e.duration_seconds), Some(60));
        assert_eq!(entry_for_key(&list, " ").and_then(|e| e.reason.as_deref()), Some("Bitte Deutsch"));
        assert!(entry_for_key(&list, "x").is_none());
        assert!(entry_for_key(&list, "Escape").is_none());
    }
}
