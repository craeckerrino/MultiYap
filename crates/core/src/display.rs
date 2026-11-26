//! Reine Teile der Darstellung (bisher src/features/display/apply.ts, colors.ts und settings.ts): anpassbare Farben und
//! ihre Werte für die Tokens, Schriftart und Schriftgröße des Chats als CSS-Werte. Das Übertragen auf die Seite liegt
//! in crates/web/src/display.rs.

use crate::i18n::tr;
use serde_json::Value;

/// Anpassbare Farben: Token aus theme.css und Bezeichnung. Fest bleiben bewusst die Chat-Ereignisse (Rosa), die
/// AutoMod-Knöpfe (Grün/Rot) und die Filter (nur ihre sechs festen Farben).
pub fn custom_colors() -> [(&'static str, &'static str); 10] {
    [
    ("mention", tr("Erwähnungen", "Mentions")),
    ("mention-blink", tr("Tab-Blinken bei Erwähnungen", "Tab blinking on mentions")),
    ("first-message", tr("Erste Nachricht", "First message")),
    ("link", "Links"),
    ("restricted", tr("Eingeschränkt", "Restricted")),
    ("monitored", tr("Überwacht", "Monitored")),
    ("mod-action", tr("Mod-Aktionen", "Mod actions")),
    ("spam", tr("Spam-Zähler", "Spam counter")),
    ("live-marker", tr("Linie „Live ab hier“", "“Live from here” line")),
    ("row-alt", tr("Abwechselnde Zeilenhintergründe", "Alternating row backgrounds")),
]
}

/// Nur die Tokens der anpassbaren Farben.
pub fn custom_color_keys() -> Vec<&'static str> {
    custom_colors().iter().map(|(key, _)| *key).collect()
}

/// Wert, den das Token bekommt ({ color: "#rrggbb" oder null, enabled }); None heißt Standard aus theme.css. Aus heißt:
/// keine Farbe, Labels und Hinweise bleiben (neutral grau).
pub fn token_value(key: &str, entry: Option<&Value>) -> Option<String> {
    let entry = entry?;
    if entry.get("enabled").and_then(Value::as_bool) == Some(false) {
        return Some(if key == "row-alt" { "transparent" } else { "var(--muted-foreground)" }.to_string());
    }
    let color = entry.get("color").and_then(Value::as_str)?;
    // Zeilenhintergrund nur leicht getönt, sonst wäre jede zweite Zeile vollflächig gefärbt.
    Some(if key == "row-alt" { format!("color-mix(in oklab, {color} 12%, transparent)") } else { color.to_string() })
}

/// CSS-Wert für font-family: gewählte Schrift, sonst und als Rückfall die Standardschrift.
pub fn chat_font_family_value(family: &str) -> String {
    let name = family.trim();
    if name.is_empty() { "var(--font-sans)".to_string() } else { format!("\"{name}\", var(--font-sans)") }
}

/// Schriftgröße des Chats in rem (Einstellung in Pixeln, 16 px je rem).
pub fn chat_font_size_value(pixels: f64) -> String {
    format!("{}rem", pixels / 16.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn eigene_farbe_standard_und_ausgeschaltet() {
        assert_eq!(token_value("mention", None), None);
        assert_eq!(token_value("mention", Some(&json!({ "color": "#ff0000", "enabled": true }))).as_deref(), Some("#ff0000"));
        assert_eq!(token_value("mention", Some(&json!({ "color": null, "enabled": true }))), None);
        assert_eq!(token_value("link", Some(&json!({ "color": "#ff0000", "enabled": false }))).as_deref(), Some("var(--muted-foreground)"));
    }

    #[test]
    fn zeilenhintergrund_leicht_getoent_aus_ohne_hintergrund() {
        assert_eq!(
            token_value("row-alt", Some(&json!({ "color": "#336699", "enabled": true }))).as_deref(),
            Some("color-mix(in oklab, #336699 12%, transparent)")
        );
        assert_eq!(token_value("row-alt", Some(&json!({ "color": null, "enabled": false }))).as_deref(), Some("transparent"));
    }

    #[test]
    fn schriftart_und_groesse() {
        assert_eq!(chat_font_family_value(""), "var(--font-sans)");
        assert_eq!(chat_font_family_value(" Segoe UI "), "\"Segoe UI\", var(--font-sans)");
        assert_eq!(chat_font_size_value(14.0), "0.875rem");
        assert_eq!(chat_font_size_value(16.0), "1rem");
        assert_eq!(custom_color_keys().len(), 10);
    }
}
