//! Updates aus dem lokalen Versionsordner (Einstellung „updates.folder“): der lokale Server prüft den Ordner
//! (crates/server/src/update.rs); installiert wird über `/update-install?…` (src-tauri/src/main.rs).

use multiyap_core::i18n::tr;
use std::cell::RefCell;
use std::rc::Rc;

use multiyap_core::updates as core_updates;
use serde_json::Value;
use wasm_bindgen_futures::spawn_local;

use crate::net::fetch_json;

const FOLDER: &str = "updates.folder";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

type Listener = Rc<dyn Fn()>;

#[derive(Default)]
struct State {
    started: bool,
    checking: bool,
    /// Neuere Version im Ordner.
    available: Option<String>,
    error: Option<String>,
    /// Laufende Prüfung; ältere Ergebnisse werden verworfen.
    run: u32,
    /// Zuletzt per Meldung angekündigte Version (jede nur einmal).
    announced: Option<String>,
    listeners: Vec<(u32, Listener)>,
    next: u32,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn emit() {
    let listeners: Vec<Listener> = STATE.with(|s| s.borrow().listeners.iter().map(|(_, l)| l.clone()).collect());
    for listener in listeners {
        listener();
    }
}

fn folder() -> String {
    crate::settings::get(FOLDER).and_then(|v| v.as_str().map(|s| s.trim().to_string())).unwrap_or_default()
}

fn encode(text: &str) -> String {
    js_sys::encode_uri_component(text).as_string().unwrap_or_default()
}

/// Beim Start einmal prüfen und bei jeder Änderung des Ordners erneut.
pub fn start() {
    if STATE.with(|s| std::mem::replace(&mut s.borrow_mut().started, true)) {
        return;
    }
    crate::settings::subscribe(FOLDER, Rc::new(check));
    check();
}

/// Prüft den Ordner (auch über „Jetzt prüfen“). Ohne Ordner wird der Stand geleert.
pub fn check() {
    let folder = folder();
    let enabled = !folder.is_empty();
    let run = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.run = s.run.wrapping_add(1);
        s.checking = enabled;
        if !enabled {
            s.available = None;
            s.error = None;
        }
        s.run
    });
    emit();
    if !enabled {
        return;
    }
    spawn_local(async move {
        let (available, error) = match fetch_json(&format!("/api/update?folder={}&lang={}", encode(&folder), crate::language::code())).await {
            Ok(Some(json)) => (
                json.get("version").and_then(Value::as_str).map(str::to_string),
                json.get("error").and_then(Value::as_str).map(str::to_string),
            ),
            Ok(None) | Err(_) => (None, Some(tr("Prüfung nicht möglich.", "Check not possible.").to_string())),
        };
        finish(run, available, error);
    });
}

/// Übernimmt ein Ergebnis, kündigt eine neue Version einmal an.
fn finish(run: u32, available: Option<String>, error: Option<String>) {
    let announce = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.run != run {
            return None;
        }
        s.checking = false;
        s.available = available.filter(|v| core_updates::is_newer(v, CURRENT_VERSION));
        s.error = error;
        let new = s.available.clone().filter(|v| s.announced.as_ref() != Some(v));
        if new.is_some() {
            s.announced = new.clone();
        }
        new
    });
    if let Some(version) = announce {
        crate::toasts::show("info", &multiyap_core::tf!("Update auf Version {version} bereit. Installieren über den Knopf oben rechts.", "Update to version {version} is ready. Install it with the button at the top right."), Some("update-available"));
    }
    emit();
}

pub fn available() -> Option<String> {
    STATE.with(|s| s.borrow().available.clone())
}

/// Fehler der Prüfung, zum Anzeigen in den Einstellungen.
pub fn error() -> Option<String> {
    STATE.with(|s| s.borrow().error.clone())
}

pub fn checking() -> bool {
    STATE.with(|s| s.borrow().checking)
}

/// Startet die Installation: das Programmfenster prüft erneut, startet den Installer und beendet MultiYap.
pub fn install() {
    let Some(version) = available() else { return };
    let url = format!("/update-install?folder={}&version={}&lang={}", encode(&folder()), encode(&version), crate::language::code());
    if let Some(window) = web_sys::window() {
        let _ = window.open_with_url_and_target_and_features(&url, "_blank", "noopener,noreferrer");
    }
}

pub fn subscribe(listener: Listener) -> u32 {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.next += 1;
        let id = s.next;
        s.listeners.push((id, listener));
        id
    })
}

pub fn unsubscribe(id: u32) {
    STATE.with(|s| s.borrow_mut().listeners.retain(|(i, _)| *i != id));
}
