//! channel.chat.message v1 (EventSub) in eine Chatnachricht umwandeln, übertragen aus
//! src/platforms/twitch/convert.ts.

use serde::{Deserialize, Deserializer};

use super::badges::{BadgeStore, RawBadge};
use super::emote_url;
use crate::model::{ChannelRef, ChatMessage, Emote, MessageFlags, MessagePart, ReplyRef, Sender};

/// Text, der bei Twitch auch `null` sein kann: dann leer.
pub fn null_as_empty<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

/// Felder aus channel.chat.message v1, die MultiYap nutzt. Unbekannte Felder werden ignoriert.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ChatMessageEvent {
    pub broadcaster_user_id: String,
    pub chatter_user_id: String,
    pub chatter_user_login: String,
    pub chatter_user_name: String,
    pub message_id: String,
    pub message: EventMessage,
    pub badges: Vec<EventBadge>,
    #[serde(deserialize_with = "null_as_empty")]
    pub color: String,
    pub reply: Option<EventReply>,
    pub source_broadcaster_user_id: Option<String>,
    pub source_broadcaster_user_login: Option<String>,
    pub source_broadcaster_user_name: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EventMessage {
    pub text: String,
    pub fragments: Vec<Fragment>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Fragment {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
    pub emote: Option<FragmentEmote>,
    pub mention: Option<FragmentMention>,
    pub cheermote: Option<FragmentCheermote>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FragmentEmote {
    pub id: String,
    pub format: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FragmentMention {
    pub user_login: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FragmentCheermote {
    pub prefix: String,
    pub bits: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EventBadge {
    pub set_id: String,
    pub id: String,
    #[serde(deserialize_with = "null_as_empty")]
    pub info: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EventReply {
    pub parent_message_id: String,
    pub parent_message_body: String,
    pub parent_user_login: String,
    pub parent_user_name: String,
}

/// Twitch-Emote als Teil einer Nachricht (`animated` je nach angebotenem Format).
pub fn twitch_emote(id: &str, name: &str, animated: bool, dark: bool) -> Emote {
    Emote {
        source: "twitch".into(),
        id: id.into(),
        name: name.into(),
        image_url: emote_url(id, animated, dark, "1.0"),
        image_url2x: Some(emote_url(id, animated, dark, "2.0")),
        animated: Some(animated),
        overlay: None,
        owner_id: None,
        global: None,
    }
}

/// Textteile aus den Fragmenten: Emotes, Erwähnungen, Bits; Text und Unbekanntes bleiben Text (zusammengefasst).
pub fn to_parts(fragments: &[Fragment], dark: bool) -> Vec<MessagePart> {
    let mut parts: Vec<MessagePart> = Vec::new();
    for fragment in fragments {
        match (fragment.kind.as_str(), &fragment.emote, &fragment.mention, &fragment.cheermote) {
            ("emote", Some(emote), _, _) => {
                let animated = emote.format.iter().any(|f| f == "animated");
                parts.push(MessagePart::Emote { emote: twitch_emote(&emote.id, &fragment.text, animated, dark) });
            }
            ("mention", _, Some(mention), _) => {
                parts.push(MessagePart::Mention { text: fragment.text.clone(), login: mention.user_login.clone() });
            }
            ("cheermote", _, _, Some(cheer)) => parts.push(MessagePart::Cheer {
                text: fragment.text.clone(),
                prefix: cheer.prefix.clone(),
                bits: cheer.bits,
            }),
            _ => push_text(&mut parts, &fragment.text),
        }
    }
    parts
}

/// Text anhängen; folgt er auf Text, wird er angefügt statt als eigener Teil.
pub fn push_text(parts: &mut Vec<MessagePart>, text: &str) {
    if let Some(MessagePart::Text { text: last }) = parts.last_mut() {
        last.push_str(text);
    } else {
        parts.push(MessagePart::Text { text: text.to_string() });
    }
}

/// Namensfarbe nur als „#rrggbb“, sonst keine.
pub fn valid_color(color: &str) -> Option<String> {
    let valid = color.len() == 7 && color.starts_with('#') && color[1..].chars().all(|c| c.is_ascii_hexdigit());
    valid.then(|| color.to_string())
}

pub fn convert_chat_message(
    event: &ChatMessageEvent,
    channel: &ChannelRef,
    badges: &BadgeStore,
    own_user_id: &str,
    received_at: f64,
    dark: bool,
) -> ChatMessage {
    let foreign_source = event
        .source_broadcaster_user_id
        .as_deref()
        .is_some_and(|source| source != event.broadcaster_user_id);
    let source_channel = match (foreign_source, &event.source_broadcaster_user_id, &event.source_broadcaster_user_login) {
        (true, Some(id), Some(login)) if !id.is_empty() && !login.is_empty() => Some(ChannelRef {
            platform: "twitch".into(),
            id: id.clone(),
            login: login.clone(),
            display_name: event.source_broadcaster_user_name.clone().unwrap_or_else(|| login.clone()),
        }),
        _ => None,
    };
    ChatMessage {
        kind: "message".into(),
        channel: channel.clone(),
        id: event.message_id.clone(),
        sender: Sender {
            id: event.chatter_user_id.clone(),
            login: event.chatter_user_login.clone(),
            display_name: event.chatter_user_name.clone(),
            color: valid_color(&event.color),
        },
        // EventSub liefert keinen Zeitstempel je Nachricht; Empfangszeit genügt für die Anzeige.
        timestamp: received_at,
        text: event.message.text.clone(),
        parts: to_parts(&event.message.fragments, dark),
        badges: badges.resolve(event.badges.iter().map(|b| RawBadge { set_id: &b.set_id, id: &b.id, info: &b.info })),
        flags: MessageFlags { own: Some(event.chatter_user_id == own_user_id), ..MessageFlags::default() },
        reply: event.reply.as_ref().map(|reply| ReplyRef {
            message_id: reply.parent_message_id.clone(),
            sender_login: reply.parent_user_login.clone(),
            sender_display_name: reply.parent_user_name.clone(),
            text: reply.parent_message_body.clone(),
        }),
        source_channel,
    }
}

#[cfg(test)]
mod tests {
    use super::super::badges::BadgeVersion;
    use super::*;

    fn channel() -> ChannelRef {
        ChannelRef { platform: "twitch".into(), id: "1".into(), login: "kanal".into(), display_name: "Kanal".into() }
    }

    fn event() -> ChatMessageEvent {
        serde_json::from_str(
            r##"{
              "broadcaster_user_id": "1", "chatter_user_id": "42", "chatter_user_login": "nutzer", "chatter_user_name": "Nutzer",
              "message_id": "m1",
              "message": { "text": "Hallo Kappa @Kanal", "fragments": [
                { "type": "text", "text": "Hallo ", "emote": null, "mention": null },
                { "type": "emote", "text": "Kappa", "emote": { "id": "25", "emote_set_id": "0", "owner_id": "0", "format": ["static"] }, "mention": null },
                { "type": "text", "text": " ", "emote": null, "mention": null },
                { "type": "mention", "text": "@Kanal", "emote": null, "mention": { "user_id": "1", "user_name": "Kanal", "user_login": "kanal" } }
              ] },
              "badges": [{ "set_id": "moderator", "id": "1", "info": "" }],
              "color": "#FF0000", "reply": null,
              "source_broadcaster_user_id": null, "source_broadcaster_user_login": null, "source_broadcaster_user_name": null,
              "unbekanntes_feld": 1
            }"##,
        )
        .unwrap()
    }

    fn global() -> BadgeStore {
        BadgeStore::new(
            [],
            [(
                "moderator".into(),
                "1".into(),
                BadgeVersion { title: "Moderator".into(), image_url_1x: "mod1".into(), image_url_2x: "mod2".into() },
            )],
        )
    }

    #[test]
    fn textteile_emotes_erwaehnungen_abzeichen() {
        let message = convert_chat_message(&event(), &channel(), &global(), "42", 1000.0, true);
        assert_eq!(
            message.parts,
            vec![
                MessagePart::Text { text: "Hallo ".into() },
                MessagePart::Emote { emote: twitch_emote("25", "Kappa", false, true) },
                MessagePart::Text { text: " ".into() },
                MessagePart::Mention { text: "@Kanal".into(), login: "kanal".into() },
            ]
        );
        assert_eq!(message.badges[0].title.as_deref(), Some("Moderator"));
        assert_eq!(message.sender.color.as_deref(), Some("#FF0000"));
        assert_eq!(message.flags.own, Some(true));
        assert_eq!(message.timestamp, 1000.0);
        let serialized = serde_json::to_value(&message).unwrap();
        assert_eq!(serialized["parts"][1]["emote"]["imageUrl"], "https://static-cdn.jtvnw.net/emoticons/v2/25/static/dark/1.0");
        assert_eq!(serialized["kind"], "message");
        assert!(serialized.get("reply").is_none(), "fehlende Felder fehlen wie in TypeScript");
    }

    #[test]
    fn animierte_emotes() {
        let mut e = event();
        e.message.fragments[1].emote.as_mut().unwrap().format = vec!["static".into(), "animated".into()];
        let message = convert_chat_message(&e, &channel(), &BadgeStore::default(), "x", 0.0, true);
        let MessagePart::Emote { emote } = &message.parts[1] else { panic!("kein Emote") };
        assert!(emote.image_url.contains("/animated/"));
    }

    #[test]
    fn ungueltige_farbe_und_shared_chat() {
        let mut e = event();
        e.color = String::new();
        e.source_broadcaster_user_id = Some("9".into());
        e.source_broadcaster_user_login = Some("anderer".into());
        e.source_broadcaster_user_name = Some("Anderer".into());
        let message = convert_chat_message(&e, &channel(), &BadgeStore::default(), "x", 0.0, true);
        assert_eq!(message.sender.color, None);
        assert_eq!(message.source_channel.as_ref().map(|c| c.login.as_str()), Some("anderer"));
        assert_eq!(message.flags.own, Some(false));
    }

    #[test]
    fn fehlende_und_null_felder_werden_vertragen() {
        let e: ChatMessageEvent = serde_json::from_str(
            r#"{ "broadcaster_user_id": "1", "chatter_user_id": "2", "chatter_user_login": "a", "chatter_user_name": "A",
                 "message_id": "m", "message": { "text": "hi" }, "color": null,
                 "badges": [{ "set_id": "vip", "id": "1", "info": null }] }"#,
        )
        .unwrap();
        let message = convert_chat_message(&e, &channel(), &BadgeStore::default(), "x", 0.0, true);
        assert_eq!(message.sender.color, None);
        assert_eq!(message.badges[0].info, None);
        assert!(message.parts.is_empty());
    }

    #[test]
    fn bits_und_unbekannte_fragmente() {
        let e: ChatMessageEvent = serde_json::from_str(
            r#"{ "broadcaster_user_id": "1", "chatter_user_id": "2", "chatter_user_login": "a", "chatter_user_name": "A",
                 "message_id": "m", "message": { "text": "Cheer100 hi", "fragments": [
                   { "type": "cheermote", "text": "Cheer100", "cheermote": { "prefix": "cheer", "bits": 100, "tier": 1 } },
                   { "type": "neu", "text": " h" }, { "type": "text", "text": "i" } ] } }"#,
        )
        .unwrap();
        let message = convert_chat_message(&e, &channel(), &BadgeStore::default(), "x", 0.0, true);
        assert_eq!(
            message.parts,
            vec![
                MessagePart::Cheer { text: "Cheer100".into(), prefix: "cheer".into(), bits: 100 },
                MessagePart::Text { text: " hi".into() },
            ]
        );
    }
}
