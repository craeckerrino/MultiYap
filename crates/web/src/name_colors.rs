//! Namensfarben für Namen in Aktionszeilen (Mod-Aktionen, Chat-Ereignisse, AutoMod): Dort steht nur, wer etwas getan
//! hat, ohne Farbe. Bekannt sind die Farben aus den Chatnachrichten der Sitzung; fehlt eine, wird sie einmal über „Get
//! User Chat Color“ nachgefragt (bis zu 100 Personen je Anfrage, kurz gesammelt). Nur im Arbeitsspeicher, nichts wird
//! gespeichert.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use wasm_bindgen_futures::spawn_local;

/// Kurz sammeln, damit viele Namen einer Liste in einer Anfrage landen.
const FETCH_DELAY_MS: f64 = 100.0;
/// Höchstens so viele Personen je Anfrage (Grenze von Twitch).
const BATCH: usize = 100;

type Listener = Rc<dyn Fn()>;

#[derive(Default)]
struct Store {
    /// Twitch-Nutzer-ID → Farbe ("" heißt: keine eigene Farbe gewählt, es gilt die Standardfarbe).
    known: HashMap<String, String>,
    /// Gefragt, aber noch nicht bekannt.
    waiting: HashSet<String>,
    fetch_scheduled: bool,
    listeners: Vec<(u32, Listener)>,
    next_listener: u32,
}

thread_local! {
    static STORE: RefCell<Store> = RefCell::new(Store::default());
}

fn emit() {
    let listeners: Vec<Listener> = STORE.with(|s| s.borrow().listeners.iter().map(|(_, l)| l.clone()).collect());
    for listener in listeners {
        listener();
    }
}

/// Farbe aus einer Chatnachricht merken. Meldet nur, wenn jemand auf genau diese Person gewartet hat, damit nicht jede
/// Nachricht alle Namen neu zeichnet.
pub fn remember(user_id: &str, color: &str) {
    if user_id.is_empty() {
        return;
    }
    let waited = STORE.with(|s| {
        let mut s = s.borrow_mut();
        if s.known.get(user_id).map(String::as_str) == Some(color) {
            return false;
        }
        s.known.insert(user_id.to_string(), color.to_string());
        s.waiting.remove(user_id)
    });
    if waited {
        emit();
    }
}

/// Bekannte Farbe: None heißt unbekannt, Some("") keine eigene Farbe.
pub fn color(user_id: &str) -> Option<String> {
    STORE.with(|s| s.borrow().known.get(user_id).cloned())
}

/// Fragt eine unbekannte Farbe nach (gebündelt). Danach meldet `subscribe`.
pub fn request(user_id: &str) {
    if user_id.is_empty() {
        return;
    }
    let schedule = STORE.with(|s| {
        let mut s = s.borrow_mut();
        if s.known.contains_key(user_id) || !s.waiting.insert(user_id.to_string()) {
            return false;
        }
        !std::mem::replace(&mut s.fetch_scheduled, true)
    });
    if schedule {
        spawn_local(fetch());
    }
}

async fn fetch() {
    crate::net::sleep(FETCH_DELAY_MS).await;
    let ids: Vec<String> = STORE.with(|s| {
        let mut s = s.borrow_mut();
        s.fetch_scheduled = false;
        s.waiting.iter().cloned().collect()
    });
    for chunk in ids.chunks(BATCH) {
        let found = crate::api::get_chat_colors(chunk).await.unwrap_or_default();
        STORE.with(|s| {
            let mut s = s.borrow_mut();
            for (id, color) in found {
                s.known.insert(id, color);
            }
            // Ohne Antwort (unbekannt oder Fehler): Standardfarbe, nicht immer wieder fragen.
            for id in chunk {
                s.waiting.remove(id);
                s.known.entry(id.clone()).or_default();
            }
        });
    }
    if !ids.is_empty() {
        emit();
    }
}

/// Meldet neu bekannte Farben; gibt die Nummer zum Abmelden zurück.
pub fn subscribe(listener: Listener) -> u32 {
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        s.next_listener += 1;
        let id = s.next_listener;
        s.listeners.push((id, listener));
        id
    })
}

pub fn unsubscribe(id: u32) {
    STORE.with(|s| s.borrow_mut().listeners.retain(|(i, _)| *i != id));
}
