//! Spam-Erkennung, übertragen aus src/features/spam/detect.ts.
//!
//! Zählt ähnliche Nachrichten derselben Person im selben Kanal innerhalb eines Zeitfensters. Nur Anzeige, nie eine
//! automatische Aktion.

use std::collections::{HashMap, VecDeque};

/// Vereinheitlicht für den Vergleich: Kleinbuchstaben, ohne Leerraum, höchstens 500 Zeichen.
pub fn normalize(text: &str) -> Vec<char> {
    text.to_lowercase().chars().filter(|c| !c.is_whitespace()).take(500).collect()
}

/// Zeichenpaare mit Häufigkeit.
fn bigrams(text: &[char]) -> HashMap<(char, char), u32> {
    let mut pairs = HashMap::with_capacity(text.len());
    for window in text.windows(2) {
        *pairs.entry((window[0], window[1])).or_insert(0) += 1;
    }
    pairs
}

/// Ähnlichkeit von 0 bis 1 (Dice-Koeffizient über Zeichenpaare): schnell, unabhängig von der Länge und robust
/// gegen kleine Änderungen wie angehängte Zeichen.
pub fn similarity(a: &[char], b: &[char]) -> f64 {
    similarity_of(&Prepared::new(a.to_vec()), &Prepared::new(b.to_vec()))
}

/// Vereinheitlichter Text mit seinen Zeichenpaaren, einmal je Nachricht berechnet (nicht je Vergleich).
struct Prepared {
    text: Vec<char>,
    pairs: HashMap<(char, char), u32>,
}

impl Prepared {
    fn new(text: Vec<char>) -> Self {
        let pairs = bigrams(&text);
        Self { text, pairs }
    }
}

fn similarity_of(a: &Prepared, b: &Prepared) -> f64 {
    if a.text == b.text {
        return 1.0;
    }
    if a.text.len() < 2 || b.text.len() < 2 {
        return 0.0;
    }
    // Über die kleinere Paarliste zählen.
    let (small, large) = if a.pairs.len() <= b.pairs.len() { (&a.pairs, &b.pairs) } else { (&b.pairs, &a.pairs) };
    let shared: u32 = small.iter().map(|(pair, count)| (*count).min(*large.get(pair).unwrap_or(&0))).sum();
    (2 * shared) as f64 / ((a.text.len() - 1) + (b.text.len() - 1)) as f64
}

#[derive(Debug, Clone, Copy)]
pub struct SpamSettings {
    pub enabled: bool,
    /// Prozent (0 bis 100).
    pub similarity: f64,
    pub min_count: u32,
    pub window_minutes: f64,
    pub min_length: usize,
}

/// Eine Nachricht für die Zählung (eigene Nachrichten gibt der Aufrufer gar nicht erst hinein).
pub struct SpamEntry<'a> {
    /// Kanal und Person, zum Beispiel „Kanal-ID:Nutzer-ID“.
    pub key: &'a str,
    /// Millisekunden.
    pub timestamp: f64,
    pub text: &'a str,
}

/// Für jede Nachricht: wie oft dieselbe Person im Zeitfenster (diese eingeschlossen) etwas Ähnliches geschrieben
/// hat; 0, wenn es unter „Zähler ab“ liegt oder nicht zählt.
pub fn spam_counts(entries: &[SpamEntry<'_>], settings: SpamSettings) -> Vec<u32> {
    let mut counts = vec![0; entries.len()];
    if !settings.enabled {
        return counts;
    }
    let window_ms = settings.window_minutes * 60_000.0;
    let threshold = settings.similarity / 100.0;
    let mut by_sender: HashMap<&str, VecDeque<(f64, Prepared)>> = HashMap::new();
    for (index, entry) in entries.iter().enumerate() {
        let normalized = normalize(entry.text);
        if normalized.len() < settings.min_length {
            continue;
        }
        let text = Prepared::new(normalized);
        let previous = by_sender.entry(entry.key).or_default();
        // Nur Nachrichten im Zeitfenster behalten.
        while previous.front().is_some_and(|(time, _)| entry.timestamp - time > window_ms) {
            previous.pop_front();
        }
        let similar =
            previous.iter().filter(|(_, earlier)| similarity_of(&text, earlier) >= threshold).count() as u32;
        let count = 1 + similar;
        if count >= settings.min_count {
            counts[index] = count;
        }
        previous.push_back((entry.timestamp, text));
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETTINGS: SpamSettings =
        SpamSettings { enabled: true, similarity: 70.0, min_count: 2, window_minutes: 5.0, min_length: 5 };

    fn entry<'a>(key: &'a str, text: &'a str, minute: f64) -> SpamEntry<'a> {
        SpamEntry { key, timestamp: minute * 60_000.0, text }
    }

    #[test]
    fn schreibung_und_leerraum_egal() {
        assert_eq!(normalize("Kauf  JETZT bei mir").into_iter().collect::<String>(), "kaufjetztbeimir");
        assert!(similarity(&normalize("Kauf jetzt bei mir"), &normalize("KAUF JETZT BEI MIR!!")) > 0.7);
        assert!(similarity(&normalize("Guten Morgen zusammen"), &normalize("Wie läuft das Spiel")) < 0.3);
    }

    #[test]
    fn zaehlt_aehnliche_nachrichten_derselben_person() {
        let entries = [
            entry("k:a", "Folgt meinem Kanal jetzt", 0.0),
            entry("k:b", "Folgt meinem Kanal jetzt", 0.0),
            entry("k:a", "folgt meinem kanal jetzt!", 1.0),
            entry("k:a", "Folgt meinem Kanal jetzt!!", 2.0),
        ];
        assert_eq!(spam_counts(&entries, SETTINGS), vec![0, 0, 2, 3]);
    }

    #[test]
    fn zeitfenster_laenge_und_kanal() {
        let entries = [
            entry("k:a", "Folgt meinem Kanal", 0.0),
            entry("k:a", "Folgt meinem Kanal", 10.0),
            entry("k:a", "hi", 10.0),
            entry("k:a", "hi", 10.0),
            entry("anderer:a", "Folgt meinem Kanal", 10.0),
        ];
        assert_eq!(spam_counts(&entries, SETTINGS), vec![0; 5]);
    }

    #[test]
    fn ausgeschaltet_keine_zaehler() {
        let entries = [entry("k:a", "Folgt meinem Kanal", 0.0), entry("k:a", "Folgt meinem Kanal", 0.0)];
        assert_eq!(spam_counts(&entries, SpamSettings { enabled: false, ..SETTINGS }), vec![0, 0]);
    }

    #[test]
    fn schnell_bei_vollem_puffer() {
        // Voller Puffer (1500 Zeilen in 2,5 Minuten): 300 Personen und eine, die 100-mal fast dasselbe schreibt.
        let keys: Vec<String> = (0..1500).map(|i| if i % 15 == 0 { "k:spam".into() } else { format!("k:{}", i % 300) }).collect();
        let texts: Vec<String> = (0..1500).map(|i| format!("Nachricht Nummer {} mit etwas Text dazu", i % 40)).collect();
        let entries: Vec<SpamEntry<'_>> = (0..1500)
            .map(|i| SpamEntry { key: &keys[i], timestamp: i as f64 * 100.0, text: &texts[i] })
            .collect();
        let start = std::time::Instant::now();
        let counts = spam_counts(&entries, SETTINGS);
        assert!(start.elapsed().as_millis() < 300, "{:?}", start.elapsed());
        assert!(counts[1485] >= 2, "der Spammer wird gezählt");
    }
}
