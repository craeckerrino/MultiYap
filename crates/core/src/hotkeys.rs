//! Tasten und Tastenkürzel, übertragen aus src/features/hotkeys/settings.ts und src/features/moderation/settings.ts:
//! - Mod-Tastenkürzel: eine Taste, Anzeige und Beschreibung der Aktion.
//! - Allgemeine Tastenkürzel: Kombinationen mit Strg, Alt, Umschalt und Win, jede Taste nur einmal, auch nicht doppelt
//!   mit den Mod-Tastenkürzeln.

use crate::i18n::tr;
use std::collections::BTreeMap;

/// Diese Tasten bleiben frei: Bedienung von Fenstern und Fokus, reine Umschalttasten.
const RESERVED_KEYS: [&str; 17] = [
    "Escape", "Tab", "Enter", "Shift", "Control", "Alt", "AltGraph", "Meta", "OS", "CapsLock", "NumLock", "ScrollLock", "Fn", "Dead",
    "Unidentified", "Process", "ContextMenu",
];

/// Tastenname aus KeyboardEvent.key: Zeichen klein, Leertaste als „Space“, Sondertasten unverändert. None: nicht
/// belegbar.
pub fn normalize_shortcut_key(key: &str) -> Option<String> {
    if key == " " {
        return Some("Space".into());
    }
    if RESERVED_KEYS.contains(&key) {
        return None;
    }
    Some(if key.chars().count() == 1 { key.to_lowercase() } else { key.to_string() })
}

fn key_labels() -> [(&'static str, &'static str); 14] {
    [
    ("Space", tr("Leertaste", "Space")),
    ("Delete", tr("Entf", "Del")),
    ("Backspace", tr("Rücktaste", "Backspace")),
    ("Insert", tr("Einfg", "Ins")),
    ("Home", tr("Pos1", "Home")),
    ("End", tr("Ende", "End")),
    ("PageUp", tr("Bild auf", "Page Up")),
    ("PageDown", tr("Bild ab", "Page Down")),
    ("ArrowUp", tr("Pfeil hoch", "Arrow up")),
    ("ArrowDown", tr("Pfeil runter", "Arrow down")),
    ("ArrowLeft", tr("Pfeil links", "Arrow left")),
    ("ArrowRight", tr("Pfeil rechts", "Arrow right")),
    ("Pause", "Pause"),
    ("PrintScreen", tr("Druck", "Print")),
]
}

/// Anzeige einer Taste, zum Beispiel „T“, „F2“, „Entf“.
pub fn shortcut_key_label(key: &str) -> String {
    if let Some((_, label)) = key_labels().iter().find(|(k, _)| *k == key) {
        return label.to_string();
    }
    if key.chars().count() == 1 { key.to_uppercase() } else { key.to_string() }
}

/// Kurze Beschriftung einer Dauer: 30s, 10m, 1h, 1T, 1W (größte glatt passende Einheit; Englisch 1d, 1w).
pub fn format_duration(seconds: i64) -> String {
    for (size, label) in [(604_800, tr("W", "w")), (86_400, tr("T", "d")), (3_600, "h"), (60, "m")] {
        if seconds % size == 0 {
            return format!("{}{label}", seconds / size);
        }
    }
    format!("{seconds}s")
}

/// Lesbare Aktion eines Mod-Tastenkürzels, zum Beispiel „Timeout 10m“ oder „Verwarnen „Spam““.
pub fn describe_shortcut(action: &str, duration_seconds: Option<i64>, reason: Option<&str>) -> String {
    match action {
        "delete" => tr("Löschen", "Delete").into(),
        "warn" => match reason.map(str::trim).filter(|r| !r.is_empty()) {
            Some(reason) => crate::tf!("Verwarnen „{reason}“", "Warn “{reason}”"),
            None => tr("Verwarnen ohne Grund", "Warn without reason").into(),
        },
        "timeout" => format!("Timeout {}", format_duration(duration_seconds.unwrap_or(0))),
        "ban" => "Ban".into(),
        "unban" => "Unban".into(),
        other => other.into(),
    }
}

// ---- Allgemeine Tastenkürzel ----

pub fn hotkey_actions() -> [(&'static str, &'static str); 5] {
    [
    ("reconnectAll", tr("Alle Kanäle neu verbinden", "Reconnect all channels")),
    ("reloadEmotes", tr("Emotes neu laden", "Reload emotes")),
    ("newTab", tr("Neuer Tab", "New tab")),
    ("openSettings", tr("Einstellungen öffnen", "Open settings")),
    ("toggleLayoutLock", tr("Layout sperren", "Lock layout")),
]
}

pub fn hotkey_label(action: &str) -> &'static str {
    hotkey_actions().iter().find(|(id, _)| *id == action).map_or("", |(_, label)| label)
}

const MODIFIERS: [&str; 4] = ["Ctrl", "Alt", "Shift", "Meta"];

/// Kombination aus einem Tastendruck. None, solange nur Umschalttasten gedrückt sind.
pub fn combo_from_event(key: &str, ctrl: bool, alt: bool, shift: bool, meta: bool) -> Option<String> {
    let key = normalize_shortcut_key(key)?;
    let mut parts: Vec<String> = Vec::new();
    for (pressed, name) in [(ctrl, "Ctrl"), (alt, "Alt"), (shift, "Shift"), (meta, "Meta")] {
        if pressed {
            parts.push(name.into());
        }
    }
    parts.push(key);
    Some(parts.join("+"))
}

/// Zerlegt „Ctrl+Shift+t“ in Umschalttasten und Taste (die Taste selbst darf „+“ sein).
pub fn parse_combo(combo: &str) -> (Vec<&'static str>, &str) {
    let mut modifiers = Vec::new();
    let mut rest = combo;
    loop {
        let found = MODIFIERS.iter().find(|m| rest.len() > m.len() + 1 && rest.starts_with(*m) && rest[m.len()..].starts_with('+'));
        let Some(modifier) = found else { break };
        if !modifiers.contains(modifier) {
            modifiers.push(*modifier);
        }
        rest = &rest[modifier.len() + 1..];
    }
    (modifiers, rest)
}

/// Anzeige, zum Beispiel „Umschalt+F5“ oder „Strg+E“.
pub fn combo_label(combo: &str) -> String {
    let (modifiers, key) = parse_combo(combo);
    let labels = [("Ctrl", tr("Strg", "Ctrl")), ("Alt", "Alt"), ("Shift", tr("Umschalt", "Shift")), ("Meta", "Win")];
    let mut parts: Vec<String> = labels.iter().filter(|(m, _)| modifiers.contains(m)).map(|(_, l)| l.to_string()).collect();
    parts.push(shortcut_key_label(key));
    parts.join("+")
}

/// Taste, mit der eine Kombination auch ein Mod-Tastenkürzel auslösen würde. Mod-Tastenkürzel reagieren nicht auf
/// Strg, Alt und Win, wohl aber auf Umschalt.
pub fn mod_key_of(combo: &str) -> Option<String> {
    let (modifiers, key) = parse_combo(combo);
    if modifiers.iter().any(|m| matches!(*m, "Ctrl" | "Alt" | "Meta")) { None } else { Some(key.to_string()) }
}

/// Fest belegt (workspace.tsx), deshalb nicht wählbar.
fn fixed_combos() -> [(&'static str, &'static str); 2] {
    [("Ctrl+f", tr("die Suche im Chat", "the chat search")), ("Ctrl+Shift+t", tr("„Zuletzt geschlossenen Tab wiederherstellen“", "“Restore last closed tab”"))]
}

/// Grund, warum eine Kombination für eine Aktion nicht geht, sonst None. Geprüft gegen die festen Tasten, die
/// anderen allgemeinen Tastenkürzel und die belegten Mod-Tastenkürzel (Taste → Beschreibung der Aktion).
pub fn find_hotkey_clash(
    hotkeys: &BTreeMap<String, Option<String>>,
    action: &str,
    combo: &str,
    mod_keys: &BTreeMap<String, String>,
) -> Option<String> {
    if let Some((_, fixed)) = fixed_combos().into_iter().find(|(c, _)| *c == combo) {
        return Some(crate::tf!("„{}“ ist fest für {fixed} belegt.", "“{}” is reserved for {fixed}.", combo_label(combo)));
    }
    let other = hotkey_actions().into_iter().find(|(id, _)| *id != action && hotkeys.get(*id).and_then(Option::as_deref) == Some(combo));
    if let Some((_, label)) = other {
        return Some(crate::tf!("„{}“ ist schon für „{label}“ belegt.", "“{}” is already used for “{label}”.", combo_label(combo)));
    }
    let mod_key = mod_key_of(combo)?;
    let mod_action = mod_keys.get(&mod_key)?;
    Some(crate::tf!("„{}“ ist schon als Mod-Tastenkürzel für {mod_action} belegt.", "“{}” is already used as a mod shortcut for {mod_action}.", shortcut_key_label(&mod_key)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasten_und_anzeige() {
        assert_eq!(normalize_shortcut_key(" ").as_deref(), Some("Space"));
        assert_eq!(normalize_shortcut_key("T").as_deref(), Some("t"));
        assert_eq!(normalize_shortcut_key("F2").as_deref(), Some("F2"));
        assert_eq!(normalize_shortcut_key("Shift"), None);
        assert_eq!(shortcut_key_label("Delete"), "Entf");
        assert_eq!(shortcut_key_label("ä"), "Ä");
        assert_eq!(format_duration(600), "10m");
        assert_eq!(format_duration(90), "90s");
        assert_eq!(format_duration(604_800), "1W");
        assert_eq!(describe_shortcut("warn", None, Some("  ")), "Verwarnen ohne Grund");
        assert_eq!(describe_shortcut("warn", None, Some(" Spam ")), "Verwarnen „Spam“");
        assert_eq!(describe_shortcut("timeout", Some(3600), None), "Timeout 1h");
    }

    #[test]
    fn kombinationen() {
        assert_eq!(combo_from_event("F5", false, false, true, false).as_deref(), Some("Shift+F5"));
        assert_eq!(combo_from_event("Control", true, false, false, false), None);
        assert_eq!(parse_combo("Ctrl+Shift++"), (vec!["Ctrl", "Shift"], "+"));
        assert_eq!(combo_label("Ctrl+e"), "Strg+E");
        assert_eq!(mod_key_of("Shift+t").as_deref(), Some("t"));
        assert_eq!(mod_key_of("Alt+t"), None);
    }

    #[test]
    fn konflikte() {
        let hotkeys: BTreeMap<String, Option<String>> =
            [("reconnectAll", Some("F5")), ("reloadEmotes", None)].into_iter().map(|(k, v)| (k.into(), v.map(Into::into))).collect();
        let mod_keys: BTreeMap<String, String> = [("t".to_string(), "Timeout 10m".to_string())].into_iter().collect();
        assert_eq!(find_hotkey_clash(&hotkeys, "newTab", "Ctrl+f", &mod_keys).unwrap(), "„Strg+F“ ist fest für die Suche im Chat belegt.");
        assert_eq!(find_hotkey_clash(&hotkeys, "newTab", "F5", &mod_keys).unwrap(), "„F5“ ist schon für „Alle Kanäle neu verbinden“ belegt.");
        assert_eq!(find_hotkey_clash(&hotkeys, "reconnectAll", "F5", &mod_keys), None);
        assert_eq!(
            find_hotkey_clash(&hotkeys, "newTab", "Shift+t", &mod_keys).unwrap(),
            "„T“ ist schon als Mod-Tastenkürzel für Timeout 10m belegt."
        );
    }
}
