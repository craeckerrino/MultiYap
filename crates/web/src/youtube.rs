//! YouTube-Kanäle der App, zuerst nur lesen. Für jeden offenen YouTube-Tab (Kanal-Tab mit Plattform „youtube“) fragt
//! die App den MultiYap-Server (`multiyap_core::youtube::SERVER`, ohne Server startet nichts) nach dem Live-Status und öffnet, solange der Kanal live ist, den Chat als Server-Sent
//! Events (`EventSource`). Eine Anmeldung ist nicht nötig; mit YouTube selbst spricht nur der Server.
//!
//! Nachrichten leben nur im Arbeitsspeicher (höchstens `chat_buffer::MAX_ITEMS` je Kanal) und haben dieselbe Form
//! wie bei Twitch, damit die Chat-Liste sie ohne Sonderweg zeigt. Was nach dem Ende einer Verbindung passiert,
//! entscheidet `multiyap_core::youtube::next_after`.
//!
//! Neue Zeilen gehen wie bei Twitch an Benachrichtigungen (`on_live_item`), der Hervorhebungen-Tab liest die Zeilen
//! über `visit_lines`, die Nutzerkarte über `recent_messages`. Der Verlauf beim Verbinden (die erste Lieferung nach
//! `start`) ist als `fromHistory` markiert und löst nichts aus.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use js_sys::Function;
use multiyap_core::chat_buffer::{BufferItem, ChatBuffer};
use multiyap_core::youtube::{self as core, ChatLine, LiveAnswer, LiveInfo, Next};
use serde::Serialize;
use serde_json::Value;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::{EventSource, MessageEvent, RequestInit};

use crate::{net, workspace};

type Listener = Rc<dyn Fn()>;
/// Zuhörer für jede neu eintreffende Zeile (nie den Verlauf).
pub type LiveListener = Rc<dyn Fn(&Value)>;
/// Zuhörer für „Kanal ist live gegangen“ (Schlüssel des Kanals).
pub type WentLiveListener = Rc<dyn Fn(&str)>;
type Handler = Closure<dyn FnMut(MessageEvent)>;

/// Zustand eines Kanals für die Oberfläche.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// Erste Prüfung läuft.
    Checking,
    /// Live, Chat verbunden.
    Live,
    /// Nicht live; die App prüft regelmäßig.
    Offline,
    /// Live, aber der Chat ist abgeschaltet.
    NoChat,
    /// Hinweis (Kontingent, Grenze, nicht gefunden, Server nicht erreichbar).
    Problem(String),
}

/// Stand eines Kanals für die Oberfläche.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelView {
    pub title: Option<String>,
    pub live: Option<LiveInfo>,
    pub status: Status,
    /// Während der Sitzung live gegangen und noch nicht angesehen (Pulsieren im Tab wie bei Twitch).
    pub went_live: bool,
}

/// Eine Zeile mit ihrem JavaScript-Objekt, das erst bei Bedarf entsteht (wie in sessions.rs).
struct Line {
    value: Rc<Value>,
    js: RefCell<Option<JsValue>>,
}

impl Line {
    fn new(value: Value) -> Self {
        Self { value: Rc::new(value), js: RefCell::new(None) }
    }

    fn js(&self) -> JsValue {
        if let Some(js) = self.js.borrow().as_ref() {
            return js.clone();
        }
        let js = self.value.serialize(&serde_wasm_bindgen::Serializer::json_compatible()).unwrap_or(JsValue::NULL);
        *self.js.borrow_mut() = Some(js.clone());
        js
    }
}

impl BufferItem for Line {
    fn id(&self) -> &str {
        self.value.id()
    }
}

struct State {
    answer: Option<LiveAnswer>,
    status: Status,
    buffer: ChatBuffer<Line>,
    source: Option<EventSource>,
    /// Zuhörer der offenen Verbindung. Sie bleiben bis zur nächsten Verbindung, weil ein Zuhörer die Verbindung
    /// selbst schließen kann (Ereignis `end`) und dabei nicht freigegeben werden darf.
    handlers: Vec<Handler>,
    /// Steigt bei jedem neuen Schritt; ältere Zeitgeber und Abfragen verfallen dann.
    generation: u32,
    disposed: bool,
    /// Die nächste Lieferung ist der Verlauf, den der Server beim Verbinden schickt.
    history_next: bool,
    went_live: bool,
}

struct YouTubeChannel {
    key: String,
    state: RefCell<State>,
}

#[derive(Default)]
struct Registry {
    channels: Vec<Rc<YouTubeChannel>>,
    listeners: Vec<(u32, Listener)>,
    live_listeners: Vec<(u32, LiveListener)>,
    went_live_listeners: Vec<(u32, WentLiveListener)>,
    next_id: u32,
    started: bool,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

fn emit() {
    let listeners: Vec<Listener> = REGISTRY.with(|r| r.borrow().listeners.iter().map(|(_, l)| l.clone()).collect());
    for listener in listeners {
        listener();
    }
}

fn set_timeout(callback: impl FnOnce() + 'static, ms: f64) {
    let callback = Closure::once_into_js(callback);
    if let Ok(set_timeout) = net::global_function("setTimeout") {
        let _ = set_timeout.call2(&JsValue::NULL, &callback, &JsValue::from_f64(ms));
    }
}

impl YouTubeChannel {
    fn new(key: &str) -> Rc<Self> {
        Rc::new(Self {
            key: key.to_string(),
            state: RefCell::new(State {
                answer: None,
                status: Status::Checking,
                buffer: ChatBuffer::default(),
                source: None,
                handlers: Vec::new(),
                generation: 0,
                disposed: false,
                history_next: false,
                went_live: false,
            }),
        })
    }

    /// Neuer Schritt: alles Laufende verfällt. Gibt die Nummer des Schritts zurück.
    fn next_generation(&self) -> u32 {
        let mut state = self.state.borrow_mut();
        state.generation = state.generation.wrapping_add(1);
        state.generation
    }

    fn current(&self, generation: u32) -> bool {
        let state = self.state.borrow();
        !state.disposed && state.generation == generation
    }

    fn close_source(&self) {
        if let Some(source) = self.state.borrow_mut().source.take() {
            source.close();
        }
    }

    fn set_status(&self, status: Status) {
        let changed = {
            let mut state = self.state.borrow_mut();
            let changed = state.status != status;
            state.status = status;
            changed
        };
        if changed {
            emit();
        }
    }

    /// Nach `ms` Millisekunden erneut prüfen (verfällt mit dem nächsten Schritt).
    fn schedule_check(self: &Rc<Self>, ms: f64) {
        let generation = self.next_generation();
        let weak = Rc::downgrade(self);
        set_timeout(
            move || {
                if let Some(channel) = weak.upgrade().filter(|c| c.current(generation)) {
                    spawn_local(channel.check());
                }
            },
            ms,
        );
    }

    /// Live-Status beim Server abfragen und je nach Ergebnis den Chat öffnen oder später erneut prüfen.
    async fn check(self: Rc<Self>) {
        self.close_source();
        let generation = self.next_generation();
        let init = RequestInit::new();
        init.set_credentials(web_sys::RequestCredentials::Omit);
        let Some(url) = core::live_url(&self.key) else {
            return self.apply("youtube_error");
        };
        let result = net::fetch(&url, &init).await;
        if !self.current(generation) {
            return;
        }
        let response = match result {
            Ok(response) => response,
            Err(_) => return self.apply("youtube_error"),
        };
        let status = response.status();
        let body = net::json_value(&response).await;
        if !self.current(generation) {
            return;
        }
        if !response.ok() {
            return self.apply(&core::error_code(status, body.as_ref()));
        }
        let Some(answer) = body.and_then(|b| serde_json::from_value::<LiveAnswer>(b).ok()) else {
            return self.apply("youtube_error");
        };
        let (live, chat) = (answer.live.is_some(), answer.live.as_ref().is_some_and(|l| l.chat));
        self.state.borrow_mut().answer = Some(answer);
        emit();
        if live && chat {
            self.connect();
        } else {
            self.apply(if live { "no_chat" } else { "offline" });
        }
    }

    /// Chat als Server-Sent Events öffnen.
    fn connect(self: &Rc<Self>) {
        self.close_source();
        let Some(Ok(source)) = core::chat_url(&self.key).map(|url| EventSource::new(&url)) else {
            return self.apply("youtube_error");
        };
        let weak = Rc::downgrade(self);
        let handler = |weak: Weak<Self>, run: fn(&Rc<Self>, String)| -> Handler {
            Closure::new(move |event: MessageEvent| {
                if let (Some(channel), Some(data)) = (weak.upgrade(), event.data().as_string()) {
                    run(&channel, data);
                }
            })
        };
        let handlers = vec![
            ("start", handler(weak.clone(), Self::on_start)),
            ("messages", handler(weak.clone(), Self::on_messages)),
            ("end", handler(weak.clone(), Self::on_end)),
        ];
        for (name, handler) in &handlers {
            let _ = source.add_event_listener_with_callback(name, handler.as_ref().unchecked_ref());
        }
        // Abbruch: Läuft die Verbindung noch (CONNECTING), verbindet der Browser selbst neu. Ist sie zu (zum Beispiel
        // Antwort 429 der Bremse), prüft die App später selbst.
        let on_error: Handler = Closure::new(move |_event: MessageEvent| {
            let Some(channel) = weak.upgrade() else { return };
            let closed = channel.state.borrow().source.as_ref().is_some_and(|s| s.ready_state() == EventSource::CLOSED);
            if closed {
                channel.close_source();
                channel.schedule_check(10_000.0);
            }
        });
        source.set_onerror(Some(on_error.as_ref().unchecked_ref::<Function>()));
        let mut state = self.state.borrow_mut();
        state.handlers = handlers.into_iter().map(|(_, h)| h).chain([on_error]).collect();
        state.source = Some(source);
    }

    fn on_start(self: &Rc<Self>, data: String) {
        let went_live = {
            let mut state = self.state.borrow_mut();
            if let Ok(answer) = serde_json::from_str::<LiveAnswer>(&data) {
                state.answer = Some(answer);
            }
            // Live gegangen: vorher als offline bekannt (nicht beim ersten Prüfen nach dem Start).
            let went_live = state.status == Status::Offline;
            state.went_live |= went_live;
            state.status = Status::Live;
            state.history_next = true;
            went_live
        };
        emit();
        if went_live {
            let listeners: Vec<WentLiveListener> = REGISTRY.with(|r| r.borrow().went_live_listeners.iter().map(|(_, l)| l.clone()).collect());
            for listener in listeners {
                listener(&self.key);
            }
        }
    }

    fn on_messages(self: &Rc<Self>, data: String) {
        let Ok(lines) = serde_json::from_str::<Vec<ChatLine>>(&data) else { return };
        let mut fresh: Vec<Rc<Value>> = Vec::new();
        let changed = {
            let mut state = self.state.borrow_mut();
            let Some(answer) = state.answer.as_ref() else { return };
            let channel = core::channel_ref(&self.key, &answer.channel);
            let history = std::mem::take(&mut state.history_next);
            let now = net::now_ms();
            for line in &lines {
                // Beim Neuverbinden kommt der Verlauf erneut: Doppelte überspringen.
                if state.buffer.contains(&line.id) {
                    continue;
                }
                if let Some(item) = core::to_item(line, &channel, now, history) {
                    let line = Line::new(item);
                    if !history {
                        fresh.push(line.value.clone());
                    }
                    state.buffer.push(line);
                }
            }
            state.buffer.flush()
        };
        if changed {
            emit();
        }
        if !fresh.is_empty() {
            let listeners: Vec<LiveListener> = REGISTRY.with(|r| r.borrow().live_listeners.iter().map(|(_, l)| l.clone()).collect());
            for item in &fresh {
                for listener in &listeners {
                    listener(item);
                }
            }
        }
    }

    fn on_end(self: &Rc<Self>, data: String) {
        let reason = serde_json::from_str::<Value>(&data).ok().and_then(|v| v.get("reason").and_then(Value::as_str).map(str::to_string)).unwrap_or_default();
        self.close_source();
        self.apply(&reason);
    }

    /// Nächster Schritt nach einem Grund (Ende des Chats oder Fehlercode des Servers).
    fn apply(self: &Rc<Self>, reason: &str) {
        match core::next_after(reason) {
            Next::Recheck { after_ms, offline } => {
                match reason {
                    "offline" | "no_chat" => {
                        if offline {
                            // Ein beendeter Livestream ist nicht mehr live.
                            if let Some(answer) = self.state.borrow_mut().answer.as_mut() {
                                answer.live = None;
                            }
                        }
                        self.set_status(if offline { Status::Offline } else { Status::NoChat });
                        emit();
                    }
                    _ => {}
                }
                self.schedule_check(after_ms);
            }
            Next::Reconnect { after_ms } => self.schedule_check(after_ms),
            Next::Wait { after_ms, message } => {
                self.set_status(Status::Problem(message.to_string()));
                self.schedule_check(after_ms);
            }
            Next::Stop { message } => {
                self.next_generation();
                self.set_status(Status::Problem(message.to_string()));
            }
        }
    }

    fn dispose(&self) {
        self.close_source();
        let mut state = self.state.borrow_mut();
        state.disposed = true;
        state.handlers.clear();
    }
}

/// YouTube-Kanäle der offenen Tabs, jeder einmal.
fn wanted_keys() -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for tab in workspace::state().tabs.iter() {
        let field = |name: &str| tab.get(name).and_then(Value::as_str).unwrap_or_default();
        if field("kind") == "channel" && field("platform") == core::PLATFORM {
            let key = field("channel").to_string();
            if core::is_channel_key(&key) && !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    keys
}

/// Gleicht die Kanäle mit den offenen Tabs ab.
fn sync() {
    let wanted = wanted_keys();
    let (removed, added) = REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        let (keep, removed): (Vec<_>, Vec<_>) = std::mem::take(&mut r.channels).into_iter().partition(|c| wanted.contains(&c.key));
        r.channels = keep;
        let mut added = Vec::new();
        for key in &wanted {
            if !r.channels.iter().any(|c| &c.key == key) {
                let channel = YouTubeChannel::new(key);
                r.channels.push(channel.clone());
                added.push(channel);
            }
        }
        (removed, added)
    });
    for channel in &removed {
        channel.dispose();
    }
    for channel in &added {
        spawn_local(channel.clone().check());
    }
    if !removed.is_empty() || !added.is_empty() {
        emit();
    }
}

fn channel(key: &str) -> Option<Rc<YouTubeChannel>> {
    REGISTRY.with(|r| r.borrow().channels.iter().find(|c| c.key == key).cloned())
}

/// Folgt den offenen Tabs (einmal starten). Ohne Anmeldung, YouTube braucht keine.
pub fn start() {
    // Ohne MultiYap-Server ist YouTube „Coming Soon“: keine Verbindungen.
    if core::SERVER.is_none() || REGISTRY.with(|r| std::mem::replace(&mut r.borrow_mut().started, true)) {
        return;
    }
    workspace::subscribe(Rc::new(sync));
    sync();
}

/// Meldet jede Änderung an irgendeinem YouTube-Kanal; gibt die Nummer zum Abmelden zurück.
pub fn subscribe(listener: Listener) -> u32 {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        r.next_id += 1;
        let id = r.next_id;
        r.listeners.push((id, listener));
        id
    })
}

pub fn unsubscribe(id: u32) {
    REGISTRY.with(|r| r.borrow_mut().listeners.retain(|(i, _)| *i != id));
}

/// Stand eines Kanals (`None`, solange kein Tab ihn öffnet).
pub fn view(key: &str) -> Option<ChannelView> {
    let channel = channel(key)?;
    let state = channel.state.borrow();
    Some(ChannelView {
        title: state.answer.as_ref().map(|a| a.channel.title.clone()),
        live: state.answer.as_ref().and_then(|a| a.live.clone()),
        status: state.status.clone(),
        went_live: state.went_live,
    })
}

/// Schlüssel aller offenen YouTube-Kanäle.
pub fn keys() -> Vec<String> {
    REGISTRY.with(|r| r.borrow().channels.iter().map(|c| c.key.clone()).collect())
}

/// Alle Zeilen eines Kanals, älteste zuerst, mit ihrem JavaScript-Objekt bei Bedarf (für den Hervorhebungen-Tab).
pub fn visit_lines(key: &str, mut visit: impl FnMut(&Rc<Value>, &dyn Fn() -> JsValue)) {
    let Some(channel) = channel(key) else { return };
    let state = channel.state.borrow();
    for line in state.buffer.items() {
        visit(&line.value, &|| line.js());
    }
}

/// Letzte Nachrichten einer Person im geladenen Chat (Nutzerkarte), nichts wird extra gespeichert.
pub fn recent_messages(key: &str, user_id: &str) -> Vec<Value> {
    let Some(channel) = channel(key) else { return Vec::new() };
    let state = channel.state.borrow();
    multiyap_core::chat_tools::recent_messages_of(state.buffer.items().iter().map(|line| &*line.value), user_id, multiyap_core::chat_tools::RECENT_MESSAGES_LIMIT)
}

/// Live-Gehen gesehen: Der Tab hört auf zu pulsieren.
pub fn acknowledge_live(key: &str) {
    let Some(channel) = channel(key) else { return };
    let changed = std::mem::take(&mut channel.state.borrow_mut().went_live);
    if changed {
        emit();
    }
}

/// Meldet jede neu eintreffende Zeile aller YouTube-Kanäle (für Benachrichtigungen), nie den Verlauf.
pub fn on_live_item(listener: LiveListener) -> u32 {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        r.next_id += 1;
        let id = r.next_id;
        r.live_listeners.push((id, listener));
        id
    })
}

/// Meldet, wenn ein offener YouTube-Kanal während der Sitzung live geht.
pub fn on_went_live(listener: WentLiveListener) -> u32 {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        r.next_id += 1;
        let id = r.next_id;
        r.went_live_listeners.push((id, listener));
        id
    })
}

/// Alle Zeilen als JavaScript-Objekte, älteste zuerst (für die Chat-Liste).
pub fn items_js(key: &str) -> js_sys::Array {
    channel(key).map_or_else(js_sys::Array::new, |c| c.state.borrow().buffer.items().iter().map(Line::js).collect())
}

/// Sofort neu prüfen und verbinden (Tab-Menü „Neu verbinden“).
pub fn reconnect(key: &str) {
    if let Some(channel) = channel(key) {
        channel.set_status(Status::Checking);
        spawn_local(channel.check());
    }
}
