//! Dienste im Browser in Rust: die IndexedDB-Schicht und die Speicher, die bisher TypeScript hielt. Die Oberfläche
//! (crates/ui) und die Exporte für TypeScript (crates/wasm) nutzen sie gemeinsam.

pub mod api;
pub mod app_commands;
pub mod auth;
pub mod backup;
pub mod chat_tools;
pub mod chatters;
pub mod db;
pub mod display;
pub mod emotes;
pub mod eventsub;
pub mod highlights;
pub mod hotkeys;
pub mod irc_reader;
pub mod language;
pub mod mod_actions;
pub mod moderation;
pub mod name_colors;
pub mod net;
pub mod notifications;
pub mod sessions;
pub mod settings;
pub mod startup;
pub mod seventv;
pub mod streams;
pub mod toasts;
pub mod updates;
pub mod workspace;
pub mod youtube;

/// Bilder für das dunkle Design (Standard) oder das helle: Klasse „dark“ am Wurzelelement; ohne Dokument dunkel.
pub fn dark_theme() -> bool {
    use wasm_bindgen::JsCast;
    let get = |target: &wasm_bindgen::JsValue, key: &str| js_sys::Reflect::get(target, &key.into()).ok().filter(|v| v.is_object());
    let Some(root) = get(&js_sys::global(), "document").and_then(|d| get(&d, "documentElement")) else { return true };
    let Some(list) = get(&root, "classList") else { return true };
    js_sys::Reflect::get(&list, &"contains".into())
        .ok()
        .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
        .and_then(|f| f.call1(&list, &"dark".into()).ok())
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}
