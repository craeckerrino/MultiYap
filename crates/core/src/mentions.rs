//! Erwähnungen, übertragen aus src/features/mentions/matcher.ts.
//!
//! Hier nur die Namenserkennung: eigener Name und zusätzliche Namen als ganze Wörter, mit oder ohne „@“,
//! Groß- und Kleinschreibung egal. Ob die Nachricht eigene ist oder eine Antwort an einen selbst, prüft der Aufrufer.

use regex::Regex;

use crate::{escape, flexible_whitespace, whole_words};

#[derive(Debug, Default)]
pub struct NameMatcher {
    regex: Option<Regex>,
    /// Je Name (längste zuerst) ein eigener Ausdruck, um nach einem Treffer den passenden Namen zu finden.
    each: Vec<(String, Regex)>,
}

impl NameMatcher {
    pub fn new(names: &[String]) -> Self {
        let mut cleaned: Vec<String> = names
            .iter()
            .map(|name| name.trim().trim_start_matches('@').to_string())
            .filter(|name| !name.is_empty())
            .collect();
        if cleaned.is_empty() {
            return Self::default();
        }
        cleaned.sort_by_key(|name| std::cmp::Reverse(name.chars().count()));
        let alternatives = cleaned.iter().map(|n| flexible_whitespace(&escape(n))).collect::<Vec<_>>().join("|");
        let each = cleaned.iter().filter_map(|n| Regex::new(&whole_words(&flexible_whitespace(&escape(n)))).ok().map(|r| (n.clone(), r))).collect();
        Self { regex: Regex::new(&whole_words(&alternatives)).ok(), each }
    }

    pub fn is_match(&self, text: &str) -> bool {
        self.regex.as_ref().is_some_and(|regex| regex.is_match(text))
    }

    /// Welcher Name passt (bei mehreren der längste), so wie er eingetragen ist (ohne „@“).
    pub fn matched_name(&self, text: &str) -> Option<&str> {
        if !self.is_match(text) {
            return None;
        }
        self.each.iter().find(|(_, regex)| regex.is_match(text)).map(|(name, _)| name.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matcher(names: &[&str]) -> NameMatcher {
        NameMatcher::new(&names.iter().map(|n| n.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn eigener_name_mit_oder_ohne_at_ganze_woerter() {
        let m = matcher(&["craecker"]);
        assert!(m.is_match("hallo @Craecker!"));
        assert!(m.is_match("CRAECKER was geht"));
        assert!(!m.is_match("craeckers Stream"));
        assert!(!m.is_match("nichts hier"));
    }

    #[test]
    fn zusaetzliche_namen_und_phrasen() {
        let m = matcher(&["Kapitaen", "der Chef", "@extra"]);
        assert!(!m.is_match("craecker"));
        assert!(m.is_match("hey kapitaen"));
        assert!(m.is_match("frag  DER chef"));
        assert!(m.is_match("hi extra"));
    }

    #[test]
    fn passender_name() {
        let m = matcher(&["Kapitaen", "der Chef", "@extra"]);
        assert_eq!(m.matched_name("hey KAPITAEN"), Some("Kapitaen"));
        assert_eq!(m.matched_name("frag der  chef"), Some("der Chef"));
        assert_eq!(m.matched_name("hi @extra"), Some("extra"));
        assert_eq!(m.matched_name("niemand"), None);
    }

    #[test]
    fn ohne_namen_keine_erwaehnungen() {
        assert!(!matcher(&[]).is_match("craecker"));
        assert!(!matcher(&["  ", "@"]).is_match("@"));
    }
}
