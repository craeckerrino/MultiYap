//! Verlauf über recent-messages (robotty), einzige Quelle für Chatverlauf. Geprüft am 15.11.2026 im Quellcode des
//! Dienstes (recent-messages2):
//! - GET https://recent-messages.robotty.de/api/v2/recent-messages/<kanal>
//! - Parameter: limit, before/after (Millisekunden), hide_moderated_messages, hide_moderation_messages
//! - Antwort: { messages: IRC-Zeilen, error, error_code }; CORS erlaubt.
//! - Standard laut Konfiguration: höchstens 500 Nachrichten pro Kanal, 24 Stunden; ein Kanal wird erst ab der ersten
//!   Anfrage mitgeschnitten.
//!
//! Ergebnisse werden nur angezeigt, nie gespeichert.

use crate::i18n::tr;
use serde::Serialize;

use crate::community_gifts::{attach_recipient, CommunityGifts, GiftPlacement};
use crate::model::{ChannelRef, ChatEventItem, ChatMessage, ModActionItem, Removed, UserRef};
use crate::twitch::badges::BadgeStore;
use crate::twitch::events::{normalize_usernotice, to_event_item};
use crate::twitch::irc::{convert_privmsg, parse_irc_line, parse_removal, tag_number, HistoryRemoval};
use crate::twitch::url_component;

const BASE_URL: &str = "https://recent-messages.robotty.de/api/v2/recent-messages/";
pub const HISTORY_MAX_AGE_MS: f64 = 24.0 * 60.0 * 60.0 * 1000.0;

/// Adresse der Anfrage. In Nicht-Mod-Kanälen entfernte Nachrichten gar nicht erst laden (wie bei Twitch für
/// Zuschauer).
pub fn history_url(login: &str, after: Option<f64>, hide_moderated: bool) -> String {
    let mut params = Vec::new();
    if let Some(after) = after.filter(|a| a.is_finite()) {
        params.push(format!("after={}", after.floor()));
    }
    if hide_moderated {
        params.push("hide_moderated_messages=true".to_string());
        params.push("hide_moderation_messages=true".to_string());
    }
    let query = if params.is_empty() { String::new() } else { format!("?{}", params.join("&")) };
    format!("{BASE_URL}{}{query}", url_component(login))
}

/// Zeilen aus der Antwort; Fehler bei unerwartetem Aufbau.
pub fn history_lines(body: Option<&serde_json::Value>) -> Result<Vec<String>, String> {
    let messages = body.and_then(|b| b.get("messages")).and_then(|m| m.as_array()).ok_or(tr("Unerwartete Antwort", "Unexpected response"))?;
    Ok(messages.iter().filter_map(|line| line.as_str().map(str::to_string)).collect())
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum HistoryItem {
    Message(Box<ChatMessage>),
    Event(ChatEventItem),
    ModAction(ModActionItem),
}

impl HistoryItem {
    pub fn id(&self) -> &str {
        match self {
            HistoryItem::Message(m) => &m.id,
            HistoryItem::Event(e) => &e.id,
            HistoryItem::ModAction(a) => &a.id,
        }
    }
}

/// Betroffene Person einer Entfernung: Anzeigename aus dem Verlauf, sonst der Login aus der IRC-Zeile.
fn target_of(removal: &HistoryRemoval, items: &[HistoryItem]) -> Option<UserRef> {
    let found = items.iter().rev().find_map(|item| match item {
        HistoryItem::Message(m) => {
            let hit = match removal {
                HistoryRemoval::Message { message_id, .. } => &m.id == message_id,
                HistoryRemoval::User { user_id, .. } => &m.sender.id == user_id,
                HistoryRemoval::All { .. } => false,
            };
            hit.then(|| UserRef { id: m.sender.id.clone(), login: m.sender.login.clone(), display_name: m.sender.display_name.clone() })
        }
        _ => None,
    });
    found.or_else(|| match removal {
        HistoryRemoval::User { user_id, login: Some(login), .. } => {
            Some(UserRef { id: user_id.clone(), login: login.clone(), display_name: login.clone() })
        }
        _ => None,
    })
}

fn removal_timestamp(removal: &HistoryRemoval) -> f64 {
    match removal {
        HistoryRemoval::Message { timestamp, .. } | HistoryRemoval::User { timestamp, .. } | HistoryRemoval::All { timestamp } => *timestamp,
    }
}

/// Zahl wie in JavaScript als Text (ganze Zahlen ohne Nachkommastellen).
fn js_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e21 { format!("{value:.0}") } else { value.to_string() }
}

/// Systemzeile zu einer Entfernung aus dem Verlauf. Ohne Mod und Grund, weil recent-messages beides nicht kennt. Die ID
/// ist fest, damit das Nachladen nach einem Reconnect nichts doppelt zeigt.
fn history_mod_action(removal: &HistoryRemoval, channel: &ChannelRef, target: Option<UserRef>) -> ModActionItem {
    let timestamp = removal_timestamp(removal);
    let (key, action, duration_seconds) = match removal {
        HistoryRemoval::Message { message_id, .. } => (message_id.as_str(), "delete", None),
        HistoryRemoval::User { user_id, reason, duration_seconds, .. } => (user_id.as_str(), reason.as_str(), *duration_seconds),
        HistoryRemoval::All { .. } => ("all", "clear", None),
    };
    ModActionItem {
        kind: "mod-action".into(),
        channel: channel.clone(),
        id: format!("history-mod-{}-{}-{key}", channel.id, js_number(timestamp)),
        timestamp,
        moderator: None,
        action: action.into(),
        target,
        reason: None,
        duration_seconds,
        detail: None,
        source_channel: None,
        from_history: Some(true),
    }
}

/// Wandelt Verlaufszeilen in Nachrichten um: nur PRIVMSG, höchstens 24 Stunden alt. Entfernungen (CLEARMSG,
/// CLEARCHAT) markieren betroffene Nachrichten und erscheinen als eigene Zeile, ohne Mod-Namen (im Verlauf ist meist
/// nur sichtbar, dass etwas passiert ist).
pub fn build_history(
    lines: &[String],
    channel: &ChannelRef,
    badges: &BadgeStore,
    own_user_id: &str,
    now: f64,
    dark: bool,
    gift_lines: bool,
) -> Vec<HistoryItem> {
    let mut items: Vec<HistoryItem> = Vec::new();
    let mut gifts = CommunityGifts::default();
    for raw in lines {
        let Some(line) = parse_irc_line(raw) else { continue };
        if let Some(message) = convert_privmsg(&line, channel, badges, own_user_id, dark) {
            if now - message.timestamp <= HISTORY_MAX_AGE_MS {
                items.push(HistoryItem::Message(Box::new(message)));
            }
            continue;
        }
        // Chat-Ereignisse (USERNOTICE), im Verlauf ausgegraut wie normale Nachrichten.
        if let Some(event) = normalize_usernotice(&line, dark) {
            let Some(timestamp) = tag_number(line.tags.get("tmi-sent-ts")) else { continue };
            if now - timestamp > HISTORY_MAX_AGE_MS {
                continue;
            }
            let item = to_event_item(&event, channel, timestamp, true);
            match gifts.place(&item, event.recipient.as_deref(), gift_lines) {
                GiftPlacement::New => items.push(HistoryItem::Event(item)),
                GiftPlacement::Attach { target_id } => {
                    let target = items.iter_mut().find(|i| i.id() == target_id);
                    if let (Some(HistoryItem::Event(target)), Some(recipient)) = (target, event.recipient.as_deref()) {
                        attach_recipient(target, recipient);
                    }
                }
            }
            continue;
        }
        let Some(removal) = parse_removal(&line) else { continue };
        let timestamp = removal_timestamp(&removal);
        if timestamp.is_finite() && now - timestamp <= HISTORY_MAX_AGE_MS {
            let target = target_of(&removal, &items);
            items.push(HistoryItem::ModAction(history_mod_action(&removal, channel, target)));
        }
        for item in items.iter_mut() {
            let HistoryItem::Message(m) = item else { continue };
            if m.flags.removed.is_some() {
                continue;
            }
            let removed = match &removal {
                HistoryRemoval::Message { message_id, .. } if &m.id == message_id => Some(("deleted", None)),
                HistoryRemoval::User { user_id, reason, duration_seconds, .. }
                    if &m.sender.id == user_id && m.timestamp <= timestamp =>
                {
                    Some((reason.as_str(), *duration_seconds))
                }
                HistoryRemoval::All { .. } if m.timestamp <= timestamp => Some(("clear", None)),
                _ => None,
            };
            if let Some((reason, duration)) = removed {
                m.flags.removed =
                    Some(Removed { reason: reason.into(), by: None, duration_seconds: duration.map(|d| d.max(0.0) as u64) });
            }
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_763_208_000_000.0;

    fn channel() -> ChannelRef {
        ChannelRef { platform: "twitch".into(), id: "430023505".into(), login: "kanal".into(), display_name: "Kanal".into() }
    }

    fn privmsg(id: &str, user: &str, user_id: &str, text: &str, ts: f64) -> String {
        let lower = user.to_lowercase();
        format!(
            "@display-name={user};emotes=;id={id};room-id=430023505;tmi-sent-ts={ts};user-id={user_id} :{lower}!{lower}@x.tmi.twitch.tv PRIVMSG #kanal :{text}"
        )
    }

    fn build(lines: &[String], now: f64) -> Vec<HistoryItem> {
        build_history(lines, &channel(), &BadgeStore::new(vec![], vec![]), "x", now, true, false)
    }

    #[test]
    fn adresse_mit_parametern() {
        assert_eq!(history_url("kanal", None, false), "https://recent-messages.robotty.de/api/v2/recent-messages/kanal");
        assert_eq!(
            history_url("kanal", Some(1234.7), true),
            "https://recent-messages.robotty.de/api/v2/recent-messages/kanal?after=1234&hide_moderated_messages=true&hide_moderation_messages=true"
        );
    }

    #[test]
    fn antwort_pruefen() {
        let body = serde_json::json!({ "messages": ["a", 3, "b"], "error": null });
        assert_eq!(history_lines(Some(&body)).unwrap(), vec!["a", "b"]);
        assert!(history_lines(Some(&serde_json::json!({ "error": "x" }))).is_err());
        assert!(history_lines(None).is_err());
    }

    #[test]
    fn alte_nachrichten_verworfen() {
        let lines = vec![privmsg("m0", "Alt", "2", "alt", NOW - 25.0 * 3_600_000.0), privmsg("m1", "Neu", "3", "neu", NOW - 1000.0)];
        let ids: Vec<_> = build(&lines, NOW).iter().map(|i| i.id().to_string()).collect();
        assert_eq!(ids, vec!["m1"]);
    }

    #[test]
    fn entfernungen_markieren_und_eigene_zeile() {
        let lines = vec![
            privmsg("m1", "Anna", "1", "eins", NOW - 5000.0),
            privmsg("m2", "B", "2", "zwei", NOW - 4500.0),
            format!("@login=anna;target-msg-id=m1;tmi-sent-ts={} :tmi.twitch.tv CLEARMSG #kanal :eins", NOW - 4000.0),
            format!("@ban-duration=600;target-user-id=2;tmi-sent-ts={} :tmi.twitch.tv CLEARCHAT #kanal :b", NOW - 3000.0),
            format!("@target-user-id=7;tmi-sent-ts={} :tmi.twitch.tv CLEARCHAT #kanal :fremd", NOW - 2000.0),
            format!("@tmi-sent-ts={} :tmi.twitch.tv CLEARCHAT #kanal", NOW - 1000.0),
        ];
        let items = build(&lines, NOW);
        let removed: Vec<_> = items
            .iter()
            .filter_map(|i| match i {
                HistoryItem::Message(m) => Some(m.flags.removed.clone().map(|r| (r.reason, r.duration_seconds))),
                _ => None,
            })
            .collect();
        assert_eq!(removed, vec![Some(("deleted".into(), None)), Some(("timeout".into(), Some(600)))]);
        let actions: Vec<_> = items
            .iter()
            .filter_map(|i| match i {
                HistoryItem::ModAction(a) => Some((a.action.clone(), a.target.as_ref().map(|t| t.display_name.clone()), a.id.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(actions[0], ("delete".into(), Some("Anna".into()), format!("history-mod-430023505-{}-m1", NOW - 4000.0)));
        assert_eq!(actions[1].1.as_deref(), Some("B"));
        assert_eq!(actions[2], ("ban".into(), Some("fremd".into()), format!("history-mod-430023505-{}-7", NOW - 2000.0)));
        assert_eq!(actions[3].0, "clear");
        assert!(actions[3].1.is_none());
    }

    #[test]
    fn community_geschenk_eine_zeile() {
        let gift = |kind: &str, extra: &str, id: &str| {
            format!(
                "@display-name=Geber;id={id};login=geber;msg-id={kind};msg-param-community-gift-id=99;{extra}tmi-sent-ts={} :tmi.twitch.tv USERNOTICE #kanal",
                NOW - 1000.0
            )
        };
        let lines = vec![
            gift("submysterygift", "msg-param-mass-gift-count=2;msg-param-sub-plan=1000;", "g"),
            gift("subgift", "msg-param-recipient-display-name=Eins;msg-param-sub-plan=1000;", "s1"),
            gift("subgift", "msg-param-recipient-display-name=Zwei;msg-param-sub-plan=1000;", "s2"),
        ];
        let items = build(&lines, NOW);
        assert_eq!(items.len(), 1);
        let HistoryItem::Event(event) = &items[0] else { panic!("kein Ereignis") };
        assert_eq!(event.recipients.as_deref(), Some(&["Eins".to_string(), "Zwei".to_string()][..]));
    }
}
