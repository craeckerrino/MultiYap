//! Chat-Ereignisse, übertragen aus src/platforms/twitch/convert-events.ts.
//!
//! Quelle live: EventSub channel.chat.notification v1; Quelle Verlauf: USERNOTICE-Zeilen von recent-messages. Beide
//! werden in dieselbe Zwischenform gebracht und daraus ein einheitlicher deutscher Text gebaut. Angezeigt: Abo, Resub,
//! Prime, Geschenk-Abo, Community-Geschenke, Upgrades, Raid, Raid abgebrochen, Watch Streak, Ankündigung,
//! Pay-it-forward. Nicht angezeigt: Modiversary, Charity und alles andere.

use crate::i18n::tr;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::chat::twitch_emote;
use super::irc::{split_emotes, tag_number, IrcLine};
use crate::model::{ChannelRef, ChatEventItem, EventLink, MessagePart, UserRef};

/// Zwischenform eines Ereignisses (Feldnamen wie in TypeScript).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NormalizedEvent {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    pub user: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prime: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub months: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub community_gift_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gifter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewers: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub streak: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Vec<MessagePart>>,
    /// Personen im Text, damit ihre Namen anklickbar sind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub users: Option<Vec<UserRef>>,
}

/// Nutzer nur mit ID und Namen; ohne Login gilt der kleingeschriebene Name.
fn user_ref(id: Option<&str>, login: Option<&str>, name: Option<&str>) -> Option<UserRef> {
    let id = id.filter(|v| !v.is_empty())?;
    let name = name.filter(|v| !v.is_empty())?;
    let login = login.filter(|v| !v.is_empty()).map_or_else(|| name.to_lowercase(), str::to_string);
    Some(UserRef { id: id.into(), login, display_name: name.into() })
}

fn refs(users: impl IntoIterator<Item = Option<UserRef>>) -> Option<Vec<UserRef>> {
    let list: Vec<UserRef> = users.into_iter().flatten().collect();
    (!list.is_empty()).then_some(list)
}

fn tier_label(tier: Option<&str>, prime: bool) -> &'static str {
    if prime || tier.is_some_and(|t| t.eq_ignore_ascii_case("prime")) {
        return "Prime";
    }
    match tier {
        Some("2000") => tr("Stufe 2", "Tier 2"),
        Some("3000") => tr("Stufe 3", "Tier 3"),
        _ => tr("Stufe 1", "Tier 1"),
    }
}

/// Zahl wie in einem JavaScript-Text: ganze Zahlen ohne Nachkommastellen.
fn plain_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        value.to_string()
    }
}

/// Zahl mit Tausendertrennern der Sprache (wie Intl.NumberFormat: Deutsch 1.234, Englisch 1,234).
pub fn german_number(value: f64) -> String {
    if value.fract() != 0.0 || value.abs() >= 1e15 {
        return if crate::i18n::is_en() { value.to_string() } else { value.to_string().replace('.', ",") };
    }
    let digits = (value.abs() as i64).to_string();
    let mut out = String::new();
    for (index, char) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(if crate::i18n::is_en() { ',' } else { '.' });
        }
        out.push(char);
    }
    if value < 0.0 {
        format!("-{out}")
    } else {
        out
    }
}

/// Einheitlicher deutscher Text eines Ereignisses.
pub fn event_text(event: &NormalizedEvent) -> String {
    let user = &event.user;
    let tier = event.tier.as_deref();
    let prime = event.prime.unwrap_or(false);
    let unknown = || "?".to_string();
    match event.kind.as_str() {
        "sub" => crate::tf!("{user} hat abonniert ({})", "{user} subscribed ({})", tier_label(tier, prime)),
        "resub" => crate::tf!("{user} hat seit {} Monaten abonniert ({})", "{user} has subscribed for {} months ({})",
            event.months.map_or_else(unknown, plain_number),
            tier_label(tier, prime)
        ),
        "sub_gift" => crate::tf!("{user} schenkt {} ein Abo ({})", "{user} gifts {} a sub ({})",
            event.recipient.as_deref().unwrap_or(tr("jemandem", "someone")),
            tier_label(tier, false)
        ),
        "community_sub_gift" => crate::tf!("{user} verschenkt {} Abos an die Community ({})", "{user} gifts {} subs to the community ({})",
            event.total.map_or_else(unknown, plain_number),
            tier_label(tier, false)
        ),
        "gift_paid_upgrade" => crate::tf!("{user} behält das geschenkte Abo{}", "{user} keeps the gifted sub{}",
            event.gifter.as_ref().map_or(String::new(), |g| crate::tf!(" von {g}", " from {g}"))
        ),
        "prime_paid_upgrade" => crate::tf!("{user} wechselt von Prime zu {}", "{user} switches from Prime to {}", tier_label(tier, false)),
        "raid" => crate::tf!("{} raidet mit {} Zuschauern", "{} raids with {} viewers",
            event.raider.as_deref().unwrap_or(user),
            german_number(event.viewers.unwrap_or(0.0))
        ),
        "unraid" => crate::tf!("{user} hat den Raid abgebrochen", "{user} canceled the raid"),
        "watch_streak" => crate::tf!("{user} schaut seit {} Streams in Folge", "{user} has watched {} streams in a row", event.streak.map_or_else(unknown, plain_number)),
        "announcement" => crate::tf!("Ankündigung von {user}", "Announcement from {user}"),
        "pay_it_forward" => crate::tf!("{user} gibt ein geschenktes Abo weiter{}", "{user} pays a gifted sub forward{}",
            event.gifter.as_ref().map_or(String::new(), |g| crate::tf!(" (bekommen von {g})", " (received from {g})"))
        ),
        _ => String::new(),
    }
}

pub fn to_event_item(event: &NormalizedEvent, channel: &ChannelRef, timestamp: f64, from_history: bool) -> ChatEventItem {
    ChatEventItem {
        kind: "event".into(),
        channel: channel.clone(),
        id: event.id.clone(),
        timestamp,
        event_type: event.kind.clone(),
        text: event_text(event),
        message: event.message.clone().filter(|parts| !parts.is_empty()),
        recipients: (event.kind == "community_sub_gift").then(Vec::new),
        community_gift_id: event.community_gift_id.clone(),
        users: event.users.clone(),
        from_history: from_history.then_some(true),
        link: None,
    }
}

/// Person aus Feldern mit gemeinsamem Präfix (zum Beispiel to_broadcaster_user_id, _login, _name).
fn user_with_prefix(event: &Value, prefix: &str) -> Option<UserRef> {
    let id = text(event, &format!("{prefix}_id")).filter(|v| !v.is_empty())?;
    let login = text(event, &format!("{prefix}_login")).filter(|v| !v.is_empty())?;
    Some(UserRef { id: id.into(), login: login.into(), display_name: text(event, &format!("{prefix}_name")).unwrap_or(login).into() })
}

fn channel_link(label: &str, login: &str) -> EventLink {
    EventLink { label: label.into(), url: format!("https://www.twitch.tv/{login}") }
}

/// Ausgeführter Raid aus diesem Kanal (channel.raid mit from_broadcaster_user_id): am Ende „Raid beitreten“, öffnet den
/// Ziel-Kanal im Browser.
pub fn convert_outgoing_raid(event: &Value, channel: &ChannelRef, timestamp: f64, id: &str) -> Option<ChatEventItem> {
    let target = user_with_prefix(event, "to_broadcaster_user")?;
    let viewers = number(event, "viewers").map_or_else(|| "?".to_string(), |v| format!("{}", v as u64));
    Some(ChatEventItem {
        kind: "event".into(),
        channel: channel.clone(),
        id: id.into(),
        timestamp,
        event_type: "raid_out".into(),
        text: crate::tf!("Raid zu {} mit {viewers} Zuschauern ausgeführt", "Raided {} with {viewers} viewers", target.display_name),
        message: None,
        recipients: None,
        community_gift_id: None,
        link: Some(channel_link(tr("Raid beitreten", "Join raid"), &target.login)),
        users: Some(vec![target]),
        from_history: None,
    })
}

/// Shoutout aus diesem Kanal (channel.shoutout.create), mit Link zum genannten Kanal.
pub fn convert_shoutout(event: &Value, channel: &ChannelRef, timestamp: f64, id: &str) -> Option<ChatEventItem> {
    let target = user_with_prefix(event, "to_broadcaster_user")?;
    let by = user_with_prefix(event, "moderator_user");
    let text = match &by {
        Some(by) => crate::tf!("{} hat einen Shoutout an {} gegeben", "{} gave a shoutout to {}", by.display_name, target.display_name),
        None => crate::tf!("Shoutout an {}", "Shoutout to {}", target.display_name),
    };
    Some(ChatEventItem {
        kind: "event".into(),
        channel: channel.clone(),
        id: id.into(),
        timestamp,
        event_type: "shoutout".into(),
        text,
        message: None,
        recipients: None,
        community_gift_id: None,
        link: Some(channel_link(tr("Kanal öffnen", "Open channel"), &target.login)),
        users: Some(by.into_iter().chain(std::iter::once(target)).collect()),
        from_history: None,
    })
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn number(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

fn flag(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

/// Teile der mitgeschickten Nachricht, je Fragment ein Teil (wie bisher, ohne Zusammenfassen).
fn notification_parts(message: Option<&Value>, dark: bool) -> Option<Vec<MessagePart>> {
    let message = message.filter(|m| !m.is_null())?;
    if text(message, "text").unwrap_or("").is_empty() {
        return None;
    }
    let fragments = message.get("fragments").and_then(Value::as_array)?;
    Some(
        fragments
            .iter()
            .map(|fragment| {
                let fragment_text = text(fragment, "text").unwrap_or("").to_string();
                let kind = text(fragment, "type").unwrap_or("");
                let emote = fragment.get("emote").filter(|e| !e.is_null());
                let mention = fragment.get("mention").filter(|m| !m.is_null());
                match (kind, emote, mention) {
                    ("emote", Some(emote), _) => {
                        let animated = emote
                            .get("format")
                            .and_then(Value::as_array)
                            .is_some_and(|formats| formats.iter().any(|f| f.as_str() == Some("animated")));
                        let id = text(emote, "id").unwrap_or("");
                        MessagePart::Emote { emote: twitch_emote(id, &fragment_text, animated, dark) }
                    }
                    ("mention", _, Some(mention)) => MessagePart::Mention {
                        text: fragment_text,
                        login: text(mention, "user_login").unwrap_or("").to_string(),
                    },
                    _ => MessagePart::Text { text: fragment_text },
                }
            })
            .collect(),
    )
}

/// Wandelt eine EventSub-Benachrichtigung um. Shared-Chat-Varianten wie die normale Art. None: nicht anzeigen.
pub fn normalize_notification(event: &Value, dark: bool) -> Option<NormalizedEvent> {
    let notice = text(event, "notice_type").unwrap_or("");
    let kind = notice.strip_prefix("shared_chat_").unwrap_or(notice);
    // Daten der Art: normal oder als Shared-Chat-Variante (null zählt als fehlend).
    let data = |key: &str| -> Option<&Value> {
        event
            .get(key)
            .filter(|v| !v.is_null())
            .or_else(|| event.get(format!("shared_chat_{key}")).filter(|v| !v.is_null()))
    };
    let anonymous = flag(event, "chatter_is_anonymous").unwrap_or(false);
    let user = if anonymous { tr("Anonym", "Anonymous").to_string() } else { text(event, "chatter_user_name").unwrap_or("").to_string() };
    let actor = if anonymous {
        None
    } else {
        user_ref(text(event, "chatter_user_id"), text(event, "chatter_user_login"), text(event, "chatter_user_name"))
    };
    let base = NormalizedEvent {
        id: text(event, "message_id").unwrap_or("").to_string(),
        user,
        message: notification_parts(event.get("message"), dark),
        users: refs([actor.clone()]),
        ..NormalizedEvent::default()
    };
    let field = |key: &str, name: &str| data(key).and_then(|d| text(d, name)).map(str::to_string);
    let field_number = |key: &str, name: &str| data(key).and_then(|d| number(d, name));
    let field_flag = |key: &str, name: &str| data(key).and_then(|d| flag(d, name));
    let gifter_of = |key: &str| -> (Option<String>, Option<UserRef>) {
        if field_flag(key, "gifter_is_anonymous").unwrap_or(false) {
            return (None, None);
        }
        let d = data(key);
        let name = d.and_then(|d| text(d, "gifter_user_name"));
        let reference = d.and_then(|d| user_ref(text(d, "gifter_user_id"), text(d, "gifter_user_login"), name));
        (name.map(str::to_string), reference)
    };

    Some(match kind {
        "sub" => NormalizedEvent { kind: "sub".into(), tier: field("sub", "sub_tier"), prime: field_flag("sub", "is_prime"), ..base },
        "resub" => NormalizedEvent {
            kind: "resub".into(),
            months: field_number("resub", "cumulative_months"),
            tier: field("resub", "sub_tier").or_else(|| field("resub", "sub_plan")),
            prime: field_flag("resub", "is_prime"),
            ..base
        },
        "sub_gift" => {
            let d = data("sub_gift");
            let recipient = d.and_then(|d| text(d, "recipient_user_name"));
            let recipient_ref = d.and_then(|d| user_ref(text(d, "recipient_user_id"), text(d, "recipient_user_login"), recipient));
            NormalizedEvent {
                kind: "sub_gift".into(),
                recipient: recipient.map(str::to_string),
                tier: field("sub_gift", "sub_tier"),
                community_gift_id: field("sub_gift", "community_gift_id"),
                users: refs([actor, recipient_ref]),
                ..base
            }
        }
        "community_sub_gift" => NormalizedEvent {
            kind: "community_sub_gift".into(),
            total: field_number("community_sub_gift", "total"),
            tier: field("community_sub_gift", "sub_tier"),
            community_gift_id: field("community_sub_gift", "id"),
            ..base
        },
        "gift_paid_upgrade" => {
            let (gifter, gifter_ref) = gifter_of("gift_paid_upgrade");
            NormalizedEvent { kind: "gift_paid_upgrade".into(), gifter, users: refs([actor, gifter_ref]), ..base }
        }
        "prime_paid_upgrade" => {
            NormalizedEvent { kind: "prime_paid_upgrade".into(), tier: field("prime_paid_upgrade", "sub_tier"), ..base }
        }
        "raid" => {
            let d = data("raid");
            let name = d.and_then(|d| text(d, "user_name"));
            let raider = d.and_then(|d| user_ref(text(d, "user_id"), text(d, "user_login"), name));
            NormalizedEvent {
                kind: "raid".into(),
                raider: name.map(str::to_string),
                viewers: field_number("raid", "viewer_count"),
                users: refs([raider]),
                ..base
            }
        }
        "unraid" => NormalizedEvent { kind: "unraid".into(), ..base },
        "watch_streak" => {
            NormalizedEvent { kind: "watch_streak".into(), streak: field_number("watch_streak", "streak_count"), ..base }
        }
        "announcement" => NormalizedEvent { kind: "announcement".into(), ..base },
        "pay_it_forward" => {
            let (gifter, gifter_ref) = gifter_of("pay_it_forward");
            NormalizedEvent { kind: "pay_it_forward".into(), gifter, users: refs([actor, gifter_ref]), ..base }
        }
        // Modiversary, Charity, Bits-Abzeichen und Unbekanntes werden nicht angezeigt.
        _ => return None,
    })
}

/// Erster nicht leerer Tag-Wert.
fn first_tag<'a>(line: &'a IrcLine, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| line.tags.get(*key).map(String::as_str).filter(|v| !v.is_empty()))
}

/// Wandelt eine USERNOTICE-Zeile aus dem Verlauf um (Tags laut offizieller IRC-Doku). Gruppierung von
/// Community-Geschenken nur, wenn die (nicht dokumentierten) Tags vorhanden sind.
pub fn normalize_usernotice(line: &IrcLine, dark: bool) -> Option<NormalizedEvent> {
    if line.command != "USERNOTICE" {
        return None;
    }
    let tag = |key: &str| line.tags.get(key).map(String::as_str);
    let id = first_tag(line, &["id"])?;
    let user = first_tag(line, &["display-name", "login"]).unwrap_or(tr("Anonym", "Anonymous")).to_string();
    let message = line
        .params
        .get(1)
        .filter(|text| !text.is_empty())
        .map(|text| split_emotes(text, tag("emotes"), dark));
    let actor = user_ref(tag("user-id"), tag("login"), first_tag(line, &["display-name", "login"]));
    let base = NormalizedEvent { id: id.into(), user: user.clone(), message, users: refs([actor.clone()]), ..NormalizedEvent::default() };
    let number_tag = |key: &str| tag_number(line.tags.get(key));
    let community_gift_id = first_tag(line, &["msg-param-community-gift-id", "msg-param-origin-id"]).map(str::to_string);
    let plan = tag("msg-param-sub-plan").map(str::to_string);

    Some(match tag("msg-id").unwrap_or("") {
        "sub" => NormalizedEvent { kind: "sub".into(), tier: plan, ..base },
        "resub" => NormalizedEvent { kind: "resub".into(), months: number_tag("msg-param-cumulative-months"), tier: plan, ..base },
        msg @ ("subgift" | "anonsubgift") => {
            let recipient_ref = user_ref(
                tag("msg-param-recipient-id"),
                first_tag(line, &["msg-param-recipient-user-name", "msg-param-recipient-name"]),
                first_tag(line, &["msg-param-recipient-display-name", "msg-param-recipient-user-name"]),
            );
            NormalizedEvent {
                kind: "sub_gift".into(),
                recipient: first_tag(line, &["msg-param-recipient-display-name", "msg-param-recipient-user-name"]).map(str::to_string),
                tier: plan,
                community_gift_id,
                users: refs([if msg == "anonsubgift" { None } else { actor }, recipient_ref]),
                ..base
            }
        }
        "submysterygift" | "anonsubmysterygift" => NormalizedEvent {
            kind: "community_sub_gift".into(),
            total: number_tag("msg-param-mass-gift-count"),
            tier: plan,
            community_gift_id,
            ..base
        },
        "giftpaidupgrade" => NormalizedEvent {
            kind: "gift_paid_upgrade".into(),
            gifter: first_tag(line, &["msg-param-sender-name"]).map(str::to_string),
            ..base
        },
        "anongiftpaidupgrade" => NormalizedEvent { kind: "gift_paid_upgrade".into(), ..base },
        "primepaidupgrade" => NormalizedEvent { kind: "prime_paid_upgrade".into(), tier: plan, ..base },
        "raid" => {
            // Bei Raids ist der Absender der Raider (msg-param-login, msg-param-displayName).
            let display = first_tag(line, &["msg-param-displayName"]).map_or(user.clone(), str::to_string);
            let raider = user_ref(tag("user-id"), first_tag(line, &["msg-param-login", "login"]), Some(&display));
            NormalizedEvent {
                kind: "raid".into(),
                raider: Some(display),
                viewers: number_tag("msg-param-viewerCount"),
                users: refs([raider]),
                ..base
            }
        }
        "unraid" => NormalizedEvent { kind: "unraid".into(), ..base },
        "viewermilestone" => {
            if tag("msg-param-category") != Some("watch-streak") {
                return None;
            }
            NormalizedEvent { kind: "watch_streak".into(), streak: number_tag("msg-param-value"), ..base }
        }
        "announcement" => NormalizedEvent { kind: "announcement".into(), ..base },
        "standardpayforward" | "communitypayforward" => NormalizedEvent {
            kind: "pay_it_forward".into(),
            gifter: first_tag(line, &["msg-param-prior-gifter-display-name"]).map(str::to_string),
            ..base
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::irc::parse_irc_line;
    use super::*;
    use serde_json::json;

    fn channel() -> ChannelRef {
        ChannelRef { platform: "twitch".into(), id: "1".into(), login: "kanal".into(), display_name: "Kanal".into() }
    }

    fn notification(notice: &str, extra: Value) -> Value {
        let mut base = json!({
            "chatter_user_id": "42", "chatter_user_login": "anna", "chatter_user_name": "Anna",
            "chatter_is_anonymous": false, "message_id": format!("id-{notice}"), "notice_type": notice,
            "message": { "text": "", "fragments": [] }
        });
        if let (Some(base), Some(extra)) = (base.as_object_mut(), extra.as_object()) {
            for (key, value) in extra {
                base.insert(key.clone(), value.clone());
            }
        }
        base
    }

    fn text_of(event: Value) -> String {
        event_text(&normalize_notification(&event, true).unwrap())
    }

    #[test]
    fn einheitliche_deutsche_texte() {
        assert_eq!(text_of(notification("sub", json!({ "sub": { "sub_tier": "1000", "is_prime": true } }))), "Anna hat abonniert (Prime)");
        assert_eq!(
            text_of(notification("resub", json!({ "resub": { "cumulative_months": 14, "sub_tier": "2000", "is_prime": false } }))),
            "Anna hat seit 14 Monaten abonniert (Stufe 2)"
        );
        assert_eq!(text_of(notification("raid", json!({ "raid": { "user_name": "Raider", "viewer_count": 1234 } }))), "Raider raidet mit 1.234 Zuschauern");
        assert_eq!(text_of(notification("watch_streak", json!({ "watch_streak": { "streak_count": 7 } }))), "Anna schaut seit 7 Streams in Folge");
        assert_eq!(text_of(notification("unraid", json!({}))), "Anna hat den Raid abgebrochen");
        assert_eq!(text_of(notification("announcement", json!({}))), "Ankündigung von Anna");
    }

    #[test]
    fn shared_chat_wie_normal_anonyme_als_anonym() {
        let event = notification(
            "shared_chat_sub_gift",
            json!({ "chatter_is_anonymous": true, "sub_gift": null, "shared_chat_sub_gift": { "recipient_user_name": "Max", "sub_tier": "1000", "community_gift_id": null } }),
        );
        let normalized = normalize_notification(&event, true).unwrap();
        assert_eq!(event_text(&normalized), "Anonym schenkt Max ein Abo (Stufe 1)");
        assert_eq!(normalized.users, None);
    }

    #[test]
    fn anklickbare_namen() {
        let gift = normalize_notification(
            &notification("sub_gift", json!({ "sub_gift": { "recipient_user_name": "Bob", "sub_tier": "1000", "community_gift_id": null, "recipient_user_id": "7", "recipient_user_login": "bob" } })),
            true,
        )
        .unwrap();
        let users = to_event_item(&gift, &channel(), 0.0, false).users.unwrap();
        assert_eq!(users.iter().map(|u| (u.id.as_str(), u.login.as_str(), u.display_name.as_str())).collect::<Vec<_>>(), [("42", "anna", "Anna"), ("7", "bob", "Bob")]);
    }

    #[test]
    fn nicht_angezeigte_arten() {
        for notice in ["modiversary", "charity_donation", "unknown"] {
            assert_eq!(normalize_notification(&notification(notice, json!({})), true), None, "{notice}");
        }
    }

    #[test]
    fn mitgeschickte_nachricht_und_eintrag() {
        let event = normalize_notification(
            &notification("resub", json!({ "resub": { "cumulative_months": 3, "sub_tier": "1000", "is_prime": false }, "message": { "text": "Hallo", "fragments": [{ "type": "text", "text": "Hallo", "emote": null, "mention": null }] } })),
            true,
        )
        .unwrap();
        let item = to_event_item(&event, &channel(), 5.0, true);
        assert_eq!(item.message, Some(vec![MessagePart::Text { text: "Hallo".into() }]));
        assert_eq!(item.from_history, Some(true));
        assert_eq!(item.recipients, None);
        let community = normalize_notification(&notification("community_sub_gift", json!({ "community_sub_gift": { "id": "c1", "total": 2, "sub_tier": "1000" } })), true).unwrap();
        let item = to_event_item(&community, &channel(), 0.0, false);
        assert_eq!(item.recipients, Some(vec![]));
        assert_eq!(item.community_gift_id.as_deref(), Some("c1"));
        assert_eq!(item.text, "Anna verschenkt 2 Abos an die Community (Stufe 1)");
    }

    #[test]
    fn verlauf_usernotice() {
        let normalize = |line: &str| normalize_usernotice(&parse_irc_line(line).unwrap(), true);
        let resub = normalize("@display-name=Anna;id=r1;login=anna;msg-id=resub;msg-param-cumulative-months=5;msg-param-sub-plan=Prime;tmi-sent-ts=1 :tmi.twitch.tv USERNOTICE #kanal :Hi").unwrap();
        assert_eq!(event_text(&resub), "Anna hat seit 5 Monaten abonniert (Prime)");
        assert_eq!(resub.message, Some(vec![MessagePart::Text { text: "Hi".into() }]));
        let gift = normalize("@display-name=Bob;id=m1;login=bob;msg-id=submysterygift;msg-param-mass-gift-count=2;msg-param-origin-id=o1;msg-param-sub-plan=1000;tmi-sent-ts=1 :tmi.twitch.tv USERNOTICE #kanal").unwrap();
        assert_eq!(event_text(&gift), "Bob verschenkt 2 Abos an die Community (Stufe 1)");
        assert_eq!(gift.community_gift_id.as_deref(), Some("o1"));
        let raid = normalize("@display-name=Raider;id=r1;login=raider;msg-id=raid;msg-param-displayName=Raider;msg-param-login=raider;msg-param-viewerCount=12;room-id=1;tmi-sent-ts=1000;user-id=99 :tmi.twitch.tv USERNOTICE #kanal").unwrap();
        assert_eq!(event_text(&raid), "Raider raidet mit 12 Zuschauern");
        assert_eq!(raid.users.unwrap()[0].id, "99");
        assert_eq!(normalize("@display-name=Mod;id=x2;login=mod;msg-id=bitsbadgetier;tmi-sent-ts=1 :tmi.twitch.tv USERNOTICE #kanal"), None);
        assert_eq!(normalize("@id=x;msg-id=viewermilestone;msg-param-category=other :tmi.twitch.tv USERNOTICE #kanal"), None);
    }

    #[test]
    fn deutsche_zahlen() {
        assert_eq!(german_number(1234.0), "1.234");
        assert_eq!(german_number(1234567.0), "1.234.567");
        assert_eq!(german_number(12.0), "12");
        assert_eq!(german_number(0.0), "0");
    }
}
