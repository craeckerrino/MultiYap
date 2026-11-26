//! Texte der Chat-Anzeige (bisher mod-action-text.ts, event-names.ts, mod-status.ts und suspicious.ts):
//! Mod-Aktionen als Satz, Hinweis an entfernten Nachrichten, Namen in Chat-Ereignissen, geschützte Absender.

use crate::i18n::tr;
use crate::hotkeys::format_duration;
use crate::model::{Badge, ModActionItem, UserRef};

/// Ungefähre Dauer: auf die größte sinnvolle Einheit gerundet (Twitch liefert nur das Ablaufdatum).
pub fn format_approx_duration(seconds: f64) -> String {
    for unit in [604_800.0, 86_400.0, 3_600.0, 60.0] {
        if seconds >= unit {
            return format_duration(((seconds / unit).round() * unit) as i64);
        }
    }
    format_duration(seconds.round().max(1.0) as i64)
}

fn follow_duration(minutes: Option<&str>) -> String {
    match minutes.and_then(|m| m.trim().parse::<f64>().ok()).filter(|v| v.is_finite() && *v != 0.0) {
        Some(value) => format_approx_duration(value * 60.0),
        None => tr("0 Min", "0 min").into(),
    }
}

/// Vorheriger Status bei suspicious_none (detail): monitored oder restricted.
fn suspicious_removed(target: &str, previous: Option<&str>, with_moderator: bool) -> String {
    match (previous, with_moderator) {
        (Some("restricted"), true) => crate::tf!("hat die Einschränkung von {target} aufgehoben", "lifted the restriction of {target}"),
        (Some("restricted"), false) => crate::tf!("{target} ist nicht mehr eingeschränkt", "{target} is no longer restricted"),
        (Some("monitored"), true) => crate::tf!("hat die Überwachung von {target} beendet", "stopped monitoring {target}"),
        (Some("monitored"), false) => crate::tf!("{target} wird nicht mehr überwacht", "{target} is no longer monitored"),
        (_, true) => crate::tf!("hat den Status „Überwacht“ oder „Eingeschränkt“ von {target} entfernt", "removed the status “Monitored” or “Restricted” from {target}"),
        (_, false) => crate::tf!("{target} ist nicht mehr überwacht oder eingeschränkt", "{target} is no longer monitored or restricted"),
    }
}

/// Satz ohne Mod (Verlauf aus recent-messages): dort ist nur sichtbar, dass etwas passiert ist.
fn describe_without_moderator(item: &ModActionItem, target: &str, base: &str) -> String {
    match base {
        "warning_acknowledge" => crate::tf!("{target} hat die Verwarnung bestätigt", "{target} acknowledged the warning"),
        "ban" => crate::tf!("{target} wurde gebannt", "{target} was banned"),
        "timeout" => crate::tf!("{target} hat einen Timeout von {} bekommen", "{target} got a timeout of {}", format_approx_duration(item.duration_seconds.unwrap_or(0.0))),
        "delete" if item.target.is_some() => crate::tf!("Eine Nachricht von {target} wurde gelöscht", "A message from {target} was deleted"),
        "delete" => tr("Eine Nachricht wurde gelöscht", "A message was deleted").into(),
        "clear" => tr("Der Chat wurde geleert", "The chat was cleared").into(),
        "suspicious_monitor" => crate::tf!("{target} wird jetzt überwacht", "{target} is now monitored"),
        "suspicious_restrict" => crate::tf!("{target} ist jetzt eingeschränkt", "{target} is now restricted"),
        "suspicious_none" => suspicious_removed(target, item.detail.as_deref(), false),
        _ => crate::tf!("Mod-Aktion ({})", "Mod action ({})", item.action),
    }
}

/// Text einer Mod-Aktion ohne Namen des Mods (der steht davor). Ohne bekannten Mod ein ganzer Satz.
pub fn describe_mod_action(item: &ModActionItem) -> String {
    let target = item.target.as_ref().map_or("jemand", |t| t.display_name.as_str());
    let base = item.action.strip_prefix("shared_chat_").unwrap_or(&item.action);
    if item.moderator.is_none() {
        return describe_without_moderator(item, target, base);
    }
    let detail = item.detail.as_deref();
    match base {
        "ban" => crate::tf!("hat {target} gebannt", "banned {target}"),
        "timeout" => crate::tf!("hat {target} einen Timeout von {} gegeben", "gave {target} a timeout of {}", format_approx_duration(item.duration_seconds.unwrap_or(0.0))),
        "unban" => crate::tf!("hat den Ban von {target} aufgehoben", "unbanned {target}"),
        "untimeout" => crate::tf!("hat den Timeout von {target} aufgehoben", "lifted the timeout of {target}"),
        "delete" => crate::tf!("hat eine Nachricht von {target} gelöscht", "deleted a message from {target}"),
        "clear" => tr("hat den Chat geleert", "cleared the chat").into(),
        "warn" => crate::tf!("hat {target} verwarnt", "warned {target}"),
        "slow" => crate::tf!("hat den Slow-Modus ({}s) aktiviert", "turned on slow mode ({}s)", detail.unwrap_or("?")),
        "slowoff" => tr("hat den Slow-Modus beendet", "turned off slow mode").into(),
        "followers" => crate::tf!("hat den Follower-Modus ({}) aktiviert", "turned on follower mode ({})", follow_duration(detail)),
        "followersoff" => tr("hat den Follower-Modus beendet", "turned off follower mode").into(),
        "subscribers" => tr("hat den Sub-Modus aktiviert", "turned on sub mode").into(),
        "subscribersoff" => tr("hat den Sub-Modus beendet", "turned off sub mode").into(),
        "emoteonly" => tr("hat den Emote-Modus aktiviert", "turned on emote mode").into(),
        "emoteonlyoff" => tr("hat den Emote-Modus beendet", "turned off emote mode").into(),
        "uniquechat" => tr("hat Unique-Chat aktiviert", "turned on unique chat").into(),
        "uniquechatoff" => tr("hat Unique-Chat beendet", "turned off unique chat").into(),
        "mod" => crate::tf!("hat {target} zum Mod gemacht", "made {target} a mod"),
        "unmod" => crate::tf!("hat {target} die Mod-Rechte entzogen", "removed mod rights from {target}"),
        "vip" => crate::tf!("hat {target} zum VIP gemacht", "made {target} a VIP"),
        "unvip" => crate::tf!("hat {target} den VIP-Status entzogen", "removed VIP status from {target}"),
        "raid" => crate::tf!("startet einen Raid zu {target} mit {} Zuschauern", "starts a raid to {target} with {} viewers", detail.unwrap_or("?")),
        "unraid" => crate::tf!("hat den Raid zu {target} abgebrochen", "canceled the raid to {target}"),
        "add_blocked_term" => crate::tf!("hat gesperrte Begriffe hinzugefügt: {}", "added blocked terms: {}", detail.unwrap_or("")),
        "remove_blocked_term" => crate::tf!("hat gesperrte Begriffe entfernt: {}", "removed blocked terms: {}", detail.unwrap_or("")),
        "add_permitted_term" => crate::tf!("hat erlaubte Begriffe hinzugefügt: {}", "added permitted terms: {}", detail.unwrap_or("")),
        "remove_permitted_term" => crate::tf!("hat erlaubte Begriffe entfernt: {}", "removed permitted terms: {}", detail.unwrap_or("")),
        "approve_unban_request" => crate::tf!("hat die Unban-Anfrage von {target} angenommen", "accepted the unban request of {target}"),
        "deny_unban_request" => crate::tf!("hat die Unban-Anfrage von {target} abgelehnt", "denied the unban request of {target}"),
        "suspicious_monitor" => crate::tf!("hat {target} als überwacht markiert", "marked {target} as monitored"),
        "suspicious_restrict" => crate::tf!("hat {target} eingeschränkt", "restricted {target}"),
        "suspicious_none" => suspicious_removed(target, detail, true),
        _ => crate::tf!("hat eine Mod-Aktion ausgeführt ({})", "performed a mod action ({})", item.action),
    }
}

/// Kurztext für „entfernt von …“ an Nachrichten, bei Timeouts mit Dauer (zum Beispiel „Timeout 10m von Mod“).
pub fn removed_note(reason: &str, by: Option<&str>, duration_seconds: Option<u64>) -> String {
    let who = by.map(|b| crate::tf!(" von {b}", " by {b}")).unwrap_or_default();
    match reason {
        "timeout" => {
            let duration = duration_seconds.filter(|d| *d > 0).map(|d| format!(" {}", format_approx_duration(d as f64))).unwrap_or_default();
            format!("Timeout{duration}{who}")
        }
        "ban" => crate::tf!("gebannt{who}", "banned{who}"),
        "clear" => crate::tf!("Chat geleert{who}", "chat cleared{who}"),
        _ => crate::tf!("gelöscht{who}", "deleted{who}"),
    }
}

/// Teil eines Ereignistextes: Text oder beteiligte Person.
#[derive(Debug, Clone, PartialEq)]
pub enum NamePart {
    Text(String),
    User(UserRef),
}

/// Teilt den Text an den Namen der beteiligten Personen (jeder Name einmal, in der Reihenfolge im Text).
pub fn split_names(text: &str, users: &[UserRef]) -> Vec<NamePart> {
    let mut parts = Vec::new();
    let mut remaining: Vec<&UserRef> = users.iter().collect();
    let mut rest = text;
    while !remaining.is_empty() {
        let best = remaining
            .iter()
            .enumerate()
            .filter(|(_, u)| !u.display_name.is_empty())
            .filter_map(|(i, u)| rest.find(&u.display_name).map(|index| (index, i)))
            .min_by_key(|(index, _)| *index);
        let Some((index, position)) = best else { break };
        let user = remaining.remove(position);
        if index > 0 {
            parts.push(NamePart::Text(rest[..index].to_string()));
        }
        parts.push(NamePart::User(user.clone()));
        rest = &rest[index + user.display_name.len()..];
    }
    if !rest.is_empty() {
        parts.push(NamePart::Text(rest.to_string()));
    }
    parts
}

/// Broadcaster, Mods und Lead Mods: keine Schnellbuttons, nicht löschbar.
pub fn is_protected_sender(badges: &[Badge]) -> bool {
    badges.iter().any(|b| matches!(b.set_id.as_str(), "broadcaster" | "moderator" | "lead_moderator"))
}

/// Anzeigename des Status „überwacht“ oder „eingeschränkt“.
pub fn suspicious_label(status: &str) -> &'static str {
    if status == "restricted" {
        tr("Eingeschränkt", "Restricted")
    } else {
        tr("Überwacht", "Monitored")
    }
}

/// Name eines Abzeichens; bei Abos mit den genauen Monaten aus badge-info.
pub fn badge_label(badge: &Badge) -> String {
    let title = badge.title.clone().unwrap_or_else(|| badge.set_id.clone());
    let months = badge.info.as_deref().and_then(|i| i.parse::<f64>().ok()).filter(|m| m.is_finite() && *m > 0.0);
    match months {
        Some(m) if badge.set_id == "subscriber" || badge.set_id == "founder" => {
            format!("{title} ({m} {})", if m == 1.0 { tr("Monat", "month") } else { tr("Monate", "months") })
        }
        _ => title,
    }
}

/// Name für „@Name“: Anzeigename, wenn er nur in der Schreibweise vom Login abweicht, sonst der Login.
pub fn mention_name(login: &str, display_name: &str) -> String {
    if display_name.to_lowercase() == login {
        display_name.to_string()
    } else {
        login.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ChannelRef;

    fn user(id: &str, name: &str) -> UserRef {
        UserRef { id: id.into(), login: name.to_lowercase(), display_name: name.into() }
    }

    fn action(action: &str, moderator: bool) -> ModActionItem {
        ModActionItem {
            kind: "mod-action".into(),
            channel: ChannelRef { platform: "twitch".into(), id: "1".into(), login: "c".into(), display_name: "C".into() },
            id: "a".into(),
            timestamp: 0.0,
            moderator: moderator.then(|| user("9", "Mod")),
            action: action.into(),
            target: Some(user("2", "Bob")),
            reason: None,
            duration_seconds: Some(590.0),
            detail: None,
            source_channel: None,
            from_history: None,
        }
    }

    #[test]
    fn namen_in_ereignissen() {
        let (anna, bob) = (user("1", "Anna"), user("2", "Bob"));
        assert_eq!(
            split_names("Anna schenkt Bob ein Abo (Stufe 1)", &[bob.clone(), anna.clone()]),
            vec![NamePart::User(anna.clone()), NamePart::Text(" schenkt ".into()), NamePart::User(bob), NamePart::Text(" ein Abo (Stufe 1)".into())]
        );
        assert_eq!(split_names("Ankündigung von Anna", std::slice::from_ref(&anna)), vec![NamePart::Text("Ankündigung von ".into()), NamePart::User(anna.clone())]);
        assert_eq!(split_names("Anonym hat abonniert", &[anna]), vec![NamePart::Text("Anonym hat abonniert".into())]);
        assert_eq!(split_names("Text", &[]), vec![NamePart::Text("Text".into())]);
    }

    #[test]
    fn mod_aktionen() {
        assert_eq!(describe_mod_action(&action("timeout", true)), "hat Bob einen Timeout von 10m gegeben");
        assert_eq!(describe_mod_action(&action("shared_chat_ban", true)), "hat Bob gebannt");
        assert_eq!(describe_mod_action(&action("ban", false)), "Bob wurde gebannt");
        assert_eq!(describe_mod_action(&action("seltsam", true)), "hat eine Mod-Aktion ausgeführt (seltsam)");
        let mut follow = action("followers", true);
        follow.detail = Some("0".into());
        assert_eq!(describe_mod_action(&follow), "hat den Follower-Modus (0 Min) aktiviert");
        assert_eq!(removed_note("timeout", Some("Mod"), Some(600)), "Timeout 10m von Mod");
        assert_eq!(removed_note("deleted", None, None), "gelöscht");
    }
}
