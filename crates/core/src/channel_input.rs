//! Kanalnamen und eingefügte Links erkennen (bisher src/core/workspace/channel-input.ts). Beispiele: „name“, „@name“,
//! „twitch.tv/name“, „https://www.twitch.tv/popout/name/chat“, „youtube.com/@name“.

use url::Url;

/// Ergebnis der Erkennung.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedChannel {
    /// Erkannte Plattform, falls ein Link eingefügt wurde.
    pub platform: Option<&'static str>,
    /// Kanalname in Kleinbuchstaben ohne @, oder None, wenn ungültig.
    pub channel: Option<String>,
}

/// Pfade auf twitch.tv, die keine Kanäle sind.
const TWITCH_RESERVED: [&str; 11] = ["directory", "videos", "settings", "subscriptions", "inventory", "wallet", "search", "downloads", "jobs", "p", "turbo"];
const LINK_HOSTS: [&str; 3] = ["twitch.tv", "youtube.com", "youtu.be"];

fn is_twitch_login(text: &str) -> bool {
    (1..=25).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Sieht die Eingabe wie ein Link zu Twitch oder YouTube aus? Wie /^(https?:\/\/)?([a-z0-9-]+\.)*(twitch\.tv|youtube\.com|youtu\.be)(\/|$)/i.
fn looks_like_link(raw: &str) -> bool {
    let lower = raw.to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://")).unwrap_or(&lower);
    let host = rest.split('/').next().unwrap_or_default();
    if host.len() < rest.len() && !rest[host.len()..].starts_with('/') {
        return false;
    }
    LINK_HOSTS.iter().any(|known| {
        let Some(prefix) = host.strip_suffix(known) else { return false };
        prefix.is_empty()
            || prefix.strip_suffix('.').is_some_and(|labels| labels.split('.').all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')))
    })
}

/// Erkennt Kanalnamen und eingefügte Links.
pub fn parse(input: &str) -> ParsedChannel {
    let raw = input.trim();
    if raw.is_empty() {
        return ParsedChannel::default();
    }
    if looks_like_link(raw) {
        let lower = raw.to_ascii_lowercase();
        let text = if lower.starts_with("http://") || lower.starts_with("https://") { raw.to_string() } else { format!("https://{raw}") };
        let Ok(url) = Url::parse(&text) else { return ParsedChannel::default() };
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        let segments: Vec<&str> = url.path().split('/').filter(|s| !s.is_empty()).collect();
        if host == "twitch.tv" || host.ends_with(".twitch.tv") {
            // popout/<kanal>/chat und moderator/<kanal> enthalten den Kanal an zweiter Stelle.
            let index = usize::from(matches!(segments.first(), Some(&"popout") | Some(&"moderator")));
            let login = segments.get(index).copied().unwrap_or_default().to_lowercase();
            let valid = is_twitch_login(&login) && !TWITCH_RESERVED.contains(&login.as_str());
            return ParsedChannel { platform: Some("twitch"), channel: valid.then_some(login) };
        }
        // YouTube: Kanal als „@name“ oder „UC…“ (crates/core/src/youtube.rs).
        return ParsedChannel { platform: Some("youtube"), channel: crate::youtube::channel_key(raw) };
    }
    let login = raw.strip_prefix('@').unwrap_or(raw).to_lowercase();
    ParsedChannel { platform: None, channel: is_twitch_login(&login).then_some(login) }
}

/// Gültiger Twitch-Kanal aus der Eingabe (Name oder Twitch-Link), sonst None.
pub fn twitch_channel(input: &str) -> Option<String> {
    let parsed = parse(input);
    parsed.channel.filter(|_| parsed.platform.is_none_or(|p| p == "twitch"))
}

/// Gültiger YouTube-Kanal aus der Eingabe (Name, Kanal-ID oder YouTube-Link), sonst None. Twitch-Links zählen nicht.
pub fn youtube_channel(input: &str) -> Option<String> {
    if parse(input).platform == Some("twitch") {
        return None;
    }
    crate::youtube::channel_key(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(platform: Option<&'static str>, channel: Option<&str>) -> ParsedChannel {
        ParsedChannel { platform, channel: channel.map(str::to_string) }
    }

    #[test]
    fn einfache_namen_auch_mit_at_und_grossbuchstaben() {
        assert_eq!(parse("Craecker"), parsed(None, Some("craecker")));
        assert_eq!(parse("  @some_name "), parsed(None, Some("some_name")));
    }

    #[test]
    fn twitch_links() {
        assert_eq!(parse("twitch.tv/xqc"), parsed(Some("twitch"), Some("xqc")));
        assert_eq!(parse("https://www.twitch.tv/Shroud?ref=x"), parsed(Some("twitch"), Some("shroud")));
        assert_eq!(parse("https://www.twitch.tv/popout/abc/chat?popout="), parsed(Some("twitch"), Some("abc")));
        assert_eq!(parse("https://m.twitch.tv/moderator/abc"), parsed(Some("twitch"), Some("abc")));
    }

    #[test]
    fn twitch_seiten_ohne_kanal() {
        assert_eq!(parse("https://www.twitch.tv/directory"), parsed(Some("twitch"), None));
    }

    #[test]
    fn youtube_links() {
        assert_eq!(parse("https://www.youtube.com/@someone"), parsed(Some("youtube"), Some("@someone")));
        assert_eq!(parse("https://youtu.be/abc"), parsed(Some("youtube"), None));
        assert_eq!(youtube_channel("SomeOne"), Some("@someone".into()));
        assert_eq!(youtube_channel("twitch.tv/someone"), None);
    }

    #[test]
    fn ungueltige_namen() {
        assert_eq!(parse(""), parsed(None, None));
        assert_eq!(parse("mit leerzeichen"), parsed(None, None));
        assert_eq!(parse(&"a".repeat(26)), parsed(None, None));
        assert_eq!(twitch_channel("youtube.com/@x"), None);
        assert_eq!(twitch_channel("twitch.tv/abc"), Some("abc".into()));
    }
}
