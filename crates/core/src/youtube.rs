//! YouTube in der App, zuerst nur lesen. Die App spricht nie selbst mit YouTube: Live-Status und Chat
//! kommen vom MultiYap-Server (`/api/youtube/live` und `/api/youtube/chat` als Server-Sent Events), der den
//! API-Schlüssel hält.
//!
//! Hier steht die reine Fachlogik: Kanal aus Name oder Link erkennen, Antworten des Servers lesen, Nachrichten in
//! das gemeinsame Nachrichtenmodell (`model.rs`) übertragen und entscheiden, wie es nach dem Ende einer Verbindung
//! weitergeht.

use serde::Deserialize;
use serde_json::Value;

use crate::i18n::tr;
use crate::links::{split_links, LinkPart};
use crate::model::{Badge, ChannelRef, ChatEventItem, ChatMessage, MessageFlags, MessagePart, Sender};

/// Adresse des MultiYap-Servers. Für lokale Tests beim Bauen über die Umgebungsvariable `MULTIYAP_WEBSITE`
/// änderbar (zum Beispiel `http://localhost:8787`), sonst immer multiyap.app.
pub const WEBSITE: &str = match option_env!("MULTIYAP_WEBSITE") {
    Some(url) => url,
    None => "https://multiyap.app",
};

/// Plattform-ID im Nachrichtenmodell und in Tabs.
pub const PLATFORM: &str = "youtube";

/// So oft prüft die App einen Offline-Kanal (der Server prüft ohnehin höchstens alle 2 Minuten).
pub const OFFLINE_CHECK_MS: f64 = 120_000.0;

// ---------------------------------------------------------------------------------------------------------
// Kanal erkennen

/// Kanal aus Name, Kanal-ID oder Link: „@name“ (klein geschrieben) oder „UC…“. Gleiche Regeln wie auf dem Server
/// (`Website/server/src/youtube/live.rs`): Namen 3 bis 30 Zeichen aus Buchstaben, Ziffern, „.“, „_“ und „-“.
pub fn channel_key(input: &str) -> Option<String> {
    let input = input.trim();
    if input.is_empty() || input.len() > 200 {
        return None;
    }
    let mut rest = input;
    let mut link = false;
    for prefix in ["https://", "http://"] {
        if let Some(stripped) = strip_prefix_ignore_case(rest, prefix) {
            rest = stripped;
            link = true;
            break;
        }
    }
    for host in ["www.youtube.com/", "m.youtube.com/", "youtube.com/"] {
        if let Some(stripped) = strip_prefix_ignore_case(rest, host) {
            rest = stripped;
            link = true;
            break;
        }
    }
    if !link {
        return match rest.strip_prefix('@') {
            Some(handle) => handle_key(handle),
            None if is_channel_id(rest) => Some(rest.to_string()),
            None => handle_key(rest),
        };
    }
    let path = rest.split(['?', '#']).next().unwrap_or_default();
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    let first = segments.next()?;
    if let Some(handle) = first.strip_prefix('@') {
        return handle_key(&percent_decode(handle)?);
    }
    if first == "channel" {
        let id = segments.next()?;
        return is_channel_id(id).then(|| id.to_string());
    }
    None
}

/// Ist das ein gespeicherter Kanal-Schlüssel (Ergebnis von `channel_key`)?
pub fn is_channel_key(key: &str) -> bool {
    match key.strip_prefix('@') {
        Some(handle) => handle_key(handle).as_deref() == Some(key),
        None => is_channel_id(key),
    }
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &text[prefix.len()..])
}

fn is_channel_id(text: &str) -> bool {
    text.len() == 24 && text.starts_with("UC") && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn handle_key(handle: &str) -> Option<String> {
    let count = handle.chars().count();
    let valid = (3..=30).contains(&count) && handle.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-'));
    valid.then(|| format!("@{}", handle.to_lowercase()))
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            out.push(u8::from_str_radix(text.get(index + 1..index + 3)?, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn encode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

/// Live-Status eines Kanals beim Server.
pub fn live_url(key: &str) -> String {
    format!("{WEBSITE}/api/youtube/live?channel={}", encode(key))
}

/// Chat eines Kanals beim Server (Server-Sent Events).
pub fn chat_url(key: &str) -> String {
    format!("{WEBSITE}/api/youtube/chat?channel={}", encode(key))
}

/// Kanal auf YouTube im Browser. Auch die Kanal-ID wird kodiert: Sie kommt bei Nutzerkarten aus dem Chat, also von
/// außen, und darf die Adresse nicht verändern.
pub fn channel_page(key: &str) -> String {
    if key.starts_with('@') {
        format!("https://www.youtube.com/{}", encode(key).replace("%40", "@"))
    } else {
        format!("https://www.youtube.com/channel/{}", encode(key))
    }
}

/// Livestream auf YouTube im Browser.
pub fn video_page(video_id: &str) -> String {
    format!("https://www.youtube.com/watch?v={}", encode(video_id))
}

// ---------------------------------------------------------------------------------------------------------
// Antworten des Servers

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChannelInfo {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub handle: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveInfo {
    pub video_id: String,
    pub title: String,
    pub chat: bool,
}

/// Antwort von `/api/youtube/live` und Ereignis `start` des Chats.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveAnswer {
    pub channel: ChannelInfo,
    pub live: Option<LiveInfo>,
    pub next_check: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Author {
    pub name: String,
    pub channel_id: String,
    #[serde(default)]
    pub owner: bool,
    #[serde(default)]
    pub moderator: bool,
    #[serde(default)]
    pub member: bool,
    #[serde(default)]
    pub verified: bool,
}

/// Eine Nachricht aus dem Ereignis `messages`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChatLine {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub time: Option<String>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub author: Option<Author>,
}

/// Kanal im Nachrichtenmodell: `login` ist der Schlüssel des Tabs („@name“ oder „UC…“).
pub fn channel_ref(key: &str, channel: &ChannelInfo) -> ChannelRef {
    ChannelRef { platform: PLATFORM.into(), id: channel.id.clone(), login: key.into(), display_name: channel.title.clone() }
}

fn parts(text: &str) -> Vec<MessagePart> {
    let mut parts = Vec::new();
    for part in split_links(text) {
        match part {
            LinkPart::Text(text) => split_shortcodes(&text, &mut parts),
            LinkPart::Link { text, url } => parts.push(MessagePart::Link { text, url }),
        }
    }
    parts
}

/// Ist das ein Emoji-Code von YouTube wie „:face-purple-sweating:“? Mindestens ein Buchstabe, damit Uhrzeiten wie
/// „12:30:45“ Text bleiben.
fn is_shortcode(code: &str) -> bool {
    let Some(inner) = code.strip_prefix(':').and_then(|c| c.strip_suffix(':')) else { return false };
    (2..=64).contains(&inner.len())
        && inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && inner.chars().any(|c| c.is_ascii_alphabetic())
}

/// Eigene YouTube-Emojis kommen nur als Text an. Sie werden eigene Teile, damit die Chat-Liste sie abgeschwächt zeigt.
fn split_shortcodes(text: &str, parts: &mut Vec<MessagePart>) {
    let mut rest = text;
    let mut plain = String::new();
    while let Some(start) = rest.find(':') {
        let after = &rest[start + 1..];
        let code = after.find(':').map(|end| &rest[start..start + end + 2]).filter(|code| is_shortcode(code));
        match code {
            Some(code) => {
                plain.push_str(&rest[..start]);
                if !plain.is_empty() {
                    parts.push(MessagePart::Text { text: std::mem::take(&mut plain) });
                }
                parts.push(MessagePart::Shortcode { text: code.to_string() });
                rest = &rest[start + code.len()..];
            }
            None => {
                plain.push_str(&rest[..=start]);
                rest = after;
            }
        }
    }
    plain.push_str(rest);
    if !plain.is_empty() {
        parts.push(MessagePart::Text { text: plain });
    }
}

/// Rollen als Abzeichen ohne Bild: Sie zählen für Rollen und Schutz (Inhaber und Mods), gezeichnet wird nichts.
fn role_badges(author: &Author) -> Vec<Badge> {
    let roles = [
        (author.owner, "broadcaster", tr("Kanalinhaber", "Channel owner")),
        (author.moderator, "moderator", tr("Moderator", "Moderator")),
        (author.member, "member", tr("Mitglied", "Member")),
        (author.verified, "verified", tr("Verifiziert", "Verified")),
    ];
    roles
        .into_iter()
        .filter(|(has, _, _)| *has)
        .map(|(_, set_id, title)| Badge {
            source: PLATFORM.into(),
            set_id: set_id.into(),
            version: "1".into(),
            info: None,
            title: Some(title.into()),
            image_url: None,
            image_url2x: None,
        })
        .collect()
}

/// Name eines Chat-Ereignisses (Super Chat, Mitgliedschaften); andere Arten zeigt die App noch nicht.
fn event_label(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "superChatEvent" => "Super Chat",
        "superStickerEvent" => "Super Sticker",
        "newSponsorEvent" => tr("Neues Mitglied", "New member"),
        "memberMilestoneChatEvent" => tr("Mitglieds-Jubiläum", "Membership milestone"),
        "membershipGiftingEvent" => tr("Verschenkte Mitgliedschaften", "Gifted memberships"),
        "giftMembershipReceivedEvent" => tr("Geschenkte Mitgliedschaft", "Gifted membership"),
        _ => return None,
    })
}

/// Zeile für die Chat-Liste: Textnachricht oder Chat-Ereignis als JSON wie bei Twitch. Gelöschte Nachrichten,
/// Banns, Umfragen und Ähnliches zeigt die App noch nicht (`None`). `from_history`: Teil des Verlaufs, den der
/// Server beim Verbinden schickt (löst keine Benachrichtigung aus und zählt nicht als ungesehen).
pub fn to_item(line: &ChatLine, channel: &ChannelRef, now_ms: f64, from_history: bool) -> Option<Value> {
    let timestamp = line.time.as_deref().and_then(crate::time::parse_rfc3339_ms).unwrap_or(now_ms);
    let author = line.author.as_ref();
    let name = author.map_or("", |a| a.name.as_str()).to_string();
    if line.kind == "textMessageEvent" {
        let author = author?;
        let message = ChatMessage {
            kind: "message".into(),
            channel: channel.clone(),
            id: line.id.clone(),
            sender: Sender { id: author.channel_id.clone(), login: name.trim_start_matches('@').to_lowercase(), display_name: name, color: None },
            timestamp,
            text: line.text.clone(),
            parts: parts(&line.text),
            badges: role_badges(author),
            flags: MessageFlags { from_history: from_history.then_some(true), ..MessageFlags::default() },
            reply: None,
            source_channel: None,
        };
        return serde_json::to_value(message).ok();
    }
    let label = event_label(&line.kind)?;
    let event = ChatEventItem {
        kind: "event".into(),
        channel: channel.clone(),
        id: line.id.clone(),
        timestamp,
        event_type: line.kind.clone(),
        text: if name.is_empty() { label.to_string() } else { format!("{label}: {name}") },
        message: (!line.text.trim().is_empty()).then(|| parts(&line.text)),
        recipients: None,
        community_gift_id: None,
        users: None,
        from_history: from_history.then_some(true),
        link: None,
    };
    serde_json::to_value(event).ok()
}

// ---------------------------------------------------------------------------------------------------------
// Wie es weitergeht

/// Nächster Schritt nach dem Ende einer Chat-Verbindung oder einer Fehlerantwort des Servers.
#[derive(Debug, Clone, PartialEq)]
pub enum Next {
    /// Kanal ist offline (oder ohne Chat): Live-Status nach dieser Zeit erneut prüfen.
    Recheck { after_ms: f64, offline: bool },
    /// Sofort oder nach kurzer Pause wieder verbinden (vorübergehend).
    Reconnect { after_ms: f64 },
    /// Hinweis zeigen und nach dieser Zeit selbst erneut versuchen.
    Wait { after_ms: f64, message: &'static str },
    /// Hinweis zeigen, nichts mehr selbst versuchen (der Nutzer kann neu verbinden).
    Stop { message: &'static str },
}

/// Auswertung eines Grundes (`end` des Chats oder Fehlercode von `/api/youtube/live`).
pub fn next_after(reason: &str) -> Next {
    match reason {
        "offline" => Next::Recheck { after_ms: OFFLINE_CHECK_MS, offline: true },
        "no_chat" => Next::Recheck { after_ms: OFFLINE_CHECK_MS, offline: false },
        "too_fast" => Next::Recheck { after_ms: 60_000.0, offline: false },
        "idle" | "restart" => Next::Reconnect { after_ms: 3_000.0 },
        "not_found" => Next::Stop { message: tr("Diesen YouTube-Kanal gibt es nicht.", "This YouTube channel does not exist.") },
        "invalid_channel" => Next::Stop { message: tr("Ungültiger YouTube-Kanal.", "Invalid YouTube channel.") },
        "too_many_chats" => Next::Stop {
            message: tr(
                "Es können höchstens 2 YouTube-Chats gleichzeitig offen sein. Einen anderen schließen und dann neu verbinden.",
                "At most 2 YouTube chats can be open at the same time. Close another one, then reconnect.",
            ),
        },
        "youtube_quota" => Next::Wait {
            after_ms: 600_000.0,
            message: tr(
                "Das YouTube-Kontingent für heute ist aufgebraucht. Ab 9 Uhr (Sommerzeit 10 Uhr) geht es wieder.",
                "Today’s YouTube quota is used up. It works again from 9:00 CET (10:00 CEST).",
            ),
        },
        "youtube_off" => Next::Wait { after_ms: 600_000.0, message: tr("YouTube ist auf dem MultiYap-Server gerade nicht verfügbar.", "YouTube is currently not available on the MultiYap server.") },
        _ => Next::Wait { after_ms: 30_000.0, message: tr("YouTube ist gerade nicht erreichbar. MultiYap versucht es weiter.", "YouTube cannot be reached right now. MultiYap keeps trying.") },
    }
}

/// Fehlercode aus einer Fehlerantwort des Servers (`{ "error": "…" }`), mit Ersatz nach HTTP-Status.
pub fn error_code(status: u16, body: Option<&Value>) -> String {
    if let Some(code) = body.and_then(|b| b.get("error")).and_then(Value::as_str) {
        return code.to_string();
    }
    match status {
        404 => "not_found",
        400 => "invalid_channel",
        429 => "too_fast",
        _ => "youtube_error",
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "UCSJ4gkVC6NrvII8umztf0Ow";

    #[test]
    fn erkennt_namen_ids_und_links() {
        assert_eq!(channel_key("@LofiGirl").as_deref(), Some("@lofigirl"));
        assert_eq!(channel_key(" LofiGirl ").as_deref(), Some("@lofigirl"));
        assert_eq!(channel_key("@Grünzeug").as_deref(), Some("@grünzeug"));
        assert_eq!(channel_key(ID).as_deref(), Some(ID));
        assert_eq!(channel_key("https://www.youtube.com/@LofiGirl/live?si=abc").as_deref(), Some("@lofigirl"));
        assert_eq!(channel_key("youtube.com/@Gr%C3%BCnzeug").as_deref(), Some("@grünzeug"));
        assert_eq!(channel_key(&format!("https://m.youtube.com/channel/{ID}")).as_deref(), Some(ID));
        for bad in ["", "ab", "zwei worte", "https://www.youtube.com/watch?v=abc", "https://youtu.be/abc", "https://example.com/@name"] {
            assert_eq!(channel_key(bad), None, "{bad}");
        }
    }

    #[test]
    fn prueft_gespeicherte_schluessel() {
        assert!(is_channel_key("@lofigirl"));
        assert!(is_channel_key(ID));
        assert!(!is_channel_key("@LofiGirl"));
        assert!(!is_channel_key("lofigirl"));
        assert!(!is_channel_key("@a"));
    }

    #[test]
    fn adressen() {
        assert_eq!(live_url("@grünzeug"), format!("{WEBSITE}/api/youtube/live?channel=%40gr%C3%BCnzeug"));
        assert_eq!(chat_url(ID), format!("{WEBSITE}/api/youtube/chat?channel={ID}"));
        assert_eq!(channel_page("@lofigirl"), "https://www.youtube.com/@lofigirl");
        assert_eq!(channel_page(ID), format!("https://www.youtube.com/channel/{ID}"));
        assert_eq!(channel_page("UC/../../watch?v=x#a"), "https://www.youtube.com/channel/UC%2F..%2F..%2Fwatch%3Fv%3Dx%23a");
        assert_eq!(video_page("4t4AQ15yj1M"), "https://www.youtube.com/watch?v=4t4AQ15yj1M");
    }

    fn channel() -> ChannelRef {
        channel_ref("@kanal", &ChannelInfo { id: "UCk".into(), title: "Kanal".into(), handle: Some("@kanal".into()) })
    }

    fn author() -> Author {
        Author { name: "@Jemand".into(), channel_id: "UCj".into(), owner: false, moderator: true, member: false, verified: false }
    }

    #[test]
    fn textnachricht_wie_bei_twitch() {
        let line = ChatLine { id: "m1".into(), kind: "textMessageEvent".into(), time: Some("2026-11-24T12:00:00.5+00:00".into()), text: "Hallo multiyap.app".into(), author: Some(author()) };
        let item = to_item(&line, &channel(), 0.0, false).unwrap();
        let message: ChatMessage = serde_json::from_value(item).unwrap();
        assert_eq!(message.flags.from_history, None);
        assert_eq!(message.kind, "message");
        assert_eq!(message.channel.platform, "youtube");
        assert_eq!(message.channel.login, "@kanal");
        assert_eq!(message.timestamp, 1_795_521_600_500.0);
        assert_eq!((message.sender.login.as_str(), message.sender.display_name.as_str(), message.sender.id.as_str()), ("jemand", "@Jemand", "UCj"));
        assert_eq!(message.badges.iter().map(|b| b.set_id.as_str()).collect::<Vec<_>>(), ["moderator"]);
        assert!(message.badges[0].image_url.is_none());
        assert!(matches!(&message.parts[1], MessagePart::Link { text, url } if text == "multiyap.app" && url.contains("multiyap.app")));
    }

    #[test]
    fn ereignisse_und_unbekanntes() {
        let line = ChatLine { id: "e1".into(), kind: "superChatEvent".into(), time: None, text: "Danke".into(), author: Some(author()) };
        let item: ChatEventItem = serde_json::from_value(to_item(&line, &channel(), 42.0, true).unwrap()).unwrap();
        assert_eq!((item.kind.as_str(), item.text.as_str(), item.timestamp), ("event", "Super Chat: @Jemand", 42.0));
        assert!(item.message.is_some());
        assert_eq!(item.from_history, Some(true));
        let tombstone = ChatLine { kind: "tombstone".into(), ..line.clone() };
        assert_eq!(to_item(&tombstone, &channel(), 0.0, false), None);
        let without_author = ChatLine { kind: "textMessageEvent".into(), author: None, ..line.clone() };
        assert_eq!(to_item(&without_author, &channel(), 0.0, false), None);
        let history: ChatMessage = serde_json::from_value(to_item(&ChatLine { kind: "textMessageEvent".into(), ..line }, &channel(), 0.0, true).unwrap()).unwrap();
        assert_eq!(history.flags.from_history, Some(true));
    }

    #[test]
    fn emoji_codes_als_eigene_teile() {
        let text = |t: &str| MessagePart::Text { text: t.into() };
        let code = |t: &str| MessagePart::Shortcode { text: t.into() };
        assert_eq!(parts("hi :face-purple-sweating::wave: ok"), [text("hi "), code(":face-purple-sweating:"), code(":wave:"), text(" ok")]);
        assert_eq!(parts("um 12:30:45 Uhr"), [text("um 12:30:45 Uhr")]);
        assert_eq!(parts("a: b :c d"), [text("a: b :c d")]);
        assert_eq!(parts(":x: :ok:"), [text(":x: "), code(":ok:")]);
        assert_eq!(parts("::elbowcough:"), [text(":"), code(":elbowcough:")]);
        assert!(matches!(&parts("siehe https://a.de/:x_y: :yay:")[..], [MessagePart::Text { .. }, MessagePart::Link { .. }, MessagePart::Text { .. }, MessagePart::Shortcode { .. }]));
    }

    #[test]
    fn naechster_schritt() {
        assert_eq!(next_after("offline"), Next::Recheck { after_ms: OFFLINE_CHECK_MS, offline: true });
        assert!(matches!(next_after("restart"), Next::Reconnect { .. }));
        assert!(matches!(next_after("too_many_chats"), Next::Stop { .. }));
        assert!(matches!(next_after("youtube_quota"), Next::Wait { .. }));
        assert!(matches!(next_after("irgendwas"), Next::Wait { after_ms, .. } if after_ms == 30_000.0));
        assert_eq!(error_code(503, Some(&serde_json::json!({ "error": "youtube_quota" }))), "youtube_quota");
        assert_eq!(error_code(404, None), "not_found");
        assert_eq!(error_code(500, Some(&Value::Null)), "youtube_error");
    }
}
