//! EventSub-Verbindung über WebSocket. Abo-Arten, Wartezeiten und Meldungen in `crates/core/src/twitch/eventsub.rs`.
//!
//! Ablauf wie bisher in TypeScript: eine Verbindung für alle Kanäle, Abos je Kanal nacheinander abgleichen, Keepalive
//! überwachen, Reconnect-Anweisungen befolgen, bei Twitch-Limits still und mit langer Wartezeit neu versuchen.
//! Während JavaScript aufgerufen wird (Zuhörer, Schließen einer Verbindung), ist kein Zustand ausgeliehen, weil diese
//! Aufrufe sofort zurück in den Client führen können.
//!
//! In crates/web: Es gibt genau eine Verbindung für die ganze App (`client()`). Zuhörer sind `ChannelListener` in Rust
//! (für die Kanal-Sitzungen).

use multiyap_core::i18n::tr;
use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::rc::{Rc, Weak};

use multiyap_core::twitch::eventsub::{self as es, ChannelStatus, Group};
use js_sys::{Reflect, JSON};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::WebSocket;

use crate::net::{global_function, helix_call, helix_user_id};

type Task = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()>>>>;

/// Zuhörer eines Kanals: Ereignisse (Abo-Art und das Ereignis von Twitch) und Zustände der Verbindung.
pub trait ChannelListener {
    fn on_event(&self, subscription_type: &str, event: &JsValue);
    fn on_status(&self, status: &ChannelStatus);
}

pub type ListenerRef = Rc<dyn ChannelListener>;

type EntryRef = Rc<RefCell<Entry>>;

struct Entry {
    broadcaster_id: String,
    listeners: Vec<(u32, ListenerRef)>,
    /// Mod-Ereignisse gewünscht (zentrale Mod-Prüfung).
    moderator: bool,
    /// Angelegte Abos: Art und Abo-ID. Gelten nur für `session_id`.
    subscriptions: Vec<(String, String)>,
    session_id: Option<String>,
    /// „Kanal beigetreten“ für diese Session schon gemeldet.
    joined_session: Option<String>,
    /// Abo-Änderungen eines Kanals laufen nacheinander.
    tasks: VecDeque<Task>,
    running: bool,
    /// Optionale Abos, die in dieser Session abgelehnt wurden (nicht erneut versuchen).
    optional_failed: HashSet<String>,
    /// Neuer Versuch nach Ablehnung wegen eines Limits (429).
    limit_timer: Option<JsValue>,
    limit_attempts: usize,
    /// Schon gemeldete Limit-Ablehnungen, damit nicht jeder neue Versuch eine Zeile im Chat erzeugt.
    limit_reported: HashSet<Group>,
}

impl Entry {
    fn has(&self, name: &str) -> bool {
        self.subscriptions.iter().any(|(n, _)| n == name)
    }

    fn set(&mut self, name: &str, id: String) {
        self.subscriptions.retain(|(n, _)| n != name);
        self.subscriptions.push((name.to_string(), id));
    }
}

struct State {
    socket: Option<(u32, WebSocket)>,
    /// Verbindung im Aufbau nach session_reconnect.
    migrating: Option<(u32, WebSocket)>,
    next_socket: u32,
    next_listener: u32,
    session_id: Option<String>,
    keepalive_ms: f64,
    keepalive_timer: Option<JsValue>,
    retry_timer: Option<JsValue>,
    retry_count: usize,
    had_disconnect: bool,
    /// Grund der letzten Trennung, für „Erneut verbunden“.
    disconnect_reason: Option<String>,
    /// In der aktuellen Session wurde ein Abo wegen eines Limits abgelehnt.
    session_limited: bool,
    /// Verbindungen hintereinander, die Twitch nach Limit-Ablehnungen als ungenutzt geschlossen hat.
    limit_streak: usize,
    /// Stiller Neuaufbau nach einem Limit: keine Zeilen „Verbindung getrennt“, „Verbinde…“, „Erneut verbunden“.
    quiet_reconnect: bool,
    channels: Vec<EntryRef>,
}

struct Inner {
    state: RefCell<State>,
}

// ---- Hilfen für JavaScript ----

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

fn get(value: &JsValue, key: &str) -> JsValue {
    Reflect::get(value, &JsValue::from_str(key)).unwrap_or(JsValue::UNDEFINED)
}

fn close_socket(socket: &WebSocket, code: u16) {
    let _ = socket.close_with_code(code);
}

fn notify(entry: &EntryRef, status: &ChannelStatus) {
    let listeners: Vec<ListenerRef> = entry.borrow().listeners.iter().map(|(_, l)| l.clone()).collect();
    for listener in listeners {
        listener.on_status(status);
    }
}

/// Abo bei Twitch entfernen; Fehler sind egal (mit der Verbindung endet es ohnehin).
async fn delete_subscription(id: String) {
    let _ = helix_call("/eventsub/subscriptions", "DELETE", &[("id".into(), vec![id])], None).await;
}

/// Helix-Fehler: Status (falls von Twitch) und Meldung.
fn failure_of(error: &JsValue) -> (Option<u16>, String) {
    let status = get(error, "helixStatus").as_f64().map(|s| s as u16);
    let message = get(error, "message").as_string().unwrap_or_default();
    (status, message)
}

// ---- Client ----

impl Inner {
    fn entry(&self, broadcaster_id: &str) -> Option<EntryRef> {
        self.state.borrow().channels.iter().find(|e| e.borrow().broadcaster_id == broadcaster_id).cloned()
    }

    fn is_current(&self, entry: &EntryRef) -> bool {
        self.state.borrow().channels.iter().any(|e| Rc::ptr_eq(e, entry))
    }

    fn notify_all(&self, status: &ChannelStatus) {
        let channels = self.state.borrow().channels.clone();
        for entry in &channels {
            notify(entry, status);
        }
    }

    fn clear_keepalive(&self) {
        let timer = self.state.borrow_mut().keepalive_timer.take();
        clear_timeout(timer);
    }
}

fn enqueue(entry: &EntryRef, task: Task) {
    let start = {
        let mut e = entry.borrow_mut();
        e.tasks.push_back(task);
        !std::mem::replace(&mut e.running, true)
    };
    if start {
        let entry = entry.clone();
        spawn_local(async move {
            loop {
                let next = {
                    let mut e = entry.borrow_mut();
                    let next = e.tasks.pop_front();
                    if next.is_none() {
                        e.running = false;
                    }
                    next
                };
                let Some(task) = next else { break };
                task().await;
            }
        });
    }
}

/// Gleicht die Abos eines Kanals mit dem Wunschzustand ab (nacheinander je Kanal).
fn sync(inner: &Rc<Inner>, entry: &EntryRef) {
    let inner = inner.clone();
    let task_entry = entry.clone();
    enqueue(entry, Box::new(move || Box::pin(sync_now(inner, task_entry))));
}

fn ensure_socket(inner: &Rc<Inner>) {
    {
        let state = inner.state.borrow();
        if state.socket.is_some() || state.retry_timer.is_some() || state.channels.is_empty() {
            return;
        }
    }
    open(inner, es::EVENTSUB_URL, false);
}

fn open(inner: &Rc<Inner>, url: &str, is_migration: bool) {
    let Ok(socket) = WebSocket::new(url) else {
        schedule_retry(inner, Some(tr("Verbindung konnte nicht aufgebaut werden", "Connection could not be established").into()));
        return;
    };
    let id = {
        let mut state = inner.state.borrow_mut();
        state.next_socket += 1;
        let id = state.next_socket;
        if is_migration {
            state.migrating = Some((id, socket.clone()));
        } else {
            state.socket = Some((id, socket.clone()));
        }
        id
    };
    let weak: Weak<Inner> = Rc::downgrade(inner);
    let on_message = Closure::<dyn FnMut(JsValue)>::new(move |event: JsValue| {
        if let (Some(inner), Some(data)) = (weak.upgrade(), get(&event, "data").as_string()) {
            handle_message(&inner, id, &data);
        }
    })
    .into_js_value();
    let weak: Weak<Inner> = Rc::downgrade(inner);
    let on_close = Closure::<dyn FnMut(JsValue)>::new(move |event: JsValue| {
        if let Some(inner) = weak.upgrade() {
            let code = get(&event, "code").as_f64().unwrap_or(0.0) as u16;
            handle_close(&inner, id, code);
        }
    })
    .into_js_value();
    socket.set_onmessage(Some(on_message.unchecked_ref()));
    socket.set_onclose(Some(on_close.unchecked_ref()));
}

fn close_sockets(inner: &Rc<Inner>) {
    inner.clear_keepalive();
    let (sockets, retry) = {
        let mut state = inner.state.borrow_mut();
        state.session_id = None;
        ([state.socket.take(), state.migrating.take()], state.retry_timer.take())
    };
    clear_timeout(retry);
    for (_, socket) in sockets.into_iter().flatten() {
        close_socket(&socket, 1000);
    }
}

fn reset_keepalive(inner: &Rc<Inner>) {
    inner.clear_keepalive();
    let ms = inner.state.borrow().keepalive_ms + 5_000.0;
    let weak = Rc::downgrade(inner);
    // Kein Ereignis und kein Keepalive innerhalb der Frist: Verbindung gilt als verloren.
    let timer = set_timeout(ms, move || {
        let socket = weak.upgrade().and_then(|inner| inner.state.borrow().socket.as_ref().map(|(_, s)| s.clone()));
        if let Some(socket) = socket {
            close_socket(&socket, 4005);
        }
    });
    inner.state.borrow_mut().keepalive_timer = timer;
}

fn entry_for_subscription(inner: &Inner, subscription: &JsValue) -> Option<EntryRef> {
    let broadcaster_id = get(&get(subscription, "condition"), "broadcaster_user_id").as_string()?;
    inner.entry(&broadcaster_id)
}

fn handle_message(inner: &Rc<Inner>, socket_id: u32, data: &str) {
    let Ok(message) = JSON::parse(data) else { return };
    let Some(message_type) = get(&get(&message, "metadata"), "message_type").as_string() else { return };
    let payload = get(&message, "payload");

    if message_type == "session_welcome" {
        let session = get(&payload, "session");
        if session.is_object() {
            handle_welcome(inner, socket_id, &session);
        }
        return;
    }
    if inner.state.borrow().socket.as_ref().map(|(id, _)| *id) != Some(socket_id) {
        return;
    }
    reset_keepalive(inner);

    let subscription = get(&payload, "subscription");
    match message_type.as_str() {
        "session_reconnect" => {
            if let Some(url) = get(&get(&payload, "session"), "reconnect_url").as_string().filter(|u| !u.is_empty()) {
                inner.notify_all(&ChannelStatus::ReconnectRequested);
                open(inner, &url, true);
            }
        }
        "notification" => {
            let event = get(&payload, "event");
            if !subscription.is_object() || !event.is_object() {
                return;
            }
            let Some(entry) = entry_for_subscription(inner, &subscription) else { return };
            let kind = get(&subscription, "type").as_string().unwrap_or_default();
            let listeners: Vec<ListenerRef> = entry.borrow().listeners.iter().map(|(_, l)| l.clone()).collect();
            for listener in listeners {
                listener.on_event(&kind, &event);
            }
        }
        "revocation" => {
            if !subscription.is_object() {
                return;
            }
            let Some(entry) = entry_for_subscription(inner, &subscription) else { return };
            let kind = get(&subscription, "type").as_string().unwrap_or_default();
            entry.borrow_mut().subscriptions.retain(|(n, _)| *n != kind);
            if kind == "channel.chat.message" {
                entry.borrow_mut().joined_session = None;
                let status = get(&subscription, "status").as_string().unwrap_or_default();
                notify(&entry, &ChannelStatus::Disconnected { reason: Some(multiyap_core::tf!("Abo von Twitch beendet ({status})", "Subscription ended by Twitch ({status})")) });
            }
        }
        _ => {}
    }
}

fn handle_welcome(inner: &Rc<Inner>, socket_id: u32, session: &JsValue) {
    let session_id = get(session, "id").as_string().unwrap_or_default();
    let keepalive_seconds = get(session, "keepalive_timeout_seconds").as_f64().unwrap_or(10.0);
    inner.state.borrow_mut().keepalive_ms = keepalive_seconds * 1000.0;

    let is_migration = inner.state.borrow().migrating.as_ref().map(|(id, _)| *id) == Some(socket_id);
    if is_migration {
        // Wiederverbindung auf Anweisung von Twitch: Abos ziehen mit um.
        let old = {
            let mut state = inner.state.borrow_mut();
            let old = state.socket.take();
            let old_session = state.session_id.take();
            state.socket = state.migrating.take();
            state.session_id = Some(session_id.clone());
            for entry in &state.channels {
                let mut e = entry.borrow_mut();
                if e.session_id == old_session {
                    e.session_id = Some(session_id.clone());
                }
                if e.joined_session == old_session {
                    e.joined_session = Some(session_id.clone());
                }
            }
            old
        };
        if let Some((_, socket)) = old {
            close_socket(&socket, 1000);
        }
        reset_keepalive(inner);
        return;
    }
    if inner.state.borrow().socket.as_ref().map(|(id, _)| *id) != Some(socket_id) {
        return;
    }

    let (quiet, had_disconnect, reason) = {
        let mut state = inner.state.borrow_mut();
        state.session_id = Some(session_id);
        state.session_limited = false;
        state.retry_count = 0;
        (state.quiet_reconnect, state.had_disconnect, state.disconnect_reason.take())
    };
    reset_keepalive(inner);
    if !quiet {
        inner.notify_all(&if had_disconnect { ChannelStatus::Reconnected { reason } } else { ChannelStatus::Connected });
    }
    let channels = {
        let mut state = inner.state.borrow_mut();
        state.quiet_reconnect = false;
        state.had_disconnect = false;
        state.channels.clone()
    };
    for entry in &channels {
        sync(inner, entry);
    }
}

fn handle_close(inner: &Rc<Inner>, socket_id: u32, code: u16) {
    let limited_delay = {
        let mut state = inner.state.borrow_mut();
        if state.migrating.as_ref().map(|(id, _)| *id) == Some(socket_id) {
            state.migrating = None;
            return;
        }
        if state.socket.as_ref().map(|(id, _)| *id) != Some(socket_id) {
            return;
        }
        state.socket = None;
        state.session_id = None;
        // Mit der Verbindung enden bei Twitch auch ihre Abos.
        for entry in &state.channels {
            let mut e = entry.borrow_mut();
            e.session_id = None;
            e.joined_session = None;
            e.subscriptions.clear();
        }
        if state.channels.is_empty() {
            None
        } else if code == 4003 && state.session_limited {
            // Twitch schließt eine Verbindung ohne aktive Abos nach 10 Sekunden (4003). Kam das von einer
            // Limit-Ablehnung, still und mit langer Wartezeit neu versuchen, statt alle paar Sekunden neu zu
            // verbinden und zu melden.
            state.quiet_reconnect = true;
            let delay = es::limit_retry_delay(state.limit_streak);
            state.limit_streak += 1;
            Some(Some(delay))
        } else {
            Some(None)
        }
    };
    inner.clear_keepalive();
    match limited_delay {
        None => {}
        Some(Some(delay)) => {
            let weak = Rc::downgrade(inner);
            let timer = set_timeout(delay, move || {
                if let Some(inner) = weak.upgrade() {
                    inner.state.borrow_mut().retry_timer = None;
                    ensure_socket(&inner);
                }
            });
            inner.state.borrow_mut().retry_timer = timer;
        }
        Some(None) => schedule_retry(inner, Some(es::close_reason_text(code))),
    }
}

fn schedule_limit_retry(inner: &Rc<Inner>, entry: &EntryRef) {
    let delay = {
        let mut e = entry.borrow_mut();
        if e.limit_timer.is_some() {
            return;
        }
        let delay = es::limit_retry_delay(e.limit_attempts);
        e.limit_attempts += 1;
        delay
    };
    let weak = Rc::downgrade(inner);
    let timer_entry = entry.clone();
    let timer = set_timeout(delay, move || {
        timer_entry.borrow_mut().limit_timer = None;
        if let Some(inner) = weak.upgrade() {
            if inner.is_current(&timer_entry) {
                sync(&inner, &timer_entry);
            }
        }
    });
    entry.borrow_mut().limit_timer = timer;
}

fn clear_limit_timer(entry: &EntryRef) {
    let timer = entry.borrow_mut().limit_timer.take();
    clear_timeout(timer);
}

fn schedule_retry(inner: &Rc<Inner>, reason: Option<String>) {
    let delay = {
        let mut state = inner.state.borrow_mut();
        state.quiet_reconnect = false;
        state.had_disconnect = true;
        state.disconnect_reason = reason.clone();
        let delay = es::retry_delay(state.retry_count);
        state.retry_count += 1;
        delay
    };
    inner.notify_all(&ChannelStatus::Disconnected { reason });
    let weak = Rc::downgrade(inner);
    let timer = set_timeout(delay, move || {
        if let Some(inner) = weak.upgrade() {
            inner.state.borrow_mut().retry_timer = None;
            inner.notify_all(&ChannelStatus::Connecting);
            ensure_socket(&inner);
        }
    });
    inner.state.borrow_mut().retry_timer = timer;
}

async fn sync_now(inner: Rc<Inner>, entry: EntryRef) {
    let Some(session_id) = inner.state.borrow().session_id.clone() else { return };
    if !inner.is_current(&entry) {
        return;
    }
    let Some(user_id) = helix_user_id() else { return };
    let still_current = |inner: &Inner| inner.state.borrow().session_id.as_deref() == Some(&session_id) && inner.is_current(&entry);

    let (broadcaster_id, wanted) = {
        let mut e = entry.borrow_mut();
        if e.session_id.as_deref() != Some(&session_id) {
            e.subscriptions.clear();
            e.optional_failed.clear();
            e.session_id = Some(session_id.clone());
        }
        let wanted = es::wanted_subscriptions(e.moderator, |name| e.optional_failed.contains(name));
        (e.broadcaster_id.clone(), wanted)
    };
    let mut limited = false;

    // Nicht mehr gewünschte Abos entfernen (zum Beispiel nach Verlust der Mod-Rechte).
    loop {
        let unwanted = {
            let mut e = entry.borrow_mut();
            let index = e.subscriptions.iter().position(|(name, _)| !wanted.iter().any(|spec| spec.name == name));
            index.map(|i| e.subscriptions.remove(i).1)
        };
        let Some(id) = unwanted else { break };
        delete_subscription(id).await;
    }

    for spec in wanted {
        if entry.borrow().has(spec.name) {
            continue;
        }
        if !still_current(&inner) {
            return;
        }
        let body = es::subscription_body(spec, &broadcaster_id, &user_id, &session_id).to_string();
        match helix_call("/eventsub/subscriptions", "POST", &[], Some(JsValue::from_str(&body))).await {
            Ok(response) => {
                let id = get(&get(&get(&response, "data"), "0"), "id").as_string();
                // Kanal inzwischen geschlossen oder Verbindung gewechselt: Abo wieder entfernen.
                if !still_current(&inner) {
                    if let Some(id) = id {
                        spawn_local(delete_subscription(id));
                    }
                    return;
                }
                if let Some(id) = id {
                    entry.borrow_mut().set(spec.name, id);
                }
            }
            Err(error) => {
                let (status, message) = failure_of(&error);
                if status == Some(409) {
                    // Abo besteht bereits für diese Session.
                    entry.borrow_mut().set(spec.name, "existing".into());
                    continue;
                }
                if spec.group == Group::Optional {
                    // Zum Beispiel Kostenbudget erschöpft: der Abgleich übernimmt, keine Meldung im Chat.
                    entry.borrow_mut().optional_failed.insert(spec.name.to_string());
                    continue;
                }
                // Twitch-Limit (429): verständlich melden, nur einmal je Störung, und später automatisch neu versuchen.
                if status == Some(429) {
                    limited = true;
                    inner.state.borrow_mut().session_limited = true;
                    let first = entry.borrow_mut().limit_reported.insert(spec.group);
                    if first {
                        let reason = es::limit_reason(&message);
                        notify(
                            &entry,
                            &if spec.group == Group::Base { ChannelStatus::Error { reason } } else { ChannelStatus::ModEventsUnavailable { reason } },
                        );
                    }
                    if spec.group == Group::Base {
                        break;
                    }
                    continue;
                }
                let reason = if status.is_some() { message } else { tr("Abo konnte nicht angelegt werden", "Subscription could not be created").to_string() };
                if spec.group == Group::Base {
                    notify(&entry, &ChannelStatus::Error { reason });
                    return;
                }
                notify(&entry, &ChannelStatus::ModEventsUnavailable { reason });
            }
        }
    }

    if limited {
        schedule_limit_retry(&inner, &entry);
    } else {
        inner.state.borrow_mut().limit_streak = 0;
        // Nach einer Limit-Störung: Mod-Ereignisse sind wieder da (Chat meldet sich über „Kanal beigetreten“).
        let restored = {
            let mut e = entry.borrow_mut();
            e.limit_attempts = 0;
            e.limit_reported.remove(&Group::Mod) && e.moderator && es::has_all_mod_subscriptions(|name| e.has(name))
        };
        if restored {
            notify(&entry, &ChannelStatus::ModEventsRestored);
        }
    }

    let joined = {
        let mut e = entry.borrow_mut();
        if e.has("channel.chat.message") && e.joined_session.as_deref() != Some(&session_id) {
            e.limit_reported.remove(&Group::Base);
            e.joined_session = Some(session_id.clone());
            true
        } else {
            false
        }
    };
    if joined {
        notify(&entry, &ChannelStatus::Joined);
    }
}

fn remove_listener(inner: &Rc<Inner>, broadcaster_id: &str, listener_id: u32) {
    let Some(entry) = inner.entry(broadcaster_id) else { return };
    {
        let mut e = entry.borrow_mut();
        let before = e.listeners.len();
        e.listeners.retain(|(id, _)| *id != listener_id);
        if e.listeners.len() == before || !e.listeners.is_empty() {
            return;
        }
    }
    let empty = {
        let mut state = inner.state.borrow_mut();
        state.channels.retain(|e| !Rc::ptr_eq(e, &entry));
        state.channels.is_empty()
    };
    clear_limit_timer(&entry);
    let ids: Vec<String> = entry.borrow_mut().subscriptions.drain(..).map(|(_, id)| id).collect();
    enqueue(
        &entry,
        Box::new(move || {
            Box::pin(async move {
                for id in ids {
                    delete_subscription(id).await;
                }
            })
        }),
    );
    if empty {
        close_sockets(inner);
    }
}

/// Eine Verbindung für alle Kanäle.
pub struct EventSub {
    inner: Rc<Inner>,
}

thread_local! {
    static CLIENT: Rc<EventSub> = Rc::new(EventSub::new());
}

/// Die eine EventSub-Verbindung der App.
pub fn client() -> Rc<EventSub> {
    CLIENT.with(Rc::clone)
}

impl EventSub {
    fn new() -> EventSub {
        EventSub {
            inner: Rc::new(Inner {
                state: RefCell::new(State {
                    socket: None,
                    migrating: None,
                    next_socket: 0,
                    next_listener: 0,
                    session_id: None,
                    keepalive_ms: 15_000.0,
                    keepalive_timer: None,
                    retry_timer: None,
                    retry_count: 0,
                    had_disconnect: false,
                    disconnect_reason: None,
                    session_limited: false,
                    limit_streak: 0,
                    quiet_reconnect: false,
                    channels: Vec::new(),
                }),
            }),
        }
    }

    /// Meldet einen Kanal an. Gibt die Nummer zum Abmelden zurück (`remove_channel`).
    pub fn add_channel(&self, broadcaster_id: &str, listener: ListenerRef, moderator: bool) -> u32 {
        let broadcaster_id = broadcaster_id.to_string();
        let inner = &self.inner;
        let (entry, listener_id, session) = {
            let mut state = inner.state.borrow_mut();
            state.next_listener += 1;
            let listener_id = state.next_listener;
            let existing = state.channels.iter().find(|e| e.borrow().broadcaster_id == broadcaster_id).cloned();
            let entry = existing.unwrap_or_else(|| {
                let entry = Rc::new(RefCell::new(Entry {
                    broadcaster_id: broadcaster_id.clone(),
                    listeners: Vec::new(),
                    moderator,
                    subscriptions: Vec::new(),
                    session_id: None,
                    joined_session: None,
                    tasks: VecDeque::new(),
                    running: false,
                    optional_failed: HashSet::new(),
                    limit_timer: None,
                    limit_attempts: 0,
                    limit_reported: HashSet::new(),
                }));
                state.channels.push(entry.clone());
                entry
            });
            entry.borrow_mut().listeners.push((listener_id, listener.clone()));
            (entry, listener_id, state.session_id.clone())
        };

        let status = |s: ChannelStatus| listener.on_status(&s);
        if let Some(session) = session {
            status(ChannelStatus::Connected);
            if entry.borrow().joined_session.as_deref() == Some(&session) {
                status(ChannelStatus::Joined);
            } else {
                sync(inner, &entry);
            }
        } else {
            status(ChannelStatus::Connecting);
            ensure_socket(inner);
        }

        listener_id
    }

    /// Meldet einen Zuhörer eines Kanals ab; ohne Zuhörer werden die Abos des Kanals entfernt.
    pub fn remove_channel(&self, broadcaster_id: &str, listener_id: u32) {
        remove_listener(&self.inner, broadcaster_id, listener_id);
    }

    /// Mod-Status eines Kanals hat sich geändert: Mod-Abos anlegen oder entfernen.
    pub fn set_moderator(&self, broadcaster_id: &str, moderator: bool) {
        let Some(entry) = self.inner.entry(broadcaster_id) else { return };
        if std::mem::replace(&mut entry.borrow_mut().moderator, moderator) == moderator {
            return;
        }
        if self.inner.state.borrow().session_id.is_some() {
            sync(&self.inner, &entry);
        }
    }

    /// „Neu verbinden“ für einen Kanal: Abos neu anlegen.
    pub fn resubscribe(&self, broadcaster_id: &str) {
        let Some(entry) = self.inner.entry(broadcaster_id) else { return };
        let task_entry = entry.clone();
        enqueue(
            &entry,
            Box::new(move || {
                Box::pin(async move {
                    let ids: Vec<String> = {
                        let mut e = task_entry.borrow_mut();
                        e.session_id = None;
                        e.joined_session = None;
                        e.subscriptions.drain(..).map(|(_, id)| id).collect()
                    };
                    for id in ids {
                        delete_subscription(id).await;
                    }
                })
            }),
        );
        notify(&entry, &ChannelStatus::Connecting);
        if self.inner.state.borrow().session_id.is_some() {
            sync(&self.inner, &entry);
        } else {
            ensure_socket(&self.inner);
        }
    }

    /// Alles schließen, zum Beispiel beim Abmelden.
    pub fn shutdown(&self) {
        let channels = std::mem::take(&mut self.inner.state.borrow_mut().channels);
        for entry in &channels {
            clear_limit_timer(entry);
        }
        close_sockets(&self.inner);
    }
}

