//! Reine Teile des Hervorhebungen-Tabs (bisher src/features/highlights-tab/classify.ts und feed.ts): warum eine Zeile
//! hineingehört, ob sie zur Anzeige-Auswahl eines Tabs passt und ob sie als neuer Treffer zählt. Die Sammlung aus allen
//! offenen Kanälen liegt in crates/web/src/highlights.rs.

use serde::Serialize;
use serde_json::Value;

/// Höchstens so viele Zeilen insgesamt im Hervorhebungen-Tab; die ältesten fallen zuerst heraus.
pub const MAX_FEED_ITEMS: usize = 1000;

fn text<'a>(value: &'a Value, name: &str) -> &'a str {
    value.get(name).and_then(Value::as_str).unwrap_or_default()
}

/// Warum eine Zeile in den Hervorhebungen-Tab gehört.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum HighlightMatch {
    #[serde(rename_all = "camelCase")]
    Message { mention: bool, term_category_id: Option<String> },
    Event,
    ModAction,
    Automod,
}

/// Ordnet eine Zeile zu: Filter-Treffer, Erwähnungen, Chat-Ereignisse, Mod-Aktionen und AutoMod. Alles andere (normale
/// Nachrichten, Hinweise, Trennlinien) gehört nicht hinein. Ob eine Mod-Aktion oder AutoMod überhaupt sichtbar ist,
/// entscheidet wie im Chat die Mod-Prüfung beim Anzeigen. `classify_message` liefert für eine Nachricht (Erwähnung,
/// Filter-Kategorie).
pub fn classify(item: &Value, classify_message: impl FnOnce(&Value) -> (bool, Option<String>)) -> Option<HighlightMatch> {
    match text(item, "kind") {
        "message" => {
            let (mention, term_category_id) = classify_message(item);
            (mention || term_category_id.is_some()).then_some(HighlightMatch::Message { mention, term_category_id })
        }
        "event" => Some(HighlightMatch::Event),
        "mod-action" => Some(HighlightMatch::ModAction),
        "automod" => Some(HighlightMatch::Automod),
        _ => None,
    }
}

/// "all" oder eine Liste, die den Wert enthält.
fn all_or_contains(choice: Option<&Value>, value: &str) -> bool {
    match choice {
        Some(Value::Array(list)) => list.iter().any(|v| v.as_str() == Some(value)),
        _ => true,
    }
}

/// Passt ein Treffer aus diesem Kanal zur Anzeige-Auswahl eines Tabs ({ terms, mentions, events, modActions, automod,
/// channels })?
pub fn shown_in(found: &HighlightMatch, channel_login: &str, show: &Value) -> bool {
    if !all_or_contains(show.get("channels"), channel_login) {
        return false;
    }
    let flag = |name: &str| show.get(name).and_then(Value::as_bool).unwrap_or(true);
    match found {
        HighlightMatch::Message { mention, term_category_id } => {
            (flag("mentions") && *mention) || term_category_id.as_deref().is_some_and(|id| all_or_contains(show.get("terms"), id))
        }
        HighlightMatch::Event => flag("events"),
        HighlightMatch::ModAction => flag("modActions"),
        HighlightMatch::Automod => flag("automod"),
    }
}

/// Zeile aus dem geladenen Verlauf (zählt nicht als neuer Treffer).
pub fn from_history(item: &Value) -> bool {
    match text(item, "kind") {
        "message" => item.get("flags").and_then(|f| f.get("fromHistory")).and_then(Value::as_bool) == Some(true),
        "event" | "mod-action" => item.get("fromHistory").and_then(Value::as_bool) == Some(true),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(text: &str) -> Value {
        json!({ "kind": "message", "id": text, "channel": { "id": "1", "login": "kanal" }, "text": text, "flags": {} })
    }

    fn classify_test(item: &Value) -> Option<HighlightMatch> {
        classify(item, |m| {
            let text = m["text"].as_str().unwrap_or_default();
            (text.contains("@ich"), text.contains("spam").then(|| "kat-spam".to_string()))
        })
    }

    fn show() -> Value {
        json!({ "terms": "all", "mentions": true, "events": true, "modActions": true, "automod": true, "channels": "all" })
    }

    fn with(name: &str, value: Value) -> Value {
        let mut show = show();
        show[name] = value;
        show
    }

    #[test]
    fn sammelt_treffer_ereignisse_mod_aktionen_und_automod_sonst_nichts() {
        assert_eq!(classify_test(&message("hallo")), None);
        assert_eq!(
            classify_test(&message("spam hier")),
            Some(HighlightMatch::Message { mention: false, term_category_id: Some("kat-spam".into()) })
        );
        assert_eq!(
            classify_test(&message("@ich spam")),
            Some(HighlightMatch::Message { mention: true, term_category_id: Some("kat-spam".into()) })
        );
        assert_eq!(classify_test(&json!({ "kind": "event" })), Some(HighlightMatch::Event));
        assert_eq!(classify_test(&json!({ "kind": "mod-action" })), Some(HighlightMatch::ModAction));
        assert_eq!(classify_test(&json!({ "kind": "automod" })), Some(HighlightMatch::Automod));
        assert_eq!(classify_test(&json!({ "kind": "divider", "variant": "live" })), None);
    }

    #[test]
    fn anzeigen_kategorien_erwaehnungen_und_kanaele_einzeln_waehlbar() {
        let spam = classify_test(&message("spam")).unwrap();
        let mention = classify_test(&message("@ich")).unwrap();
        assert!(shown_in(&spam, "kanal", &show()));
        assert!(!shown_in(&spam, "kanal", &with("terms", json!([]))));
        assert!(shown_in(&spam, "kanal", &with("terms", json!(["kat-spam"]))));
        assert!(!shown_in(&mention, "kanal", &with("mentions", json!(false))));
        assert!(!shown_in(&spam, "kanal", &with("channels", json!(["anderer"]))));
        assert!(shown_in(&spam, "kanal", &with("channels", json!(["kanal"]))));
        assert!(!shown_in(&HighlightMatch::Event, "kanal", &with("events", json!(false))));
        assert!(!shown_in(&HighlightMatch::ModAction, "kanal", &with("modActions", json!(false))));
        assert!(!shown_in(&HighlightMatch::Automod, "kanal", &with("automod", json!(false))));
    }

    #[test]
    fn verlauf_zaehlt_nicht_als_neu() {
        assert!(from_history(&json!({ "kind": "message", "flags": { "fromHistory": true } })));
        assert!(!from_history(&message("neu")));
        assert!(from_history(&json!({ "kind": "event", "fromHistory": true })));
        assert!(from_history(&json!({ "kind": "mod-action", "fromHistory": true })));
        assert!(!from_history(&json!({ "kind": "automod" })));
    }

    #[test]
    fn treffer_fuer_typescript() {
        let found = HighlightMatch::Message { mention: true, term_category_id: None };
        assert_eq!(serde_json::to_value(found).unwrap(), json!({ "kind": "message", "mention": true, "termCategoryId": null }));
        assert_eq!(serde_json::to_value(HighlightMatch::ModAction).unwrap(), json!({ "kind": "mod-action" }));
    }
}
