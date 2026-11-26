//! Sprache der Oberfläche: Deutsch oder Englisch. Beim ersten Start wie die Sprache des Systems (Deutsch, sonst
//! Englisch), unter Einstellungen → Allgemein umstellbar (`app.language`).
//!
//! Die Texte stehen direkt im Code, beide Sprachen nebeneinander: `tr("Deutsch", "English")` für feste Texte,
//! `tf!("Deutsch {x}", "English {x}")` für Texte mit Platzhaltern. Ohne Umstellung gilt Deutsch (auch in Tests).
//! Die Sprache gilt für den ganzen Prozess: im Browser-Teil für die Seite, im Programm für dessen Meldungen.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    De,
    En,
}

impl Lang {
    /// Kürzel für `lang` in HTML und in Adressen.
    pub fn code(self) -> &'static str {
        match self {
            Lang::De => "de",
            Lang::En => "en",
        }
    }

    /// Aus einem Kürzel wie „de“ oder „en-US“; alles außer Deutsch ist Englisch.
    pub fn from_code(code: &str) -> Lang {
        let code = code.trim().to_ascii_lowercase();
        if code == "de" || code.starts_with("de-") || code.starts_with("de_") {
            Lang::De
        } else {
            Lang::En
        }
    }

    /// Wert der Einstellung `app.language` („auto“, „de“, „en“) mit der Sprache des Systems für „auto“.
    pub fn from_setting(setting: &str, system: &str) -> Lang {
        match setting {
            "de" => Lang::De,
            "en" => Lang::En,
            _ => Lang::from_code(system),
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set(lang: Lang) {
    CURRENT.store(if lang == Lang::En { 1 } else { 0 }, Ordering::Relaxed);
}

pub fn current() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == 1 {
        Lang::En
    } else {
        Lang::De
    }
}

pub fn is_en() -> bool {
    current() == Lang::En
}

/// Wert für eine bestimmte Sprache.
pub fn pick<T>(lang: Lang, de: T, en: T) -> T {
    match lang {
        Lang::De => de,
        Lang::En => en,
    }
}

/// Fester Text in der aktuellen Sprache.
pub fn tr(de: &'static str, en: &'static str) -> &'static str {
    pick(current(), de, en)
}

/// Text als Paar („Deutsch“, „English“), zum Beispiel in einmal gebauten Schemas oder festen Tabellen.
pub type Text = (&'static str, &'static str);

/// Text aus einer festen Tabelle (`const`), dort als Paar („Deutsch“, „English“) abgelegt.
pub fn t(text: Text) -> &'static str {
    tr(text.0, text.1)
}

/// Gebietsschema für Datums- und Zahlenformate des Browsers.
pub fn locale() -> &'static str {
    tr("de-DE", "en-US")
}

/// Text mit Platzhaltern in der aktuellen Sprache, wie `format!`: `tf!("Version {v} bereit", "Version {v} ready")`.
#[macro_export]
macro_rules! tf {
    ($de:literal, $en:literal $(, $($args:tt)*)?) => {
        if $crate::i18n::is_en() { format!($en $(, $($args)*)?) } else { format!($de $(, $($args)*)?) }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_codes_and_settings() {
        assert_eq!(Lang::from_code("de-DE"), Lang::De);
        assert_eq!(Lang::from_code("DE"), Lang::De);
        assert_eq!(Lang::from_code("en-US"), Lang::En);
        assert_eq!(Lang::from_code("fr"), Lang::En);
        assert_eq!(Lang::from_code("des"), Lang::En);
        assert_eq!(Lang::from_setting("auto", "de-AT"), Lang::De);
        assert_eq!(Lang::from_setting("auto", "en-GB"), Lang::En);
        assert_eq!(Lang::from_setting("en", "de-DE"), Lang::En);
        assert_eq!(Lang::from_setting("de", "en-US"), Lang::De);
    }

    #[test]
    fn picks_text_by_language() {
        // Tests schalten die Sprache nie um (laufen parallel und prüfen deutsche Texte).
        assert_eq!(pick(Lang::En, "Hallo", "Hello"), "Hello");
        assert_eq!(pick(Lang::De, "Hallo", "Hello"), "Hallo");
        let name = "x";
        assert_eq!(tr("Hallo", "Hello"), "Hallo");
        assert_eq!(tf!("Hallo {name}", "Hello {name}"), "Hallo x");
        assert_eq!(tf!("Hallo {}", "Hello {}", name), "Hallo x");
    }
}
