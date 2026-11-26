//! 7TV-Abzeichen und Namensfarben über die EventAPI v3. Prüfen, Umwandeln und Speicher in `crates/core/src/seventv.rs`.
//! Es wird nur empfangen und abonniert, nie etwas an 7TV gemeldet. Die Verbindung besteht nur, solange Badges oder
//! Namensfarben eingeschaltet und Kanäle offen sind.
//!
//! In crates/web: eine Verbindung für die ganze App, die der Einstellung „thirdParty.seventv“ selbst folgt. Änderungen
//! werden gebündelt höchstens einmal pro Bild gemeldet.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use multiyap_core::seventv::{self as st, op, Cosmetics};
use js_sys::{Reflect, JSON};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::WebSocket;

use crate::net::global_function;

struct State {
    store: Cosmetics,
    enabled: bool,
    channels: Vec<String>,
    subscribed: HashSet<String>,
    /// 7TV-Emotes eingeschaltet: Änderungen an den Emote-Sets der Kanäle abonnieren.
    emotes_enabled: bool,
    /// Emote-Set-ID → Twitch-Kanal-ID.
    emote_sets: HashMap<String, String>,
    subscribed_sets: HashSet<String>,
    socket: Option<(u32, WebSocket)>,
    next_socket: u32,
    heartbeat_timer: Option<JsValue>,
    reconnect_timer: Option<JsValue>,
    heartbeat_interval_ms: f64,
    backoff_ms: f64,
}

struct Inner {
    state: RefCell<State>,
}

fn get(value: &JsValue, key: &str) -> JsValue {
    Reflect::get(value, &JsValue::from_str(key)).unwrap_or(JsValue::UNDEFINED)
}

fn set_timeout(ms: f64, callback: impl FnOnce() + 'static) -> Option<JsValue> {
    let mut callback = Some(callback);
    let function = Closure::<dyn FnMut()>::new(move || {
        if let Some(callback) = callback.take() {
            callback();
        }
    })
    .into_js_value();
    global_function("setTimeout").ok()?.call2(&JsValue::NULL, &function, &JsValue::from_f64(ms)).ok()
}

fn clear_timeout(handle: Option<JsValue>) {
    if let (Some(handle), Ok(clear)) = (handle, global_function("clearTimeout")) {
        let _ = clear.call1(&JsValue::NULL, &handle);
    }
}

impl Inner {
    fn wanted(&self) -> bool {
        let state = self.state.borrow();
        (state.enabled && !state.channels.is_empty()) || (state.emotes_enabled && !state.emote_sets.is_empty())
    }

    fn changed(&self) {
        schedule_emit();
    }
}

/// Sendet die Abos eines Kanals, sobald die Verbindung offen ist.
fn subscribe_channel(inner: &Inner, channel_id: &str) {
    let socket = {
        let mut state = inner.state.borrow_mut();
        let Some((_, socket)) = state.socket.as_ref().filter(|(_, s)| s.ready_state() == WebSocket::OPEN).cloned() else {
            return;
        };
        if !state.enabled || !state.subscribed.insert(channel_id.to_string()) {
            return;
        }
        socket
    };
    for message in st::subscribe_messages(channel_id) {
        let _ = socket.send_with_str(&message);
    }
}

/// Sendet das Abo auf Änderungen eines Emote-Sets, sobald die Verbindung offen ist.
fn subscribe_set(inner: &Inner, set_id: &str) {
    let socket = {
        let mut state = inner.state.borrow_mut();
        let Some((_, socket)) = state.socket.as_ref().filter(|(_, s)| s.ready_state() == WebSocket::OPEN).cloned() else {
            return;
        };
        if !state.emotes_enabled || !state.subscribed_sets.insert(set_id.to_string()) {
            return;
        }
        socket
    };
    let _ = socket.send_with_str(&st::emote_set_subscribe_message(set_id));
}

fn subscribe_all(inner: &Inner) {
    let (channels, sets) = {
        let state = inner.state.borrow();
        (state.channels.clone(), state.emote_sets.keys().cloned().collect::<Vec<_>>())
    };
    for channel in channels {
        subscribe_channel(inner, &channel);
    }
    for set in sets {
        subscribe_set(inner, &set);
    }
}

/// Änderung am Emote-Set eines Kanals: Liste anpassen und im Chat melden.
fn emote_set_changed(inner: &Inner, body: &serde_json::Value) {
    let Some(change) = st::parse_emote_set_update(body).filter(|c| !c.is_empty()) else { return };
    let Some(channel_id) = inner.state.borrow().emote_sets.get(&change.set_id).cloned() else { return };
    crate::emotes::apply_seventv_change(&channel_id, &change);
    let by = change.actor.as_ref().map(|a| multiyap_core::tf!(" (von {a})", " (by {a})")).unwrap_or_default();
    for emote in &change.added {
        crate::sessions::channel_notice(Some(&channel_id), "seventv-added", &format!("{}{by}", st::emote_name(emote)));
    }
    for emote in &change.removed {
        crate::sessions::channel_notice(Some(&channel_id), "seventv-removed", &format!("{}{by}", st::emote_name(emote)));
    }
    for (old, new) in &change.renamed {
        let (old, new) = (st::emote_name(old), st::emote_name(new));
        if old != new {
            crate::sessions::channel_notice(Some(&channel_id), "seventv-renamed", &multiyap_core::tf!("{old} heißt jetzt {new}{by}", "{old} is now called {new}{by}"));
        }
    }
}

/// Bleibt der Herzschlag aus, gilt die Verbindung als tot und wird neu aufgebaut.
fn arm_heartbeat(inner: &Rc<Inner>) {
    let (old, ms) = {
        let mut state = inner.state.borrow_mut();
        (state.heartbeat_timer.take(), state.heartbeat_interval_ms * 2.0)
    };
    clear_timeout(old);
    let weak = Rc::downgrade(inner);
    let timer = set_timeout(ms, move || {
        let socket = weak.upgrade().and_then(|inner| inner.state.borrow().socket.as_ref().map(|(_, s)| s.clone()));
        if let Some(socket) = socket {
            let _ = socket.close();
        }
    });
    inner.state.borrow_mut().heartbeat_timer = timer;
}

fn handle_message(inner: &Rc<Inner>, data: &str) {
    let Ok(message) = JSON::parse(data) else { return };
    arm_heartbeat(inner);
    let payload = get(&message, "d");
    match get(&message, "op").as_f64().map(|o| o as i64) {
        Some(op::HELLO) => {
            {
                let mut state = inner.state.borrow_mut();
                state.backoff_ms = 1000.0;
                if let Some(interval) = get(&payload, "heartbeat_interval").as_f64().filter(|i| i.is_finite() && *i > 0.0) {
                    state.heartbeat_interval_ms = interval;
                }
            }
            subscribe_all(inner);
        }
        Some(op::DISPATCH) => {
            let Some(kind) = get(&payload, "type").as_string() else { return };
            if kind == "emote_set.update" {
                if let Ok(body) = serde_wasm_bindgen::from_value::<serde_json::Value>(get(&payload, "body")) {
                    emote_set_changed(inner, &body);
                }
                return;
            }
            let object = get(&get(&payload, "body"), "object");
            if !object.is_object() {
                return;
            }
            let Ok(object) = serde_wasm_bindgen::from_value::<serde_json::Value>(object) else { return };
            let changed = inner.state.borrow_mut().store.handle_dispatch(&kind, &object);
            if changed {
                inner.changed();
            }
        }
        Some(op::RECONNECT) | Some(op::END_OF_STREAM) => {
            let socket = inner.state.borrow().socket.as_ref().map(|(_, s)| s.clone());
            if let Some(socket) = socket {
                let _ = socket.close();
            }
        }
        _ => {}
    }
}

fn connect(inner: &Rc<Inner>) {
    if inner.state.borrow().socket.is_some() || !inner.wanted() {
        return;
    }
    let Ok(socket) = WebSocket::new(st::EVENTS_URL) else { return };
    let id = {
        let mut state = inner.state.borrow_mut();
        state.next_socket += 1;
        state.socket = Some((state.next_socket, socket.clone()));
        state.subscribed.clear();
        state.subscribed_sets.clear();
        state.next_socket
    };
    let weak: Weak<Inner> = Rc::downgrade(inner);
    let on_message = Closure::<dyn FnMut(JsValue)>::new(move |event: JsValue| {
        let data = get(&event, "data").as_string();
        if let (Some(inner), Some(data)) = (weak.upgrade(), data) {
            if inner.state.borrow().socket.as_ref().map(|(sid, _)| *sid) == Some(id) {
                handle_message(&inner, &data);
            }
        }
    })
    .into_js_value();
    let weak: Weak<Inner> = Rc::downgrade(inner);
    let on_close = Closure::<dyn FnMut()>::new(move || {
        let Some(inner) = weak.upgrade() else { return };
        let (current, heartbeat) = {
            let mut state = inner.state.borrow_mut();
            let current = state.socket.as_ref().map(|(sid, _)| *sid) == Some(id);
            if current {
                state.socket = None;
                state.subscribed.clear();
                state.subscribed_sets.clear();
            }
            (current, if current { state.heartbeat_timer.take() } else { None })
        };
        clear_timeout(heartbeat);
        if !current || !inner.wanted() {
            return;
        }
        // Erneut verbinden mit wachsender Wartezeit.
        let delay = {
            let mut state = inner.state.borrow_mut();
            let delay = state.backoff_ms;
            state.backoff_ms = (state.backoff_ms * 2.0).min(st::MAX_BACKOFF_MS);
            delay
        };
        let weak = Rc::downgrade(&inner);
        let timer = set_timeout(delay, move || {
            if let Some(inner) = weak.upgrade() {
                inner.state.borrow_mut().reconnect_timer = None;
                connect(&inner);
            }
        });
        inner.state.borrow_mut().reconnect_timer = timer;
    })
    .into_js_value();
    socket.set_onmessage(Some(on_message.unchecked_ref()));
    socket.set_onclose(Some(on_close.unchecked_ref()));
}

fn disconnect(inner: &Rc<Inner>) {
    let (timer, heartbeat, socket) = {
        let mut state = inner.state.borrow_mut();
        state.subscribed.clear();
        state.subscribed_sets.clear();
        (state.reconnect_timer.take(), state.heartbeat_timer.take(), state.socket.take())
    };
    clear_timeout(timer);
    clear_timeout(heartbeat);
    if let Some((_, socket)) = socket {
        let _ = socket.close();
    }
}

fn sync(inner: &Rc<Inner>) {
    if inner.wanted() {
        connect(inner);
        subscribe_all(inner);
    } else {
        disconnect(inner);
    }
    inner.changed();
}

// ---- Meldungen (gebündelt, höchstens einmal pro Bild) ----

type Listener = Rc<dyn Fn()>;

#[derive(Default)]
struct Notify {
    version: u32,
    scheduled: bool,
    listeners: Vec<(u32, Listener)>,
    next: u32,
    watching: bool,
}

thread_local! {
    static INNER: Rc<Inner> = Rc::new(Inner {
        state: RefCell::new(State {
            store: Cosmetics::default(),
            enabled: false,
            channels: Vec::new(),
            subscribed: HashSet::new(),
            emotes_enabled: false,
            emote_sets: HashMap::new(),
            subscribed_sets: HashSet::new(),
            socket: None,
            next_socket: 0,
            heartbeat_timer: None,
            reconnect_timer: None,
            heartbeat_interval_ms: 45_000.0,
            backoff_ms: 1000.0,
        }),
    });
    static NOTIFY: RefCell<Notify> = RefCell::new(Notify::default());
}

fn inner() -> Rc<Inner> {
    INNER.with(Rc::clone)
}

fn schedule_emit() {
    if NOTIFY.with(|n| std::mem::replace(&mut n.borrow_mut().scheduled, true)) {
        return;
    }
    let run = Closure::once_into_js(|| {
        let listeners: Vec<Listener> = NOTIFY.with(|n| {
            let mut n = n.borrow_mut();
            n.scheduled = false;
            n.version = n.version.wrapping_add(1);
            n.listeners.iter().map(|(_, l)| l.clone()).collect()
        });
        for listener in listeners {
            listener();
        }
    });
    // Viele Meldungen kurz hintereinander: nur einmal pro Bild neu zeichnen (ohne requestAnimationFrame nach 16 ms).
    match global_function("requestAnimationFrame") {
        Ok(raf) => {
            let _ = raf.call1(&JsValue::NULL, &run);
        }
        Err(_) => {
            if let Ok(set_timeout) = global_function("setTimeout") {
                let _ = set_timeout.call2(&JsValue::NULL, &run, &JsValue::from_f64(16.0));
            }
        }
    }
}

/// Einstellung „thirdParty.seventv“: Verbindung nur, solange Badges, Namensfarben oder (für Änderungen an den
/// Emote-Sets) Emotes an sind.
fn flag(name: &str) -> bool {
    crate::settings::get("thirdParty.seventv").and_then(|v| v.get(name).and_then(serde_json::Value::as_bool)).unwrap_or(true)
}

fn setting() -> (bool, bool) {
    (flag("badges"), flag("paints"))
}

fn apply_setting() {
    let (badges, paints) = setting();
    let inner = inner();
    {
        let mut state = inner.state.borrow_mut();
        state.enabled = badges || paints;
        state.emotes_enabled = flag("emotes");
    }
    sync(&inner);
}

fn watch_setting() {
    if !NOTIFY.with(|n| std::mem::replace(&mut n.borrow_mut().watching, true)) {
        crate::settings::subscribe("thirdParty.seventv", Rc::new(apply_setting));
        crate::settings::subscribe(PAINT_SHADOWS, Rc::new(apply_setting));
        apply_setting();
    }
}

/// Emote-Set eines Kanals melden (nach dem Laden der 7TV-Emotes): Änderungen daran werden abonniert und im Chat
/// gemeldet.
pub fn watch_emote_set(channel_id: &str, set_id: &str) {
    watch_setting();
    let inner = inner();
    let added = inner.state.borrow_mut().emote_sets.insert(set_id.to_string(), channel_id.to_string()).is_none();
    if added {
        sync(&inner);
    }
}

/// Kanal anmelden (beim Beitreten); folgt ab dem ersten Kanal der Einstellung.
pub fn ensure_channel(channel_id: &str) {
    watch_setting();
    let inner = inner();
    {
        let mut state = inner.state.borrow_mut();
        if !state.channels.iter().any(|c| c == channel_id) {
            state.channels.push(channel_id.to_string());
        }
    }
    sync(&inner);
}

/// 7TV-Abzeichen einer Person (nur wenn Badges an sind).
pub fn badge(twitch_user_id: &str) -> Option<st::SevenTvBadge> {
    if !setting().0 {
        return None;
    }
    inner().state.borrow().store.badge(twitch_user_id).cloned()
}

/// Einstellung: Schatten der Namensfarben zeichnen. Aus (Standard) wirken die Namen schärfer, weil der Schatten als
/// weicher Schein um die kleine Schrift liegt.
const PAINT_SHADOWS: &str = "thirdParty.seventvPaintShadows";

/// 7TV-Namensfarbe einer Person (nur wenn Namensfarben an sind), auf Wunsch aufgehellt.
pub fn paint(twitch_user_id: &str, readable: bool) -> Option<st::SevenTvPaint> {
    if !setting().1 {
        return None;
    }
    let mut paint = inner().state.borrow_mut().store.paint(twitch_user_id, readable, crate::dark_theme())?;
    if !crate::settings::get(PAINT_SHADOWS).and_then(|v| v.as_bool()).unwrap_or(false) {
        paint.style.filter = None;
    }
    Some(paint)
}

pub fn version() -> u32 {
    NOTIFY.with(|n| n.borrow().version)
}

pub fn subscribe(listener: Listener) -> u32 {
    NOTIFY.with(|n| {
        let mut n = n.borrow_mut();
        n.next += 1;
        let id = n.next;
        n.listeners.push((id, listener));
        id
    })
}

pub fn unsubscribe(id: u32) {
    NOTIFY.with(|n| n.borrow_mut().listeners.retain(|(i, _)| *i != id));
}

// ---- Für TypeScript (bis die übrigen Teile in Rust sind) ----


thread_local! {
    /// 7TV-Profile je Twitch-ID für die Sitzung (None: kein 7TV-Konto). Fehler werden nicht gemerkt.
    static PROFILES: RefCell<HashMap<String, Option<String>>> = RefCell::new(HashMap::new());
}

/// Adresse des 7TV-Profils einer Person (Nutzerkarte). None: kein 7TV-Konto oder 7TV nicht erreichbar. Ist 7TV in den
/// Einstellungen ganz aus (Emotes, Abzeichen und Namensfarben), fragt MultiYap 7TV auch hier nicht an.
pub async fn profile_url(twitch_id: &str) -> Option<String> {
    let enabled = crate::settings::get("thirdParty.seventv")
        .and_then(|v| v.as_object().map(|o| o.values().any(|on| on.as_bool() == Some(true))))
        .unwrap_or(true);
    if !enabled {
        return None;
    }
    if let Some(known) = PROFILES.with(|p| p.borrow().get(twitch_id).cloned()) {
        return known;
    }
    let url = format!("https://7tv.io/v3/users/twitch/{}", multiyap_core::twitch::url_component(twitch_id));
    let answer = crate::net::fetch_json(&url).await.ok()?;
    let profile = st::profile_url(answer.as_ref());
    PROFILES.with(|p| p.borrow_mut().insert(twitch_id.to_string(), profile.clone()));
    profile
}
