//! Mod-Aktionen (channel.moderate v2) und AutoMod (automod.message.hold und .update v2), übertragen aus
//! src/platforms/twitch/convert-moderation.ts. Daten werden als allgemeines JSON gelesen, weil Twitch sie unter dem
//! Aktionsnamen ablegt.

use crate::i18n::tr;
use serde::Serialize;
use serde_json::Value;

use super::emote_url;
use crate::model::{AutoModItem, ChannelRef, Emote, MessagePart, ModActionItem, UserRef};
use crate::time::parse_rfc3339_ms;

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn present(value: Option<&Value>) -> Option<&Value> {
    value.filter(|v| !v.is_null())
}

/// Zahl wie JavaScript `String(zahl)`: ganze Zahlen ohne Nachkommastellen.
fn number_text(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        value.to_string()
    }
}

/// Objekt zur Aktion: Twitch legt die Daten unter dem Aktionsnamen ab (shared_chat_* wie die normale Aktion).
fn action_object<'a>(event: &'a Value, action: &str) -> Option<&'a Value> {
    if let Some(direct) = present(event.get(action)).filter(|v| v.is_object()) {
        return Some(direct);
    }
    if action == "approve_unban_request" || action == "deny_unban_request" {
        return present(event.get("unban_request"));
    }
    if action.ends_with("_term") {
        return present(event.get("automod_terms"));
    }
    None
}

fn user_of(object: Option<&Value>) -> Option<UserRef> {
    let object = object?;
    let id = text(object, "user_id").filter(|v| !v.is_empty())?;
    let login = text(object, "user_login").filter(|v| !v.is_empty())?;
    Some(UserRef {
        id: id.into(),
        login: login.into(),
        display_name: text(object, "user_name").unwrap_or(login).into(),
    })
}

pub fn convert_moderate(event: &Value, channel: &ChannelRef, received_at: f64, id: &str) -> ModActionItem {
    let action = text(event, "action").unwrap_or("");
    let object = action_object(event, action);
    let field = |key: &str| object.and_then(|o| present(o.get(key)));
    let detail = field("wait_time_seconds")
        .or_else(|| field("follow_duration_minutes"))
        .or_else(|| field("viewer_count"))
        .and_then(Value::as_f64)
        .map(number_text)
        .or_else(|| field("message_body").and_then(Value::as_str).map(str::to_string))
        .or_else(|| {
            field("terms")
                .and_then(Value::as_array)
                .map(|terms| terms.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
        });
    let expires = field("expires_at").and_then(Value::as_str).and_then(parse_rfc3339_ms);

    ModActionItem {
        kind: "mod-action".into(),
        channel: channel.clone(),
        id: id.into(),
        timestamp: received_at,
        moderator: Some(UserRef {
            id: text(event, "moderator_user_id").unwrap_or("").into(),
            login: text(event, "moderator_user_login").unwrap_or("").into(),
            display_name: text(event, "moderator_user_name").unwrap_or("").into(),
        }),
        action: action.into(),
        target: user_of(object),
        // Verwarnungen nennen statt eines Grundes oft nur zitierte Chat-Regeln (channel.moderate v2, Feld
        // chat_rules_cited; Doku geprüft am 23.11.2026, beide Felder dürfen fehlen). Beides wird angezeigt.
        reason: warn_reason(field("reason").and_then(Value::as_str), field("chat_rules_cited").and_then(Value::as_array)),
        // Wie Math.max(1, Math.round(...)) im Browser.
        duration_seconds: expires.map(|expires| ((expires - received_at) / 1000.0 + 0.5).floor().max(1.0)),
        detail,
        source_channel: present(event.get("source_broadcaster_user_login")).and_then(Value::as_str).map(str::to_string),
        from_history: None,
    }
}

/// Grund einer Mod-Aktion, bei Verwarnungen mit den zitierten Chat-Regeln.
fn warn_reason(reason: Option<&str>, rules: Option<&Vec<Value>>) -> Option<String> {
    let reason = reason.map(str::trim).filter(|r| !r.is_empty());
    let rules: Vec<&str> = rules.into_iter().flatten().filter_map(Value::as_str).map(str::trim).filter(|r| !r.is_empty()).collect();
    let rules = (!rules.is_empty()).then(|| crate::tf!("Regeln: {}", "Rules: {}", rules.join(", ")));
    match (reason, rules) {
        (Some(reason), Some(rules)) => Some(format!("{reason} ({rules})")),
        (Some(reason), None) => Some(reason.to_string()),
        (None, rules) => rules,
    }
}

/// Nachricht, die durch die Aktion entfernt wurde.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Removal {
    #[serde(rename_all = "camelCase")]
    Message { message_id: String },
    #[serde(rename_all = "camelCase")]
    User { user_id: String, reason: String },
    All,
}

pub fn removal_of(item: &ModActionItem, event: &Value) -> Option<Removal> {
    let base = item.action.strip_prefix("shared_chat_").unwrap_or(&item.action);
    match base {
        "delete" => {
            let object = action_object(event, text(event, "action").unwrap_or(""))?;
            let message_id = text(object, "message_id").filter(|v| !v.is_empty())?;
            Some(Removal::Message { message_id: message_id.into() })
        }
        "ban" | "timeout" => item
            .target
            .as_ref()
            .map(|target| Removal::User { user_id: target.id.clone(), reason: base.into() }),
        "clear" => Some(Removal::All),
        _ => None,
    }
}

fn category_name(category: &str) -> &str {
    match category {
        "aggression" => "Aggression",
        "bullying" => tr("Mobbing", "Bullying"),
        "disability" => tr("Behinderung", "Disability"),
        "sexuality" => tr("Sexualität", "Sexuality"),
        "misogyny" => tr("Frauenfeindlichkeit", "Misogyny"),
        "race" => tr("Herkunft", "Origin"),
        "racism" => tr("Rassismus", "Racism"),
        "religion" => "Religion",
        "homophobia" => tr("Homophobie", "Homophobia"),
        "swearing" => tr("Schimpfwörter", "Swearing"),
        "sex-based-terms" => tr("Sexuelle Begriffe", "Sexual terms"),
        other => other,
    }
}

fn automod_reason(event: &Value) -> String {
    match text(event, "reason") {
        Some("blocked_term") => {
            let count = present(event.get("blocked_term"))
                .and_then(|b| b.get("terms_found"))
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            if count == 1 { tr("Gesperrter Begriff", "Blocked term").into() } else { crate::tf!("Gesperrte Begriffe ({count})", "Blocked terms ({count})") }
        }
        Some("blocked_link") => tr("Gesperrter Link", "Blocked link").into(),
        _ => match present(event.get("automod")) {
            Some(automod) => {
                let level = automod.get("level").and_then(Value::as_f64).map_or_else(|| "undefined".into(), number_text);
                crate::tf!("AutoMod: {} (Stufe {level})", "AutoMod: {} (level {level})", category_name(text(automod, "category").unwrap_or("")))
            }
            None => "AutoMod".into(),
        },
    }
}

fn automod_parts(event: &Value, dark: bool) -> Vec<MessagePart> {
    let fragments = event.get("message").and_then(|m| m.get("fragments")).and_then(Value::as_array);
    fragments
        .map(|fragments| {
            fragments
                .iter()
                .map(|fragment| {
                    let fragment_text = text(fragment, "text").unwrap_or("").to_string();
                    match (text(fragment, "type"), present(fragment.get("emote"))) {
                        (Some("emote"), Some(emote)) => {
                            let id = text(emote, "id").unwrap_or("");
                            MessagePart::Emote {
                                emote: Emote {
                                    source: "twitch".into(),
                                    id: id.into(),
                                    name: fragment_text,
                                    image_url: emote_url(id, false, dark, "1.0"),
                                    image_url2x: Some(emote_url(id, false, dark, "2.0")),
                                    animated: None,
                                    overlay: None,
                                    owner_id: None,
                                    global: None,
                                },
                            }
                        }
                        _ => MessagePart::Text { text: fragment_text },
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn convert_automod_hold(event: &Value, channel: &ChannelRef, received_at: f64, dark: bool) -> AutoModItem {
    let held_at = text(event, "held_at").and_then(parse_rfc3339_ms);
    AutoModItem {
        kind: "automod".into(),
        channel: channel.clone(),
        id: text(event, "message_id").unwrap_or("").into(),
        timestamp: held_at.unwrap_or(received_at),
        sender: UserRef {
            id: text(event, "user_id").unwrap_or("").into(),
            login: text(event, "user_login").unwrap_or("").into(),
            display_name: text(event, "user_name").unwrap_or("").into(),
        },
        text: event.get("message").and_then(|m| text(m, "text")).unwrap_or("").into(),
        parts: automod_parts(event, dark),
        reason: automod_reason(event),
        status: "pending".into(),
        resolved_by: None,
        own: false,
    }
}

/// Bestätigte Verwarnung (channel.warning.acknowledge): die Person hat die Verwarnung gelesen und bestätigt.
pub fn convert_warning_acknowledge(event: &Value, channel: &ChannelRef, received_at: f64, id: &str) -> ModActionItem {
    ModActionItem {
        kind: "mod-action".into(),
        channel: channel.clone(),
        id: id.into(),
        timestamp: received_at,
        moderator: None,
        action: "warning_acknowledge".into(),
        target: user_of(Some(event)),
        reason: None,
        duration_seconds: None,
        detail: None,
        source_channel: None,
        from_history: None,
    }
}

/// Eigene Nachricht, die AutoMod zurückhält (channel.chat.user_message_hold, ohne Grund und ohne Mod-Rechte).
pub fn convert_own_hold(event: &Value, channel: &ChannelRef, received_at: f64, dark: bool) -> AutoModItem {
    let mut item = convert_automod_hold(event, channel, received_at, dark);
    item.reason = tr("Deine Nachricht wartet auf die Prüfung durch die Mods", "Your message is waiting for review by the mods").into();
    item.own = true;
    item
}

/// Status aus automod.message.update; None bei unbekannten Werten.
pub fn automod_status_of(status: Option<&str>) -> Option<&'static str> {
    match status?.to_lowercase().as_str() {
        "approved" => Some("approved"),
        "denied" => Some("denied"),
        "expired" => Some("expired"),
        "invalid" => Some("invalid"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn channel() -> ChannelRef {
        ChannelRef { platform: "twitch".into(), id: "1".into(), login: "kanal".into(), display_name: "Kanal".into() }
    }

    fn moderate(action: &str, object: Value) -> Value {
        let mut event = json!({
            "broadcaster_user_id": "1", "source_broadcaster_user_login": null,
            "moderator_user_id": "9", "moderator_user_login": "modi", "moderator_user_name": "Modi", "action": action
        });
        if !object.is_null() {
            event[action] = object;
        }
        event
    }

    #[test]
    fn verwarnung_bestaetigt() {
        let event = json!({ "broadcaster_user_id": "1", "user_id": "5", "user_login": "anna", "user_name": "Anna" });
        let item = convert_warning_acknowledge(&event, &channel(), 0.0, "a1");
        assert_eq!(crate::chat_text::describe_mod_action(&item), "Anna hat die Verwarnung bestätigt");
    }

    #[test]
    fn verwarnung_mit_zitierten_regeln() {
        let now = 0.0;
        let only_rules = moderate("warn", json!({ "user_id": "5", "user_login": "anna", "user_name": "Anna", "reason": null, "chat_rules_cited": ["Kein Spam"] }));
        assert_eq!(convert_moderate(&only_rules, &channel(), now, "w1").reason.as_deref(), Some("Regeln: Kein Spam"));
        let both = moderate("warn", json!({ "user_id": "5", "user_login": "anna", "user_name": "Anna", "reason": "Lass das", "chat_rules_cited": ["A", "B"] }));
        assert_eq!(convert_moderate(&both, &channel(), now, "w2").reason.as_deref(), Some("Lass das (Regeln: A, B)"));
        let none = moderate("warn", json!({ "user_id": "5", "user_login": "anna", "user_name": "Anna", "reason": "", "chat_rules_cited": null }));
        assert_eq!(convert_moderate(&none, &channel(), now, "w3").reason, None);
    }

    #[test]
    fn timeout_mit_dauer_ziel_und_grund() {
        let event = moderate(
            "timeout",
            json!({ "user_id": "5", "user_login": "anna", "user_name": "Anna", "reason": "Spam", "expires_at": "2026-11-15T12:10:00Z" }),
        );
        let now = parse_rfc3339_ms("2026-11-15T12:00:00Z").unwrap();
        let item = convert_moderate(&event, &channel(), now, "mod-1");
        assert_eq!(item.action, "timeout");
        assert_eq!(item.duration_seconds, Some(600.0));
        assert_eq!(item.target.as_ref().map(|t| t.display_name.as_str()), Some("Anna"));
        assert_eq!(item.reason.as_deref(), Some("Spam"));
        assert_eq!(item.moderator.as_ref().map(|m| m.login.as_str()), Some("modi"));
        assert_eq!(removal_of(&item, &event), Some(Removal::User { user_id: "5".into(), reason: "timeout".into() }));
    }

    #[test]
    fn zusatz_je_aktion() {
        assert_eq!(convert_moderate(&moderate("slow", json!({ "wait_time_seconds": 30 })), &channel(), 0.0, "x").detail.as_deref(), Some("30"));
        assert_eq!(convert_moderate(&moderate("followers", json!({ "follow_duration_minutes": 10 })), &channel(), 0.0, "x").detail.as_deref(), Some("10"));
        let delete = moderate("delete", json!({ "user_id": "5", "user_login": "anna", "user_name": "Anna", "message_id": "m1", "message_body": "böse" }));
        let item = convert_moderate(&delete, &channel(), 0.0, "x");
        assert_eq!(item.detail.as_deref(), Some("böse"));
        assert_eq!(removal_of(&item, &delete), Some(Removal::Message { message_id: "m1".into() }));
        let mut terms = moderate("add_blocked_term", Value::Null);
        terms["automod_terms"] = json!({ "action": "add", "list": "blocked", "terms": ["a", "b"] });
        assert_eq!(convert_moderate(&terms, &channel(), 0.0, "x").detail.as_deref(), Some("a, b"));
        let clear = moderate("clear", Value::Null);
        assert_eq!(removal_of(&convert_moderate(&clear, &channel(), 0.0, "x"), &clear), Some(Removal::All));
    }

    #[test]
    fn shared_chat_und_entbannungsanfragen() {
        let mut shared = moderate("shared_chat_ban", json!({ "user_id": "5", "user_login": "anna", "user_name": "Anna" }));
        shared["source_broadcaster_user_login"] = json!("anderer");
        let item = convert_moderate(&shared, &channel(), 0.0, "x");
        assert_eq!(item.source_channel.as_deref(), Some("anderer"));
        assert_eq!(removal_of(&item, &shared), Some(Removal::User { user_id: "5".into(), reason: "ban".into() }));
        let mut unban = moderate("approve_unban_request", Value::Null);
        unban["unban_request"] = json!({ "user_id": "5", "user_login": "anna", "user_name": "Anna", "is_approved": true });
        assert_eq!(convert_moderate(&unban, &channel(), 0.0, "x").target.map(|t| t.login), Some("anna".into()));
    }

    #[test]
    fn automod_gruende_teile_und_status() {
        let event = json!({
            "user_id": "5", "user_login": "anna", "user_name": "Anna", "message_id": "a1",
            "message": { "text": "du Kappa", "fragments": [{ "type": "text", "text": "du ", "emote": null }, { "type": "emote", "text": "Kappa", "emote": { "id": "25" } }] },
            "reason": "automod", "automod": { "category": "swearing", "level": 2 }, "blocked_term": null,
            "held_at": "2026-11-15T12:00:00Z"
        });
        let item = convert_automod_hold(&event, &channel(), 5.0, true);
        assert_eq!(item.reason, "AutoMod: Schimpfwörter (Stufe 2)");
        assert_eq!(item.timestamp, parse_rfc3339_ms("2026-11-15T12:00:00Z").unwrap());
        assert!(matches!(&item.parts[1], MessagePart::Emote { emote } if emote.name == "Kappa"));
        assert_eq!(item.status, "pending");

        let blocked = json!({ "reason": "blocked_term", "blocked_term": { "terms_found": [{}, {}] }, "held_at": "kaputt" });
        let item = convert_automod_hold(&blocked, &channel(), 5.0, true);
        assert_eq!(item.reason, "Gesperrte Begriffe (2)");
        assert_eq!(item.timestamp, 5.0);

        assert_eq!(automod_status_of(Some("APPROVED")), Some("approved"));
        assert_eq!(automod_status_of(Some("invalid")), Some("invalid"));
        assert_eq!(automod_status_of(Some("unbekannt")), None);
        assert_eq!(automod_status_of(None), None);
    }
}
