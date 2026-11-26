//! Allgemeine Tastenkürzel (bisher src/features/hotkeys/hotkeys.ts). Ein `keydown`-Listener am Fenster. Gilt überall,
//! auch im Eingabefeld, außer bei einfachen Zeichentasten ohne Strg oder Alt (die sind dort zum Tippen da). Kombinationen
//! und Belegung in crates/core/src/hotkeys.rs.

use multiyap_core::i18n::tr;
use std::cell::Cell;
use std::rc::Rc;

use multiyap_core::hotkeys::{combo_from_event, parse_combo, hotkey_actions};
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::chat_tools::{toast, toast_with_id};
use crate::{app_commands, emotes, sessions, settings};

const RELOAD_TOAST: &str = "hotkeys-reload-emotes";

/// Tippen in einem Feld.
fn is_typing(event: &web_sys::KeyboardEvent) -> bool {
    event.target().and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok()).is_some_and(|target| {
        target.is_content_editable() || target.closest("input, textarea, select").ok().flatten().is_some()
    })
}

/// Beim Wählen einer Taste im Einstellungsfenster nichts auslösen.
fn capturing_key() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.active_element())
        .and_then(|e| e.closest("[data-key-capture=\"listening\"]").ok().flatten())
        .is_some()
}

fn reconnect_all() {
    let open: Vec<Rc<sessions::Session>> = sessions::logins().iter().filter_map(|login| sessions::session(login)).collect();
    if open.is_empty() {
        toast("info", tr("Kein Kanal offen.", "No channel open."));
        return;
    }
    for session in &open {
        session.reconnect();
    }
    toast("success", &if open.len() == 1 { tr("Kanal verbindet neu.", "Channel is reconnecting.").to_string() } else { multiyap_core::tf!("{} Kanäle verbinden neu.", "{} channels are reconnecting.", open.len()) });
}

async fn reload_emotes() {
    toast_with_id("loading", tr("Emotes werden neu geladen …", "Reloading emotes …"), RELOAD_TOAST);
    emotes::reset_twitch_emotes();
    let failed = emotes::reload_third_party().await;
    if failed == 0 {
        toast_with_id("success", tr("Emotes neu geladen.", "Emotes reloaded."), RELOAD_TOAST);
    } else {
        let plural = if failed == 1 { "" } else { tr("n", "s") };
        toast_with_id("warning", &multiyap_core::tf!("Emotes neu geladen, {failed} Abfrage{plural} fehlgeschlagen (alte Liste bleibt).", "Emotes reloaded, {failed} request{plural} failed (the old list stays)."), RELOAD_TOAST);
    }
}

fn run(action: &str) {
    match action {
        "reconnectAll" => reconnect_all(),
        "reloadEmotes" => spawn_local(reload_emotes()),
        "newTab" | "openSettings" => {
            app_commands::run(action);
        }
        "toggleLayoutLock" => spawn_local(async {
            let locked = settings::get("layout.locked").and_then(|v| v.as_bool()).unwrap_or(false);
            if settings::save("layout.locked", &json!(!locked)).await.is_err() {
                toast("error", tr("Die Layout-Sperre konnte nicht gespeichert werden.", "The layout lock could not be saved."));
            }
        }),
        _ => {}
    }
}

fn handle(event: web_sys::KeyboardEvent) {
    if event.repeat() || capturing_key() {
        return;
    }
    let Some(combo) = combo_from_event(&event.key(), event.ctrl_key(), event.alt_key(), event.shift_key(), event.meta_key()) else { return };
    let hotkeys = settings::get("hotkeys.general").unwrap_or(Value::Null);
    let Some((action, _)) = hotkey_actions().into_iter().find(|(id, _)| hotkeys.get(*id).and_then(Value::as_str) == Some(combo.as_str())) else {
        return;
    };
    let (modifiers, key) = parse_combo(&combo);
    let plain_char = key.chars().count() == 1 || key == "Space";
    if plain_char && !modifiers.iter().any(|m| matches!(*m, "Ctrl" | "Alt" | "Meta")) && is_typing(&event) {
        return;
    }
    event.prevent_default();
    run(action);
}

thread_local! {
    static STARTED: Cell<bool> = const { Cell::new(false) };
}

/// Startet die allgemeinen Tastenkürzel (einmal).
pub fn start() {
    if STARTED.with(|s| s.replace(true)) {
        return;
    }
    let Some(window) = web_sys::window() else { return };
    // Gleich laden, damit schon der erste Tastendruck die gespeicherte Belegung kennt.
    let _ = settings::get("hotkeys.general");
    let listener = Closure::<dyn Fn(web_sys::KeyboardEvent)>::new(handle).into_js_value();
    let _ = window.add_event_listener_with_callback("keydown", listener.unchecked_ref());
}

