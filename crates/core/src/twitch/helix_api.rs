//! Reine Teile der Helix-Endpunkte (bisher src/platforms/twitch/api.ts und moderation.ts; Endpunkte am 15.11.2026 in
//! der offiziellen Doku geprüft): Chat-Modi umwandeln und Anfragekörper bauen. Die Anfragen selbst laufen in
//! crates/web/src/api.rs.

use crate::i18n::tr;
use serde_json::{json, Map, Value};

/// Timeout: 1 Sekunde bis 1.209.600 Sekunden (2 Wochen).
pub const MAX_TIMEOUT_SECONDS: u32 = 1_209_600;
/// Gründe und Ankündigungen höchstens 500 Zeichen.
pub const MAX_REASON_LENGTH: usize = 500;
/// Send Chat Message: höchstens 500 Zeichen.
pub const MAX_MESSAGE_LENGTH: usize = 500;
/// Längste Pin-Dauer laut Doku (Sekunden); ohne Dauer bleibt die Nachricht bis zum Streamende angepinnt.
pub const MAX_PIN_SECONDS: u32 = 1800;
/// Höchstens so viele Seiten zu je 1000 Chattern (sehr große Chats werden gekürzt angezeigt).
pub const MAX_CHATTER_PAGES: usize = 10;

/// Die ersten `max` UTF-16-Einheiten (wie `slice(0, max)` in JavaScript), ohne ein Zeichen zu teilen.
pub fn truncate_utf16(text: &str, max: usize) -> String {
    let mut units = 0;
    let mut end = 0;
    for (index, ch) in text.char_indices() {
        units += ch.len_utf16();
        if units > max {
            break;
        }
        end = index + ch.len_utf8();
    }
    text[..end].to_string()
}

/// Zahl wie in JavaScript als JSON: ganze Zahlen ohne Nachkommastelle.
pub fn number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9e15 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

fn field(source: &Value, name: &str) -> Value {
    source.get(name).cloned().unwrap_or(Value::Null)
}

/// Chat-Modi (Felder wie `ChatModes` in src/core/chat/chat-modes.ts) aus Get/Update Chat Settings. Die
/// Verzögerung gibt es nur mit Mod-Rechten; fehlt sie, fehlen auch die Felder.
pub fn chat_modes_from_settings(settings: &Value) -> Value {
    let mut modes = json!({
        "slow": field(settings, "slow_mode"),
        "slowSeconds": field(settings, "slow_mode_wait_time"),
        "emoteOnly": field(settings, "emote_mode"),
        "followerOnly": field(settings, "follower_mode"),
        "followerMinutes": field(settings, "follower_mode_duration"),
        "subscriberOnly": field(settings, "subscriber_mode"),
        "uniqueChat": field(settings, "unique_chat_mode"),
    });
    if let Some(delay) = settings.get("non_moderator_chat_delay") {
        modes["delay"] = delay.clone();
    }
    if let Some(seconds) = settings.get("non_moderator_chat_delay_duration") {
        modes["delaySeconds"] = seconds.clone();
    }
    modes
}

/// Live-Änderung (channel.chat_settings.update v1) übernehmen; die Verzögerung kommt im Ereignis nicht vor und
/// bleibt, wie sie war.
pub fn chat_modes_from_event(event: &Value, previous: Option<&Value>) -> Value {
    let mut modes = json!({
        "slow": field(event, "slow_mode"),
        "slowSeconds": field(event, "slow_mode_wait_time_seconds"),
        "emoteOnly": field(event, "emote_mode"),
        "followerOnly": field(event, "follower_mode"),
        "followerMinutes": field(event, "follower_mode_duration_minutes"),
        "subscriberOnly": field(event, "subscriber_mode"),
        "uniqueChat": field(event, "unique_chat_mode"),
    });
    for name in ["delay", "delaySeconds"] {
        if let Some(value) = previous.and_then(|p| p.get(name)) {
            modes[name] = value.clone();
        }
    }
    modes
}

/// Körper für Update Chat Settings aus einer Teiländerung der Chat-Modi: Slow 3 bis 120 Sekunden, Follower 0 bis
/// 129.600 Minuten, Unique-Chat, Verzögerung 2, 4 oder 6 Sekunden (Doku geprüft am 15.11.2026). Dauern werden nur
/// beim Einschalten mitgeschickt.
pub fn chat_settings_body(patch: &Value) -> Value {
    let mut body = Map::new();
    let on = |name: &str| patch.get(name).and_then(Value::as_bool) == Some(true);
    if let Some(slow) = patch.get("slow") {
        body.insert("slow_mode".into(), slow.clone());
        if on("slow") {
            body.insert("slow_mode_wait_time".into(), field(patch, "slowSeconds"));
        }
    }
    if let Some(emote) = patch.get("emoteOnly") {
        body.insert("emote_mode".into(), emote.clone());
    }
    if let Some(follower) = patch.get("followerOnly") {
        body.insert("follower_mode".into(), follower.clone());
        if on("followerOnly") {
            body.insert("follower_mode_duration".into(), field(patch, "followerMinutes"));
        }
    }
    if let Some(subscriber) = patch.get("subscriberOnly") {
        body.insert("subscriber_mode".into(), subscriber.clone());
    }
    if let Some(unique) = patch.get("uniqueChat") {
        body.insert("unique_chat_mode".into(), unique.clone());
    }
    if let Some(delay) = patch.get("delay") {
        body.insert("non_moderator_chat_delay".into(), delay.clone());
        if on("delay") {
            body.insert("non_moderator_chat_delay_duration".into(), field(patch, "delaySeconds"));
        }
    }
    Value::Object(body)
}

/// Körper für Ban (ohne Dauer) oder Timeout (mit Dauer). Ein leerer Grund wird nicht mitgeschickt.
pub fn ban_body(user_id: &str, duration_seconds: Option<f64>, reason: Option<&str>) -> Value {
    let mut data = json!({ "user_id": user_id });
    if let Some(duration) = duration_seconds {
        data["duration"] = number(duration);
    }
    if let Some(reason) = reason.map(str::trim).filter(|r| !r.is_empty()) {
        data["reason"] = json!(truncate_utf16(reason, MAX_REASON_LENGTH));
    }
    json!({ "data": data })
}

/// Körper für Verwarnen: Twitch verlangt einen Grund.
pub fn warn_body(user_id: &str, reason: &str) -> Value {
    json!({ "data": { "user_id": user_id, "reason": truncate_utf16(reason.trim(), MAX_REASON_LENGTH) } })
}

/// Ergebnis von Send Chat Message: (gesendet, Grund von Twitch). None bei unerwarteter Antwort.
pub fn send_result(response: &Value) -> Option<(bool, Option<String>)> {
    let result = response.get("data")?.get(0)?;
    if result.get("is_sent").and_then(Value::as_bool) == Some(true) {
        return Some((true, None));
    }
    let reason = result.get("drop_reason").and_then(|d| d.get("message")).and_then(Value::as_str).unwrap_or(tr("Unbekannter Grund", "Unknown reason"));
    Some((false, Some(reason.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_become_chat_modes() {
        let settings = json!({ "emote_mode": false, "follower_mode": true, "follower_mode_duration": 10, "slow_mode": true,
            "slow_mode_wait_time": 30, "subscriber_mode": false, "unique_chat_mode": true });
        let modes = chat_modes_from_settings(&settings);
        assert_eq!(modes["followerMinutes"], 10);
        assert_eq!(modes["slowSeconds"], 30);
        assert!(modes.get("delay").is_none());
        let with_delay = chat_modes_from_settings(&json!({ "non_moderator_chat_delay": true, "non_moderator_chat_delay_duration": 4 }));
        assert_eq!(with_delay["delay"], true);
        assert_eq!(with_delay["delaySeconds"], 4);
    }

    #[test]
    fn event_keeps_previous_delay() {
        let event = json!({ "emote_mode": true, "follower_mode": false, "follower_mode_duration_minutes": null, "slow_mode": true,
            "slow_mode_wait_time_seconds": 10, "subscriber_mode": false, "unique_chat_mode": false });
        let next = chat_modes_from_event(&event, Some(&json!({ "delay": true, "delaySeconds": 4 })));
        assert_eq!(next["emoteOnly"], true);
        assert_eq!(next["slowSeconds"], 10);
        assert_eq!(next["delay"], true);
        assert_eq!(next["delaySeconds"], 4);
        assert!(chat_modes_from_event(&event, None).get("delay").is_none());
    }

    #[test]
    fn settings_body_sends_durations_only_when_switching_on() {
        assert_eq!(chat_settings_body(&json!({ "slow": true, "slowSeconds": 30 })), json!({ "slow_mode": true, "slow_mode_wait_time": 30 }));
        assert_eq!(chat_settings_body(&json!({ "slow": false, "slowSeconds": 30 })), json!({ "slow_mode": false }));
        assert_eq!(
            chat_settings_body(&json!({ "followerOnly": true, "followerMinutes": 0, "delay": true, "delaySeconds": 2 })),
            json!({ "follower_mode": true, "follower_mode_duration": 0, "non_moderator_chat_delay": true, "non_moderator_chat_delay_duration": 2 })
        );
        assert_eq!(chat_settings_body(&json!({ "emoteOnly": true, "subscriberOnly": false, "uniqueChat": true })),
            json!({ "emote_mode": true, "subscriber_mode": false, "unique_chat_mode": true }));
    }

    #[test]
    fn ban_body_trims_and_skips_empty_reason() {
        assert_eq!(ban_body("1", None, Some("  ")), json!({ "data": { "user_id": "1" } }));
        assert_eq!(ban_body("1", Some(60.0), Some(" Spam ")), json!({ "data": { "user_id": "1", "duration": 60, "reason": "Spam" } }));
        let long = "a".repeat(600);
        assert_eq!(ban_body("1", None, Some(&long))["data"]["reason"].as_str().unwrap().len(), 500);
    }

    #[test]
    fn truncation_counts_utf16_units() {
        assert_eq!(truncate_utf16("abc", 2), "ab");
        // Ein Emoji sind zwei UTF-16-Einheiten und wird nicht geteilt.
        assert_eq!(truncate_utf16("a😀b", 2), "a");
        assert_eq!(truncate_utf16("a😀b", 3), "a😀");
    }

    #[test]
    fn send_result_reads_drop_reason() {
        assert_eq!(send_result(&json!({ "data": [{ "is_sent": true }] })), Some((true, None)));
        assert_eq!(
            send_result(&json!({ "data": [{ "is_sent": false, "drop_reason": { "message": "Zu schnell" } }] })),
            Some((false, Some("Zu schnell".into())))
        );
        assert_eq!(send_result(&json!({ "data": [] })), None);
    }
}
