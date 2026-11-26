//! Suche im Einstellungsfenster (bisher settings-search.ts): Groß- und Kleinschreibung egal, alle Wörter der Suche
//! müssen vorkommen, Reihenfolge egal.

/// Vereinheitlicht Text für die Suche: Kleinbuchstaben, Leerraum zusammengefasst.
pub fn normalize_search(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Alle Wörter der Suche müssen im Text vorkommen.
pub fn matches_search(text: &str, query: &str) -> bool {
    let haystack = normalize_search(text);
    normalize_search(query)
        .split(' ')
        .filter(|w| !w.is_empty())
        .all(|word| haystack.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alle_woerter_reihenfolge_egal() {
        assert!(matches_search("Gesamtlautstärke", "LAUT"));
        assert!(matches_search(
            "Kanal geht live: Ton, Tab blinkt",
            "blinkt kanal"
        ));
        assert!(!matches_search("Kanal geht live", "kanal sound"));
        assert!(matches_search("Format  der\nUhrzeit", "der uhrzeit"));
        assert!(matches_search("Irgendwas", "   "));
    }
}
