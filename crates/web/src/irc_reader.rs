//! Nur lesende IRC-Verbindung für „Erste Nachricht“ und die eigene Rolle VIP. Reine Teile in
//! `crates/core/src/twitch/irc_reader.rs`. Das Token geht nur in die PASS-Zeile an Twitch und wird nie ausgegeben.
//! Während JavaScript aufgerufen wird, ist kein Zustand ausgeliehen.
//!
//! In crates/web: eine Verbindung für die ganze App (`reader()`). Eigene Abzeichen gehen direkt an den Mod-Status
//! (VIP), die Verbindung folgt selbst der Anmeldung (neues Token baut neu auf, Abmelden beendet).

use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::rc::{Rc, Weak};

use multiyap_core::twitch::irc_reader as ir;
use js_sys::Reflect;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::WebSocket;

use crate::net::{global_function, sleep};

struct State {
    socket: Option<(u32, WebSocket)>,
    next_socket: u32,
    next_listener: u32,
    ready: bool,
    retry_timer: Option<JsValue>,
    retry_count: usize,
    /// Kanal (Login in Kleinbuchstaben) und seine Zuhörer.
    channels: Vec<(String, Vec<(u32, FirstMessageListener)>)>,
    joined: HashSet<String>,
    join_queue: VecDeque<String>,
    joining: bool,
}

/// Zuhörer für „Erste Nachricht“: bekommt die Nachrichten-ID.
pub type FirstMessageListener = Rc<dyn Fn(&str)>;

struct Inner {
    state: RefCell<State>,
    /// Kennung des zuletzt genutzten Tokens (Konto und Anmeldezeitpunkt, nie das Token selbst).
    token_key: RefCell<Option<String>>,
}

fn send(socket: &WebSocket, line: &str) {
    let _ = socket.send_with_str(line);
}

fn current_socket(inner: &Inner) -> Option<WebSocket> {
    inner.state.borrow().socket.as_ref().map(|(_, s)| s.clone())
}

/// Token und Login aus der Anmeldung in Rust (crates/web/src/auth.rs), nur mit chat:read.
fn credentials() -> Option<(String, String)> {
    crate::auth::irc_credentials().filter(|(token, login)| !token.is_empty() && !login.is_empty())
}

fn ensure_socket(inner: &Rc<Inner>) {
    {
        let state = inner.state.borrow();
        if state.socket.is_some() || state.retry_timer.is_some() || state.channels.is_empty() {
            return;
        }
    }
    // Ohne chat:read bleibt die Verbindung aus; „Erste Nachricht“ gibt es dann nur im Verlauf.
    let Some((token, login)) = credentials() else { return };
    let Ok(socket) = WebSocket::new(ir::IRC_URL) else {
        schedule_retry(inner);
        return;
    };
    let id = {
        let mut state = inner.state.borrow_mut();
        state.next_socket += 1;
        state.socket = Some((state.next_socket, socket.clone()));
        state.next_socket
    };
    let open_socket = socket.clone();
    let on_open = Closure::<dyn FnMut()>::new(move || {
        for line in ir::handshake(&token, &login) {
            send(&open_socket, &line);
        }
    })
    .into_js_value();
    let weak: Weak<Inner> = Rc::downgrade(inner);
    let on_message = Closure::<dyn FnMut(JsValue)>::new(move |event: JsValue| {
        let data = Reflect::get(&event, &"data".into()).ok().and_then(|d| d.as_string());
        if let (Some(inner), Some(data)) = (weak.upgrade(), data) {
            handle(&inner, id, &data);
        }
    })
    .into_js_value();
    let weak: Weak<Inner> = Rc::downgrade(inner);
    let on_close = Closure::<dyn FnMut()>::new(move || {
        let Some(inner) = weak.upgrade() else { return };
        let retry = {
            let mut state = inner.state.borrow_mut();
            if state.socket.as_ref().map(|(sid, _)| *sid) != Some(id) {
                return;
            }
            state.socket = None;
            state.ready = false;
            state.joined.clear();
            !state.channels.is_empty()
        };
        if retry {
            schedule_retry(&inner);
        }
    })
    .into_js_value();
    socket.set_onopen(Some(on_open.unchecked_ref()));
    socket.set_onmessage(Some(on_message.unchecked_ref()));
    socket.set_onclose(Some(on_close.unchecked_ref()));
}

fn handle(inner: &Rc<Inner>, socket_id: u32, data: &str) {
    let Some(socket) = inner.state.borrow().socket.as_ref().filter(|(id, _)| *id == socket_id).map(|(_, s)| s.clone()) else {
        return;
    };
    for line in data.split("\r\n").filter(|l| !l.is_empty()) {
        if let Some(pong) = ir::pong(line) {
            send(&socket, &pong);
            continue;
        }
        // Anmeldung bestätigt (001): Kanäle betreten.
        if !inner.state.borrow().ready && ir::is_welcome(line) {
            let channels: Vec<String> = {
                let mut state = inner.state.borrow_mut();
                state.ready = true;
                state.retry_count = 0;
                state.channels.iter().map(|(c, _)| c.clone()).collect()
            };
            for channel in channels {
                join(inner, channel);
            }
            continue;
        }
        if let Some((channel, badges)) = ir::own_badges_from_line(line) {
            crate::moderation::report_own_channel_badges(&channel, &badges);
            continue;
        }
        let Some((channel, message_id)) = ir::first_message_from_line(line) else { continue };
        let listeners: Vec<FirstMessageListener> = inner
            .state
            .borrow()
            .channels
            .iter()
            .find(|(c, _)| *c == channel)
            .map(|(_, l)| l.iter().map(|(_, f)| f.clone()).collect())
            .unwrap_or_default();
        for listener in listeners {
            listener(&message_id);
        }
    }
}

/// JOIN nacheinander mit Abstand.
fn join(inner: &Rc<Inner>, channel: String) {
    let start = {
        let mut state = inner.state.borrow_mut();
        if !state.joined.insert(channel.clone()) {
            return;
        }
        state.join_queue.push_back(channel);
        !std::mem::replace(&mut state.joining, true)
    };
    if !start {
        return;
    }
    let inner = inner.clone();
    spawn_local(async move {
        loop {
            let next = {
                let mut state = inner.state.borrow_mut();
                let next = state.join_queue.pop_front();
                if next.is_none() {
                    state.joining = false;
                }
                next
            };
            let Some(channel) = next else { break };
            let socket = {
                let state = inner.state.borrow();
                let wanted = state.ready && state.channels.iter().any(|(c, _)| *c == channel);
                if wanted { state.socket.as_ref().map(|(_, s)| s.clone()) } else { None }
            };
            if let Some(socket) = socket {
                send(&socket, &format!("JOIN #{channel}"));
            }
            sleep(ir::JOIN_SPACING_MS).await;
        }
    });
}

fn schedule_retry(inner: &Rc<Inner>) {
    let delay = {
        let mut state = inner.state.borrow_mut();
        let delay = ir::retry_delay(state.retry_count);
        state.retry_count += 1;
        delay
    };
    let weak = Rc::downgrade(inner);
    let callback = Closure::<dyn FnMut()>::new(move || {
        if let Some(inner) = weak.upgrade() {
            inner.state.borrow_mut().retry_timer = None;
            ensure_socket(&inner);
        }
    })
    .into_js_value();
    let timer = global_function("setTimeout").ok().and_then(|f| f.call2(&JsValue::NULL, &callback, &JsValue::from_f64(delay)).ok());
    inner.state.borrow_mut().retry_timer = timer;
}

fn close(inner: &Rc<Inner>) {
    let (timer, socket) = {
        let mut state = inner.state.borrow_mut();
        state.ready = false;
        state.joined.clear();
        (state.retry_timer.take(), state.socket.take())
    };
    if let (Some(timer), Ok(clear)) = (timer, global_function("clearTimeout")) {
        let _ = clear.call1(&JsValue::NULL, &timer);
    }
    if let Some((_, socket)) = socket {
        let _ = socket.close();
    }
}

fn unwatch(inner: &Rc<Inner>, channel: &str, listener_id: u32) {
    let (part, empty) = {
        let mut state = inner.state.borrow_mut();
        let Some(index) = state.channels.iter().position(|(c, _)| c == channel) else { return };
        state.channels[index].1.retain(|(id, _)| *id != listener_id);
        if !state.channels[index].1.is_empty() {
            return;
        }
        state.channels.remove(index);
        let part = state.ready && state.joined.contains(channel);
        state.joined.remove(channel);
        (part, state.channels.is_empty())
    };
    if part {
        if let Some(socket) = current_socket(inner) {
            send(&socket, &format!("PART #{channel}"));
        }
    }
    if empty {
        close(inner);
    }
}

/// Die eine nur lesende IRC-Verbindung der App.
pub struct IrcReader {
    inner: Rc<Inner>,
}

thread_local! {
    static READER: Rc<IrcReader> = Rc::new(IrcReader::new());
}

/// Die eine nur lesende IRC-Verbindung; folgt beim ersten Gebrauch der Anmeldung.
pub fn reader() -> Rc<IrcReader> {
    READER.with(Rc::clone)
}

/// Kennung des Tokens ohne das Token selbst: Konto und Zeitpunkt der Anmeldung (neu bei jedem Token).
fn token_key() -> Option<String> {
    crate::auth::view().account.filter(|_| crate::auth::own_user_id().is_some()).map(|a| format!("{}:{}", a.user_id, a.obtained_at))
}

impl IrcReader {
    fn new() -> IrcReader {
        let reader = IrcReader {
            inner: Rc::new(Inner {
                state: RefCell::new(State {
                    socket: None,
                    next_socket: 0,
                    next_listener: 0,
                    ready: false,
                    retry_timer: None,
                    retry_count: 0,
                    channels: Vec::new(),
                    joined: HashSet::new(),
                    join_queue: VecDeque::new(),
                    joining: false,
                }),
                token_key: RefCell::new(token_key()),
            }),
        };
        // Neues Token (zum Beispiel nach Neuanmeldung mit chat:read) baut neu auf, Abmelden beendet.
        let weak = Rc::downgrade(&reader.inner);
        crate::auth::subscribe(Rc::new(move || {
            let Some(inner) = weak.upgrade() else { return };
            let key = token_key();
            if std::mem::replace(&mut *inner.token_key.borrow_mut(), key.clone()) == key {
                return;
            }
            reconnect(&inner);
        }));
        reader
    }

    /// Kanal beobachten; `listener(messageId)` bei einer ersten Nachricht. Gibt die Nummer zum Beenden zurück.
    pub fn watch(&self, login: &str, listener: FirstMessageListener) -> u32 {
        let channel = login.to_lowercase();
        let (listener_id, ready) = {
            let mut state = self.inner.state.borrow_mut();
            state.next_listener += 1;
            let id = state.next_listener;
            match state.channels.iter_mut().find(|(c, _)| *c == channel) {
                Some((_, list)) => list.push((id, listener)),
                None => state.channels.push((channel.clone(), vec![(id, listener)])),
            }
            (id, state.ready)
        };
        ensure_socket(&self.inner);
        if ready {
            join(&self.inner, channel);
        }
        listener_id
    }

    pub fn unwatch(&self, login: &str, listener_id: u32) {
        unwatch(&self.inner, &login.to_lowercase(), listener_id);
    }

    /// Mit aktuellem Token neu verbinden (oder beenden, wenn keine Berechtigung mehr).
    pub fn reconnect(&self) {
        reconnect(&self.inner);
    }

    /// Alles beenden, zum Beispiel beim Abmelden.
    pub fn shutdown(&self) {
        self.inner.state.borrow_mut().channels.clear();
        close(&self.inner);
    }
}

fn reconnect(inner: &Rc<Inner>) {
    close(inner);
    inner.state.borrow_mut().retry_count = 0;
    ensure_socket(inner);
}

