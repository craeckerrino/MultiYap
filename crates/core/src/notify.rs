//! Reine Teile der Benachrichtigungen (bisher src/features/notifications/*.ts und die Erkennung aus
//! src/features/highlights/live-match.ts, exceptions.ts und src/features/mentions/matcher.ts): Erwähnungen und
//! Filter-Treffer mit Ausnahmen, das Signal einer neuen Nachricht, die Sperrzeit gegen Dauer-Piepen, der Vorrang beim
//! Blinken und welche Zeilen als ungesehen zählen. Die Speicher und das Abspielen liegen in
//! crates/web/src/notifications.rs.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::mentions::NameMatcher;
use crate::terms::{CategoryTerms, TermMatcher};

/// Sperrzeit gegen Dauer-Piepen: pro Ereignisart höchstens ein Ton in dieser Zeit.
pub const SOUND_COOLDOWN_MS: f64 = 3000.0;

fn text<'a>(value: &'a Value, name: &str) -> &'a str {
    value.get(name).and_then(Value::as_str).unwrap_or_default()
}

fn flag(value: &Value, name: &str) -> bool {
    value.get("flags").and_then(|f| f.get(name)).and_then(Value::as_bool) == Some(true)
}

fn badge_ids(message: &Value) -> Vec<&str> {
    message.get("badges").and_then(Value::as_array).into_iter().flatten().map(|b| text(b, "setId")).collect()
}

/// Farbklasse fürs Blinken je Kategorie-Farbe (Tailwind braucht die Namen ausgeschrieben).
pub fn blink_class(color: &str) -> &'static str {
    match color {
        "term-2" => "bg-term-2/25",
        "term-3" => "bg-term-3/25",
        "term-4" => "bg-term-4/25",
        "term-5" => "bg-term-5/25",
        "term-6" => "bg-term-6/25",
        _ => "bg-term-1/25",
    }
}

/// Wo Begriffe hervorgehoben werden: "all", "mod" (nur Mod-Kanäle) oder "off".
pub fn highlights_enabled(scope: &str, is_moderator: bool) -> bool {
    scope == "all" || (scope == "mod" && is_moderator)
}

/// Einstellungen, von denen die Erkennung abhängt.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MatchSettings {
    /// Angemeldetes Konto: (Nutzer-ID, Login).
    pub own: Option<(String, String)>,
    pub own_name: bool,
    pub extra_names: Vec<String>,
    pub scope: String,
    /// Kategorien wie in „highlights.categories“ (id, name, color, terms, regex, scrollMarker).
    pub categories: Vec<Value>,
    /// Ausnahmen wie in „highlights.exceptions“ (userId, allCategories, categoryIds).
    pub exceptions: Vec<Value>,
    pub exempt_broadcaster: bool,
    pub exempt_mods: bool,
}

/// Erkennung von Erwähnungen und Filter-Treffern, einmal je Stand der Einstellungen gebaut.
pub struct Matcher {
    settings: MatchSettings,
    names: NameMatcher,
    terms: TermMatcher,
    exceptions: HashMap<String, Value>,
}

/// Was eine neue Nachricht auslöst: Erwähnung vor Filter-Treffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    /// `name`: der passende weitere Name (für seinen eigenen Sound), None beim eigenen Namen oder einer Antwort.
    Mention { name: Option<String> },
    Term { category_id: String, class_name: String },
}

impl Matcher {
    pub fn new(settings: MatchSettings) -> Matcher {
        let mut names = settings.extra_names.clone();
        if let (true, Some((_, login))) = (settings.own_name, &settings.own) {
            names.push(login.clone());
        }
        let categories: Vec<CategoryTerms> = settings
            .categories
            .iter()
            .map(|c| CategoryTerms {
                terms: c.get("terms").and_then(Value::as_array).into_iter().flatten().filter_map(|t| t.as_str().map(str::to_string)).collect(),
                regex: c.get("regex").and_then(Value::as_bool) == Some(true),
            })
            .collect();
        let exceptions = settings.exceptions.iter().map(|e| (text(e, "userId").to_string(), e.clone())).collect();
        Matcher { names: NameMatcher::new(&names), terms: TermMatcher::new(&categories), exceptions, settings }
    }

    pub fn settings(&self) -> &MatchSettings {
        &self.settings
    }

    /// Eigene Nachricht des angemeldeten Kontos.
    pub fn is_own(&self, message: &Value) -> bool {
        flag(message, "own") || self.settings.own.as_ref().is_some_and(|(id, _)| message.get("sender").map(|s| text(s, "id")) == Some(id.as_str()))
    }

    /// Erwähnung: eigener Name und zusätzliche Namen als ganze Wörter, dazu Antworten auf eigene Nachrichten. Eigene
    /// Nachrichten zählen nie.
    pub fn is_mention(&self, message: &Value) -> bool {
        if self.is_own(message) {
            return false;
        }
        if let (true, Some((_, login))) = (self.settings.own_name, &self.settings.own) {
            let reply_to = message.get("reply").map(|r| text(r, "senderLogin").to_lowercase());
            if reply_to.as_deref() == Some(login.to_lowercase().as_str()) {
                return true;
            }
        }
        self.names.is_match(text(message, "text"))
    }

    /// Wird diese Kategorie für die Nachricht übersprungen (Ausnahmen)?
    pub fn is_exempt(&self, message: &Value, category_id: &str) -> bool {
        let sender = message.get("sender").map(|s| text(s, "id")).unwrap_or_default();
        let badges = badge_ids(message);
        if self.settings.exempt_broadcaster {
            let channel = message.get("channel").map(|c| text(c, "id")).unwrap_or_default();
            if sender == channel || badges.contains(&"broadcaster") {
                return true;
            }
        }
        if self.settings.exempt_mods && badges.iter().any(|b| *b == "moderator" || *b == "lead_moderator") {
            return true;
        }
        let Some(exception) = self.exceptions.get(sender) else { return false };
        exception.get("allCategories").and_then(Value::as_bool) == Some(true)
            || exception.get("categoryIds").and_then(Value::as_array).is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(category_id)))
    }

    /// Ausgenommen von der Spam-Erkennung: automatisch ausgenommene Broadcaster und Mods wie bei den Filtern und jede
    /// Person mit einer Ausnahme, egal für welche Kategorien.
    pub fn is_spam_exempt(&self, sender_id: &str, channel_id: &str, badges: &[&str]) -> bool {
        if self.settings.exempt_broadcaster && (sender_id == channel_id || badges.contains(&"broadcaster")) {
            return true;
        }
        if self.settings.exempt_mods && badges.iter().any(|b| *b == "moderator" || *b == "lead_moderator") {
            return true;
        }
        self.exceptions.contains_key(sender_id)
    }

    /// Filter-Kategorie einer Nachricht, genau wie die Markierung im Chat: nur dort, wo Filter angewendet werden, ohne
    /// ausgenommene Personen. Eigene Nachrichten zählen hier mit.
    pub fn term_category(&self, message: &Value, is_moderator: bool) -> Option<&Value> {
        if !highlights_enabled(&self.settings.scope, is_moderator) {
            return None;
        }
        self.terms
            .matching(text(message, "text"))
            .into_iter()
            .filter_map(|index| self.settings.categories.get(index))
            .find(|category| !self.is_exempt(message, text(category, "id")))
    }

    /// Welches Signal eine neue Nachricht auslöst; eigene Nachrichten nie.
    pub fn signal(&self, message: &Value, is_moderator: bool) -> Option<Signal> {
        if self.is_own(message) {
            return None;
        }
        if self.is_mention(message) {
            let own = self.settings.own.as_ref().filter(|_| self.settings.own_name).map(|(_, login)| login.to_lowercase());
            let name = self
                .names
                .matched_name(text(message, "text"))
                .filter(|name| own.as_deref() != Some(name.to_lowercase().as_str()))
                .map(str::to_string);
            return Some(Signal::Mention { name });
        }
        let category = self.term_category(message, is_moderator)?;
        Some(Signal::Term { category_id: text(category, "id").to_string(), class_name: blink_class(text(category, "color")).to_string() })
    }
}

/// Ton an oder aus, Blinken an oder aus, Lautstärke in Prozent.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NotifyEvent {
    #[serde(default)]
    pub sound: bool,
    #[serde(default)]
    pub blink: bool,
    #[serde(default = "full_volume")]
    pub volume: f64,
}

fn full_volume() -> f64 {
    100.0
}

/// Standard für eine Begriffs-Kategorie ohne eigenen Eintrag: Ton und Blinken aus. Filter-Kategorien: Tab leuchtet
/// standardmäßig in der Farbe des Filters, Ton aus (Vorher war auch das Blinken aus).
pub const TERM_NOTIFY_DEFAULT: NotifyEvent = NotifyEvent { sound: false, blink: true, volume: 100.0 };

/// Einstellung einer Begriffs-Kategorie aus „notifications.termCategories“ (Schlüssel ist die Kategorie-ID).
/// Wert in „notifications.soundChoice“ für den eingebauten Standard-Sound (überschreibt eine allgemeinere Wahl).
pub const DEFAULT_SOUND: &str = "default";

/// Schlüssel in „notifications.soundChoice“ für den Sound eines weiteren Namens.
pub fn mention_sound_key(name: &str) -> String {
    format!("mention:{}", name.trim().trim_start_matches('@').to_lowercase())
}

/// Gewählter eigener Sound: der erste Schlüssel mit Eintrag gilt (zum Beispiel Name vor Erwähnung allgemein).
/// None heißt eingebauter Standard-Sound.
pub fn sound_choice(choices: &Value, keys: &[String]) -> Option<String> {
    keys.iter()
        .find_map(|key| choices.get(key).and_then(Value::as_str))
        .filter(|id| *id != DEFAULT_SOUND)
        .map(str::to_string)
}

pub fn term_notify_for(settings: &Value, category_id: &str) -> NotifyEvent {
    settings.get(category_id).and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or(TERM_NOTIFY_DEFAULT)
}

/// Sperrzeit je Ereignisart.
#[derive(Debug, Default)]
pub struct Cooldown {
    last: HashMap<String, f64>,
}

impl Cooldown {
    /// true, wenn der Ton jetzt gespielt werden darf (und merkt sich den Zeitpunkt).
    pub fn allow(&mut self, key: &str, now: f64, window_ms: f64) -> bool {
        if self.last.get(key).is_some_and(|previous| now - previous < window_ms) {
            return false;
        }
        self.last.insert(key.to_string(), now);
        true
    }
}

/// Tab-Blinken eines Kanals: "mention" oder "term" mit Farbklasse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attention {
    pub kind: String,
    pub class_name: String,
}

/// Soll das neue Signal das bisherige ersetzen? Erwähnung geht vor Begriff; gleiches Signal ändert nichts.
pub fn replaces_attention(current: Option<&Attention>, next: &Attention) -> bool {
    match current {
        Some(current) if current.kind == "mention" && next.kind == "term" => false,
        Some(current) => current != next,
        None => true,
    }
}

/// Zählt diese neue Zeile als ungesehen? Nachrichten anderer, Chat-Ereignisse, Mod-Aktionen und AutoMod; nicht eigene
/// Nachrichten, Systemzeilen und der geladene Verlauf.
pub fn counts_as_unread(item: &Value, own: bool) -> bool {
    let from_history = item.get("fromHistory").and_then(Value::as_bool) == Some(true);
    match text(item, "kind") {
        "message" => !flag(item, "fromHistory") && !own,
        "event" | "mod-action" => !from_history,
        "automod" => true,
        _ => false,
    }
}

/// Logins, deren Tab gerade in einem Bereich sichtbar ist (dort kein Blinken und nichts Ungesehenes).
pub fn visible_logins(tabs: &[Value], active_tab_ids: &HashSet<String>) -> HashSet<String> {
    tabs.iter()
        .filter(|t| text(t, "kind") == "channel" && active_tab_ids.contains(text(t, "id")))
        .map(|t| text(t, "channel").to_string())
        .collect()
}

// ---- Eigene Sounds ----

/// Höchstens so viele eigene Sounds (hält Sicherungen und automatische Sicherungen klein).
pub const MAX_CUSTOM_SOUNDS: usize = 10;
/// Höchstgröße einer Sounddatei in Bytes.
pub const MAX_SOUND_BYTES: usize = 1024 * 1024;

/// Erlaubter MIME-Typ einer hochgeladenen Datei (MP3, WAV, OGG): vom Browser gemeldet, sonst aus der Endung.
pub fn sound_mime_type(name: &str, reported: &str) -> Option<&'static str> {
    const KNOWN: [&str; 7] = ["audio/mpeg", "audio/mp3", "audio/wav", "audio/x-wav", "audio/wave", "audio/vnd.wave", "audio/ogg"];
    if let Some(known) = KNOWN.iter().find(|k| **k == reported) {
        return Some(known);
    }
    match name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).as_deref() {
        Some("mp3") => Some("audio/mpeg"),
        Some("wav") => Some("audio/wav"),
        Some("ogg" | "oga") => Some("audio/ogg"),
        _ => None,
    }
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Bytes nach Base64 (wie btoa), damit Sounds unverändert in die Sicherungsdatei passen.
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            out.push(if i <= chunk.len() { BASE64[((n >> shift) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// Base64 nach Bytes (wie atob); None bei ungültigem Text.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0;
    for c in text.bytes().filter(|c| !c.is_ascii_whitespace()) {
        if c == b'=' {
            break;
        }
        let value = BASE64.iter().position(|b| *b == c)? as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(sender: &str, text: &str, flags: Value) -> Value {
        json!({ "kind": "message", "id": format!("m-{sender}"), "channel": { "id": "k1", "login": "kanal" },
            "sender": { "id": sender, "login": sender, "displayName": sender }, "text": text, "badges": [], "flags": flags })
    }

    fn settings() -> MatchSettings {
        MatchSettings {
            own: Some(("ich".into(), "craecker".into())),
            own_name: true,
            extra_names: Vec::new(),
            scope: "all".into(),
            categories: vec![json!({ "id": "k", "name": "Neue Kategorie", "color": "term-1", "terms": ["hund"], "regex": false, "scrollMarker": true })],
            exceptions: Vec::new(),
            exempt_broadcaster: false,
            exempt_mods: false,
        }
    }

    #[test]
    fn own_term_counts_for_highlights_but_signals_nothing() {
        let matcher = Matcher::new(settings());
        let own = message("ich", "hund", json!({ "own": true }));
        assert_eq!(matcher.term_category(&own, false).map(|c| c["id"].as_str().unwrap()), Some("k"));
        assert_eq!(matcher.signal(&own, false), None);
        let other = message("fremd", "hund", json!({}));
        assert_eq!(matcher.signal(&other, false), Some(Signal::Term { category_id: "k".into(), class_name: "bg-term-1/25".into() }));
        let mention = message("fremd", "hallo @Craecker", json!({}));
        assert_eq!(matcher.signal(&mention, false), Some(Signal::Mention { name: None }));
        let mut s = settings();
        s.extra_names = vec!["@Kraecki".into()];
        let extra = Matcher::new(s);
        assert_eq!(extra.signal(&message("fremd", "hallo KRAECKI", json!({})), false), Some(Signal::Mention { name: Some("Kraecki".into()) }));
        assert_eq!(extra.signal(&message("fremd", "hallo craecker", json!({})), false), Some(Signal::Mention { name: None }));
    }

    #[test]
    fn sound_je_name() {
        assert_eq!(mention_sound_key(" @Kraecki "), "mention:kraecki");
        let choices = json!({ "mention": "eigener", "mention:a": "default", "mention:b": "anderer" });
        let keys = |name: &str| [mention_sound_key(name), "mention".to_string()];
        assert_eq!(sound_choice(&choices, &keys("A")), None);
        assert_eq!(sound_choice(&choices, &keys("b")).as_deref(), Some("anderer"));
        assert_eq!(sound_choice(&choices, &keys("c")).as_deref(), Some("eigener"));
        assert_eq!(sound_choice(&json!({}), &keys("c")), None);
    }

    #[test]
    fn scope_and_exceptions() {
        let mut s = settings();
        s.scope = "mod".into();
        let other = message("fremd", "hund", json!({}));
        assert!(Matcher::new(s.clone()).term_category(&other, false).is_none());
        assert!(Matcher::new(s.clone()).term_category(&other, true).is_some());
        s.scope = "all".into();
        s.exceptions = vec![json!({ "userId": "fremd", "allCategories": false, "categoryIds": ["k"] })];
        assert!(Matcher::new(s.clone()).term_category(&other, false).is_none());
        s.exceptions = Vec::new();
        s.exempt_broadcaster = true;
        let broadcaster = message("k1", "hund", json!({}));
        assert!(Matcher::new(s.clone()).term_category(&broadcaster, false).is_none());
        s.exempt_mods = true;
        let mut moderator = message("mod", "hund", json!({}));
        moderator["badges"] = json!([{ "setId": "lead_moderator" }]);
        assert!(Matcher::new(s).term_category(&moderator, false).is_none());
    }

    #[test]
    fn mentions_reply_own_and_extra_names() {
        let matcher = Matcher::new(settings());
        assert!(matcher.is_mention(&message("fremd", "hey CRAECKER wie gehts", json!({}))));
        assert!(!matcher.is_mention(&message("fremd", "craeckerfan", json!({}))));
        let mut reply = message("fremd", "stimmt", json!({}));
        reply["reply"] = json!({ "senderLogin": "Craecker" });
        assert!(matcher.is_mention(&reply));
        assert!(!matcher.is_mention(&message("ich", "@craecker", json!({}))));
        assert!(!matcher.is_mention(&message("andere", "@craecker", json!({ "own": true }))));
        let mut s = settings();
        s.own_name = false;
        s.extra_names = vec!["Kraecki".into()];
        let extra = Matcher::new(s.clone());
        assert!(extra.is_mention(&message("fremd", "hallo kraecki", json!({}))));
        assert!(!extra.is_mention(&message("fremd", "hallo craecker", json!({}))));
        assert!(!extra.is_mention(&reply));
        s.own = None;
        s.extra_names = Vec::new();
        assert!(!Matcher::new(s).is_mention(&message("fremd", "craecker", json!({}))));
    }

    #[test]
    fn spam_ausnahmen() {
        let mut s = settings();
        s.exceptions = vec![json!({ "userId": "x", "allCategories": false, "categoryIds": ["a"] })];
        let m = Matcher::new(s.clone());
        assert!(m.is_spam_exempt("x", "kanal", &[]));
        assert!(!m.is_spam_exempt("y", "kanal", &["moderator"]));
        s.exempt_mods = true;
        s.exempt_broadcaster = true;
        let m = Matcher::new(s);
        assert!(m.is_spam_exempt("y", "kanal", &["moderator"]));
        assert!(m.is_spam_exempt("kanal", "kanal", &[]));
        assert!(!m.is_spam_exempt("z", "kanal", &["vip"]));
    }

    #[test]
    fn exceptions_all_or_next_matching_category() {
        let mut s = settings();
        s.categories = vec![
            json!({ "id": "a", "name": "A", "color": "term-1", "terms": ["spam"], "regex": false, "scrollMarker": true }),
            json!({ "id": "b", "name": "B", "color": "term-2", "terms": ["spam"], "regex": false, "scrollMarker": true }),
        ];
        let id = |m: &Matcher, sender: &str| m.term_category(&message(sender, "spam", json!({})), false).map(|c| c["id"].as_str().unwrap().to_string());
        s.exceptions = vec![json!({ "userId": "x", "allCategories": true, "categoryIds": [] })];
        assert_eq!(id(&Matcher::new(s.clone()), "x"), None);
        assert_eq!(id(&Matcher::new(s.clone()), "y").as_deref(), Some("a"));
        s.exceptions = vec![json!({ "userId": "x", "allCategories": false, "categoryIds": ["a"] })];
        assert_eq!(id(&Matcher::new(s.clone()), "x").as_deref(), Some("b"));
        s.exempt_broadcaster = true;
        let mut by_badge = message("z", "spam", json!({}));
        by_badge["badges"] = json!([{ "setId": "broadcaster" }]);
        assert!(Matcher::new(s.clone()).term_category(&by_badge, false).is_none());
        let mut moderator = message("m", "spam", json!({}));
        moderator["badges"] = json!([{ "setId": "moderator" }]);
        assert!(Matcher::new(s.clone()).term_category(&moderator, false).is_some());
    }

    #[test]
    fn cooldown_per_event() {
        let mut cooldown = Cooldown::default();
        assert!(cooldown.allow("mention", 0.0, 3000.0));
        assert!(!cooldown.allow("mention", 1000.0, 3000.0));
        assert!(cooldown.allow("live", 1000.0, 3000.0));
        assert!(cooldown.allow("mention", 3000.0, 3000.0));
    }

    #[test]
    fn term_notify_defaults_off() {
        let settings = json!({ "a": { "sound": true, "blink": false, "volume": 40 } });
        assert_eq!(term_notify_for(&settings, "a"), NotifyEvent { sound: true, blink: false, volume: 40.0 });
        assert_eq!(term_notify_for(&settings, "b"), TERM_NOTIFY_DEFAULT);
        const { assert!(!TERM_NOTIFY_DEFAULT.sound && TERM_NOTIFY_DEFAULT.blink) };
    }

    #[test]
    fn mention_beats_term_when_blinking() {
        let attention = |kind: &str, class: &str| Attention { kind: kind.into(), class_name: class.into() };
        let mention = attention("mention", "bg-mention/25");
        assert!(!replaces_attention(Some(&mention), &attention("term", "bg-term-1/25")));
        assert!(replaces_attention(Some(&attention("term", "bg-term-2/25")), &attention("term", "bg-term-3/25")));
        assert!(replaces_attention(Some(&attention("term", "bg-term-3/25")), &mention));
        assert!(!replaces_attention(Some(&mention), &mention));
        assert!(replaces_attention(None, &mention));
    }

    #[test]
    fn unread_counts_only_new_lines_of_others() {
        assert!(counts_as_unread(&message("andere", "hallo", json!({})), false));
        assert!(counts_as_unread(&json!({ "kind": "event" }), false));
        assert!(counts_as_unread(&json!({ "kind": "mod-action" }), false));
        assert!(counts_as_unread(&json!({ "kind": "automod" }), false));
        assert!(!counts_as_unread(&message("ich", "hallo", json!({})), true));
        assert!(!counts_as_unread(&message("andere", "hallo", json!({ "fromHistory": true })), false));
        assert!(!counts_as_unread(&json!({ "kind": "mod-action", "fromHistory": true }), false));
        assert!(!counts_as_unread(&json!({ "kind": "event", "fromHistory": true }), false));
        assert!(!counts_as_unread(&json!({ "kind": "system" }), false));
    }

    #[test]
    fn sound_formats_and_base64() {
        assert_eq!(sound_mime_type("a.MP3", ""), Some("audio/mpeg"));
        assert_eq!(sound_mime_type("a.bin", "audio/x-wav"), Some("audio/x-wav"));
        assert_eq!(sound_mime_type("a.oga", "application/octet-stream"), Some("audio/ogg"));
        assert_eq!(sound_mime_type("a.flac", ""), None);
        for sample in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar", &[0, 255, 128, 7]] {
            assert_eq!(base64_decode(&base64_encode(sample)).as_deref(), Some(sample));
        }
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_decode("Zm8="), Some(b"fo".to_vec()));
        assert_eq!(base64_decode("@@@"), None);
    }
}
