//! Programmstart (In Rust, vorher src/start.tsx und src/core/persistence/status.ts): Datenbank öffnen, dauerhaften
//! Speicher erbitten, dann Anmeldung und Tabs laden und die Dienste starten. Der Stand des Speichers (öffnet, bereit,
//! blockiert, veraltet, neuere Daten, Fehler) steuert die Hinweise der Oberfläche.

use multiyap_core::i18n::tr;
use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Function, Promise, Reflect};
use serde::Serialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use crate::db;

type Listener = Rc<dyn Fn()>;

/// Stand des Speichers.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum PersistenceStatus {
    Opening,
    Ready { persistent: bool },
    /// Ein anderer Tab mit älterer Version hält die Datenbank offen.
    Blocked,
    /// Ein anderer Tab hat eine neuere Version geöffnet; dieses Fenster muss neu laden.
    Outdated,
    /// Die gespeicherten Daten stammen von einer neueren Version.
    Newer,
    Error { message: String },
}

struct State {
    status: PersistenceStatus,
    started: bool,
    listeners: Vec<(u32, Listener)>,
    next_listener: u32,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State { status: PersistenceStatus::Opening, started: false, listeners: Vec::new(), next_listener: 0 });
}

fn set_status(status: PersistenceStatus) {
    let listeners: Vec<Listener> = STATE.with(|s| {
        let mut state = s.borrow_mut();
        state.status = status;
        state.listeners.iter().map(|(_, l)| l.clone()).collect()
    });
    for listener in listeners {
        listener();
    }
}

pub fn persistence_status() -> PersistenceStatus {
    STATE.with(|s| s.borrow().status.clone())
}

pub fn subscribe_persistence(listener: Listener) -> u32 {
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        state.next_listener += 1;
        let id = state.next_listener;
        state.listeners.push((id, listener));
        id
    })
}

pub fn unsubscribe_persistence(id: u32) {
    STATE.with(|s| s.borrow_mut().listeners.retain(|(i, _)| *i != id));
}

fn get(value: &JsValue, key: &str) -> JsValue {
    Reflect::get(value, &JsValue::from_str(key)).unwrap_or(JsValue::UNDEFINED)
}

/// Ruft `target[method]()` auf und wartet auf die Zusage; None, wenn es die Methode nicht gibt oder sie fehlschlägt.
async fn call_async(target: &JsValue, method: &str) -> Option<JsValue> {
    let function: Function = get(target, method).dyn_into().ok()?;
    let promise: Promise = function.call0(target).ok()?.dyn_into().ok()?;
    JsFuture::from(promise).await.ok()
}

/// Bittet den Browser, die Daten dauerhaft zu behalten und nicht bei Speicherknappheit zu löschen. Ergebnis: Speicher
/// gilt als dauerhaft.
async fn request_persistent_storage() -> bool {
    let storage = get(&get(&js_sys::global(), "navigator"), "storage");
    if storage.is_undefined() || storage.is_null() {
        return false;
    }
    if call_async(&storage, "persisted").await.and_then(|v| v.as_bool()) == Some(true) {
        return true;
    }
    call_async(&storage, "persist").await.and_then(|v| v.as_bool()).unwrap_or(false)
}

fn error_message(error: &JsValue) -> String {
    get(error, "message").as_string().filter(|m| !m.is_empty()).or_else(|| error.as_string()).unwrap_or_else(|| tr("Unbekannter Fehler.", "Unknown error.").to_string())
}

/// Startet alles einmal. `redirect`: ein schon erkannter Rücksprung von Twitch (Adresse und erwarteter Zustand), den
/// der Einstieg vor dem Laden von WebAssembly aus der Adresse entfernt hat.
pub async fn start(redirect: Option<(String, Option<String>)>) {
    if STATE.with(|s| std::mem::replace(&mut s.borrow_mut().started, true)) {
        return;
    }
    let connection = match db::open_app(|| set_status(PersistenceStatus::Blocked)).await {
        Ok(connection) => connection,
        Err(error) => {
            if get(&error, "name").as_string().as_deref() == Some("VersionError") {
                set_status(PersistenceStatus::Newer);
            } else {
                set_status(PersistenceStatus::Error { message: error_message(&error) });
            }
            return;
        }
    };
    let on_outdated = Closure::<dyn FnMut()>::new(|| set_status(PersistenceStatus::Outdated)).into_js_value();
    connection.on_outdated(on_outdated.unchecked_into());
    db::set_main_database(&connection);
    let persistent = request_persistent_storage().await;
    set_status(PersistenceStatus::Ready { persistent });

    futures::join!(crate::auth::init(redirect), crate::workspace::init());
    crate::moderation::start();
    crate::streams::start();
    crate::sessions::start();
    crate::youtube::start();
    crate::notifications::start();
    crate::display::start();
    crate::mod_actions::start_shortcuts();
    crate::hotkeys::start();
    crate::highlights::start();
}
