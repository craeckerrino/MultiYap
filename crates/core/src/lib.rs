//! Fachlogik von MultiYap ohne Oberfläche.
//!
//! Reines Rust ohne Browser-Abhängigkeiten, damit es nativ getestet werden kann. Im Browser kommt es über
//! `crates/wasm` als WebAssembly an. Verhalten wie die bisherigen TypeScript-Module, die Tests sind mitübertragen.

pub mod autofill;
pub mod backup;
pub mod channel_input;
pub mod chat_buffer;
pub mod chat_modes;
pub mod chat_tools;
pub mod chat_text;
pub mod chatters;
pub mod color;
pub mod commands;
pub mod community_gifts;
pub mod display;
pub mod emotes;
pub mod hotkeys;
pub mod highlights;
pub mod i18n;
pub mod history;
pub mod links;
pub mod mentions;
pub mod modules;
pub mod net;
pub mod notify;
pub mod persistence;
pub mod render;
pub mod roles;
pub mod schema;
pub mod sent_history;
pub mod search;
pub mod settings;
pub mod shortcuts;
pub mod snapping;
pub mod seventv;
pub mod model;
pub mod spam;
pub mod stored;
pub mod streams;
pub mod terms;
pub mod time;
pub mod twitch;
pub mod updates;
pub mod workspace;
pub mod youtube;

/// Sonderzeichen für reguläre Ausdrücke maskieren (wie `escapeRegex` im TypeScript-Code).
pub(crate) fn escape(text: &str) -> String {
    regex::escape(text)
}

/// Leerraum in Phrasen flexibel: ein oder mehrere Leerzeichen zählen gleich (wie `\s+` im TypeScript-Code).
pub(crate) fn flexible_whitespace(escaped: &str) -> String {
    let mut out = String::with_capacity(escaped.len());
    let mut in_space = false;
    for char in escaped.chars() {
        if char.is_whitespace() {
            if !in_space {
                out.push_str(r"\s+");
                in_space = true;
            }
        } else {
            out.push(char);
            in_space = false;
        }
    }
    out
}

/// Ganze Wörter: davor und danach kein Buchstabe, keine Ziffer, kein Unterstrich. Rusts regex kennt keine
/// Rückschau, deshalb werden die Grenzzeichen mit verbraucht; für „kommt vor“ ist das gleichwertig.
pub(crate) fn whole_words(alternatives: &str) -> String {
    format!(r"(?i)(?:^|[^\p{{L}}\p{{N}}_])(?:{alternatives})(?:$|[^\p{{L}}\p{{N}}_])")
}
