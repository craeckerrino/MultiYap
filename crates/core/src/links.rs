//! Link-Erkennung, übertragen aus src/features/chat/links.ts.
//!
//! Mit „http(s)://“ oder „www.“ immer; ohne nur bei bekannten Endungen, damit „datei.txt“ oder „z.B.“ kein Link
//! werden. Die echte Zieladresse (internationale Namen als „xn--“) erscheint beim Drüberfahren, damit ähnlich
//! aussehende Adressen auffallen. Nur http und https, alles andere bleibt Text.

use std::sync::OnceLock;

use regex::Regex;
use url::Url;

const TLDS: &[&str] = &[
    "com", "net", "org", "de", "at", "ch", "eu", "tv", "gg", "io", "co", "me", "ly", "be", "nl", "fr", "es", "it",
    "pl", "uk", "us", "ca", "au", "ru", "info", "biz", "xyz", "app", "dev", "link", "live", "shop", "store",
    "online", "site", "club", "top", "to", "fm", "im", "cc", "gl", "sh", "so", "ai", "gle", "page", "art", "fun",
    "games",
];

fn url_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        // Wortgrenzen wie in JavaScript ohne „u“: nur ASCII-Buchstaben, Ziffern und Unterstrich.
        let pattern = format!(
            r#"(?i)(?:https?://|www\.)[^\s<>"]+|(?-u:\b)(?:[a-z0-9](?:[a-z0-9-]{{0,61}}[a-z0-9])?\.)+(?:{})(?-u:\b)(?::\d{{2,5}})?(?:/[^\s<>"]*)?"#,
            TLDS.join("|")
        );
        Regex::new(&pattern).expect("festes Muster ist gültig")
    })
}

/// Ein Teil eines Textes: normaler Text oder ein erkannter Link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkPart {
    Text(String),
    Link { text: String, url: String },
}

/// Satzzeichen am Ende gehören nicht zum Link; „)“ nur, wenn es keine passende „(“ im Link gibt.
fn trim_trailing(candidate: &str) -> &str {
    let mut end = candidate.len();
    while let Some(char) = candidate[..end].chars().next_back() {
        if ".,!?;:'\"\u{201C}\u{201D}\u{2018}\u{2019}]}".contains(char) {
            end -= char.len_utf8();
            continue;
        }
        if char == ')' {
            let body = &candidate[..end];
            if body.matches('(').count() < body.matches(')').count() {
                end -= 1;
                continue;
            }
        }
        break;
    }
    &candidate[..end]
}

/// Zieladresse eines erkannten Links oder None, wenn es keine gültige http(s)-Adresse ist.
pub fn link_target(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let with_scheme =
        if lower.starts_with("http://") || lower.starts_with("https://") { text.to_string() } else { format!("https://{text}") };
    let url = Url::parse(&with_scheme).ok()?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return None;
    }
    if !url.host_str().is_some_and(|host| host.contains('.')) {
        return None;
    }
    Some(url.to_string())
}

/// Darf vor einem Link stehen? Kein Buchstabe, keine Ziffer und kein Adressteil direkt davor (zum Beispiel keine
/// Mail-Adresse „a@b.de“ und kein „ftp://…“).
fn blocks_link_before(char: char) -> bool {
    char.is_alphanumeric() || "@_.:/-".contains(char)
}

/// Teilt Text in Text- und Link-Teile.
pub fn split_links(text: &str) -> Vec<LinkPart> {
    let mut parts = Vec::new();
    let mut last = 0;
    for found in url_pattern().find_iter(text) {
        let start = found.start();
        let raw = trim_trailing(found.as_str());
        if raw.is_empty() || text[..start].chars().next_back().is_some_and(blocks_link_before) {
            continue;
        }
        let Some(url) = link_target(raw) else {
            continue;
        };
        if start > last {
            parts.push(LinkPart::Text(text[last..start].to_string()));
        }
        parts.push(LinkPart::Link { text: raw.to_string(), url });
        last = start + raw.len();
    }
    if parts.is_empty() {
        return vec![LinkPart::Text(text.to_string())];
    }
    if last < text.len() {
        parts.push(LinkPart::Text(text[last..].to_string()));
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links(text: &str) -> Vec<String> {
        split_links(text)
            .into_iter()
            .filter_map(|part| match part {
                LinkPart::Link { text, url } => Some(format!("{text} -> {url}")),
                LinkPart::Text(_) => None,
            })
            .collect()
    }

    #[test]
    fn mit_und_ohne_schema_ohne_satzzeichen_am_ende() {
        assert_eq!(links("schau mal https://www.twitch.tv/kanal!"), vec!["https://www.twitch.tv/kanal -> https://www.twitch.tv/kanal"]);
        assert_eq!(links("geh auf youtube.com/watch?v=abc."), vec!["youtube.com/watch?v=abc -> https://youtube.com/watch?v=abc"]);
        assert_eq!(links("www.example.org, danke"), vec!["www.example.org -> https://www.example.org/"]);
        assert_eq!(
            links("(siehe de.wikipedia.org/wiki/Test_(Begriff))"),
            vec!["de.wikipedia.org/wiki/Test_(Begriff) -> https://de.wikipedia.org/wiki/Test_(Begriff)"]
        );
    }

    #[test]
    fn kein_link_bei_dateien_abkuerzungen_mail_und_anderen_schemata() {
        assert!(links("datei.txt z.B. usw.").is_empty());
        assert!(links("schreib an mail@example.com").is_empty());
        assert!(links("javascript:alert(1)").is_empty());
        assert!(links("ftp://example.com/x").is_empty());
    }

    #[test]
    fn internationale_namen_als_xn() {
        assert_eq!(link_target("https://\u{0430}pple.com").as_deref(), Some("https://xn--pple-43d.com/"));
    }

    #[test]
    fn text_bleibt_erhalten() {
        let parts = split_links("a twitch.tv/x b");
        assert_eq!(
            parts,
            vec![
                LinkPart::Text("a ".into()),
                LinkPart::Link { text: "twitch.tv/x".into(), url: "https://twitch.tv/x".into() },
                LinkPart::Text(" b".into()),
            ]
        );
        assert_eq!(split_links("nur Text"), vec![LinkPart::Text("nur Text".into())]);
        assert_eq!(split_links(""), vec![LinkPart::Text(String::new())]);
    }

    #[test]
    fn umlaute_vor_und_nach_dem_link() {
        assert_eq!(links("über twitch.tv/x äh"), vec!["twitch.tv/x -> https://twitch.tv/x"]);
        assert!(links("äwww.example.org").is_empty());
    }
}
