//! Eingeschränkte und überwachte Nutzer, übertragen aus src/core/moderation/suspicious.ts (Status und Gründe) und
//! src/platforms/twitch/convert-suspicious.ts.

use crate::i18n::tr;
use serde::Serialize;
use serde_json::{json, Value};

use crate::model::{ChannelRef, ModActionItem, UserRef};

/// EventSub-Wert von low_trust_status: none, active_monitoring, restricted.
pub fn status_from_event(value: Option<&str>) -> Option<&'static str> {
    match value {
        Some("restricted") => Some("restricted"),
        Some("active_monitoring") => Some("monitored"),
        _ => None,
    }
}

/// Helix-Wert: ACTIVE_MONITORING, RESTRICTED (beim Entfernen NO_TREATMENT).
pub fn status_from_helix(value: Option<&str>) -> Option<&'static str> {
    match value {
        Some("RESTRICTED") => Some("restricted"),
        Some("ACTIVE_MONITORING") => Some("monitored"),
        _ => None,
    }
}

/// Lesbare Gründe aus den EventSub-Feldern. Unbekannte Werte werden ausgelassen.
pub fn reasons_from_event(event: &Value) -> Vec<String> {
    let mut reasons = Vec::new();
    let types = event.get("types").and_then(Value::as_array).cloned().unwrap_or_default();
    for kind in types.iter().filter_map(Value::as_str) {
        match kind {
            "manually_added" => reasons.push(tr("Manuell hinzugefügt", "Added manually").to_string()),
            "ban_evader" => {
                // Wortlaut nach (23.11.2026), vorher „Möglicher Ban-Umgeher (möglich)“.
                reasons.push(match event.get("ban_evasion_evaluation").and_then(Value::as_str) {
                    Some("likely") => tr("Wahrscheinliche Bannumgehung", "Likely ban evasion").into(),
                    _ => tr("Mögliche Bannumgehung", "Possible ban evasion").into(),
                });
            }
            "banned_in_shared_channel" => {
                let count = event.get("shared_ban_channel_ids").and_then(Value::as_array).map_or(0, Vec::len);
                reasons.push(match count {
                    0 => tr("In geteilten Kanälen gebannt", "Banned in shared channels").into(),
                    1 => tr("In 1 geteiltem Kanal gebannt", "Banned in 1 shared channel").into(),
                    n => crate::tf!("In {n} geteilten Kanälen gebannt", "Banned in {n} shared channels"),
                });
            }
            _ => {}
        }
    }
    reasons
}

/// Gründe aus der Helix-Antwort (types in Großbuchstaben, ohne Bewertung und Kanalliste).
pub fn reasons_from_helix(types: &[String]) -> Vec<String> {
    types
        .iter()
        .filter_map(|kind| match kind.as_str() {
            "MANUALLY_ADDED" => Some(tr("Manuell hinzugefügt", "Added manually")),
            "DETECTED_BAN_EVADER" => Some(tr("Mögliche Bannumgehung", "Possible ban evasion")),
            "DETECTED_SUS_CHATTER" => Some(tr("Verdächtig laut Twitch", "Suspicious according to Twitch")),
            "BANNED_IN_SHARED_CHANNEL" => Some(tr("In geteilten Kanälen gebannt", "Banned in shared channels")),
            _ => None,
        })
        .map(str::to_string)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SuspiciousInfo {
    pub status: &'static str,
    pub reasons: Vec<String>,
}

pub fn suspicious_info_of(event: &Value) -> Option<SuspiciousInfo> {
    let status = status_from_event(event.get("low_trust_status").and_then(Value::as_str))?;
    Some(SuspiciousInfo { status, reasons: reasons_from_event(event) })
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Mod-Zeile „… hat … als überwacht markiert“ usw. Beim Entfernen steht der vorherige Status in detail.
pub fn convert_suspicious_update(
    event: &Value,
    channel: &ChannelRef,
    previous: Option<&str>,
    received_at: f64,
    id: &str,
) -> ModActionItem {
    let status = status_from_event(event.get("low_trust_status").and_then(Value::as_str));
    let moderator_id = text(event, "moderator_user_id");
    let or_login = |name: &str, login: &str| if name.is_empty() { login.to_string() } else { name.to_string() };
    ModActionItem {
        kind: "mod-action".into(),
        channel: channel.clone(),
        id: id.into(),
        timestamp: received_at,
        moderator: (!moderator_id.is_empty()).then(|| UserRef {
            id: moderator_id.into(),
            login: text(event, "moderator_user_login").into(),
            display_name: or_login(text(event, "moderator_user_name"), text(event, "moderator_user_login")),
        }),
        action: match status {
            Some("restricted") => "suspicious_restrict",
            Some("monitored") => "suspicious_monitor",
            _ => "suspicious_none",
        }
        .into(),
        target: Some(UserRef {
            id: text(event, "user_id").into(),
            login: text(event, "user_login").into(),
            display_name: or_login(text(event, "user_name"), text(event, "user_login")),
        }),
        reason: None,
        duration_seconds: None,
        detail: if status.is_some() { None } else { previous.map(str::to_string) },
        source_channel: None,
        from_history: None,
    }
}

/// Aus channel.suspicious_user.message eine channel.chat.message machen, falls Twitch die Nachricht nicht
/// zusätzlich schickt (zum Beispiel bei eingeschränkten Personen). Ohne Abzeichen und Namensfarbe.
pub fn to_chat_message_event(event: &Value) -> Value {
    let message = event.get("message").cloned().unwrap_or(Value::Null);
    let fragments: Vec<Value> = message
        .get("fragments")
        .and_then(Value::as_array)
        .map(|fragments| {
            fragments
                .iter()
                .map(|fragment| {
                    let emote = fragment.get("emote").filter(|e| !e.is_null());
                    let kind = if text(fragment, "type") == "emote" && emote.is_some() { "emote" } else { "text" };
                    let emote = emote.map(|e| {
                        let mut copy = e.clone();
                        copy["owner_id"] = json!("");
                        copy["format"] = json!([]);
                        copy
                    });
                    json!({ "type": kind, "text": text(fragment, "text"), "emote": emote, "mention": null })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "broadcaster_user_id": text(event, "broadcaster_user_id"),
        "chatter_user_id": text(event, "user_id"),
        "chatter_user_login": text(event, "user_login"),
        "chatter_user_name": text(event, "user_name"),
        "message_id": text(&message, "message_id"),
        "message": { "text": text(&message, "text"), "fragments": fragments },
        "badges": [],
        "color": "",
        "reply": null,
        "source_broadcaster_user_id": null,
        "source_broadcaster_user_login": null,
        "source_broadcaster_user_name": null,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel() -> ChannelRef {
        ChannelRef { platform: "twitch".into(), id: "1".into(), login: "kanal".into(), display_name: "Kanal".into() }
    }

    #[test]
    fn status_und_gruende() {
        assert_eq!(status_from_event(Some("active_monitoring")), Some("monitored"));
        assert_eq!(status_from_event(Some("none")), None);
        assert_eq!(status_from_helix(Some("RESTRICTED")), Some("restricted"));
        let reasons = reasons_from_event(&json!({
            "types": ["manually_added", "ban_evader", "banned_in_shared_channel", "neu"],
            "ban_evasion_evaluation": "likely", "shared_ban_channel_ids": ["a", "b"]
        }));
        assert_eq!(reasons, ["Manuell hinzugefügt", "Wahrscheinliche Bannumgehung", "In 2 geteilten Kanälen gebannt"]);
        assert_eq!(reasons_from_event(&json!({ "types": ["banned_in_shared_channel"], "shared_ban_channel_ids": ["a"] })), ["In 1 geteiltem Kanal gebannt"]);
        assert_eq!(reasons_from_event(&json!({ "types": null })), Vec::<String>::new());
        assert_eq!(reasons_from_helix(&["DETECTED_SUS_CHATTER".into(), "X".into()]), ["Verdächtig laut Twitch"]);
    }

    #[test]
    fn mod_zeile_bei_aenderung() {
        let event = json!({ "moderator_user_id": "9", "moderator_user_login": "modi", "moderator_user_name": "", "user_id": "5", "user_login": "anna", "user_name": "Anna", "low_trust_status": "restricted" });
        let item = convert_suspicious_update(&event, &channel(), None, 1.0, "mod-1");
        assert_eq!(item.action, "suspicious_restrict");
        assert_eq!(item.moderator.unwrap().display_name, "modi");
        let removed = json!({ "user_id": "5", "user_login": "anna", "user_name": "Anna", "low_trust_status": "none" });
        let item = convert_suspicious_update(&removed, &channel(), Some("monitored"), 1.0, "mod-2");
        assert_eq!(item.action, "suspicious_none");
        assert_eq!(item.detail.as_deref(), Some("monitored"));
        assert_eq!(item.moderator, None);
    }

    #[test]
    fn nachricht_als_chat_ereignis() {
        let event = json!({
            "broadcaster_user_id": "1", "user_id": "5", "user_login": "anna", "user_name": "Anna", "low_trust_status": "restricted",
            "message": { "message_id": "m1", "text": "hi Kappa", "fragments": [
                { "type": "text", "text": "hi ", "emote": null, "cheermote": null },
                { "type": "emote", "text": "Kappa", "emote": { "id": "25", "emote_set_id": "0" }, "cheermote": null } ] }
        });
        let converted = to_chat_message_event(&event);
        assert_eq!(converted["chatter_user_login"], "anna");
        assert_eq!(converted["message_id"], "m1");
        assert_eq!(converted["message"]["fragments"][1]["type"], "emote");
        assert_eq!(converted["message"]["fragments"][1]["emote"]["format"], json!([]));
        assert_eq!(suspicious_info_of(&event).map(|i| i.status), Some("restricted"));
    }
}
