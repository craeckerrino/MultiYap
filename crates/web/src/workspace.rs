//! Arbeitsfläche: Tabs und Layout (bisher core/workspace/tabs-store.ts). Jede Änderung wird erst geprüft und
//! gespeichert und danach angezeigt, damit Anzeige und Speicher nie auseinanderlaufen. Tabs und Layout ändern sich
//! immer gemeinsam in einer Transaktion (ein Wert). Änderungen laufen nacheinander ab. Zuletzt geschlossene Tabs gibt
//! es nur für diese Sitzung.

use multiyap_core::i18n::tr;
use std::cell::RefCell;
use std::rc::Rc;

use multiyap_core::persistence::WORKSPACE;
use multiyap_core::workspace::{self as layout, ClosedTab, Target, WorkspaceState};
use futures::lock::Mutex;
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

use crate::db::main_database;

/// Schlüssel des Layouts im Speicher „workspace“.
const LAYOUT_KEY: &str = "layout";
/// Höchstens so viele zuletzt geschlossene Tabs.
const MAX_CLOSED: usize = 20;

type Listener = Rc<dyn Fn()>;

#[derive(Default)]
struct Store {
    state: WorkspaceState,
    loaded: bool,
    closed: Vec<ClosedTab>,
    listeners: Vec<(u32, Listener)>,
    next_listener: u32,
}

thread_local! {
    static STORE: RefCell<Store> = RefCell::new(Store::default());
    /// Änderungen nacheinander: Lesen, Berechnen und Schreiben ohne Überholen.
    static QUEUE: Rc<Mutex<()>> = Rc::new(Mutex::new(()));
}

fn emit() {
    let listeners: Vec<Listener> = STORE.with(|s| s.borrow().listeners.iter().map(|(_, l)| l.clone()).collect());
    for listener in listeners {
        listener();
    }
}

fn console_warn(text: &str) {
    web_sys_warn(text);
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = warn)]
    fn web_sys_warn(message: &str);
}

fn new_id() -> String {
    web_sys::window().and_then(|w| w.crypto().ok()).map(|c| c.random_uuid()).unwrap_or_default()
}

/// Gespeicherter Stand, geprüft; bei Fehlern der leere Stand (beschädigte Daten werden nicht überschrieben, solange
/// nichts geändert wird).
async fn read_stored() -> WorkspaceState {
    let Some(db) = main_database() else { return WorkspaceState::default() };
    match db.read_json(WORKSPACE, LAYOUT_KEY).await {
        Ok(Some(value)) => match multiyap_core::stored::named("workspace").map(|schema| schema.parse(&value)) {
            Some(Ok(parsed)) => serde_json::from_value(parsed).unwrap_or_default(),
            _ => {
                console_warn("Gespeichertes Layout ist ungültig; es bleibt unverändert, bis etwas geändert wird.");
                WorkspaceState::default()
            }
        },
        Ok(None) => WorkspaceState::default(),
        Err(_) => {
            console_warn("Layout konnte nicht gelesen werden.");
            WorkspaceState::default()
        }
    }
}

/// Lädt den Stand und folgt Änderungen aus anderen Browser-Tabs.
pub async fn init() {
    let state = read_stored().await;
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        s.state = state;
        s.loaded = true;
    });
    emit();
    if let Some(db) = main_database() {
        let _ = db.watch(|stores, remote| {
            if !remote || !stores.iter().any(|s| s == WORKSPACE) {
                return;
            }
            wasm_bindgen_futures::spawn_local(async {
                let queue = QUEUE.with(Rc::clone);
                let _guard = queue.lock().await;
                let state = read_stored().await;
                STORE.with(|s| s.borrow_mut().state = state);
                emit();
            });
        });
    }
}

pub fn state() -> WorkspaceState {
    STORE.with(|s| s.borrow().state.clone())
}

pub fn loaded() -> bool {
    STORE.with(|s| s.borrow().loaded)
}

pub fn has_closed() -> bool {
    STORE.with(|s| !s.borrow().closed.is_empty())
}

/// Meldet jede Änderung; gibt die Nummer zum Abmelden zurück.
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

/// Führt eine Änderung aus: berechnen, prüfen, speichern, anzeigen. Ohne Änderung passiert nichts.
async fn update(change: impl FnOnce(&WorkspaceState, &mut Vec<ClosedTab>) -> Option<WorkspaceState>) -> Result<(), String> {
    let queue = QUEUE.with(Rc::clone);
    let _guard = queue.lock().await;
    let Some(db) = main_database() else { return Ok(()) };
    let (current, mut closed) = STORE.with(|s| {
        let s = s.borrow();
        (s.state.clone(), s.closed.clone())
    });
    let Some(next) = change(&current, &mut closed) else { return Ok(()) };
    if next == current {
        return Ok(());
    }
    let value = serde_json::to_value(&next).map_err(|e| e.to_string())?;
    let schema = multiyap_core::stored::named("workspace").ok_or("Schema fehlt")?;
    schema.parse(&value).map_err(|_| tr("Das Layout ist ungültig und wurde nicht gespeichert.", "The layout is invalid and was not saved.").to_string())?;
    db.write_json(WORKSPACE, LAYOUT_KEY, &value).await.map_err(|_| tr("Die Änderung an Tabs oder Layout konnte nicht gespeichert werden.", "The change to tabs or layout could not be saved.").to_string())?;
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        s.state = next;
        s.closed = closed;
    });
    emit();
    Ok(())
}

/// Höchstens 50 UTF-16-Einheiten wie `slice(0, 50)` im Browser, ohne ein Zeichen zu zerteilen.
fn limit_name(text: &str) -> String {
    let mut units = 0;
    text.chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= 50
        })
        .collect()
}

fn channel_tab(kind: &str, channel: &str) -> (String, Value) {
    let id = new_id();
    (id.clone(), json!({ "id": id, "kind": kind, "platform": "twitch", "channel": channel.to_lowercase(), "nickname": null }))
}

/// Ziel eines neuen Inhalts: mit Bereich rechts daneben auf dessen Seite, ohne Bereich eine neue Seite.
fn target(area: Option<String>) -> Target {
    area.map_or(Target::NewPage, |area| Target::Split { area, side: "right".into() })
}

pub async fn add_channel_tab(channel: &str, area: Option<String>) -> Result<(), String> {
    let (_, tab) = channel_tab("channel", channel);
    update(|state, _| Some(layout::add_tab(state, tab, &target(area), &mut new_id))).await
}

/// Neuer YouTube-Tab: `channel` ist der Schlüssel aus `multiyap_core::youtube::channel_key` („@name“ oder „UC…“).
pub async fn add_youtube_tab(channel: &str, area: Option<String>) -> Result<(), String> {
    let id = new_id();
    let tab = json!({ "id": id, "kind": "channel", "platform": multiyap_core::youtube::PLATFORM, "channel": channel, "nickname": null });
    update(|state, _| Some(layout::add_tab(state, tab, &target(area), &mut new_id))).await
}

/// Neuer Stream-Tab mit dem Twitch-Player. Gibt die ID des Tabs zurück.
pub async fn add_stream_tab(channel: &str, area: Option<String>) -> Result<String, String> {
    let (id, tab) = channel_tab("stream", channel);
    update(|state, _| Some(layout::add_tab(state, tab, &target(area), &mut new_id))).await?;
    Ok(id)
}

/// „Player öffnen“: Stream-Tab in einem neuen Bereich über dem Chat (frei weiter verschiebbar wie jeder Tab).
pub async fn open_player_above(channel: &str, area: String) -> Result<(), String> {
    let (_, tab) = channel_tab("stream", channel);
    update(|state, _| Some(layout::add_tab(state, tab, &Target::Split { area, side: "top".into() }, &mut new_id))).await
}

/// Neuer Hervorhebungen-Tab: zeigt alles, Zähler an.
pub async fn add_highlights_tab(area: Option<String>) -> Result<(), String> {
    let tab = json!({
        "id": new_id(),
        "kind": "highlights",
        "nickname": null,
        "show": { "terms": "all", "mentions": true, "events": true, "modActions": true, "automod": true, "channels": "all" },
        "counter": true,
    });
    update(|state, _| Some(layout::add_tab(state, tab, &target(area), &mut new_id))).await
}

/// Ändert einen Tab; ohne passenden Tab keine Änderung.
fn update_tab(state: &WorkspaceState, id: &str, change: impl Fn(&mut serde_json::Map<String, Value>)) -> Option<WorkspaceState> {
    if !state.tabs.iter().any(|t| t.get("id").and_then(Value::as_str) == Some(id)) {
        return None;
    }
    let tabs = state
        .tabs
        .iter()
        .map(|tab| {
            let mut tab = tab.clone();
            if tab.get("id").and_then(Value::as_str) == Some(id) {
                if let Some(object) = tab.as_object_mut() {
                    change(object);
                }
            }
            tab
        })
        .collect();
    Some(WorkspaceState { tabs, ..state.clone() })
}

/// Anzeige-Auswahl oder Zähler eines Hervorhebungen-Tabs (`patch` mit show und/oder counter).
pub async fn update_highlights_tab(id: &str, patch: Value) -> Result<(), String> {
    update(|state, _| {
        update_tab(state, id, |tab| {
            if tab.get("kind").and_then(Value::as_str) != Some("highlights") {
                return;
            }
            for key in ["show", "counter"] {
                if let Some(value) = patch.get(key) {
                    tab.insert(key.into(), value.clone());
                }
            }
        })
    })
    .await
}

pub async fn activate_tab(id: &str) -> Result<(), String> {
    update(|state, _| layout::activate_tab(state, id)).await
}

pub async fn focus_area(area: &str) -> Result<(), String> {
    update(|state, _| layout::focus_area(state, area)).await
}

pub async fn close_tab(id: &str) -> Result<(), String> {
    update(|state, closed| {
        let (next, info) = layout::close_tab(state, id)?;
        closed.push(info);
        if closed.len() > MAX_CLOSED {
            closed.remove(0);
        }
        Some(next)
    })
    .await
}

pub async fn restore_last_closed_tab() -> Result<(), String> {
    update(|state, closed| {
        let entry = closed.pop()?;
        Some(layout::restore_tab(state, &entry, &mut new_id))
    })
    .await
}

pub async fn rename_tab(id: &str, nickname: Option<String>) -> Result<(), String> {
    let value = nickname.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).map(|n| limit_name(&n));
    update(|state, _| update_tab(state, id, |tab| {
        tab.insert("nickname".into(), value.clone().map_or(Value::Null, Value::String));
    }))
    .await
}

/// Kanal wechseln: Tab bleibt an seinem Platz, eigener Anzeigename entfällt.
pub async fn change_tab_channel(id: &str, channel: &str) -> Result<(), String> {
    let login = channel.to_lowercase();
    update(|state, _| {
        update_tab(state, id, |tab| {
            if tab.get("kind").and_then(Value::as_str) == Some("channel") && tab.get("platform").and_then(Value::as_str) == Some("twitch") {
                tab.insert("channel".into(), Value::String(login.clone()));
                tab.insert("nickname".into(), Value::Null);
            }
        })
    })
    .await
}

pub async fn activate_page(id: &str) -> Result<(), String> {
    update(|state, _| layout::activate_page(state, id)).await
}

/// Schließt eine Seite mit allen Inhalten (wiederherstellbar wie ein Tab).
pub async fn close_page(id: &str) -> Result<(), String> {
    update(|state, closed| {
        let (next, info) = layout::close_page(state, id)?;
        closed.push(info);
        if closed.len() > MAX_CLOSED {
            closed.remove(0);
        }
        Some(next)
    })
    .await
}

/// Eigener Name einer Seite; leer: die Namen der Inhalte.
pub async fn rename_page(id: &str, name: Option<String>) -> Result<(), String> {
    let value = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).map(|n| limit_name(&n));
    update(|state, _| layout::rename_page(state, id, value.clone())).await
}

/// Seite in der Leiste an eine Position schieben.
pub async fn move_page(id: &str, index: usize) -> Result<(), String> {
    update(|state, _| layout::move_page(state, id, index)).await
}

/// Inhalt auf eine andere Seite (None: auf eine neue Seite).
pub async fn move_tab_to_page(id: &str, page: Option<String>) -> Result<(), String> {
    update(|state, _| layout::move_tab_to_page(state, id, page.as_deref(), &mut new_id)).await
}

pub async fn split_with_tab(id: &str, area: &str, side: &str) -> Result<(), String> {
    update(|state, _| layout::split_with_tab(state, id, area, side, &mut new_id)).await
}

pub async fn set_split_ratio(split: &str, ratio: f64) -> Result<(), String> {
    update(|state, _| layout::set_ratio(state, split, ratio)).await
}

// ---- Exporte für TypeScript ----

