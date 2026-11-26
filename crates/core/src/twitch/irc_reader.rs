//! Zusätzliche, nur lesende IRC-Verbindung für die Markierung „Erste Nachricht“. EventSub liefert diese Information
//! nicht. Doku geprüft am 15.11.2026: wss://irc-ws.chat.twitch.tv:443, PASS oauth:<token> vor NICK <login>,
//! Berechtigung chat:read, CAP REQ twitch.tv/tags twitch.tv/commands, PING mit PONG beantworten. Über diese Verbindung
//! wird nie etwas gesendet außer Anmeldung, JOIN/PART und PONG. Ausgewertet werden nur Zeilen mit „first-msg=1“ und die
//! eigene USERSTATE-Zeile (Rolle VIP); alle anderen werden ohne Zerlegen verworfen.
//!
//! Hier die reinen Teile; die Verbindung selbst steckt in `crates/wasm/src/irc_reader.rs`.

use super::irc::parse_irc_line;

pub const IRC_URL: &str = "wss://irc-ws.chat.twitch.tv:443";
const RETRY_DELAYS_MS: [f64; 5] = [2_000.0, 5_000.0, 15_000.0, 30_000.0, 60_000.0];
/// Abstand zwischen JOINs (die Doku nennt kein Limit; vorsichtig nacheinander).
pub const JOIN_SPACING_MS: f64 = 600.0;

pub fn retry_delay(attempt: usize) -> f64 {
    RETRY_DELAYS_MS[attempt.min(RETRY_DELAYS_MS.len() - 1)]
}

/// Zeilen für die Anmeldung, in dieser Reihenfolge zu senden.
pub fn handshake(token: &str, login: &str) -> [String; 3] {
    ["CAP REQ :twitch.tv/tags twitch.tv/commands".to_string(), format!("PASS oauth:{token}"), format!("NICK {}", login.to_lowercase())]
}

/// Antwort auf PING (gleicher Rest der Zeile).
pub fn pong(line: &str) -> Option<String> {
    line.strip_prefix("PING").map(|rest| format!("PONG{rest}"))
}

/// Anmeldung bestätigt (001)?
pub fn is_welcome(line: &str) -> bool {
    line.contains(" 001 ")
}

fn channel_of(param: Option<&String>) -> Option<String> {
    let channel = param?.strip_prefix('#').unwrap_or(param?);
    (!channel.is_empty()).then(|| channel.to_string())
}

/// Kanal und Nachrichten-ID einer PRIVMSG mit first-msg=1. Schneller Vorfilter vor dem Zerlegen.
pub fn first_message_from_line(line: &str) -> Option<(String, String)> {
    if !line.contains("first-msg=1") || !line.contains(" PRIVMSG #") {
        return None;
    }
    let parsed = parse_irc_line(line)?;
    if parsed.command != "PRIVMSG" || parsed.tags.get("first-msg").map(String::as_str) != Some("1") {
        return None;
    }
    let channel = channel_of(parsed.params.first())?;
    let id = parsed.tags.get("id").filter(|id| !id.is_empty())?.clone();
    Some((channel, id))
}

/// Eigene Abzeichen beim Betreten eines Kanals (USERSTATE): Kanal und Abzeichen-Namen (zum Beispiel „vip“). Twitch
/// liefert die Rolle VIP des eigenen Kontos sonst nirgends (/unvip für sich selbst).
pub fn own_badges_from_line(line: &str) -> Option<(String, Vec<String>)> {
    if !line.contains(" USERSTATE #") {
        return None;
    }
    let parsed = parse_irc_line(line)?;
    if parsed.command != "USERSTATE" {
        return None;
    }
    let channel = channel_of(parsed.params.first())?;
    let badges = parsed
        .tags
        .get("badges")
        .map(|list| list.split(',').filter_map(|b| b.split('/').next()).filter(|b| !b.is_empty()).map(str::to_string).collect())
        .unwrap_or_default();
    Some((channel, badges))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erste_nachricht() {
        let line = "@badges=;first-msg=1;id=abc-123;room-id=1;user-id=2 :neu!neu@neu.tmi.twitch.tv PRIVMSG #kanal :Hallo";
        assert_eq!(first_message_from_line(line), Some(("kanal".into(), "abc-123".into())));
        assert_eq!(first_message_from_line("@first-msg=0;id=x :a!a@a PRIVMSG #kanal :hi"), None);
        assert_eq!(first_message_from_line("PING :tmi.twitch.tv"), None);
        assert_eq!(first_message_from_line("@first-msg=1;id=x :tmi.twitch.tv USERNOTICE #kanal :hi"), None);
    }

    #[test]
    fn eigene_abzeichen() {
        let line = "@badge-info=;badges=lead_moderator/1,subscriber/12;color=#00FF00;display-name=Ich;mod=1 :tmi.twitch.tv USERSTATE #kanal";
        assert_eq!(own_badges_from_line(line), Some(("kanal".into(), vec!["lead_moderator".into(), "subscriber".into()])));
        assert_eq!(own_badges_from_line("@badges= :tmi.twitch.tv USERSTATE #kanal"), Some(("kanal".into(), vec![])));
        assert_eq!(own_badges_from_line("@badges=lead_moderator/1 :a!a@a PRIVMSG #kanal :hi"), None);
    }

    #[test]
    fn anmeldung_und_ping() {
        assert_eq!(handshake("t", "Ich")[2], "NICK ich");
        assert_eq!(pong("PING :tmi.twitch.tv").as_deref(), Some("PONG :tmi.twitch.tv"));
        assert!(is_welcome(":tmi.twitch.tv 001 ich :Welcome, GLHF!"));
        assert_eq!(retry_delay(9), 60_000.0);
    }
}
