//! Alle Einstellungen mit Schlüssel, Schema und Standardwert. Übertragen aus den zod-Schemas in
//! `src/features/*/settings.ts` und weiteren Dateien, mit derselben Wirkung, damit gespeicherte Werte gültig bleiben.
//! Bedeutung der einzelnen Einstellungen: und die TypeScript-Dateien.

use crate::i18n::Text;
use std::sync::OnceLock;

use serde_json::{json, Value};

use crate::persistence::shortcut_signature;
use crate::schema::*;

pub const MAX_TIMEOUT_SECONDS: i64 = 1_209_600;
pub const MAX_REASON_LENGTH: usize = 500;

pub struct Setting {
    pub key: &'static str,
    pub schema: Schema,
    pub default: Value,
}

fn setting(key: &'static str, schema: Schema, default: Value) -> Setting {
    Setting { key, schema, default }
}

const MOD_ACTIONS: [&str; 5] = ["delete", "warn", "timeout", "ban", "unban"];
const TERM_COLORS: [&str; 6] = ["term-1", "term-2", "term-3", "term-4", "term-5", "term-6"];

/// Nur Timeouts haben eine Dauer, und jeder Timeout braucht eine.
fn timeout_has_duration(value: &Value) -> bool {
    (value["action"] == "timeout") == value.get("durationSeconds").is_some()
}

const TIMEOUT_MESSAGE: Text = ("Nur Timeouts haben eine Dauer, und jeder Timeout braucht eine.", "Only timeouts have a duration, and every timeout needs one.");

fn quick_button() -> Schema {
    refine(
        object(vec![
            field("id", non_empty()),
            field("action", one_of_texts(&MOD_ACTIONS)),
            optional("durationSeconds", int(1, MAX_TIMEOUT_SECONDS)),
            optional("fixedReason", text_check(None, Some(MAX_REASON_LENGTH), |_| true, ("", ""))),
        ]),
        timeout_has_duration,
        TIMEOUT_MESSAGE,
    )
}

fn hotkey() -> Schema {
    text_check(None, None, |t| t.len() == 1 && t.chars().all(|c| c.is_ascii_digit() || c.is_ascii_lowercase()), ("Ein Buchstabe oder eine Ziffer", "One letter or digit"))
}

fn reason_template() -> Schema {
    object(vec![field("id", non_empty()), field("text", trimmed(1, MAX_REASON_LENGTH)), field("hotkey", nullable(hotkey()))])
}

fn shortcut_entry() -> Schema {
    refine(
        object(vec![
            field("id", non_empty()),
            field("key", nullable(text_len(1, 32))),
            field("action", one_of_texts(&MOD_ACTIONS)),
            optional("durationSeconds", int(1, MAX_TIMEOUT_SECONDS)),
            optional("reason", text_check(None, Some(MAX_REASON_LENGTH), |_| true, ("", ""))),
        ]),
        timeout_has_duration,
        TIMEOUT_MESSAGE,
    )
}

fn entries(value: &Value) -> &[Value] {
    value["entries"].as_array().map_or(&[], Vec::as_slice)
}

fn unique<T: Eq + std::hash::Hash>(items: impl Iterator<Item = T>) -> bool {
    let mut seen = std::collections::HashSet::new();
    items.into_iter().all(|item| seen.insert(item))
}

fn sound_event() -> Vec<Field> {
    vec![field("sound", boolean()), field("volume", int(0, 100))]
}

fn notify_event() -> Schema {
    let mut fields = sound_event();
    fields.push(field("blink", boolean()));
    object(fields)
}

fn booleans(names: &[&'static str]) -> Schema {
    object(names.iter().map(|name| field(name, boolean())).collect())
}

/// Name einer Schriftfamilie; Anführungszeichen, Semikolon, Klammern und Zeilenumbrüche sind nicht erlaubt (CSS).
/// Wie bisher in TypeScript ist der Backslash erlaubt.
pub fn font_family_name() -> Schema {
    text_check(None, Some(100), |t| !t.chars().any(|c| "\"';{}<>\n\r".contains(c)), ("Ungültiger Name einer Schriftart", "Invalid font name"))
}

fn https_url() -> Schema {
    text_check(None, Some(300), |t| t.starts_with("https://"), ("Nur https-Adressen", "Only https addresses"))
}

/// Ein zuletzt benutztes Emote (nur Name und Bild).
pub fn recent_emote() -> Schema {
    object(vec![
        field("source", one_of_texts(&["twitch", "seventv", "bttv", "ffz"])),
        field("id", text_len(1, 100)),
        field("name", text_len(1, 100)),
        field("imageUrl", https_url()),
        optional("imageUrl2x", https_url()),
    ])
}

/// Kanäle: Twitch-Logins (bis 25 Zeichen) und YouTube-Schlüssel („@name“ bis 31, Kanal-ID 24).
fn channel_list() -> Schema {
    array(text_len(1, 31), 500)
}

fn build() -> Vec<Setting> {
    vec![
        // ---- Chat ----
        setting("chat.timeFormat", one_of_texts(&["HH:MM", "HH:MM:SS"]), json!("HH:MM")),
        setting(
            "chat.removedDisplay",
            object(vec![
                field("mode", one_of_texts(&["styled", "normal", "hidden"])),
                field("strikethrough", boolean()),
                field("dimmed", boolean()),
                field("italic", boolean()),
            ]),
            json!({ "mode": "styled", "strikethrough": true, "dimmed": true, "italic": false }),
        ),
        setting("chat.pauseTrigger", one_of_texts(&["hover", "alt-hover", "off"]), json!("hover")),
        setting("chat.pauseMaxSeconds", one_of_numbers(&[0.0, 5.0, 10.0, 30.0, 60.0]), json!(10)),
        setting("chat.replyButton", boolean(), json!(true)),
        // Markierungen ein- und ausschaltbar.
        setting("chat.markFirstMessage", boolean(), json!(true)),
        setting("chat.markLinks", boolean(), json!(true)),
        // Links aus dem Chat im privaten Fenster des Standardbrowsers öffnen.
        setting("chat.openLinksPrivate", boolean(), json!(false)),
        // Gift Subs: jede beschenkte Person in eigener Zeile.
        setting("chat.giftSubLines", boolean(), json!(false)),
        // Ordner mit den freigegebenen Versionen für Updates; leer: keine Prüfung.
        setting("updates.folder", text_len(0, 500), json!("")),
        // Updates von multiyap.app suchen und laden.
        setting("updates.fromWebsite", boolean(), json!(true)),
        // Sprache der Oberfläche: „auto“ folgt der Sprache des Systems.
        setting("app.language", one_of_texts(&["auto", "de", "en"]), json!("auto")),
        // ---- Darstellung ----
        setting("display.fontSize", one_of_numbers(&[12.0, 13.0, 14.0, 15.0, 16.0, 18.0, 20.0]), json!(14)),
        setting("display.inputSize", one_of_texts(&["normal", "small", "xsmall"]), json!("normal")),
        setting("display.inputFontSize", int(11, 20), json!(14)),
        setting(
            "display.separators",
            object(vec![field("enabled", boolean()), field("thickness", one_of_numbers(&[1.0, 2.0]))]),
            json!({ "enabled": false, "thickness": 1 }),
        ),
        setting("display.zebra", boolean(), json!(false)),
        setting("display.density", one_of_texts(&["normal", "compact"]), json!("normal")),
        setting("display.readableNames", boolean(), json!(true)),
        setting("display.chatFont", font_family_name(), json!("")),
        setting(
            "display.colors",
            record(
                one_of_texts(&crate::display::custom_color_keys()),
                object(vec![
                    field(
                        "color",
                        nullable(text_check(
                            None,
                            None,
                            |t| t.len() == 7 && t.starts_with('#') && t[1..].chars().all(|c| c.is_ascii_hexdigit()),
                            ("Farbe als #rrggbb", "Color as #rrggbb"),
                        )),
                    ),
                    field("enabled", boolean()),
                ]),
            ),
            json!({}),
        ),
        // ---- Layout und Tabs ----
        setting("layout.locked", boolean(), json!(false)),
        setting("tabs.liveBlink", boolean(), json!(true)),
        setting(
            "tabs.streamInfo",
            booleans(&["title", "category", "status", "viewers", "uptime", "channelName", "sharedWith"]),
            json!({ "title": true, "category": true, "status": true, "viewers": true, "uptime": true, "channelName": true, "sharedWith": true }),
        ),
        // ---- Erwähnungen ----
        setting("mentions.ownName", boolean(), json!(true)),
        setting("mentions.extraNames", array(trimmed(1, 50), 50), json!([])),
        // ---- Spam ----
        setting(
            "spam.settings",
            object(vec![
                field("enabled", boolean()),
                field("similarity", int(50, 100)),
                field("minCount", int(2, 20)),
                field("windowMinutes", int(1, 60)),
                field("minLength", int(1, 50)),
                field("highlightAt", int(2, 50)),
            ]),
            json!({ "enabled": true, "similarity": 70, "minCount": 2, "windowMinutes": 5, "minLength": 5, "highlightAt": 5 }),
        ),
        // ---- Hervorhebungen ----
        setting(
            "highlights.categories",
            array(
                object(vec![
                    field("id", non_empty()),
                    field("name", trimmed(1, 40)),
                    field("color", one_of_texts(&TERM_COLORS)),
                    field("terms", array(trimmed(1, 100), 500)),
                    with_default("regex", boolean(), || Value::Bool(false)),
                    field("scrollMarker", boolean()),
                ]),
                30,
            ),
            json!([]),
        ),
        setting("highlights.scope", one_of_texts(&["all", "mod", "off"]), json!("mod")),
        setting(
            "highlights.exceptions",
            array(
                object(vec![
                    field("userId", non_empty()),
                    field("login", text_len(1, 50)),
                    field("displayName", text_len(1, 50)),
                    field("allCategories", boolean()),
                    field("categoryIds", array(non_empty(), 30)),
                ]),
                2000,
            ),
            json!([]),
        ),
        setting("highlights.exemptBroadcaster", boolean(), json!(true)),
        setting("highlights.exemptMods", boolean(), json!(false)),
        // ---- Benachrichtigungen ----
        setting("notifications.volume", int(0, 100), json!(70)),
        setting("notifications.live", object(sound_event()), json!({ "sound": true, "volume": 100 })),
        setting("notifications.mention", notify_event(), json!({ "sound": true, "blink": true, "volume": 100 })),
        setting("notifications.termCategories", record(non_empty(), notify_event()), json!({})),
        setting("notifications.mutedChannels", channel_list(), json!([])),
        setting("notifications.liveOffChannels", channel_list(), json!([])),
        setting("notifications.soundChoice", record(text_len(1, 80), text_len(1, 64)), json!({})),
        // ---- Emotes und Drittanbieter ----
        setting("emotes.recent", array(recent_emote(), 24), json!([])),
        setting("thirdParty.seventv", booleans(&["emotes", "badges", "paints"]), json!({ "emotes": true, "badges": true, "paints": true })),
        // Schatten der 7TV-Namensfarben (Namen wirkten damit unscharf), Standard aus.
        setting("thirdParty.seventvPaintShadows", boolean(), json!(false)),
        setting("thirdParty.bttv", boolean(), json!(true)),
        setting("thirdParty.ffz", boolean(), json!(true)),
        // ---- Allgemeine Tastenkürzel ----
        setting(
            "hotkeys.general",
            refine(
                object(
                    ["reconnectAll", "reloadEmotes", "newTab", "openSettings", "toggleLayoutLock"]
                        .into_iter()
                        .map(|name| field(name, nullable(text_len(1, 40))))
                        .collect(),
                ),
                |value| unique(value.as_object().into_iter().flat_map(|o| o.values()).filter_map(Value::as_str)),
                ("Jede Taste nur einmal.", "Each key only once."),
            ),
            json!({ "reconnectAll": "F5", "reloadEmotes": "Shift+F5", "newTab": null, "openSettings": null, "toggleLayoutLock": null }),
        ),
        // ---- Moderation ----
        setting(
            "moderation.quickButtons",
            array(quick_button(), 20),
            json!([
                { "id": "delete", "action": "delete" },
                { "id": "warn", "action": "warn" },
                { "id": "timeout-600", "action": "timeout", "durationSeconds": 600 },
                { "id": "timeout-3600", "action": "timeout", "durationSeconds": 3600 },
                { "id": "ban", "action": "ban" },
            ]),
        ),
        setting("moderation.quickButtonsInChat", boolean(), json!(true)),
        setting("moderation.reasonDialog", booleans(&["timeout", "ban"]), json!({ "timeout": true, "ban": true })),
        setting(
            "moderation.reasonTemplates",
            object(vec![
                field("warn", array(reason_template(), 50)),
                field("timeout", array(reason_template(), 50)),
                field("ban", array(reason_template(), 50)),
            ]),
            json!({ "warn": [], "timeout": [], "ban": [] }),
        ),
        setting("moderation.noReasonHotkey", hotkey(), json!("0")),
        setting(
            "moderation.shortcutList",
            refine(
                refine(
                    object(vec![field("enabled", boolean()), field("entries", array(shortcut_entry(), 30))]),
                    |value| unique(entries(value).iter().filter_map(|e| e["key"].as_str())),
                    ("Jede Taste nur einmal.", "Each key only once."),
                ),
                // Nur belegte Einträge zählen; neue ohne Taste sind Entwürfe (Migration 12 räumt alte Doppelungen auf).
                |value| {
                    unique(entries(value).iter().filter(|e| !e["key"].is_null()).map(|e| {
                        shortcut_signature(e["action"].as_str().unwrap_or(""), e["durationSeconds"].as_f64(), e["reason"].as_str())
                    }))
                },
                ("Jede Aktion nur eine Taste.", "Each action only one key."),
            ),
            json!({ "enabled": false, "entries": [
                { "id": "delete", "key": "d", "action": "delete" },
                { "id": "timeout-600", "key": "t", "action": "timeout", "durationSeconds": 600 },
                { "id": "ban", "key": "b", "action": "ban" },
                { "id": "unban", "key": "u", "action": "unban" },
            ] }),
        ),
        // Letzte Nachrichten in der Nutzerkarte aufgeklappt.
        setting("usercard.recentMessagesOpen", boolean(), json!(true)),
        setting(
            "usercard.actions",
            refine(
                array(quick_button(), 20),
                |value| value.as_array().is_some_and(|list| list.iter().all(|b| b["action"] != "delete")),
                ("Löschen gibt es in der Nutzerkarte nicht.", "Delete does not exist in the user card."),
            ),
            json!([
                { "id": "warn", "action": "warn" },
                { "id": "timeout-60", "action": "timeout", "durationSeconds": 60 },
                { "id": "timeout-600", "action": "timeout", "durationSeconds": 600 },
                { "id": "timeout-3600", "action": "timeout", "durationSeconds": 3600 },
                { "id": "timeout-86400", "action": "timeout", "durationSeconds": 86400 },
                { "id": "timeout-604800", "action": "timeout", "durationSeconds": 604800 },
                { "id": "ban", "action": "ban" },
                { "id": "unban", "action": "unban" },
            ]),
        ),
    ]
}

/// Alle Einstellungen (einmal aufgebaut).
pub fn all() -> &'static [Setting] {
    static SETTINGS: OnceLock<Vec<Setting>> = OnceLock::new();
    SETTINGS.get_or_init(build)
}

pub fn find(key: &str) -> Option<&'static Setting> {
    all().iter().find(|s| s.key == key)
}

/// Prüft einen Wert gegen das Schema der Einstellung; unbekannte Einstellung ist ein Fehler.
pub fn parse(key: &str, value: &Value) -> Result<Value, Vec<Issue>> {
    match find(key) {
        Some(setting) => setting.schema.parse(value),
        None => Err(vec![Issue { path: Vec::new(), message: crate::tf!("Unbekannte Einstellung „{key}“", "Unknown setting “{key}”") }]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alle_standardwerte_gueltig_und_schluessel_eindeutig() {
        assert_eq!(all().len(), 53);
        assert!(unique(all().iter().map(|s| s.key)));
        for setting in all() {
            assert_eq!(setting.schema.parse(&setting.default).as_ref(), Ok(&setting.default), "{}", setting.key);
        }
    }

    #[test]
    fn kategorien_ohne_regex_bekommen_aus() {
        let stored = json!([{ "id": "a", "name": " Spam ", "color": "term-1", "terms": [" x "], "scrollMarker": true, "patterns": [] }]);
        let parsed = parse("highlights.categories", &stored).unwrap();
        assert_eq!(parsed, json!([{ "id": "a", "name": "Spam", "color": "term-1", "terms": ["x"], "regex": false, "scrollMarker": true }]));
    }

    #[test]
    fn tastenkuerzel_regeln() {
        let list = |entries: Value| json!({ "enabled": true, "entries": entries });
        let ok = list(json!([{ "id": "a", "key": "d", "action": "delete" }, { "id": "b", "key": null, "action": "delete" }]));
        assert!(parse("moderation.shortcutList", &ok).is_ok());
        let same_key = list(json!([{ "id": "a", "key": "d", "action": "delete" }, { "id": "b", "key": "d", "action": "ban" }]));
        assert_eq!(parse("moderation.shortcutList", &same_key).unwrap_err()[0].message, "Jede Taste nur einmal.");
        let same_action = list(json!([{ "id": "a", "key": "d", "action": "warn", "reason": "Spam" }, { "id": "b", "key": "e", "action": "warn", "reason": " spam" }]));
        assert_eq!(parse("moderation.shortcutList", &same_action).unwrap_err()[0].message, "Jede Aktion nur eine Taste.");
        let no_duration = list(json!([{ "id": "a", "key": "t", "action": "timeout" }]));
        assert_eq!(parse("moderation.shortcutList", &no_duration).unwrap_err()[0].message, TIMEOUT_MESSAGE.0);
    }

    #[test]
    fn weitere_regeln() {
        assert!(parse("usercard.actions", &json!([{ "id": "d", "action": "delete" }])).is_err());
        assert!(parse("display.fontSize", &json!(17)).is_err());
        assert!(parse("display.colors", &json!({ "link": { "color": "#AbCdEf", "enabled": true } })).is_ok());
        assert!(parse("display.colors", &json!({ "fremd": { "color": null, "enabled": true } })).is_err());
        assert!(parse("display.chatFont", &json!("Inter\"; x")).is_err());
        assert!(parse("hotkeys.general", &json!({ "reconnectAll": "F5", "reloadEmotes": "F5", "newTab": null, "openSettings": null, "toggleLayoutLock": null })).is_err());
        assert!(parse("emotes.recent", &json!([{ "source": "bttv", "id": "1", "name": "x", "imageUrl": "http://x" }])).is_err());
        assert!(parse("moderation.noReasonHotkey", &json!("A")).is_err());
        assert!(parse("gibt.es.nicht", &json!(1)).is_err());
    }
}
