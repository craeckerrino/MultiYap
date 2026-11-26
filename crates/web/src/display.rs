//! Darstellung auf die Seite übertragen (bisher src/features/display/apply.ts und colors.ts): Schriftgröße und
//! Schriftart des Chats als CSS-Variablen, Zeilenabstand, Trennstriche und Zeilenhintergründe als Attribute am
//! html-Element, eigene Farben als Tokens am html-Element. Die Chat-Zeilen lesen sie über das Designsystem (theme.css),
//! Änderungen gelten sofort. Werte in crates/core/src/display.rs.

use std::cell::Cell;
use std::rc::Rc;

use multiyap_core::display::{chat_font_family_value, chat_font_size_value, token_value, custom_colors};
use serde_json::Value;

use crate::settings;

const DISPLAY_KEYS: [&str; 5] = ["display.fontSize", "display.chatFont", "display.density", "display.separators", "display.zebra"];
const COLORS: &str = "display.colors";

fn root() -> Option<web_sys::HtmlElement> {
    use wasm_bindgen::JsCast;
    web_sys::window()?.document()?.document_element()?.dyn_into::<web_sys::HtmlElement>().ok()
}

fn setting(key: &str) -> Value {
    settings::get(key).unwrap_or(Value::Null)
}

fn apply_display() {
    let Some(root) = root() else { return };
    let style = root.style();
    let dataset = root.dataset();
    let _ = style.set_property("--chat-font-size", &chat_font_size_value(setting("display.fontSize").as_f64().unwrap_or(14.0)));
    let _ = style.set_property("--chat-font-family", &chat_font_family_value(setting("display.chatFont").as_str().unwrap_or_default()));
    let _ = dataset.set("chatDensity", setting("display.density").as_str().unwrap_or("normal"));
    let separators = setting("display.separators");
    if separators.get("enabled").and_then(Value::as_bool) == Some(true) {
        let thickness = separators.get("thickness").and_then(Value::as_f64).unwrap_or(1.0);
        let _ = dataset.set("chatSeparators", &thickness.to_string());
    } else {
        dataset.delete("chatSeparators");
    }
    if setting("display.zebra").as_bool() == Some(true) {
        let _ = dataset.set("chatZebra", "");
    } else {
        dataset.delete("chatZebra");
    }
}

fn apply_colors() {
    let Some(root) = root() else { return };
    let style = root.style();
    let colors = setting(COLORS);
    for (key, _) in custom_colors() {
        let property = format!("--{key}");
        match token_value(key, colors.get(key)) {
            Some(value) => {
                let _ = style.set_property(&property, &value);
            }
            None => {
                let _ = style.remove_property(&property);
            }
        }
    }
}

thread_local! {
    static STARTED: Cell<bool> = const { Cell::new(false) };
}

/// Überträgt die Darstellung und beobachtet ihre Einstellungen (einmal).
pub fn start() {
    if STARTED.with(|s| s.replace(true)) {
        return;
    }
    for key in DISPLAY_KEYS {
        settings::subscribe(key, Rc::new(apply_display));
    }
    settings::subscribe(COLORS, Rc::new(apply_colors));
    apply_display();
    apply_colors();
}

