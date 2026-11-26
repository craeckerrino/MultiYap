//! Autofill, übertragen aus src/features/autofill/autofill.ts.
//!
//! Namen: mit „@“ sofort; ohne „@“ per Tab-Taste. Quelle: Leute, die im Chat geschrieben haben, zuletzt aktive
//! zuerst. Emotes: „:“ plus mindestens 2 Zeichen. Stellen im Text sind hier Byte-Stellen; die Brücke rechnet sie
//! von und nach UTF-16 (Browser) um.

use serde::Serialize;

pub const MIN_EMOTE_QUERY: usize = 2;
const MAX_SUGGESTIONS: usize = 10;

/// Länge in UTF-16-Einheiten (wie `String.length` im Browser).
fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Wort direkt vor dem Cursor (bis zum letzten Leerzeichen oder Zeilenumbruch). Rückgabe: Anfang (Byte-Stelle) und
/// das Wort.
pub fn word_at_caret(text: &str, caret: usize) -> (usize, &str) {
    // Nie mitten in einem Zeichen schneiden.
    let mut end = caret.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let before = &text[..end];
    let start = before.rfind([' ', '\n']).map_or(0, |index| index + 1);
    (start, &before[start..])
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Trigger {
    #[serde(rename_all = "camelCase")]
    Name { query: String, with_at: bool },
    Emote { query: String },
    /// Tab-Taste ohne „@“ oder „:“: Namen und Emotes gemeinsam.
    Tab { query: String },
}

/// Was der Cursor gerade auslöst. Ohne „@“ oder „:“ nur über die Tab-Taste.
pub fn detect_trigger(word: &str, via_tab: bool) -> Option<Trigger> {
    if let Some(query) = word.strip_prefix('@') {
        return Some(Trigger::Name { query: query.to_string(), with_at: true });
    }
    if let Some(query) = word.strip_prefix(':') {
        if utf16_len(query) >= MIN_EMOTE_QUERY {
            return Some(Trigger::Emote { query: query.to_string() });
        }
    }
    if via_tab && !word.is_empty() {
        return Some(Trigger::Tab { query: word.to_string() });
    }
    None
}

/// Zuerst Treffer am Anfang, dann Treffer irgendwo; innerhalb gleicher Güte bleibt die Reihenfolge. Höchstens 10.
/// Rückgabe: Stellen in `keys`.
fn rank(keys: &[Vec<String>], query: &str) -> Vec<usize> {
    let q = query.to_lowercase();
    let mut starts = Vec::new();
    let mut contains = Vec::new();
    for (index, values) in keys.iter().enumerate() {
        if values.iter().any(|value| value.starts_with(&q)) {
            starts.push(index);
        } else if !q.is_empty() && values.iter().any(|value| value.contains(&q)) {
            contains.push(index);
        }
    }
    starts.into_iter().chain(contains).take(MAX_SUGGESTIONS).collect()
}

/// Name einer Person zum Vorschlagen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name {
    pub login: String,
    pub display_name: String,
}

/// Leute, die im Chat geschrieben haben, zuletzt aktive zuerst, ohne Dopplungen und ohne einen selbst. `senders` in
/// zeitlicher Reihenfolge (älteste zuerst). Rückgabe: Stellen in `senders`.
pub fn recent_chatters(senders: &[Name], own_login: Option<&str>) -> Vec<usize> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for (index, sender) in senders.iter().enumerate().rev() {
        if own_login == Some(sender.login.as_str()) || !seen.insert(sender.login.as_str()) {
            continue;
        }
        result.push(index);
    }
    result
}

/// Namensvorschläge. Rückgabe: Stellen in `chatters`.
pub fn suggest_names(chatters: &[Name], query: &str) -> Vec<usize> {
    let keys: Vec<Vec<String>> =
        chatters.iter().map(|n| vec![n.login.to_lowercase(), n.display_name.to_lowercase()]).collect();
    rank(&keys, query)
}

/// Suchindex für Emote-Namen, einmal je Emote-Liste gebaut (es können Tausende sein).
#[derive(Debug, Default)]
pub struct EmoteIndex {
    names: Vec<String>,
    lower: Vec<Vec<String>>,
}

impl EmoteIndex {
    pub fn new(names: Vec<String>) -> Self {
        let lower = names.iter().map(|n| vec![n.to_lowercase()]).collect();
        Self { names, lower }
    }

    /// Emote-Vorschläge: Treffer am Anfang (kürzere Namen zuerst), dann Treffer in der Mitte. Rückgabe: Stellen.
    pub fn suggest(&self, query: &str) -> Vec<usize> {
        let ranked = rank(&self.lower, query);
        let q = query.to_lowercase();
        let (mut starts, rest): (Vec<usize>, Vec<usize>) = ranked.into_iter().partition(|&i| self.lower[i][0].starts_with(&q));
        // Stabil sortieren: bei gleicher Länge bleibt die Reihenfolge.
        starts.sort_by_key(|&i| utf16_len(&self.names[i]));
        starts.into_iter().chain(rest).collect()
    }

    fn starts_with(&self, index: usize, query_lower: &str) -> bool {
        self.lower[index][0].starts_with(query_lower)
    }
}

/// Ein Eintrag der gemeinsamen Liste für die Tab-Taste.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mixed {
    Name(usize),
    Emote(usize),
}

/// Gemeinsame Liste für die Tab-Taste: zuerst Namen und Emotes, die so anfangen, dann solche, die den Text
/// enthalten. Höchstens 10 Einträge.
pub fn suggest_mixed(chatters: &[Name], emotes: &EmoteIndex, query: &str) -> Vec<Mixed> {
    let q = query.to_lowercase();
    let names = suggest_names(chatters, query);
    let emote_list = emotes.suggest(query);
    let name_starts =
        |&i: &usize| chatters[i].login.starts_with(&q) || chatters[i].display_name.to_lowercase().starts_with(&q);
    let emote_starts = |&i: &usize| emotes.starts_with(i, &q);
    names
        .iter()
        .filter(|i| name_starts(i))
        .map(|&i| Mixed::Name(i))
        .chain(emote_list.iter().filter(|i| emote_starts(i)).map(|&i| Mixed::Emote(i)))
        .chain(names.iter().filter(|i| !name_starts(i)).map(|&i| Mixed::Name(i)))
        .chain(emote_list.iter().filter(|i| !emote_starts(i)).map(|&i| Mixed::Emote(i)))
        .take(MAX_SUGGESTIONS)
        .collect()
}

/// Text, der für einen Namen eingesetzt wird. Anzeigename, wenn er nur in der Schreibweise abweicht.
pub fn name_text(login: &str, display_name: &str, with_at: bool) -> String {
    let name = if display_name.to_lowercase() == login { display_name } else { login };
    format!("{}{name}", if with_at { "@" } else { "" })
}

/// Ersetzt das Wort zwischen `start` und `end` (Byte-Stellen) und hängt ein Leerzeichen an, falls keins folgt.
/// Rückgabe: neuer Text und neue Cursor-Stelle (Byte-Stelle).
pub fn replace_word(text: &str, start: usize, end: usize, replacement: &str) -> (String, usize) {
    let after = &text[end.min(text.len())..];
    let insert = if after.starts_with(' ') { replacement.to_string() } else { format!("{replacement} ") };
    let next = format!("{}{insert}{after}", &text[..start.min(text.len())]);
    (next, start + insert.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(login: &str, display: &str) -> Name {
        Name { login: login.into(), display_name: display.into() }
    }

    #[test]
    fn ausloeser() {
        assert_eq!(detect_trigger("@", false), Some(Trigger::Name { query: String::new(), with_at: true }));
        assert_eq!(detect_trigger("@ma", false), Some(Trigger::Name { query: "ma".into(), with_at: true }));
        assert_eq!(detect_trigger(":k", false), None);
        assert_eq!(detect_trigger(":ka", false), Some(Trigger::Emote { query: "ka".into() }));
        assert_eq!(detect_trigger("ma", false), None);
        assert_eq!(detect_trigger("ma", true), Some(Trigger::Tab { query: "ma".into() }));
        assert_eq!(detect_trigger("", true), None);
    }

    #[test]
    fn wort_vor_dem_cursor() {
        assert_eq!(word_at_caret("hallo @ma", 9), (6, "@ma"));
        assert_eq!(word_at_caret("hallo @ma welt", 9), (6, "@ma"));
        assert_eq!(word_at_caret("", 0), (0, ""));
        assert_eq!(word_at_caret("a\nb", 3), (2, "b"));
    }

    #[test]
    fn namen_zuletzt_aktive_zuerst() {
        let senders = [name("anna", "anna"), name("max", "Max"), name("marie", "marie"), name("anna", "anna"), name("ich", "ich")];
        let recent = recent_chatters(&senders, Some("ich"));
        let logins: Vec<&str> = recent.iter().map(|&i| senders[i].login.as_str()).collect();
        assert_eq!(logins, ["anna", "marie", "max"]);

        let chatters: Vec<Name> = recent.iter().map(|&i| senders[i].clone()).collect();
        let found: Vec<&str> = suggest_names(&chatters, "ma").iter().map(|&i| chatters[i].login.as_str()).collect();
        assert_eq!(found, ["marie", "max"]);
        let found: Vec<&str> = suggest_names(&chatters, "n").iter().map(|&i| chatters[i].login.as_str()).collect();
        assert_eq!(found, ["anna"]);
    }

    #[test]
    fn namen_einsetzen() {
        assert_eq!(name_text("max", "Max", true), "@Max");
        assert_eq!(name_text("abc", "日本", false), "abc");
        let (start, _) = word_at_caret("hi @ma", 6);
        assert_eq!(replace_word("hi @ma", start, 6, "@Max"), ("hi @Max ".to_string(), 8));
        assert_eq!(replace_word("hi @ma welt", 3, 6, "@Max"), ("hi @Max welt".to_string(), 7));
    }

    #[test]
    fn tab_taste_namen_und_emotes_gemeinsam() {
        let chatters = [name("kai", "Kai"), name("nika", "Nika")];
        let emotes = EmoteIndex::new(vec!["KappaPride".into(), "Kappa".into(), "MiniK".into()]);
        let result: Vec<String> = suggest_mixed(&chatters, &emotes, "ka")
            .into_iter()
            .map(|entry| match entry {
                Mixed::Name(i) => format!("@{}", chatters[i].login),
                Mixed::Emote(i) => emotes.names[i].clone(),
            })
            .collect();
        assert_eq!(result, ["@kai", "Kappa", "KappaPride", "@nika"]);
    }

    #[test]
    fn emotes_kuerzere_anfangstreffer_zuerst() {
        let emotes = EmoteIndex::new(vec!["KappaPride".into(), "Kappa".into(), "Keepo".into(), "MiniK".into()]);
        let names = |q: &str| -> Vec<String> { emotes.suggest(q).iter().map(|&i| emotes.names[i].clone()).collect() };
        assert_eq!(names("ka"), ["Kappa", "KappaPride"]);
        assert_eq!(names("k"), ["Kappa", "Keepo", "KappaPride", "MiniK"]);
    }
}
