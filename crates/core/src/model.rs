//! Gemeinsames, plattformunabhängiges Nachrichtenmodell, übertragen aus src/core/model/message.ts. Feldnamen und Form
//! sind 1:1 gleich, damit die Oberfläche dieselben Objekte bekommt. Chatnachrichten werden nie dauerhaft gespeichert,
//! sie leben nur im Arbeitsspeicher.

use serde::{Deserialize, Serialize};

/// Kanal auf einer Plattform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelRef {
    pub platform: String,
    /// Feste ID der Plattform (bei Twitch die Nutzer-ID des Kanals).
    pub id: String,
    /// Login-Name in Kleinbuchstaben.
    pub login: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sender {
    pub id: String,
    pub login: String,
    pub display_name: String,
    /// Namensfarbe als CSS-Farbwert, falls die Plattform eine liefert.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// Abzeichen am Namen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Badge {
    pub source: String,
    pub set_id: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url2x: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Emote {
    pub source: String,
    pub id: String,
    pub name: String,
    pub image_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url2x: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub animated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global: Option<bool>,
}

/// Bestandteile eines Nachrichtentextes. Alles wird als Text gerendert, nie als HTML.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum MessagePart {
    Text { text: String },
    Emote { emote: Emote },
    Mention { text: String, login: String },
    Link { text: String, url: String },
    /// Bits (Cheermote), zum Beispiel „Cheer100“.
    Cheer { text: String, prefix: String, bits: u64 },
    /// Emoji-Code von YouTube wie „:face-purple-sweating:“ (ohne Bild, nur Text; abgeschwächt gezeigt).
    Shortcode { text: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suspicious {
    /// "monitored" oder "restricted".
    pub status: String,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Removed {
    /// "deleted", "timeout", "ban" oder "clear".
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageFlags {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_history: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_message: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suspicious: Option<Suspicious>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub own: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed: Option<Removed>,
}

/// Antwort auf eine andere Nachricht.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyRef {
    pub message_id: String,
    pub sender_login: String,
    pub sender_display_name: String,
    pub text: String,
}

/// Nutzer in Mod-Aktionen, AutoMod-Zeilen und Chat-Ereignissen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserRef {
    pub id: String,
    pub login: String,
    pub display_name: String,
}

/// Mod-Aktion: wer hat was gemacht. Nur in Mod-Kanälen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModActionItem {
    /// Immer "mod-action".
    pub kind: String,
    pub channel: ChannelRef,
    pub id: String,
    pub timestamp: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moderator: Option<UserRef>,
    /// Twitch-Aktionsname (zum Beispiel "timeout") oder eigene Namen wie "suspicious_monitor".
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<UserRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_channel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_history: Option<bool>,
}

/// Von AutoMod zurückgehaltene Nachricht. Nur in Mod-Kanälen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoModItem {
    /// Immer "automod".
    pub kind: String,
    pub channel: ChannelRef,
    pub id: String,
    pub timestamp: f64,
    pub sender: UserRef,
    pub text: String,
    pub parts: Vec<MessagePart>,
    pub reason: String,
    /// "pending", "approved", "denied", "expired" oder "invalid".
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_by: Option<String>,
    /// Eigene Nachricht in einem Kanal ohne Mod-Rechte (channel.chat.user_message_hold): nur Hinweis, keine Knöpfe.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub own: bool,
}

/// Chat-Ereignis: Abos, Raids, Watch Streaks und ähnliche, in leichtem Rosa.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatEventItem {
    /// Immer "event".
    pub kind: String,
    pub channel: ChannelRef,
    pub id: String,
    pub timestamp: f64,
    pub event_type: String,
    /// Deutscher Meldungstext.
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Vec<MessagePart>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipients: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub community_gift_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub users: Option<Vec<UserRef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_history: Option<bool>,
    /// Link am Ende der Zeile, öffnet im normalen Browser (zum Beispiel „Raid beitreten“).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<EventLink>,
}

/// Link eines Chat-Ereignisses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventLink {
    pub label: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    /// Immer "message".
    pub kind: String,
    pub channel: ChannelRef,
    pub id: String,
    pub sender: Sender,
    /// Millisekunden seit 1970 (UTC).
    pub timestamp: f64,
    pub text: String,
    pub parts: Vec<MessagePart>,
    pub badges: Vec<Badge>,
    pub flags: MessageFlags,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<ReplyRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_channel: Option<ChannelRef>,
}
