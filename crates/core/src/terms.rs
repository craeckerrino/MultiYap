//! Filter-Kategorien, übertragen aus src/features/highlights/matcher.ts.
//!
//! Ohne „Regex“: ganze Wörter oder Phrasen, Groß- und Kleinschreibung egal, Leerraum in Phrasen flexibel. Mit „Regex“:
//! abgewandelte Schreibweisen automatisch, auch als Wortteil. Rusts regex arbeitet in linearer Zeit, ein Muster kann
//! den Chat also nie aufhalten.

use regex::Regex;

use crate::{escape, flexible_whitespace, whole_words};

/// Übliche Ersatzzeichen für „Regex“: 4 und @ für a, 3 für e, 1 und ! für i, 0 für o, 5 und $ für s.
fn substitutes(char: char) -> Option<&'static str> {
    match char {
        'a' => Some("a4@"),
        'e' => Some("e3"),
        'i' => Some("i1!"),
        'o' => Some("o0"),
        's' => Some("s5$"),
        _ => None,
    }
}

/// Trennzeichen zwischen zwei Buchstaben bei „Regex“: alles außer Buchstaben, Ziffern und Ersatzzeichen, höchstens
/// drei hintereinander („s.p.a.m“, „s p a m“).
const GAP: &str = r"[^\p{L}\p{N}@$!]{0,3}";

/// Muster für ein Wort mit „Regex“: gedehnte Buchstaben („spaaam“), Trennzeichen dazwischen („s.p.a.m“),
/// Ersatzzeichen („sp4m“). Gleiche Buchstaben direkt hintereinander zählen als einer. Andere Zeichen im Wort (zum
/// Beispiel „+“) müssen genau so vorkommen.
pub fn fuzzy_term_pattern(term: &str) -> String {
    let mut units: Vec<String> = Vec::new();
    let mut previous: Option<char> = None;
    for char in term.to_lowercase().chars() {
        if char.is_whitespace() {
            previous = None;
            continue;
        }
        let letter = char.is_alphanumeric();
        if letter && previous == Some(char) {
            continue;
        }
        previous = Some(char);
        let unit = match substitutes(char) {
            Some(set) => format!("[{set}]+"),
            None if letter => format!("{}+", escape(&char.to_string())),
            None => escape(&char.to_string()),
        };
        units.push(unit);
    }
    units.join(GAP)
}

/// Eine Filter-Kategorie, so weit die Erkennung sie braucht.
#[derive(Debug, Clone, Default)]
pub struct CategoryTerms {
    pub terms: Vec<String>,
    /// Schalter „Regex“.
    pub regex: bool,
}

/// Erkennung für alle Kategorien, einmal gebaut.
#[derive(Debug, Default)]
pub struct TermMatcher {
    /// Je Kategorie (gleiche Reihenfolge wie übergeben) ein Ausdruck, None ohne Begriffe.
    compiled: Vec<Option<Regex>>,
}

impl TermMatcher {
    pub fn new(categories: &[CategoryTerms]) -> Self {
        let compiled = categories.iter().map(compile).collect();
        Self { compiled }
    }

    /// Stellen aller passenden Kategorien in der eingestellten Reihenfolge. Die erste passende, die nicht wegen
    /// einer Ausnahme übersprungen wird, gilt (das entscheidet der Aufrufer).
    pub fn matching(&self, text: &str) -> Vec<usize> {
        self.compiled
            .iter()
            .enumerate()
            .filter_map(|(index, regex)| regex.as_ref().filter(|r| r.is_match(text)).map(|_| index))
            .collect()
    }

    /// Erste passende Kategorie.
    pub fn first(&self, text: &str) -> Option<usize> {
        self.compiled
            .iter()
            .position(|regex| regex.as_ref().is_some_and(|r| r.is_match(text)))
    }
}

fn compile(category: &CategoryTerms) -> Option<Regex> {
    let mut terms: Vec<&str> = category.terms.iter().map(|t| t.trim()).filter(|t| !t.is_empty()).collect();
    if terms.is_empty() {
        return None;
    }
    // Längere zuerst, wie bisher.
    terms.sort_by_key(|t| std::cmp::Reverse(t.chars().count()));
    let pattern = if category.regex {
        let joined = terms
            .iter()
            .map(|t| fuzzy_term_pattern(t))
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("|");
        if joined.is_empty() {
            return None;
        }
        format!("(?i){joined}")
    } else {
        whole_words(&terms.iter().map(|t| flexible_whitespace(&escape(t))).collect::<Vec<_>>().join("|"))
    };
    // Ein Ausdruck, der sich nicht bauen lässt (zum Beispiel zu groß), trifft nichts statt abzustürzen.
    Regex::new(&pattern).ok()
}

/// Wörter und Phrasen aus dem Textfeld: eine pro Zeile oder durch Komma getrennt, ohne Dopplungen (Groß- und
/// Kleinschreibung egal), je höchstens 100 Zeichen, insgesamt höchstens 500.
pub fn parse_terms(text: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut terms = Vec::new();
    for raw in text.split(['\n', ',']) {
        let term: String = raw.trim().chars().take(100).collect();
        if term.is_empty() || !seen.insert(term.to_lowercase()) {
            continue;
        }
        terms.push(term);
        if terms.len() == 500 {
            break;
        }
    }
    terms
}

#[cfg(test)]
mod tests {
    use super::*;

    fn category(terms: &[&str], regex: bool) -> CategoryTerms {
        CategoryTerms { terms: terms.iter().map(|t| t.to_string()).collect(), regex }
    }

    fn first(matcher: &TermMatcher, text: &str) -> Option<usize> {
        matcher.first(text)
    }

    #[test]
    fn ganze_woerter_und_phrasen_schreibung_egal() {
        let m = TermMatcher::new(&[category(&["spam", "kauf jetzt"], false)]);
        assert_eq!(first(&m, "Das ist SPAM!"), Some(0));
        assert_eq!(first(&m, "bitte KAUF   jetzt hier"), Some(0));
        assert_eq!(first(&m, "spamming geht nicht"), None);
        assert_eq!(first(&m, "nospam"), None);
        assert_eq!(first(&m, "Äpfel"), None);
    }

    #[test]
    fn umlaute_und_sonderzeichen_sicher() {
        let m = TermMatcher::new(&[category(&["grüße", "c++", "(x)"], false)]);
        assert_eq!(first(&m, "Liebe Grüße!"), Some(0));
        assert_eq!(first(&m, "ich mag c++ sehr"), Some(0));
        assert_eq!(first(&m, "Variante (x) gewählt"), Some(0));
        assert_eq!(first(&m, "grüßen"), None);
    }

    #[test]
    fn erste_passende_kategorie_gilt() {
        let m = TermMatcher::new(&[category(&["hallo"], false), category(&["hallo", "welt"], false)]);
        assert_eq!(first(&m, "hallo welt"), Some(0));
        assert_eq!(m.matching("hallo welt"), vec![0, 1]);
        assert_eq!(first(&m, "welt"), Some(1));
    }

    #[test]
    fn ohne_begriffe_trifft_nichts() {
        assert_eq!(first(&TermMatcher::new(&[]), "irgendwas"), None);
        assert_eq!(first(&TermMatcher::new(&[category(&[], false)]), "leer"), None);
    }

    #[test]
    fn eingabe_pro_zeile_oder_komma_ohne_dopplungen() {
        assert_eq!(parse_terms("spam\n Kauf jetzt , SPAM,,\n\nwerbung"), vec!["spam", "Kauf jetzt", "werbung"]);
    }

    #[test]
    fn regex_gedehnt_getrennt_ersetzt_wortteil() {
        let m = TermMatcher::new(&[category(&["spam", "kauf jetzt"], true)]);
        for text in ["spaaaam", "SSPPAAMM", "s p a m", "s.p.a.m", "s-p_a*m", "sp4m", "5p@m", "$pam", "spamming", "nospam", "k4uf   j3tzt"] {
            assert_eq!(first(&m, text), Some(0), "{text}");
        }
        for text in ["spa am Meer", "Spaß macht das", "s  ,  p a m"] {
            assert_eq!(first(&m, text), None, "{text}");
        }
    }

    #[test]
    fn ohne_regex_bleibt_es_bei_ganzen_woertern() {
        let m = TermMatcher::new(&[category(&["spam"], false)]);
        assert_eq!(first(&m, "spam"), Some(0));
        assert_eq!(first(&m, "spamming"), None);
        assert_eq!(first(&m, "sp4m"), None);
    }

    #[test]
    fn sonderzeichen_im_wort_genau_so() {
        let m = TermMatcher::new(&[category(&["c++"], true)]);
        assert_eq!(first(&m, "ich mag c++"), Some(0));
        assert_eq!(first(&m, "ich mag c"), None);
    }

    #[test]
    fn schnell_bei_langen_nachrichten() {
        let m = TermMatcher::new(&[category(&["spam", "kauf jetzt"], true)]);
        let start = std::time::Instant::now();
        first(&m, &format!("{}x", "s p a ".repeat(80)));
        first(&m, &format!("{}!", "a".repeat(499)));
        assert!(start.elapsed().as_millis() < 200);
    }
}
