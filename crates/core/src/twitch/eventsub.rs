//! EventSub über WebSocket (am 15.11.2026 in der offiziellen Doku geprüft):
//! - Eine Verbindung für alle Kanäle (Twitch erlaubt 3 Verbindungen und 300 Abos je Verbindung).
//! - Abos innerhalb von 10 Sekunden nach dem Welcome anlegen; deshalb wird nur verbunden, wenn Kanäle offen sind.
//! - Keepalive überwachen, Reconnect-Anweisungen befolgen (neue Verbindung, alte erst nach Welcome schließen).
//!
//! Hier die reinen Teile (Abo-Arten, Bedingungen, Wartezeiten, Meldungen); die Verbindung selbst steckt in
//! `crates/wasm/src/eventsub.rs`.

use crate::i18n::tr;
use serde::Serialize;
use serde_json::{json, Value};

pub const EVENTSUB_URL: &str = "wss://eventsub.wss.twitch.tv/ws";
const RETRY_DELAYS_MS: [f64; 5] = [1_000.0, 2_000.0, 5_000.0, 10_000.0, 30_000.0];

/// Wartezeiten für einen neuen Versuch nach einer Ablehnung wegen eines Twitch-Limits (429, Doku geprüft am
/// 16.11.2026: höchstens 3 Verbindungen mit aktiven Abos und höchstens 3 aktive Abos mit gleicher Art und Bedingung
/// je Account). Meist ist MultiYap mehrfach offen; sobald ein Fenster zu ist, klappt der nächste Versuch.
const LIMIT_RETRY_DELAYS_MS: [f64; 4] = [30_000.0, 60_000.0, 120_000.0, 300_000.0];

/// Wartezeit vor dem nächsten Verbindungsversuch nach einer Trennung.
pub fn retry_delay(attempt: usize) -> f64 {
    RETRY_DELAYS_MS[attempt.min(RETRY_DELAYS_MS.len() - 1)]
}

pub fn limit_retry_delay(attempt: usize) -> f64 {
    LIMIT_RETRY_DELAYS_MS[attempt.min(LIMIT_RETRY_DELAYS_MS.len() - 1)]
}

/// Verständliche Meldung zu einer Ablehnung wegen eines Twitch-Limits.
pub fn limit_reason(twitch_message: &str) -> String {
    let message = twitch_message.to_lowercase();
    let what = if message.contains("transport") {
        tr("zu viele Verbindungen", "too many connections")
    } else if message.contains("type and condition") {
        tr("Kanal in zu vielen Fenstern offen", "channel open in too many windows")
    } else {
        tr("Grenze erreicht", "limit reached")
    };
    crate::tf!("Twitch-Limit, {what}. MultiYap ist wohl mehrfach offen (Browser, Programm oder weitere Tabs). Neuer Versuch läuft automatisch.", "Twitch limit, {what}. MultiYap is probably open more than once (browser, program or more tabs). A new attempt runs automatically.")
}

/// Grund zu einem Schließcode von Twitch.
pub fn close_reason(code: u16) -> Option<&'static str> {
    Some(match code {
        4000 => tr("Interner Fehler bei Twitch", "Internal error at Twitch"),
        4001 => tr("Unerlaubte Daten gesendet", "Disallowed data sent"),
        4002 => tr("Ping nicht beantwortet", "Ping not answered"),
        4003 => tr("Verbindung ungenutzt", "Connection unused"),
        4004 => tr("Wiederverbindung zu spät", "Reconnect too late"),
        4005 => tr("Netzwerk-Zeitüberschreitung", "Network timeout"),
        4006 => tr("Netzwerkfehler", "Network error"),
        4007 => tr("Ungültige Wiederverbindungsadresse", "Invalid reconnect address"),
        1000 => tr("Twitch hat die Verbindung beendet", "Twitch closed the connection"),
        1001 => tr("Twitch-Server nicht mehr erreichbar", "Twitch server no longer reachable"),
        1006 => tr("Verbindung unterbrochen (Netzwerk, Ruhezustand oder Twitch)", "Connection interrupted (network, sleep or Twitch)"),
        _ => return None,
    })
}

/// Grund zu jedem Schließcode, unbekannte mit Code (Jede Neuverbindung mit Grund).
pub fn close_reason_text(code: u16) -> String {
    close_reason(code).map_or_else(|| crate::tf!("Verbindung beendet (Code {code})", "Connection closed (code {code})"), str::to_string)
}

/// Zustand eines Kanals für die Oberfläche (Feldnamen wie in TypeScript).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ChannelStatus {
    Connecting,
    Connected,
    Joined,
    Disconnected {
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Reconnected {
        /// Grund der vorherigen Trennung.
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Twitch hat eine neue Verbindung angefordert (session_reconnect); sie wird ohne Unterbrechung übernommen.
    ReconnectRequested,
    Error {
        reason: String,
    },
    /// Mod-Ereignisse (Mod-Aktionen, AutoMod) konnten nicht abonniert werden.
    ModEventsUnavailable {
        reason: String,
    },
    /// Mod-Ereignisse nach einer Ablehnung wegen eines Limits doch noch abonniert.
    ModEventsRestored,
}

/// Gruppe einer Abo-Art.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    /// Für jeden Kanal.
    Base,
    /// Nur in Kanälen mit Mod-Rechten.
    Mod,
    /// Darf fehlschlagen, ohne Meldung.
    Optional,
}

/// Bedingung je Abo-Art (laut Doku unterschiedlich benannt).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Condition {
    /// broadcaster_user_id und user_id.
    Chat,
    /// broadcaster_user_id und moderator_user_id.
    Moderator,
    /// nur broadcaster_user_id.
    Broadcaster,
    /// nur from_broadcaster_user_id.
    FromBroadcaster,
}

#[derive(Debug, PartialEq, Eq)]
pub struct SubscriptionSpec {
    pub name: &'static str,
    pub version: &'static str,
    pub group: Group,
    condition: Condition,
}

const fn spec(name: &'static str, version: &'static str, group: Group, condition: Condition) -> SubscriptionSpec {
    SubscriptionSpec { name, version, group, condition }
}

pub const SUBSCRIPTIONS: [SubscriptionSpec; 14] = [
    spec("channel.chat.message", "1", Group::Base, Condition::Chat),
    // Chat-Modi live (Plaketten in jedem Kanal), Doku geprüft am 15.11.2026.
    spec("channel.chat_settings.update", "1", Group::Base, Condition::Chat),
    // Chat-Ereignisse (Abos, Raids, Watch Streaks, Ankündigungen …), Doku geprüft am 15.11.2026.
    spec("channel.chat.notification", "1", Group::Base, Condition::Chat),
    spec("channel.moderate", "2", Group::Mod, Condition::Moderator),
    spec("automod.message.hold", "2", Group::Mod, Condition::Moderator),
    spec("automod.message.update", "2", Group::Mod, Condition::Moderator),
    // Eingeschränkte und überwachte Nutzer, Doku geprüft am 15.11.2026: Scope moderator:read:suspicious_users,
    // Bedingung broadcaster_user_id und moderator_user_id.
    spec("channel.suspicious_user.message", "1", Group::Mod, Condition::Moderator),
    spec("channel.suspicious_user.update", "1", Group::Mod, Condition::Moderator),
    // Live-Meldung. Kostet bei fremden Kanälen 1 von 10 Kostenpunkten pro Verbindung. Reicht das Budget nicht,
    // übernimmt der minütliche Abgleich über „Get Streams“; ein Fehler ist daher kein Problem.
    spec("stream.online", "1", Group::Optional, Condition::Broadcaster),
    // Eigene Nachrichten, die AutoMod zurückhält, auch ohne Mod-Rechte. Doku geprüft am 23.11.2026: Version 1, Scope
    // user:read:chat, Bedingung broadcaster_user_id und user_id; das Update meldet approved, denied oder invalid. In
    // Mod-Kanälen zeigt AutoMod die Nachricht ohnehin (automod.message.hold).
    spec("channel.chat.user_message_hold", "1", Group::Optional, Condition::Chat),
    spec("channel.chat.user_message_update", "1", Group::Optional, Condition::Chat),
    // Bestätigte Verwarnungen. Doku geprüft am 23.11.2026: Version 1, Scope moderator:read:warnings oder
    // moderator:manage:warnings, Bedingung broadcaster_user_id und moderator_user_id.
    spec("channel.warning.acknowledge", "1", Group::Mod, Condition::Moderator),
    // Shoutouts. Doku geprüft am 23.11.2026: Version 1, Scope moderator:read:shoutouts oder moderator:manage:shoutouts,
    // Bedingung broadcaster_user_id und moderator_user_id.
    spec("channel.shoutout.create", "1", Group::Mod, Condition::Moderator),
    // Ausgeführter Raid aus dem Kanal. Doku geprüft am 23.11.2026: Version 1, keine Berechtigung nötig, Bedingung
    // from_broadcaster_user_id. Kostet bei fremden Kanälen einen Kostenpunkt, daher optional; der Start eines Raids
    // kommt als Mod-Aktion „raid“ (channel.moderate).
    spec("channel.raid", "1", Group::Optional, Condition::FromBroadcaster),
];

/// Gewünschte Abos eines Kanals in fester Reihenfolge: Grund-Abos, Mod-Abos (nur mit Mod-Rechten), dann optionale,
/// die in dieser Session noch nicht abgelehnt wurden.
pub fn wanted_subscriptions(moderator: bool, optional_failed: impl Fn(&str) -> bool) -> Vec<&'static SubscriptionSpec> {
    SUBSCRIPTIONS
        .iter()
        .filter(|spec| match spec.group {
            Group::Base => true,
            Group::Mod => moderator,
            Group::Optional => !optional_failed(spec.name),
        })
        .collect()
}

/// Mod-Abos vollständig vorhanden?
pub fn has_all_mod_subscriptions(has: impl Fn(&str) -> bool) -> bool {
    SUBSCRIPTIONS.iter().filter(|spec| spec.group == Group::Mod).all(|spec| has(spec.name))
}

impl SubscriptionSpec {
    pub fn condition(&self, broadcaster_id: &str, user_id: &str) -> Value {
        match self.condition {
            Condition::Chat => json!({ "broadcaster_user_id": broadcaster_id, "user_id": user_id }),
            Condition::Moderator => json!({ "broadcaster_user_id": broadcaster_id, "moderator_user_id": user_id }),
            Condition::Broadcaster => json!({ "broadcaster_user_id": broadcaster_id }),
            Condition::FromBroadcaster => json!({ "from_broadcaster_user_id": broadcaster_id }),
        }
    }
}

/// Inhalt für POST /eventsub/subscriptions.
pub fn subscription_body(spec: &SubscriptionSpec, broadcaster_id: &str, user_id: &str, session_id: &str) -> Value {
    json!({
        "type": spec.name,
        "version": spec.version,
        "condition": spec.condition(broadcaster_id, user_id),
        "transport": { "method": "websocket", "session_id": session_id },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wartezeiten() {
        let limits: Vec<f64> = [0, 1, 2, 3, 9].into_iter().map(limit_retry_delay).collect();
        assert_eq!(limits, vec![30_000.0, 60_000.0, 120_000.0, 300_000.0, 300_000.0]);
        assert_eq!(retry_delay(0), 1_000.0);
        assert_eq!(retry_delay(20), 30_000.0);
    }

    #[test]
    fn limit_meldung() {
        assert!(limit_reason("number of websocket transports limit exceeded").contains("zu viele Verbindungen"));
        assert!(limit_reason("maximum subscriptions with type and condition exceeded").contains("Kanal in zu vielen Fenstern offen"));
        assert!(limit_reason("something else").contains("Grenze erreicht"));
    }

    #[test]
    fn abos_je_kanal() {
        let names = |list: Vec<&SubscriptionSpec>| list.iter().map(|s| s.name).collect::<Vec<_>>();
        let viewer = names(wanted_subscriptions(false, |_| false));
        assert_eq!(
            viewer,
            vec![
                "channel.chat.message",
                "channel.chat_settings.update",
                "channel.chat.notification",
                "stream.online",
                "channel.chat.user_message_hold",
                "channel.chat.user_message_update",
                "channel.raid"
            ]
        );
        let moderator = wanted_subscriptions(true, |name| name == "stream.online");
        assert_eq!(moderator.len(), 13);
        assert_eq!(moderator.iter().filter(|s| s.group == Group::Optional).count(), 3);
    }

    #[test]
    fn bedingungen_und_inhalt() {
        let moderate = SUBSCRIPTIONS.iter().find(|s| s.name == "channel.moderate").unwrap();
        assert_eq!(
            subscription_body(moderate, "b1", "u1", "s1"),
            json!({
                "type": "channel.moderate",
                "version": "2",
                "condition": { "broadcaster_user_id": "b1", "moderator_user_id": "u1" },
                "transport": { "method": "websocket", "session_id": "s1" },
            })
        );
        assert_eq!(SUBSCRIPTIONS[0].condition("b1", "u1"), json!({ "broadcaster_user_id": "b1", "user_id": "u1" }));
        assert_eq!(SUBSCRIPTIONS[8].condition("b1", "u1"), json!({ "broadcaster_user_id": "b1" }));
    }

    #[test]
    fn status_wie_in_typescript() {
        let text = |s: ChannelStatus| serde_json::to_string(&s).unwrap();
        assert_eq!(text(ChannelStatus::ModEventsUnavailable { reason: "x".into() }), r#"{"type":"mod-events-unavailable","reason":"x"}"#);
        assert_eq!(text(ChannelStatus::Disconnected { reason: None }), r#"{"type":"disconnected"}"#);
        assert_eq!(text(ChannelStatus::Joined), r#"{"type":"joined"}"#);
    }
}
