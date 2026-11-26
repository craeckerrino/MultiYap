//! Umwandlung der Twitch-Daten in das gemeinsame Nachrichtenmodell, übertragen aus src/platforms/twitch/convert*.ts und
//! irc.ts. Nur Umwandlung, keine Netzwerkzugriffe (die folgen).

pub mod auth;
pub mod badges;
pub mod chat;
pub mod config;
pub mod events;
pub mod eventsub;
pub mod helix_api;
pub mod irc;
pub mod irc_reader;
pub mod moderation;
pub mod suspicious;

/// Emote-Bild laut Vorlage der Twitch-Doku. `dark`: Bild für das dunkle Design.
pub fn emote_url(id: &str, animated: bool, dark: bool, scale: &str) -> String {
    format!(
        "https://static-cdn.jtvnw.net/emoticons/v2/{}/{}/{}/{scale}",
        url_component(id),
        if animated { "animated" } else { "static" },
        if dark { "dark" } else { "light" },
    )
}

/// Wie `encodeURIComponent`: alles außer Buchstaben, Ziffern und `-_.!~*'()` wird kodiert.
pub fn url_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emote_url_wie_twitch_doku() {
        assert_eq!(emote_url("25", false, true, "1.0"), "https://static-cdn.jtvnw.net/emoticons/v2/25/static/dark/1.0");
        assert_eq!(emote_url("emotesv2_ab", true, false, "2.0"), "https://static-cdn.jtvnw.net/emoticons/v2/emotesv2_ab/animated/light/2.0");
        assert_eq!(url_component("a/b c"), "a%2Fb%20c");
    }
}
