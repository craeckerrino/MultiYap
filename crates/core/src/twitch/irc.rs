//! Twitch-IRC-Zeilen (IRCv3 mit Tags), wie sie recent-messages liefert, übertragen aus
//! src/platforms/twitch/irc.ts. Nur Text wird erzeugt, nie HTML.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::badges::{BadgeStore, RawBadge};
use super::emote_url;
use super::chat::valid_color;
use crate::model::{ChannelRef, ChatMessage, Emote, MessageFlags, MessagePart, ReplyRef, Sender};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrcLine {
    pub tags: BTreeMap<String, String>,
    pub prefix: String,
    pub command: String,
    pub params: Vec<String>,
}

/// Escapes in Tag-Werten auflösen: \s Leerzeichen, \: Semikolon, \r, \n, sonst das Zeichen selbst.
fn unescape_tag(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(char) = chars.next() {
        if char != '\\' {
            out.push(char);
            continue;
        }
        // Wie der JavaScript-Ausdruck /\\(.)/: Zeilenumbrüche zählen nicht als Zeichen danach.
        match chars.peek().copied() {
            Some(next) if !matches!(next, '\n' | '\r' | '\u{2028}' | '\u{2029}') => {
                chars.next();
                out.push(match next {
                    's' => ' ',
                    ':' => ';',
                    'r' => '\r',
                    'n' => '\n',
                    other => other,
                });
            }
            _ => out.push('\\'),
        }
    }
    out
}

pub fn parse_irc_line(line: &str) -> Option<IrcLine> {
    let mut rest = line;
    let mut tags = BTreeMap::new();
    if let Some(after) = rest.strip_prefix('@') {
        let end = after.find(' ')?;
        for pair in after[..end].split(';') {
            match pair.find('=') {
                None => {
                    tags.insert(pair.to_string(), String::new());
                }
                Some(eq) => {
                    tags.insert(pair[..eq].to_string(), unescape_tag(&pair[eq + 1..]));
                }
            }
        }
        rest = &after[end + 1..];
    }
    let mut prefix = String::new();
    if let Some(after) = rest.strip_prefix(':') {
        let end = after.find(' ')?;
        prefix = after[..end].to_string();
        rest = &after[end + 1..];
    }
    let mut params = Vec::new();
    while !rest.is_empty() {
        if let Some(trailing) = rest.strip_prefix(':') {
            params.push(trailing.to_string());
            break;
        }
        match rest.find(' ') {
            None => {
                params.push(rest.to_string());
                break;
            }
            Some(end) => {
                params.push(rest[..end].to_string());
                rest = &rest[end + 1..];
            }
        }
    }
    if params.is_empty() {
        return None;
    }
    let command = params.remove(0);
    if command.is_empty() {
        return None;
    }
    Some(IrcLine { tags, prefix, command, params })
}

/// "subscriber/12,premium/1" in Paare zerlegen.
fn parse_badge_list(value: Option<&String>) -> Vec<(String, String)> {
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return Vec::new();
    };
    value
        .split(',')
        .filter_map(|entry| {
            let mut parts = entry.split('/');
            let set = parts.next()?;
            let id = parts.next()?;
            (!set.is_empty()).then(|| (set.to_string(), id.to_string()))
        })
        .collect()
}

fn mention_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"@[A-Za-z0-9_]{1,25}").expect("festes Muster"))
}

/// Text anhängen und Erwähnungen (@name) als eigene Teile abtrennen, wie bei EventSub.
fn push_text_with_mentions(parts: &mut Vec<MessagePart>, value: &str) {
    let mut last = 0;
    for found in mention_pattern().find_iter(value) {
        if found.start() > last {
            super::chat::push_text(parts, &value[last..found.start()]);
        }
        let text = found.as_str();
        parts.push(MessagePart::Mention { text: text.to_string(), login: text[1..].to_lowercase() });
        last = found.end();
    }
    if last < value.len() {
        super::chat::push_text(parts, &value[last..]);
    }
}

/// Teilt den Text an Emote-Positionen. Twitch zählt Positionen in Unicode-Zeichen (Codepoints).
pub fn split_emotes(text: &str, emotes_tag: Option<&str>, dark: bool) -> Vec<MessagePart> {
    let chars: Vec<char> = text.chars().collect();
    let mut ranges: Vec<(usize, usize, &str)> = Vec::new();
    for group in emotes_tag.unwrap_or("").split('/') {
        let mut pieces = group.split(':');
        let (Some(id), Some(positions)) = (pieces.next(), pieces.next()) else {
            continue;
        };
        if id.is_empty() || positions.is_empty() {
            continue;
        }
        for range in positions.split(',') {
            let mut bounds = range.split('-');
            let start = bounds.next().and_then(|s| s.parse::<usize>().ok());
            let end = bounds.next().and_then(|s| s.parse::<usize>().ok());
            if let (Some(start), Some(end)) = (start, end) {
                if start <= end && end < chars.len() {
                    ranges.push((start, end, id));
                }
            }
        }
    }
    ranges.sort_by_key(|range| range.0);

    let mut parts = Vec::new();
    let mut cursor = 0;
    for (start, end, id) in ranges {
        if start < cursor {
            continue;
        }
        push_text_with_mentions(&mut parts, &chars[cursor..start].iter().collect::<String>());
        parts.push(MessagePart::Emote {
            emote: Emote {
                source: "twitch".into(),
                id: id.into(),
                name: chars[start..=end].iter().collect(),
                image_url: emote_url(id, false, dark, "1.0"),
                image_url2x: Some(emote_url(id, false, dark, "2.0")),
                animated: None,
                overlay: None,
                owner_id: None,
                global: None,
            },
        });
        cursor = end + 1;
    }
    push_text_with_mentions(&mut parts, &chars[cursor..].iter().collect::<String>());
    parts
}

/// Zahl aus einem Tag wie JavaScript `Number(...)`: leer gilt 0, fehlt oder ungültig ist es keine Zahl.
pub fn tag_number(value: Option<&String>) -> Option<f64> {
    let value = value?.trim();
    if value.is_empty() {
        return Some(0.0);
    }
    value.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Wandelt eine PRIVMSG-Zeile in eine Chatnachricht um (aus dem Verlauf). Bits nur, wenn Twitch sie mit dem Tag „bits“
/// als Cheer kennzeichnet.
pub fn convert_privmsg(
    line: &IrcLine,
    channel: &ChannelRef,
    badges: &BadgeStore,
    own_user_id: &str,
    dark: bool,
) -> Option<ChatMessage> {
    if line.command != "PRIVMSG" {
        return None;
    }
    let tags = &line.tags;
    let mut text = line.params.get(1).cloned().unwrap_or_default();
    // „/me“-Nachrichten: ACTION-Rahmen entfernen.
    if let Some(inner) = text.strip_prefix("\u{1}ACTION ").and_then(|t| t.strip_suffix('\u{1}')) {
        if !inner.contains(['\n', '\r']) {
            text = inner.to_string();
        }
    }
    let login = line.prefix.split('!').next().unwrap_or("").to_string();
    let id = tags.get("id").filter(|v| !v.is_empty())?.clone();
    let user_id = tags.get("user-id").filter(|v| !v.is_empty())?.clone();
    if login.is_empty() {
        return None;
    }
    let timestamp = tag_number(tags.get("tmi-sent-ts"))?;

    let badge_info: BTreeMap<String, String> = parse_badge_list(tags.get("badge-info")).into_iter().collect();
    let raw_badges = parse_badge_list(tags.get("badges"));
    let badges = badges.resolve(raw_badges.iter().map(|(set, id)| RawBadge {
        set_id: set,
        id,
        info: badge_info.get(set).map_or("", String::as_str),
    }));
    let source_room = tags.get("source-room-id").filter(|v| !v.is_empty());
    let source_channel = source_room.filter(|room| Some(*room) != tags.get("room-id")).map(|room| ChannelRef {
        platform: "twitch".into(),
        id: room.clone(),
        login: String::new(),
        display_name: crate::i18n::tr("anderer Kanal", "another channel").into(),
    });
    let reply = tags.get("reply-parent-msg-id").filter(|v| !v.is_empty()).map(|parent| {
        let login = tags.get("reply-parent-user-login").cloned().unwrap_or_default();
        ReplyRef {
            message_id: parent.clone(),
            sender_display_name: tags.get("reply-parent-display-name").cloned().unwrap_or_else(|| login.clone()),
            sender_login: login,
            text: tags.get("reply-parent-msg-body").cloned().unwrap_or_default(),
        }
    });

    Some(ChatMessage {
        kind: "message".into(),
        channel: channel.clone(),
        id,
        sender: Sender {
            id: user_id.clone(),
            display_name: tags.get("display-name").filter(|v| !v.is_empty()).cloned().unwrap_or_else(|| login.clone()),
            login,
            color: tags.get("color").and_then(|c| valid_color(c)),
        },
        timestamp,
        parts: {
            let parts = split_emotes(&text, tags.get("emotes").map(String::as_str), dark);
            if tag_number(tags.get("bits")).is_some_and(|bits| bits > 0.0) { crate::emotes::split_cheers(parts) } else { parts }
        },
        text,
        badges,
        flags: MessageFlags {
            from_history: Some(true),
            own: Some(user_id == own_user_id),
            first_message: (tags.get("first-msg").map(String::as_str) == Some("1")).then_some(true),
            ..MessageFlags::default()
        },
        reply,
        source_channel,
    })
}

/// Entfernungen aus dem Verlauf: CLEARMSG (eine Nachricht), CLEARCHAT (Nutzer oder ganzer Chat). Ohne Mod-Namen.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum HistoryRemoval {
    #[serde(rename_all = "camelCase")]
    Message {
        message_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        login: Option<String>,
        timestamp: f64,
    },
    #[serde(rename_all = "camelCase")]
    User {
        user_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        login: Option<String>,
        /// "ban" oder "timeout".
        reason: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        duration_seconds: Option<f64>,
        timestamp: f64,
    },
    All {
        timestamp: f64,
    },
}

pub fn parse_removal(line: &IrcLine) -> Option<HistoryRemoval> {
    let timestamp = tag_number(line.tags.get("tmi-sent-ts")).unwrap_or(f64::NAN);
    let non_empty = |key: &str| line.tags.get(key).filter(|v| !v.is_empty()).cloned();
    if line.command == "CLEARMSG" {
        let message_id = non_empty("target-msg-id")?;
        return Some(HistoryRemoval::Message { message_id, login: non_empty("login"), timestamp });
    }
    if line.command == "CLEARCHAT" {
        let Some(user_id) = non_empty("target-user-id") else {
            return Some(HistoryRemoval::All { timestamp });
        };
        let login = line.params.get(1).filter(|v| !v.is_empty()).cloned();
        return Some(match non_empty("ban-duration") {
            Some(duration) => HistoryRemoval::User {
                user_id,
                login,
                reason: "timeout".into(),
                duration_seconds: tag_number(Some(&duration)),
                timestamp,
            },
            None => HistoryRemoval::User { user_id, login, reason: "ban".into(), duration_seconds: None, timestamp },
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel() -> ChannelRef {
        ChannelRef { platform: "twitch".into(), id: "430023505".into(), login: "kanal".into(), display_name: "Kanal".into() }
    }

    fn privmsg(id: &str, user: &str, user_id: &str, text: &str, ts: i64, emotes: &str) -> String {
        let lower = user.to_lowercase();
        format!("@badge-info=subscriber/14;badges=subscriber/12,vip/1;color=#FF0000;display-name={user};emotes={emotes};first-msg=0;id={id};room-id=430023505;tmi-sent-ts={ts};user-id={user_id} :{lower}!{lower}@x.tmi.twitch.tv PRIVMSG #kanal :{text}")
    }

    #[test]
    fn zerlegt_tags_absender_befehl_text() {
        let line = parse_irc_line(r"@display-name=A\sB;msg=x\:y :a!a@a PRIVMSG #kanal :Hallo Welt").unwrap();
        assert_eq!(line.tags["display-name"], "A B");
        assert_eq!(line.tags["msg"], "x;y");
        assert_eq!(line.prefix, "a!a@a");
        assert_eq!(line.command, "PRIVMSG");
        assert_eq!(line.params, ["#kanal", "Hallo Welt"]);
        assert_eq!(parse_irc_line("@nur-tags"), None);
        assert_eq!(parse_irc_line(":nur-praefix"), None);
        assert_eq!(parse_irc_line("PING :tmi.twitch.tv").unwrap().params, ["tmi.twitch.tv"]);
    }

    #[test]
    fn escapes_wie_javascript() {
        assert_eq!(unescape_tag(r"a\sb\\c\:d\x"), r"a b\c;dx");
        assert_eq!(unescape_tag("ende\\"), "ende\\");
    }

    #[test]
    fn emote_stellen_in_unicode_zeichen() {
        let parts = split_emotes("😀 Kappa @Max", Some("25:2-6"), true);
        assert_eq!(parts[0], MessagePart::Text { text: "😀 ".into() });
        assert!(matches!(&parts[1], MessagePart::Emote { emote } if emote.id == "25" && emote.name == "Kappa" && emote.animated.is_none()));
        assert_eq!(parts[2], MessagePart::Text { text: " ".into() });
        assert_eq!(parts[3], MessagePart::Mention { text: "@Max".into(), login: "max".into() });
        // Ungültige oder überlappende Bereiche werden übergangen.
        assert_eq!(split_emotes("hi", Some("1:0-9,x:0-1"), true), vec![MessagePart::Text { text: "hi".into() }]);
    }

    #[test]
    fn verlauf_nachricht_mit_abo_monaten() {
        let line = parse_irc_line(&privmsg("m1", "Anna", "1", "Hallo Kappa", 1000, "25:6-10")).unwrap();
        let message = convert_privmsg(&line, &channel(), &BadgeStore::default(), "1", true).unwrap();
        assert_eq!(message.id, "m1");
        assert_eq!(message.timestamp, 1000.0);
        assert_eq!(message.flags.from_history, Some(true));
        assert_eq!(message.flags.own, Some(true));
        assert_eq!(message.flags.first_message, None);
        assert_eq!(message.sender.login, "anna");
        assert_eq!(message.sender.display_name, "Anna");
        assert_eq!(message.sender.color.as_deref(), Some("#FF0000"));
        assert_eq!(message.badges.iter().find(|b| b.set_id == "subscriber").and_then(|b| b.info.as_deref()), Some("14"));
        assert!(matches!(message.parts[1], MessagePart::Emote { .. }));
    }

    #[test]
    fn me_nachrichten_antworten_und_fremde_raeume() {
        let line = parse_irc_line("@id=m;user-id=2;tmi-sent-ts=5;room-id=1;source-room-id=9;reply-parent-msg-id=p;reply-parent-user-login=bob;reply-parent-msg-body=hi :a!a@a PRIVMSG #k :\u{1}ACTION winkt\u{1}").unwrap();
        let message = convert_privmsg(&line, &channel(), &BadgeStore::default(), "x", true).unwrap();
        assert_eq!(message.text, "winkt");
        let reply = message.reply.unwrap();
        assert_eq!((reply.sender_login.as_str(), reply.sender_display_name.as_str(), reply.text.as_str()), ("bob", "bob", "hi"));
        assert_eq!(message.source_channel.unwrap().display_name, "anderer Kanal");
        assert_eq!(message.sender.display_name, "a");
    }

    #[test]
    fn ohne_pflichtangaben_keine_nachricht() {
        for line in [
            "@user-id=2;tmi-sent-ts=5 :a!a@a PRIVMSG #k :x",
            "@id=m;tmi-sent-ts=5 :a!a@a PRIVMSG #k :x",
            "@id=m;user-id=2 :a!a@a PRIVMSG #k :x",
            "@id=m;user-id=2;tmi-sent-ts=abc :a!a@a PRIVMSG #k :x",
            "@id=m;user-id=2;tmi-sent-ts=5 :a!a@a NOTICE #k :x",
        ] {
            let parsed = parse_irc_line(line).unwrap();
            assert!(convert_privmsg(&parsed, &channel(), &BadgeStore::default(), "x", true).is_none(), "{line}");
        }
    }

    #[test]
    fn entfernungen() {
        let removal = |line: &str| parse_removal(&parse_irc_line(line).unwrap());
        assert_eq!(
            removal("@login=anna;target-msg-id=m1;tmi-sent-ts=4 :tmi.twitch.tv CLEARMSG #kanal :eins"),
            Some(HistoryRemoval::Message { message_id: "m1".into(), login: Some("anna".into()), timestamp: 4.0 })
        );
        assert_eq!(
            removal("@ban-duration=600;target-user-id=1;tmi-sent-ts=3 :tmi.twitch.tv CLEARCHAT #kanal :anna"),
            Some(HistoryRemoval::User { user_id: "1".into(), login: Some("anna".into()), reason: "timeout".into(), duration_seconds: Some(600.0), timestamp: 3.0 })
        );
        assert_eq!(
            removal("@target-user-id=7;tmi-sent-ts=2 :tmi.twitch.tv CLEARCHAT #kanal :fremd"),
            Some(HistoryRemoval::User { user_id: "7".into(), login: Some("fremd".into()), reason: "ban".into(), duration_seconds: None, timestamp: 2.0 })
        );
        assert_eq!(removal("@tmi-sent-ts=1 :tmi.twitch.tv CLEARCHAT #kanal"), Some(HistoryRemoval::All { timestamp: 1.0 }));
        assert_eq!(removal("@tmi-sent-ts=1 :tmi.twitch.tv PRIVMSG #kanal :x"), None);
    }
}
