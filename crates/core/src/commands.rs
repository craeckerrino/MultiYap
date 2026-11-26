//! „/“-Befehle, übertragen aus src/features/commands/commands.ts.
//!
//! Nur echte Twitch-Befehle, keine „!“-Bot-Befehle. Das Tool übersetzt sie in die passende Aktion; sie werden nie als
//! Text in den Chat geschickt. Mod-Befehle nur in Mod-Kanälen. Ausgeführt werden sie weiter in TypeScript
//! (execute-command.ts), weil das Twitch-Anfragen sind.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandArgument {
    None,
    User,
    Text,
    OptionalNumber,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SelfAllowed {
    Moderator,
    Vip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandInfo {
    pub name: &'static str,
    /// Schreibweise, zum Beispiel „/timeout <Name> [Dauer] [Grund]“.
    pub usage: crate::i18n::Text,
    pub description: crate::i18n::Text,
    /// Braucht Mod-Rechte im Kanal.
    #[serde(rename = "mod")]
    pub needs_mod: bool,
    /// Erstes Argument ist ein Name (dann folgen Namensvorschläge).
    pub first_argument: CommandArgument,
    /// Sichtbar, aber noch nicht nutzbar.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub coming_soon: bool,
    /// Nur im eigenen Kanal (Twitch erlaubt es nur dem Broadcaster).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub broadcaster_only: bool,
    /// Für sich selbst erlaubt, wenn man die Rolle hat (Mods und VIPs dürfen sich selbst entfernen).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_allowed: Option<SelfAllowed>,
}

const fn command(
    name: &'static str,
    usage: crate::i18n::Text,
    description: crate::i18n::Text,
    needs_mod: bool,
    first_argument: CommandArgument,
) -> CommandInfo {
    CommandInfo {
        name,
        usage,
        description,
        needs_mod,
        first_argument,
        coming_soon: false,
        broadcaster_only: false,
        self_allowed: None,
    }
}

const fn broadcaster(mut info: CommandInfo, self_allowed: Option<SelfAllowed>) -> CommandInfo {
    info.broadcaster_only = true;
    info.self_allowed = self_allowed;
    info
}

use CommandArgument::{None as NoArg, OptionalNumber, Text, User};

pub const COMMANDS: &[CommandInfo] = &[
    command("ban", ("/ban <Name> [Grund]", "/ban <name> [reason]"), ("Person dauerhaft bannen", "Ban a person permanently"), true, User),
    command("unban", ("/unban <Name>", "/unban <name>"), ("Ban aufheben", "Lift a ban"), true, User),
    command("timeout", ("/timeout <Name> [Dauer] [Grund]", "/timeout <name> [duration] [reason]"), ("Timeout, Dauer zum Beispiel 30s, 10m, 1h, 1d, 2w (Standard 10m)", "Timeout, duration for example 30s, 10m, 1h, 1d, 2w (default 10m)"),
        true,
        User,
    ),
    command("untimeout", ("/untimeout <Name>", "/untimeout <name>"), ("Timeout aufheben", "Lift a timeout"), true, User),
    command("warn", ("/warn <Name> <Grund>", "/warn <name> <reason>"), ("Person verwarnen (Grund nötig)", "Warn a person (reason required)"), true, User),
    command("clear", ("/clear", "/clear"), ("Chat leeren", "Clear the chat"), true, NoArg),
    command("slow", ("/slow [Sekunden]", "/slow [seconds]"), ("Slow-Modus, 3 bis 120 Sekunden (Standard 30)", "Slow mode, 3 to 120 seconds (default 30)"), true, OptionalNumber),
    command("slowoff", ("/slowoff", "/slowoff"), ("Slow-Modus aus", "Slow mode off"), true, NoArg),
    command("followers", ("/followers [Dauer]", "/followers [duration]"), ("Nur Follower, Dauer zum Beispiel 10m, 1h, 1d, 1w, 1mo (Standard 0)", "Followers only, duration for example 10m, 1h, 1d, 1w, 1mo (default 0)"),
        true,
        OptionalNumber,
    ),
    command("followersoff", ("/followersoff", "/followersoff"), ("Follower-Modus aus", "Follower mode off"), true, NoArg),
    command("subscribers", ("/subscribers", "/subscribers"), ("Nur Abonnenten", "Subscribers only"), true, NoArg),
    command("subscribersoff", ("/subscribersoff", "/subscribersoff"), ("Abonnenten-Modus aus", "Subscriber mode off"), true, NoArg),
    command("emoteonly", ("/emoteonly", "/emoteonly"), ("Nur Emotes", "Emotes only"), true, NoArg),
    command("emoteonlyoff", ("/emoteonlyoff", "/emoteonlyoff"), ("Emote-Modus aus", "Emote mode off"), true, NoArg),
    command("announce", ("/announce <Nachricht>", "/announce <message>"), ("Ankündigung", "Announcement"), true, Text),
    command("announceblue", ("/announceblue <Nachricht>", "/announceblue <message>"), ("Ankündigung in Blau", "Announcement in blue"), true, Text),
    command("announcegreen", ("/announcegreen <Nachricht>", "/announcegreen <message>"), ("Ankündigung in Grün", "Announcement in green"), true, Text),
    command("announceorange", ("/announceorange <Nachricht>", "/announceorange <message>"), ("Ankündigung in Orange", "Announcement in orange"), true, Text),
    command("announcepurple", ("/announcepurple <Nachricht>", "/announcepurple <message>"), ("Ankündigung in Lila", "Announcement in purple"), true, Text),
    command("shoutout", ("/shoutout <Name>", "/shoutout <name>"), ("Shoutout (nur wenn der Kanal live ist)", "Shoutout (only while the channel is live)"), true, User),
    // (15.11.2026): öffnet Twitchs eigene Nutzerkarte im Browser, in jedem Kanal.
    command("user", ("/user <Name>", "/user <name>"), ("Twitch-Nutzerkarte im Browser öffnen", "Open the Twitch user card in the browser"), false, User),
    // Hinzufügen nur im eigenen Kanal; Entfernen im eigenen Kanal für alle, sonst für sich selbst, wo man die Rolle hat
    // (Als Lead Mod lehnt Twitch es über die Schnittstelle ab).
    broadcaster(command("mod", ("/mod <Name>", "/mod <name>"), ("Zum Mod machen (nur im eigenen Kanal)", "Make a mod (only in your own channel)"), true, User), None),
    broadcaster(
        command("unmod", ("/unmod <Name>", "/unmod <name>"), ("Mod-Rechte entziehen (eigener Kanal oder dir selbst)", "Remove mod rights (own channel or yourself)"), true, User),
        Some(SelfAllowed::Moderator),
    ),
    broadcaster(command("vip", ("/vip <Name>", "/vip <name>"), ("Zum VIP machen (nur im eigenen Kanal)", "Make a VIP (only in your own channel)"), true, User), None),
    broadcaster(
        command("unvip", ("/unvip <Name>", "/unvip <name>"), ("VIP entziehen (eigener Kanal oder dir selbst)", "Remove VIP (own channel or yourself)"), false, User),
        Some(SelfAllowed::Vip),
    ),
    command("uniquechat", ("/uniquechat", "/uniquechat"), ("Unique-Chat (keine doppelten Nachrichten)", "Unique chat (no duplicate messages)"), true, NoArg),
    command("uniquechatoff", ("/uniquechatoff", "/uniquechatoff"), ("Unique-Chat aus", "Unique chat off"), true, NoArg),
];

const MAX_SUGGESTIONS: usize = 10;

/// Eigene Rollen im Kanal.
#[derive(Debug, Clone, Copy, Default)]
pub struct OwnRoles {
    pub moderator: bool,
    pub broadcaster: bool,
    pub vip: bool,
}

/// Befehle nur für den Broadcaster.
pub fn allowed_role(command: &CommandInfo, roles: OwnRoles) -> bool {
    !command.broadcaster_only || roles.broadcaster
}

/// Nur für sich selbst möglich (zum Beispiel /unmod als Mod, /unvip als VIP).
pub fn allowed_for_self(command: &CommandInfo, roles: OwnRoles) -> bool {
    match command.self_allowed {
        Some(SelfAllowed::Moderator) => roles.moderator,
        Some(SelfAllowed::Vip) => roles.vip,
        None => false,
    }
}

/// Befehle zu einer Eingabe nach „/“: zuerst Anfang, dann enthalten. Mod-Befehle nur mit Mod-Rechten.
pub fn suggest_commands(query: &str, roles: OwnRoles) -> Vec<&'static CommandInfo> {
    let q = query.to_lowercase();
    let available: Vec<&CommandInfo> = COMMANDS
        .iter()
        .filter(|c| (!c.needs_mod || roles.moderator) && (allowed_role(c, roles) || allowed_for_self(c, roles)))
        .collect();
    let starts = available.iter().filter(|c| c.name.starts_with(&q));
    let contains = available.iter().filter(|c| !c.name.starts_with(&q) && c.name.contains(&q));
    starts.chain(contains).take(MAX_SUGGESTIONS).copied().collect()
}

pub fn find_command(name: &str) -> Option<&'static CommandInfo> {
    let name = name.to_lowercase();
    COMMANDS.iter().find(|c| c.name == name)
}

/// Größte ganze Zahl, die JavaScript genau darstellen kann (Number.MAX_SAFE_INTEGER).
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// Kurzform einer Dauer in Sekunden (zum Beispiel 30m, 1d, 2w). Ohne Einheit: Sekunden. „mo“ zählt 30 Tage.
pub fn parse_duration(text: &str) -> Option<u64> {
    let text = text.trim();
    let digits_end = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    let (digits, unit) = text.split_at(digits_end);
    if digits.is_empty() || digits.len() > 7 {
        return None;
    }
    let factor = match unit.to_ascii_lowercase().as_str() {
        "" | "s" => 1,
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        "w" => 604_800,
        "mo" => 2_592_000,
        _ => return None,
    };
    let value = digits.parse::<u64>().ok()?.checked_mul(factor)?;
    (value <= MAX_SAFE_INTEGER).then_some(value)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParsedCommand {
    pub name: String,
    /// Argumente nach dem Befehl, durch Leerraum getrennt.
    pub args: Vec<String>,
    /// Alles nach dem Befehl als ein Text (für Ankündigungen).
    pub rest: String,
}

/// Zerlegt eine Eingabe, die mit „/“ beginnt.
pub fn parse_command_line(text: &str) -> Option<ParsedCommand> {
    let trimmed = text.trim();
    let after_slash = trimmed.strip_prefix('/')?;
    let name_end = after_slash.find(char::is_whitespace).unwrap_or(after_slash.len());
    let name = &after_slash[..name_end];
    if name.is_empty() {
        return None;
    }
    let rest = after_slash[name_end..].trim().to_string();
    let args = if rest.is_empty() { Vec::new() } else { rest.split_whitespace().map(str::to_string).collect() };
    Some(ParsedCommand { name: name.to_lowercase(), args, rest })
}

/// Login aus einem Namensargument (ohne „@“, in Kleinbuchstaben); ungültig: None.
pub fn login_from_argument(argument: Option<&str>) -> Option<String> {
    let raw = argument.unwrap_or("");
    let login = raw.strip_prefix('@').unwrap_or(raw).to_lowercase();
    let valid = !login.is_empty()
        && login.len() <= 25
        && login.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    valid.then_some(login)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CommandContext {
    /// Cursor im Befehlswort: Befehlsliste.
    Command { query: String },
    /// Cursor im Namensargument eines Befehls: Namensvorschläge.
    #[serde(rename_all = "camelCase")]
    CommandUser { query: String, with_at: bool },
}

/// Erkennt, ob gerade ein Befehl oder sein Namensargument getippt wird. `caret` ist eine Byte-Stelle in `text`.
pub fn detect_command_context(text: &str, caret: usize) -> Option<CommandContext> {
    if !text.starts_with('/') {
        return None;
    }
    // Schreibmarke vor dem „/“ (zum Beispiel nach Strg+A): kein Befehl.
    let before = text.get(..caret.min(text.len())).filter(|b| !b.is_empty())?;
    let Some(first_space) = before.find(char::is_whitespace) else {
        return Some(CommandContext::Command { query: before[1..].to_string() });
    };
    let command = find_command(&before[1..first_space])?;
    if command.first_argument != CommandArgument::User {
        return None;
    }
    let argument = before[first_space..].trim_start();
    if argument.contains(char::is_whitespace) {
        return None;
    }
    let with_at = argument.starts_with('@');
    let query = argument.strip_prefix('@').unwrap_or(argument).to_string();
    Some(CommandContext::CommandUser { query, with_at })
}

/// Stelle in UTF-16-Einheiten (wie im Browser) in eine Byte-Stelle umrechnen; liegt sie mitten in einem Zeichen,
/// gilt das Ende dieses Zeichens nicht, sondern sein Anfang.
pub fn utf16_to_byte_index(text: &str, utf16_index: usize) -> usize {
    let mut units = 0;
    for (byte, char) in text.char_indices() {
        if units + char.len_utf16() > utf16_index {
            return byte;
        }
        units += char.len_utf16();
    }
    text.len()
}

/// Byte-Stelle in eine Stelle in UTF-16-Einheiten (wie im Browser) umrechnen.
pub fn byte_to_utf16_index(text: &str, byte_index: usize) -> usize {
    text[..byte_index.min(text.len())].encode_utf16().count()
}

#[cfg(test)]
mod tests {
    #[test]
    fn schreibmarke_am_anfang() {
        assert_eq!(super::detect_command_context("/u", 0), None);
        assert_eq!(super::detect_command_context("/", 5), Some(super::CommandContext::Command { query: String::new() }));
        assert_eq!(super::detect_command_context("/ü", 2), None);
    }

    use super::*;

    fn names(list: Vec<&CommandInfo>) -> Vec<&str> {
        list.into_iter().map(|c| c.name).collect()
    }

    fn roles(moderator: bool, broadcaster: bool, vip: bool) -> OwnRoles {
        OwnRoles { moderator, broadcaster, vip }
    }

    #[test]
    fn kurzformen_fuer_dauern() {
        assert_eq!(parse_duration("30"), Some(30));
        assert_eq!(parse_duration("30s"), Some(30));
        assert_eq!(parse_duration("10m"), Some(600));
        assert_eq!(parse_duration("1h"), Some(3600));
        assert_eq!(parse_duration("1d"), Some(86_400));
        assert_eq!(parse_duration("2w"), Some(1_209_600));
        assert_eq!(parse_duration("1mo"), Some(2_592_000));
        assert_eq!(parse_duration("1D"), Some(86_400));
        assert_eq!(parse_duration(" 5m "), Some(300));
        assert_eq!(parse_duration("zehn"), None);
        assert_eq!(parse_duration("-5m"), None);
        assert_eq!(parse_duration("12345678"), None);
        assert_eq!(parse_duration("5x"), None);
    }

    #[test]
    fn befehlsliste_anfang_dann_enthalten_mod_nur_mit_rechten() {
        assert_eq!(names(suggest_commands("time", roles(true, false, false))), ["timeout", "untimeout"]);
        assert_eq!(names(suggest_commands("ban", roles(true, false, false))), ["ban", "unban"]);
        assert_eq!(names(suggest_commands("", roles(false, false, false))), ["user"]);
    }

    #[test]
    fn rollenbefehle_nur_im_eigenen_kanal() {
        assert!(suggest_commands("vip", roles(true, false, false)).is_empty());
        assert_eq!(names(suggest_commands("vip", roles(true, true, false))), ["vip", "unvip"]);
        assert_eq!(names(suggest_commands("mod", roles(true, true, false))), ["mod", "unmod"]);
        assert!(suggest_commands("editor", roles(true, true, false)).is_empty());
    }

    #[test]
    fn fuer_sich_selbst() {
        assert_eq!(names(suggest_commands("mod", roles(true, false, false))), ["unmod"]);
        assert_eq!(names(suggest_commands("vip", roles(false, false, true))), ["unvip"]);
        assert!(suggest_commands("vip", roles(false, false, false)).is_empty());
        assert!(suggest_commands("mod", roles(false, false, false)).is_empty());
        assert_eq!(names(suggest_commands("user", roles(false, false, false))), ["user"]);
    }

    #[test]
    fn zerlegt_befehl_argumente_und_rest() {
        assert_eq!(
            parse_command_line("/timeout @Spammer 1d Werbung im Chat"),
            Some(ParsedCommand {
                name: "timeout".into(),
                args: vec!["@Spammer".into(), "1d".into(), "Werbung".into(), "im".into(), "Chat".into()],
                rest: "@Spammer 1d Werbung im Chat".into(),
            })
        );
        assert_eq!(parse_command_line("/clear"), Some(ParsedCommand { name: "clear".into(), args: vec![], rest: String::new() }));
        assert_eq!(parse_command_line("kein Befehl"), None);
        assert_eq!(parse_command_line("/"), None);
    }

    #[test]
    fn namen_ohne_at_nur_gueltige_logins() {
        assert_eq!(login_from_argument(Some("@Spammer")).as_deref(), Some("spammer"));
        assert_eq!(login_from_argument(Some("böse!")), None);
        assert_eq!(login_from_argument(None), None);
        assert_eq!(login_from_argument(Some(&"a".repeat(26))), None);
    }

    #[test]
    fn erkennt_befehlswort_und_namensargument() {
        assert_eq!(detect_command_context("/tim", 4), Some(CommandContext::Command { query: "tim".into() }));
        assert_eq!(
            detect_command_context("/timeout sp", 11),
            Some(CommandContext::CommandUser { query: "sp".into(), with_at: false })
        );
        assert_eq!(
            detect_command_context("/ban @sp", 8),
            Some(CommandContext::CommandUser { query: "sp".into(), with_at: true })
        );
        assert_eq!(detect_command_context("/timeout spammer 1", 18), None);
        assert_eq!(detect_command_context("/clear ", 7), None);
        assert_eq!(detect_command_context("hallo", 5), None);
    }

    #[test]
    fn utf16_stellen_umrechnen() {
        // „😀“ sind zwei UTF-16-Einheiten und vier Bytes, „ä“ eine Einheit und zwei Bytes.
        let text = "/ban 😀ä";
        assert_eq!(utf16_to_byte_index(text, 5), 5);
        assert_eq!(utf16_to_byte_index(text, 7), 9);
        assert_eq!(utf16_to_byte_index(text, 8), 11);
        assert_eq!(utf16_to_byte_index(text, 99), text.len());
        assert_eq!(utf16_to_byte_index(text, 6), 5, "mitten im Zeichen: dessen Anfang");
    }
}
