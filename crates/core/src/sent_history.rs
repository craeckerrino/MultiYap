//! Zuletzt gesendete eigene Nachrichten pro Kanal (bisher sent-history.ts): mit Pfeil hoch und runter im Eingabefeld
//! durchgehen. Nur im Arbeitsspeicher bis zum Schließen, nie gespeichert. Enthält auch erfolgreich ausgeführte
//! „/“-Befehle.

use std::collections::HashMap;

pub const SENT_HISTORY_LIMIT: usize = 50;

#[derive(Default)]
pub struct SentHistory {
    lists: HashMap<String, Vec<String>>,
}

impl SentHistory {
    /// Gesendete Nachricht merken: älteste zuerst, gleiche direkt hintereinander nur einmal.
    pub fn remember(&mut self, channel_login: &str, message: &str) {
        let text = message.trim();
        if text.is_empty() {
            return;
        }
        let list = self.lists.entry(channel_login.to_lowercase()).or_default();
        if list.last().map(String::as_str) != Some(text) {
            list.push(text.to_string());
        }
        if list.len() > SENT_HISTORY_LIMIT {
            let excess = list.len() - SENT_HISTORY_LIMIT;
            list.drain(..excess);
        }
    }

    pub fn list(&self, channel_login: &str) -> &[String] {
        self.lists.get(&channel_login.to_lowercase()).map_or(&[], Vec::as_slice)
    }
}

/// Nächste Position im Verlauf. `index` None ist der eigene Entwurf, sonst die Stelle im Verlauf. Pfeil hoch geht
/// zur älteren Nachricht und bleibt bei der ältesten stehen, Pfeil runter zur neueren und hinter der neuesten zurück
/// zum Entwurf. Ergebnis None: keine Änderung; Some(None): zurück zum Entwurf.
pub fn step(len: usize, index: Option<usize>, older: bool) -> Option<Option<usize>> {
    if len == 0 {
        return None;
    }
    if older {
        return match index {
            None => Some(Some(len - 1)),
            Some(i) if i > 0 => Some(Some(i - 1)),
            Some(_) => None,
        };
    }
    let i = index?;
    Some(if i < len - 1 { Some(i + 1) } else { None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pro_kanal_ohne_gross_klein() {
        let mut h = SentHistory::default();
        h.remember("Kanal", "hallo");
        h.remember("kanal", "welt");
        h.remember("anderer", "woanders");
        assert_eq!(h.list("KANAL"), ["hallo", "welt"]);
        assert_eq!(h.list("anderer"), ["woanders"]);
    }

    #[test]
    fn leere_und_wiederholte() {
        let mut h = SentHistory::default();
        for text in ["a", " a ", "   ", "b", "a"] {
            h.remember("kanal", text);
        }
        assert_eq!(h.list("kanal"), ["a", "b", "a"]);
    }

    #[test]
    fn hoechstens_die_neuesten() {
        let mut h = SentHistory::default();
        for i in 0..SENT_HISTORY_LIMIT + 5 {
            h.remember("kanal", &format!("n{i}"));
        }
        let list = h.list("kanal");
        assert_eq!(list.len(), SENT_HISTORY_LIMIT);
        assert_eq!(list[0], "n5");
        assert_eq!(list[list.len() - 1], format!("n{}", SENT_HISTORY_LIMIT + 4));
    }

    #[test]
    fn pfeiltasten() {
        assert_eq!(step(3, None, true), Some(Some(2)));
        assert_eq!(step(3, Some(2), true), Some(Some(1)));
        assert_eq!(step(3, Some(0), true), None);
        assert_eq!(step(3, Some(1), false), Some(Some(2)));
        assert_eq!(step(3, Some(2), false), Some(None));
        assert_eq!(step(3, None, false), None);
        assert_eq!(step(0, None, true), None);
    }
}
