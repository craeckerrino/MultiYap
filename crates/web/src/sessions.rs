//! Kanal-Sitzungen (bisher src/platforms/twitch/channel-session.ts, src/core/chat/channel-session.ts und
//! src/core/chat/sessions.ts): genau eine Sitzung pro offenem Kanal, auch wenn mehrere Tabs denselben Kanal zeigen.
//! Tabs im Hintergrund bleiben verbunden. Nachrichten liegen nur im Arbeitsspeicher, begrenzt auf `MAX_ITEMS`
//! (Ringpuffer in crates/core/src/chat_buffer.rs), und werden nie gespeichert.
//!
//! Eine Sitzung tritt über EventSub bei, lädt den Verlauf, hält Chat-Modi, markiert erste Nachrichten (IRC), meldet
//! Rollen, eigene Abzeichen und verdächtige Nutzer an die Speicher in crates/web/src/moderation.rs und beobachtet den
//! Live-Status. Zeilen sind JSON in der Form des Nachrichtenmodells; das JavaScript-Objekt einer Zeile entsteht einmal
//! und bleibt gleich, solange die Zeile unverändert ist.

use multiyap_core::i18n::tr;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use multiyap_core::chat_buffer::{self as cb, BufferItem, ChatBuffer, OLDER_PAGE_SIZE};
use multiyap_core::community_gifts::{attach_recipient, CommunityGifts, GiftPlacement};
use multiyap_core::history::HistoryItem;
use multiyap_core::model::{ChannelRef, ChatEventItem};
use multiyap_core::roles::{KnownRole, OrderedMap};
use multiyap_core::twitch::badges::{BadgeStore, BadgeVersion};
use multiyap_core::twitch::eventsub::ChannelStatus;
use multiyap_core::twitch::{chat, events, helix_api, moderation as tm, suspicious as ts};
use js_sys::{Function, Reflect};
use serde::Serialize;
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::eventsub::{self, ChannelListener};
use crate::moderation::{self as md, SuspiciousInfo};
use crate::net::{global_function, now_ms};
use crate::{api, auth, irc_reader, streams, workspace};

/// Bei sehr aktiven Chats zuerst nur die neuesten Verlaufszeilen zeigen, ältere beim Hochscrollen.
const HISTORY_VISIBLE: usize = 100;
/// Puffer beim Nachladen nach einem Reconnect, weil Live-Nachrichten die Empfangszeit tragen.
const GAP_MARGIN_MS: f64 = 5_000.0;
/// Vormerkungen (erste Nachricht, verdächtige Person) nicht unbegrenzt sammeln.
const MAX_MARKS: usize = 200;

type Listener = Rc<dyn Fn()>;

// ---- Zeilen ----

/// Eine Zeile mit ihrem JavaScript-Objekt, das erst bei Bedarf entsteht.
#[derive(Clone)]
struct Line {
    value: Rc<Value>,
    js: RefCell<Option<JsValue>>,
}

impl Line {
    fn new(value: Value) -> Line {
        Line { value: Rc::new(value), js: RefCell::new(None) }
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

/// Gift Subs: jede beschenkte Person in eigener Zeile (Einstellung chat.giftSubLines).
fn gift_lines() -> bool {
    crate::settings::get("chat.giftSubLines").and_then(|v| v.as_bool()).unwrap_or(false)
}

fn text<'a>(value: &'a Value, name: &str) -> &'a str {
    value.get(name).and_then(Value::as_str).unwrap_or_default()
}

fn badge_set_ids(message: &Value) -> Vec<String> {
    message.get("badges").and_then(Value::as_array).map(|list| list.iter().map(|b| text(b, "setId").to_string()).collect()).unwrap_or_default()
}

fn has(value: &Value, name: &str) -> bool {
    value.get(name).is_some_and(|v| !v.is_null())
}

/// Entfernte Nachricht markieren (nur Nachrichten, die noch nicht als entfernt markiert sind).
fn mark_removed(item: &Value, removed: &Value) -> Option<Value> {
    if text(item, "kind") != "message" || item.get("flags").is_some_and(|f| has(f, "removed")) {
        return None;
    }
    let mut next = item.clone();
    next["flags"]["removed"] = removed.clone();
    Some(next)
}

/// Mitternacht (örtliche Zeit) des Tages eines Zeitpunkts.
fn start_of_day(timestamp: f64) -> f64 {
    let date = js_sys::Date::new(&JsValue::from_f64(timestamp));
    date.set_hours(0);
    date.set_minutes(0);
    date.set_seconds(0);
    date.set_milliseconds(0);
    date.get_time()
}

fn day_label(timestamp: f64, now: f64) -> String {
    let (day, today) = (start_of_day(timestamp), start_of_day(now));
    if day == today {
        return tr("Heute", "Today").into();
    }
    if day == start_of_day(today - 12.0 * 60.0 * 60.0 * 1000.0) {
        return tr("Gestern", "Yesterday").into();
    }
    js_sys::Date::new(&JsValue::from_f64(timestamp)).to_locale_date_string(tr("de-DE", "en-US"), &JsValue::UNDEFINED).as_string().unwrap_or_default()
}

/// Fügt Datumstrenner ein, wo im Verlauf Mitternacht liegt.
fn with_date_dividers(messages: Vec<Value>, now: f64) -> Vec<Value> {
    let mut items = Vec::with_capacity(messages.len());
    let mut previous_day: Option<f64> = None;
    for message in messages {
        let timestamp = message.get("timestamp").and_then(Value::as_f64).unwrap_or_default();
        let day = start_of_day(timestamp);
        if previous_day.is_some_and(|p| p != day) {
            let channel = message.get("channel").cloned().unwrap_or(Value::Null);
            items.push(json!({
                "kind": "divider",
                "channel": channel,
                "id": format!("date-{}-{}", text(&channel, "login"), day),
                "timestamp": timestamp,
                "variant": "date",
                "label": day_label(timestamp, now),
            }));
        }
        previous_day = Some(day);
        items.push(message);
    }
    items
}

/// Rollen aus Verlaufszeilen merken (älteste zuerst, die neueste Angabe gilt).
fn remember_roles(channel_id: &str, items: &[Value]) {
    for item in items.iter().filter(|i| text(i, "kind") == "message" && !has(i, "sourceChannel")) {
        md::note_badges(channel_id, text(item.get("sender").unwrap_or(&Value::Null), "id"), &badge_set_ids(item));
    }
}

/// Mod-Aktionen verraten Rollen: wer sie ausführt, ist Mod; /mod, /unmod, /vip und /unvip ändern die Rolle.
fn remember_moderate_roles(channel_id: &str, item: &Value) {
    if let Some(moderator) = item.get("moderator").map(|m| text(m, "id")).filter(|id| !id.is_empty() && *id != channel_id) {
        md::note_role(channel_id, moderator, Some(KnownRole::Moderator));
    }
    let Some(target) = item.get("target").map(|t| text(t, "id")).filter(|id| !id.is_empty()) else { return };
    match text(item, "action") {
        "mod" => md::note_role(channel_id, target, Some(KnownRole::Moderator)),
        "vip" => md::note_role(channel_id, target, Some(KnownRole::Vip)),
        "unmod" | "unvip" => md::note_role(channel_id, target, None),
        _ => {}
    }
}

/// Abzeichen-Speicher aus den Antworten von „Get Chat Badges“ (Kanal geht vor global).
fn badge_store(channel: &Value, global: &Value) -> BadgeStore {
    let entries = |sets: &Value| -> Vec<(String, String, BadgeVersion)> {
        sets.as_array()
            .into_iter()
            .flatten()
            .flat_map(|set| {
                let set_id = text(set, "set_id").to_string();
                set.get("versions").and_then(Value::as_array).cloned().unwrap_or_default().into_iter().map(move |v| {
                    let version = BadgeVersion {
                        title: text(&v, "title").into(),
                        image_url_1x: text(&v, "image_url_1x").into(),
                        image_url_2x: text(&v, "image_url_2x").into(),
                    };
                    (set_id.clone(), text(&v, "id").to_string(), version)
                })
            })
            .collect()
    };
    BadgeStore::new(entries(channel), entries(global))
}

fn to_value<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// Lesbare Meldung aus einem Fehler von Helix ({ helixStatus, message }) oder einem Error.
fn error_message(error: &JsValue, fallback: &str) -> String {
    Reflect::get(error, &"message".into()).ok().and_then(|m| m.as_string()).filter(|m| !m.is_empty()).unwrap_or_else(|| fallback.to_string())
}

// ---- Zeilen aller Kanäle für TypeScript (Benachrichtigungen, Ungelesen) ----

/// Zuhörer in Rust für neue Zeilen aller Kanäle (nie der geladene Verlauf).
pub type LiveListener = Rc<dyn Fn(&Value)>;

thread_local! {
    static RUST_LIVE_LISTENERS: RefCell<Vec<(u32, LiveListener)>> = RefCell::new(Vec::new());
    static LIVE_LISTENERS: RefCell<Vec<(u32, Function)>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: Cell<u32> = const { Cell::new(0) };
}

fn next_id() -> u32 {
    NEXT_ID.with(|n| {
        n.set(n.get() + 1);
        n.get()
    })
}

// ---- Sitzung ----

struct State {
    channel: Option<ChannelRef>,
    buffer: ChatBuffer<Line>,
    joined: bool,
    error: Option<String>,
    chat_modes: Option<Value>,
    /// Letzter Shoutout aus dem Kanal: Login und Anzeigename (oben rechts, bis man ihn schließt).
    shoutout: Option<(String, String)>,
    flush_scheduled: bool,
    notice_counter: u64,
    last_notice: Option<(String, Option<String>)>,
    event_counter: u64,
    /// Erste Nachrichten, deren IRC-Meldung vor der EventSub-Nachricht ankam.
    pending_first: OrderedMap<String, ()>,
    /// Meldungen zu verdächtigen Personen, deren normale Nachricht noch nicht da ist (Nachrichten-ID).
    pending_suspicious: OrderedMap<String, Value>,
    /// Zeitpunkt der letzten bekannten Nachricht (für das Nachladen nach Reconnect).
    last_timestamp: f64,
    /// Community-Geschenke als eine Zeile zusammenfassen.
    gifts: CommunityGifts,
    badges: Rc<BadgeStore>,
    disposed: bool,
    eventsub_listener: Option<u32>,
    mod_listener: Option<u32>,
    watching_stream: bool,
    live_listener: Option<u32>,
    irc_listener: Option<u32>,
    version: u32,
    listeners: Vec<(u32, Listener)>,
}

pub struct Session {
    pub id: u32,
    pub login: String,
    state: RefCell<State>,
}

/// Zuhörer für EventSub, solange die Sitzung lebt.
struct SessionListener(Weak<Session>);

impl ChannelListener for SessionListener {
    fn on_event(&self, subscription_type: &str, event: &JsValue) {
        if let Some(session) = self.0.upgrade() {
            session.handle_event(subscription_type, event);
        }
    }

    fn on_status(&self, status: &ChannelStatus) {
        if let Some(session) = self.0.upgrade() {
            session.handle_status(status);
        }
    }
}

impl Session {
    fn new(login: &str) -> Rc<Session> {
        Rc::new(Session {
            id: next_id(),
            login: login.to_string(),
            state: RefCell::new(State {
                channel: None,
                buffer: ChatBuffer::default(),
                joined: false,
                error: None,
                chat_modes: None,
                shoutout: None,
                flush_scheduled: false,
                notice_counter: 0,
                last_notice: None,
                event_counter: 0,
                pending_first: OrderedMap::new(MAX_MARKS),
                pending_suspicious: OrderedMap::new(MAX_MARKS),
                last_timestamp: 0.0,
                gifts: CommunityGifts::default(),
                badges: Rc::new(BadgeStore::default()),
                disposed: false,
                eventsub_listener: None,
                mod_listener: None,
                watching_stream: false,
                live_listener: None,
                irc_listener: None,
                version: 0,
                listeners: Vec::new(),
            }),
        })
    }

    fn disposed(&self) -> bool {
        self.state.borrow().disposed
    }

    /// Kanal, sobald er aufgelöst ist.
    pub fn channel(&self) -> Option<ChannelRef> {
        self.state.borrow().channel.clone()
    }

    /// Aktive Chat-Modi, sobald bekannt (in jedem Kanal sichtbar).
    pub fn chat_modes(&self) -> Option<Value> {
        self.state.borrow().chat_modes.clone()
    }

    /// Letzter Shoutout (Login, Anzeigename), bis er geschlossen wird.
    pub fn shoutout(&self) -> Option<(String, String)> {
        self.state.borrow().shoutout.clone()
    }

    pub fn clear_shoutout(&self) {
        self.set(|s| s.shoutout = None);
    }

    /// Kanal beigetreten: Schreiben möglich.
    pub fn joined(&self) -> bool {
        self.state.borrow().joined
    }

    /// Dauerhafter Fehler, zum Beispiel „Kanal nicht gefunden“.
    pub fn error(&self) -> Option<String> {
        self.state.borrow().error.clone()
    }

    /// Es gibt ältere Verlaufszeilen, die beim Hochscrollen erscheinen.
    pub fn has_older(&self) -> bool {
        self.state.borrow().buffer.has_older()
    }

    /// Alle Zeilen als JavaScript-Objekte, älteste zuerst (für die Chat-Liste; je Zeile zwischengespeichert, eine
    /// unveränderte Zeile ist dasselbe Objekt).
    pub fn items_js(&self) -> js_sys::Array {
        self.state.borrow().buffer.items().iter().map(Line::js).collect()
    }

    fn emit(&self) {
        let listeners: Vec<Listener> = {
            let mut state = self.state.borrow_mut();
            state.version = state.version.wrapping_add(1);
            state.listeners.iter().map(|(_, l)| l.clone()).collect()
        };
        for listener in listeners {
            listener();
        }
    }

    pub fn version(&self) -> u32 {
        self.state.borrow().version
    }

    pub fn subscribe(&self, listener: Listener) -> u32 {
        let id = next_id();
        self.state.borrow_mut().listeners.push((id, listener));
        id
    }

    pub fn unsubscribe(&self, id: u32) {
        self.state.borrow_mut().listeners.retain(|(i, _)| *i != id);
    }

    /// Alle Zeilen im Puffer, älteste zuerst. Eine geänderte Zeile ist ein neuer Wert (anderes `Rc`). Das Objekt für
    /// TypeScript entsteht nur, wenn `visit` es abruft, und wird je Zeile zwischengespeichert. `visit` darf die Sitzung
    /// nicht verändern.
    pub fn visit_lines(&self, mut visit: impl FnMut(&Rc<Value>, &dyn Fn() -> JsValue)) {
        let state = self.state.borrow();
        for line in state.buffer.items() {
            visit(&line.value, &|| line.js());
        }
    }

    fn set(&self, change: impl FnOnce(&mut State)) {
        change(&mut self.state.borrow_mut());
        self.emit();
    }

    /// Neue Zeile; sichtbar mit dem nächsten Bild (gebündelt). Neue Zeilen gehen auch an die Zuhörer aller Kanäle,
    /// der geladene Verlauf nie.
    fn push(self: &Rc<Self>, item: Value) {
        let line = Line::new(item);
        if text(&line.value, "kind") == "message" {
            self.state.borrow_mut().last_notice = None;
            // Namensfarbe für Namen in Aktionszeilen merken (crates/web/src/name_colors.rs), nur Twitch-Konten.
            if line.value.get("channel").and_then(|c| c.get("platform")).and_then(Value::as_str) == Some("twitch") {
                if let Some(sender) = line.value.get("sender") {
                    let id = sender.get("id").and_then(Value::as_str).unwrap_or_default();
                    crate::name_colors::remember(id, sender.get("color").and_then(Value::as_str).unwrap_or_default());
                }
            }
        }
        let from_history = text(&line.value, "kind") == "message"
            && line.value.get("flags").and_then(|f| f.get("fromHistory")).and_then(Value::as_bool) == Some(true);
        if !from_history {
            let rust_listeners: Vec<LiveListener> = RUST_LIVE_LISTENERS.with(|l| l.borrow().iter().map(|(_, f)| f.clone()).collect());
            for listener in rust_listeners {
                listener(&line.value);
            }
            let listeners: Vec<Function> = LIVE_LISTENERS.with(|l| l.borrow().iter().map(|(_, f)| f.clone()).collect());
            if !listeners.is_empty() {
                let js = line.js();
                for listener in listeners {
                    let _ = listener.call1(&JsValue::NULL, &js);
                }
            }
        }
        let schedule = {
            let mut state = self.state.borrow_mut();
            state.buffer.push(line);
            !std::mem::replace(&mut state.flush_scheduled, true)
        };
        if schedule {
            let weak = Rc::downgrade(self);
            let run = Closure::once_into_js(move || {
                if let Some(session) = weak.upgrade() {
                    session.flush();
                }
            });
            // Höchstens einmal pro Bild; ohne requestAnimationFrame (Tests) nach 16 ms.
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
    }

    fn flush(&self) {
        let changed = {
            let mut state = self.state.borrow_mut();
            state.flush_scheduled = false;
            state.buffer.flush()
        };
        if changed {
            self.emit();
        }
    }

    /// Ändert vorhandene Zeilen (zum Beispiel „gelöscht von …“ oder AutoMod-Status), auch wartende und ältere.
    fn update_items(&self, mut update: impl FnMut(&Value) -> Option<Value>) {
        let changed = self.state.borrow_mut().buffer.update(|line| update(&line.value).map(Line::new));
        if changed {
            self.emit();
        }
    }

    fn notice(self: &Rc<Self>, kind: &str, detail: Option<&str>) {
        let item = {
            let mut state = self.state.borrow_mut();
            // Dieselbe Meldung nicht doppelt hintereinander (zum Beispiel „Verbinde…“ von Kanal und Verbindung).
            let key = (kind.to_string(), detail.map(str::to_string));
            if state.last_notice.as_ref() == Some(&key) {
                return;
            }
            state.last_notice = Some(key);
            state.notice_counter += 1;
            let now = now_ms();
            let channel = state.channel.as_ref().map(to_value).unwrap_or_else(|| {
                json!({ "platform": "twitch", "id": "", "login": self.login, "displayName": self.login })
            });
            let mut item = json!({
                "kind": "system",
                "channel": channel,
                "id": format!("notice-{}-{}-{}", self.login, now, state.notice_counter),
                "timestamp": now,
                "type": kind,
            });
            if let Some(detail) = detail {
                item["detail"] = json!(detail);
            }
            item
        };
        self.push(item);
    }

    fn next_event_id(&self, prefix: &str) -> String {
        let mut state = self.state.borrow_mut();
        state.event_counter += 1;
        format!("{prefix}-{}-{}-{}", self.login, now_ms(), state.event_counter)
    }

    async fn start(self: Rc<Self>) {
        self.notice("connecting", None);
        let user = match api::get_user_by_login(&self.login).await {
            Ok(user) => user,
            Err(error) => {
                if self.disposed() {
                    return;
                }
                let message = error_message(&error, tr("Kanal konnte nicht geladen werden.", "Channel could not be loaded."));
                self.set(|s| s.error = Some(message));
                self.notice("disconnected", Some(tr("Kanal konnte nicht geladen werden", "Channel could not be loaded")));
                return;
            }
        };
        if self.disposed() {
            return;
        }
        let Some(user) = user else {
            let message = multiyap_core::tf!("Den Kanal „{}“ gibt es bei Twitch nicht.", "The channel “{}” does not exist on Twitch.", self.login);
            self.set(|s| s.error = Some(message));
            return;
        };
        let channel = ChannelRef {
            platform: "twitch".into(),
            id: text(&user, "id").into(),
            login: text(&user, "login").into(),
            display_name: text(&user, "display_name").into(),
        };
        let channel_id = channel.id.clone();

        // Abzeichen vor dem Beitritt laden, damit jede Nachricht sofort vollständig ist. Fehler sind nicht fatal.
        let (global, channel_badges) = futures::join!(api::get_global_badges(), api::get_channel_badges(&channel_id));
        if self.disposed() {
            return;
        }
        let store = badge_store(&channel_badges.unwrap_or(Value::Null), &global.unwrap_or(Value::Null));
        self.set(|s| {
            s.badges = Rc::new(store);
            s.channel = Some(channel.clone());
        });

        let listener = Rc::new(SessionListener(Rc::downgrade(&self)));
        let eventsub_listener = eventsub::client().add_channel(&channel_id, listener, md::can_moderate(&channel_id));
        // Mod-Ereignisse nur mit Mod-Rechten; Änderungen sofort übernehmen.
        let mod_channel = channel_id.clone();
        let mod_listener = md::subscribe_mod(Rc::new(move || eventsub::client().set_moderator(&mod_channel, md::can_moderate(&mod_channel))));
        // Live-Status und Stream-Info beobachten, solange der Kanal offen ist.
        streams::watch_channel(&channel_id);
        // Systemnachricht, wenn der Kanal live oder offline geht.
        let weak = Rc::downgrade(&self);
        let live_channel = channel_id.clone();
        let live_listener = streams::on_live_change(Rc::new(move |id: &str, live: bool| {
            if let Some(session) = weak.upgrade().filter(|s| id == live_channel && !s.disposed()) {
                session.notice(if live { "stream-online" } else { "stream-offline" }, None);
            }
        }));
        // „Erste Nachricht“ kommt nur über IRC (gleiche Nachrichten-ID wie bei EventSub).
        let weak = Rc::downgrade(&self);
        let irc_listener = irc_reader::reader().watch(
            &channel.login,
            Rc::new(move |message_id: &str| {
                if let Some(session) = weak.upgrade() {
                    session.mark_first_message(message_id);
                }
            }),
        );
        {
            let mut state = self.state.borrow_mut();
            state.eventsub_listener = Some(eventsub_listener);
            state.mod_listener = Some(mod_listener);
            state.watching_stream = true;
            state.live_listener = Some(live_listener);
            state.irc_listener = Some(irc_listener);
        }
        // Verlauf parallel laden; er wird vor die schon eingetroffenen Live-Nachrichten gesetzt.
        spawn_local(self.clone().load_history());
        // Chat-Modi laden (Plaketten in jedem Kanal); Änderungen kommen danach live über EventSub.
        spawn_local(self.clone().load_chat_modes());
    }

    /// Markiert eine Nachricht als erste Nachricht; kommt IRC vor EventSub an, wird die ID vorgemerkt.
    fn mark_first_message(&self, message_id: &str) {
        let found = self.state.borrow().buffer.contains(message_id);
        self.update_items(|item| {
            let already = item.get("flags").and_then(|f| f.get("firstMessage")).and_then(Value::as_bool) == Some(true);
            (text(item, "kind") == "message" && text(item, "id") == message_id && !already).then(|| {
                let mut next = item.clone();
                next["flags"]["firstMessage"] = json!(true);
                next
            })
        });
        if !found {
            self.state.borrow_mut().pending_first.insert(message_id.to_string(), ());
        }
    }

    async fn load_chat_modes(self: Rc<Self>) {
        let Some(channel) = self.channel() else { return };
        let moderator = md::can_moderate(&channel.id).then(|| auth::own_user_id().unwrap_or_default());
        // Ohne Plaketten weiter; die nächste Live-Änderung bringt den Stand.
        if let Ok(modes) = api::get_chat_settings(&channel.id, moderator.as_deref()).await {
            if !self.disposed() {
                self.set(|s| s.chat_modes = modes);
            }
        }
    }

    /// Verlauf beim Öffnen: neueste Nachrichten zuerst sichtbar, „Live ab hier“ danach.
    async fn load_history(self: Rc<Self>) {
        let Some(channel) = self.channel() else { return };
        let lines = match crate::net::fetch_recent_messages(&channel.login, None, !md::can_moderate(&channel.id)).await {
            Ok(lines) => lines,
            Err(_) => {
                if !self.disposed() {
                    self.notice("history-failed", None);
                }
                return;
            }
        };
        if self.disposed() {
            return;
        }
        let now = now_ms();
        let messages = self.build_history(&lines, &channel, now);
        remember_roles(&channel.id, &messages);
        let Some(last) = messages.last().and_then(|m| m.get("timestamp")).and_then(Value::as_f64) else { return };
        {
            let mut state = self.state.borrow_mut();
            state.last_timestamp = state.last_timestamp.max(last);
        }
        let marker = json!({
            "kind": "divider",
            "channel": to_value(&channel),
            "id": format!("live-{}-{}", channel.login, now),
            "timestamp": now,
            "variant": "live",
            "label": tr("Live ab hier", "Live from here"),
        });
        let mut items = with_date_dividers(messages.clone(), now);
        items.push(marker);
        let (had_pending, changed) = {
            let mut state = self.state.borrow_mut();
            let had_pending = state.buffer.has_pending();
            (had_pending, state.buffer.prepend_history(items.into_iter().map(Line::new).collect(), HISTORY_VISIBLE + 1))
        };
        if had_pending || changed {
            self.emit();
        }
        spawn_local(self.clone().resolve_history_bans(messages));
    }

    fn build_history(&self, lines: &[String], channel: &ChannelRef, now: f64) -> Vec<Value> {
        let badges = self.state.borrow().badges.clone();
        let own = auth::own_user_id().unwrap_or_default();
        multiyap_core::history::build_history(lines, channel, &badges, &own, now, crate::dark_theme(), gift_lines()).iter().map(|item: &HistoryItem| to_value(item)).collect()
    }

    /// Eigener Kanal: Mod und Grund zu Bans und Timeouts aus dem Verlauf über Get Banned Users ergänzen (Twitch liefert
    /// die Liste nur dem Broadcaster). Eine Anfrage je 100 Personen; schlägt sie fehl, bleibt es ohne Mod.
    async fn resolve_history_bans(self: Rc<Self>, history: Vec<Value>) {
        let Some(channel) = self.channel() else { return };
        if auth::own_user_id().as_deref() != Some(channel.id.as_str()) || !md::can_moderate(&channel.id) {
            return;
        }
        let open: Vec<Value> = history.into_iter().filter(cb::is_unresolved_history_ban).collect();
        if open.is_empty() {
            return;
        }
        let mut targets: Vec<String> = Vec::new();
        for action in &open {
            let id = text(action.get("target").unwrap_or(&Value::Null), "id").to_string();
            if !targets.contains(&id) {
                targets.push(id);
            }
        }
        let Ok(bans) = api::get_own_channel_bans(&channel.id, &targets).await else { return };
        if self.disposed() {
            return;
        }
        let resolved = cb::match_bans(&open, &bans);
        if resolved.is_empty() {
            return;
        }
        self.update_items(|item| {
            if text(item, "kind") == "mod-action" {
                return resolved.iter().find(|r| r.id() == item.id()).cloned();
            }
            // „Timeout von …“ bzw. „gebannt von …“ auch an den entfernten Nachrichten nachtragen.
            if text(item, "kind") != "message" {
                return None;
            }
            let sender = text(item.get("sender").unwrap_or(&Value::Null), "id");
            let action = resolved.iter().find(|a| text(a.get("target").unwrap_or(&Value::Null), "id") == sender)?;
            let removed = item.get("flags").and_then(|f| f.get("removed")).filter(|r| r.is_object())?;
            if has(removed, "by") || text(removed, "reason") != text(action, "action") {
                return None;
            }
            let (time, action_time) = (item.get("timestamp").and_then(Value::as_f64), action.get("timestamp").and_then(Value::as_f64));
            if time > action_time {
                return None;
            }
            let mut next = item.clone();
            next["flags"]["removed"]["by"] = json!(text(action.get("moderator").unwrap_or(&Value::Null), "displayName"));
            Some(next)
        });
    }

    /// Nach einem Reconnect verpasste Nachrichten nachladen. Doppelte werden verworfen.
    async fn fill_gap(self: Rc<Self>) {
        let Some(channel) = self.channel() else { return };
        let last = self.state.borrow().last_timestamp;
        if last == 0.0 {
            return;
        }
        let lines = match crate::net::fetch_recent_messages(&channel.login, Some(last - GAP_MARGIN_MS), !md::can_moderate(&channel.id)).await {
            Ok(lines) => lines,
            Err(_) => {
                if !self.disposed() {
                    self.notice("history-failed", None);
                }
                return;
            }
        };
        if self.disposed() {
            return;
        }
        let known = self.state.borrow().buffer.known_ids();
        let fresh: Vec<Value> = self.build_history(&lines, &channel, now_ms()).into_iter().filter(|item| !known.contains(item.id())).collect();
        remember_roles(&channel.id, &fresh);
        for item in &fresh {
            self.push(item.clone());
        }
        spawn_local(self.clone().resolve_history_bans(fresh));
    }

    fn handle_event(self: &Rc<Self>, kind: &str, event: &JsValue) {
        let Some(channel) = self.channel() else { return };
        let Ok(value) = serde_wasm_bindgen::from_value::<Value>(event.clone()) else { return };
        let now = now_ms();
        let dark = crate::dark_theme();
        let badges = self.state.borrow().badges.clone();
        let own = auth::own_user_id().unwrap_or_default();
        match kind {
            "channel.chat.message" => {
                let Ok(raw) = serde_json::from_value::<chat::ChatMessageEvent>(value) else { return };
                let mut message = to_value(&chat::convert_chat_message(&raw, &channel, &badges, &own, now, dark));
                let id = text(&message, "id").to_string();
                if self.state.borrow_mut().pending_first.remove(&id).is_some() {
                    message["flags"]["firstMessage"] = json!(true);
                }
                self.state.borrow_mut().last_timestamp = now;
                let own_message = message.get("flags").and_then(|f| f.get("own")).and_then(Value::as_bool) == Some(true);
                let shared = has(&message, "sourceChannel");
                // Eigene Abzeichen zeigen sofort, ob man hier (noch) Mod ist.
                if own_message && !shared {
                    md::report_own_badges(&channel.id, &badge_set_ids(&message), Some(&channel.login));
                }
                // Rollen für die Chatter-Liste merken (Abzeichen aus Shared Chat gehören zum anderen Kanal).
                if !shared {
                    md::note_badges(&channel.id, text(message.get("sender").unwrap_or(&Value::Null), "id"), &badge_set_ids(&message));
                }
                let suspicious = self.state.borrow_mut().pending_suspicious.remove(&id);
                if let Some(info) = suspicious {
                    // Die Meldung zur verdächtigen Person kam zuerst und steht schon als Ersatzzeile da: ersetzen.
                    message["flags"]["suspicious"] = info;
                    if self.state.borrow().buffer.contains(&id) {
                        let full = message;
                        self.update_items(|item| (item.id() == full.id()).then(|| full.clone()));
                        return;
                    }
                }
                self.push(message);
            }
            "channel.suspicious_user.message" => {
                if !md::can_moderate(&channel.id) {
                    return;
                }
                let Some(info) = ts::suspicious_info_of(&value) else { return };
                let info_value = to_value(&info);
                let user_id = text(&value, "user_id").to_string();
                md::set_known_suspicious(&channel.id, &user_id, Some(SuspiciousInfo { status: info.status.to_string(), reasons: info.reasons.clone() }));
                let message_id = text(value.get("message").unwrap_or(&Value::Null), "message_id").to_string();
                let found = self.state.borrow().buffer.contains(&message_id);
                self.update_items(|item| {
                    (text(item, "kind") == "message" && item.id() == message_id).then(|| {
                        let mut next = item.clone();
                        next["flags"]["suspicious"] = info_value.clone();
                        next
                    })
                });
                if found {
                    return;
                }
                // Noch keine normale Nachricht dazu: als Ersatzzeile zeigen und vormerken, falls sie noch kommt.
                self.state.borrow_mut().pending_suspicious.insert(message_id, info_value.clone());
                let Ok(raw) = serde_json::from_value::<chat::ChatMessageEvent>(ts::to_chat_message_event(&value)) else { return };
                let mut fallback = to_value(&chat::convert_chat_message(&raw, &channel, &badges, &own, now, dark));
                fallback["flags"]["suspicious"] = info_value;
                self.state.borrow_mut().last_timestamp = now;
                self.push(fallback);
            }
            "channel.suspicious_user.update" => {
                if !md::can_moderate(&channel.id) {
                    return;
                }
                let user_id = text(&value, "user_id").to_string();
                let status = ts::status_from_event(value.get("low_trust_status").and_then(Value::as_str));
                let previous = md::known_suspicious(&channel.id, &user_id);
                let previous_status = md::known_or_removed_status(&channel.id, &user_id);
                md::set_known_suspicious(
                    &channel.id,
                    &user_id,
                    status.map(|status| SuspiciousInfo { status: status.to_string(), reasons: previous.map(|p| p.reasons).unwrap_or_default() }),
                );
                let id = self.next_event_id("mod");
                self.push(to_value(&ts::convert_suspicious_update(&value, &channel, previous_status.as_deref(), now, &id)));
            }
            "channel.moderate" => {
                if !md::can_moderate(&channel.id) {
                    return;
                }
                let id = self.next_event_id("mod");
                let action = tm::convert_moderate(&value, &channel, now, &id);
                let item = to_value(&action);
                if !has(&item, "sourceChannel") {
                    remember_moderate_roles(&channel.id, &item);
                }
                let by = action.moderator.as_ref().map(|m| m.display_name.clone());
                let removed = |reason: &str, duration: Option<f64>| {
                    let mut removed = json!({ "reason": reason });
                    if let Some(by) = &by {
                        removed["by"] = json!(by);
                    }
                    if let Some(duration) = duration {
                        removed["durationSeconds"] = helix_api::number(duration);
                    }
                    removed
                };
                match tm::removal_of(&action, &value) {
                    Some(tm::Removal::Message { message_id }) => {
                        let removed = removed("deleted", None);
                        self.update_items(|i| if i.id() == message_id { mark_removed(i, &removed) } else { None });
                    }
                    Some(tm::Removal::User { user_id, reason }) => {
                        let duration = if reason == "timeout" { action.duration_seconds } else { None };
                        let removed = removed(&reason, duration);
                        self.update_items(|i| {
                            let sender = text(i.get("sender").unwrap_or(&Value::Null), "id");
                            if text(i, "kind") == "message" && sender == user_id { mark_removed(i, &removed) } else { None }
                        });
                    }
                    Some(tm::Removal::All) => {
                        let removed = removed("clear", None);
                        self.update_items(|i| mark_removed(i, &removed));
                    }
                    None => {}
                }
                self.push(item);
            }
            "channel.chat.notification" => {
                let Some(normalized) = events::normalize_notification(&value, dark) else { return };
                self.state.borrow_mut().last_timestamp = now;
                let item = events::to_event_item(&normalized, &channel, now, false);
                let placement = self.state.borrow_mut().gifts.place(&item, normalized.recipient.as_deref(), gift_lines());
                match placement {
                    GiftPlacement::New => self.push(to_value(&item)),
                    GiftPlacement::Attach { target_id } => {
                        let recipient = normalized.recipient.clone().unwrap_or_default();
                        self.update_items(|i| {
                            if text(i, "kind") != "event" || i.id() != target_id {
                                return None;
                            }
                            let mut target: ChatEventItem = serde_json::from_value(i.clone()).ok()?;
                            if target.recipients.as_ref().is_some_and(|r| r.contains(&recipient)) {
                                return None;
                            }
                            attach_recipient(&mut target, &recipient);
                            Some(to_value(&target))
                        });
                    }
                }
            }
            "channel.chat_settings.update" => {
                let previous = self.state.borrow().chat_modes.clone();
                let modes = helix_api::chat_modes_from_event(&value, previous.as_ref());
                self.set(|s| s.chat_modes = Some(modes));
            }
            "stream.online" => {
                streams::report_online(&channel.id, value.get("started_at").and_then(Value::as_str));
            }
            "automod.message.hold" => {
                if !md::can_moderate(&channel.id) {
                    return;
                }
                self.push(to_value(&tm::convert_automod_hold(&value, &channel, now, dark)));
            }
            // Shoutout aus dem Kanal: Zeile im Chat und oben rechts.
            "channel.shoutout.create" => {
                if !md::can_moderate(&channel.id) {
                    return;
                }
                let id = self.next_event_id("shoutout");
                let Some(item) = events::convert_shoutout(&value, &channel, now, &id) else { return };
                if let Some(target) = item.users.as_ref().and_then(|u| u.last()) {
                    let target = (target.login.clone(), target.display_name.clone());
                    self.set(|s| s.shoutout = Some(target));
                }
                self.push(to_value(&item));
            }
            // Ausgeführter Raid aus dem Kanal, mit „Raid beitreten“.
            "channel.raid" => {
                if text(&value, "from_broadcaster_user_id") != channel.id {
                    return;
                }
                let id = self.next_event_id("raid");
                if let Some(item) = events::convert_outgoing_raid(&value, &channel, now, &id) {
                    self.push(to_value(&item));
                }
            }
            // Eigene zurückgehaltene Nachricht ohne Mod-Rechte.
            "channel.chat.user_message_hold" => {
                if md::can_moderate(&channel.id) {
                    return;
                }
                self.push(to_value(&tm::convert_own_hold(&value, &channel, now, dark)));
            }
            // Bestätigte Verwarnung.
            "channel.warning.acknowledge" => {
                if !md::can_moderate(&channel.id) {
                    return;
                }
                let id = self.next_event_id("mod");
                self.push(to_value(&tm::convert_warning_acknowledge(&value, &channel, now, &id)));
            }
            "automod.message.update" | "channel.chat.user_message_update" => {
                let Some(status) = tm::automod_status_of(value.get("status").and_then(Value::as_str)) else { return };
                let message_id = text(&value, "message_id").to_string();
                let resolved_by = value.get("moderator_user_name").and_then(Value::as_str).map(str::to_string);
                self.update_items(|i| {
                    if text(i, "kind") != "automod" || i.id() != message_id {
                        return None;
                    }
                    let mut next = i.clone();
                    next["status"] = json!(status);
                    match &resolved_by {
                        Some(name) => next["resolvedBy"] = json!(name),
                        None => {
                            if let Some(object) = next.as_object_mut() {
                                object.remove("resolvedBy");
                            }
                        }
                    }
                    Some(next)
                });
            }
            _ => {}
        }
    }

    fn handle_status(self: &Rc<Self>, status: &ChannelStatus) {
        match status {
            ChannelStatus::Connecting => {
                self.set(|s| s.joined = false);
                self.notice("connecting", None);
            }
            ChannelStatus::Connected => self.notice("connected", None),
            ChannelStatus::ReconnectRequested => self.notice("reconnect-requested", None),
            ChannelStatus::Reconnected { reason } => {
                self.notice("reconnected", reason.as_deref().map(|r| multiyap_core::tf!("vorher getrennt: {r}", "previously disconnected: {r}")).as_deref());
                spawn_local(self.clone().fill_gap());
            }
            ChannelStatus::Joined => {
                self.set(|s| {
                    s.joined = true;
                    s.error = None;
                });
                self.notice("joined", None);
                // Emotes, Bits-Bilder und 7TV-Kosmetik dieses Kanals vorladen.
                if let Some(channel) = self.channel() {
                    crate::emotes::preload_channel(&channel.id);
                }
            }
            ChannelStatus::Disconnected { reason } => {
                self.set(|s| s.joined = false);
                self.notice("disconnected", reason.as_deref());
            }
            ChannelStatus::Error { reason } => {
                self.set(|s| s.joined = false);
                self.notice("disconnected", Some(reason));
            }
            ChannelStatus::ModEventsUnavailable { reason } => self.notice("mod-events-unavailable", Some(reason)),
            ChannelStatus::ModEventsRestored => self.notice("mod-events-restored", None),
        }
    }

    /// Sendet eine Nachricht, auf Wunsch als Antwort auf eine andere (Nachrichten-ID).
    pub async fn send(&self, text: &str, reply_to_id: Option<&str>) -> Result<api::SendResult, JsValue> {
        let Some(channel) = self.channel() else {
            return Ok(api::SendResult { sent: false, reason: Some(tr("Kanal ist noch nicht verbunden.", "Channel is not connected yet.").into()) });
        };
        api::send_chat_message(&channel.id, text, reply_to_id).await
    }

    /// Chat-Modus ändern (nur mit Mod-Rechten). Fehler als lesbare Meldung.
    pub async fn update_chat_modes(&self, patch: &Value) -> Result<(), String> {
        let Some(channel) = self.channel().filter(|c| md::can_moderate(&c.id)) else {
            return Err(tr("Keine Mod-Rechte in diesem Kanal.", "No mod rights in this channel.").into());
        };
        let modes = api::update_chat_settings(&channel.id, patch)
            .await
            .map_err(|error| error_message(&error, tr("Chat-Modus konnte nicht geändert werden.", "Chat mode could not be changed.")))?;
        if let Some(mut modes) = modes.filter(|_| !self.disposed()) {
            if !has(&modes, "delay") {
                if let Some(delay) = self.state.borrow().chat_modes.as_ref().and_then(|m| m.get("delay")).cloned() {
                    modes["delay"] = delay;
                }
            }
            self.set(|s| s.chat_modes = Some(modes));
        }
        Ok(())
    }

    /// Ältere Verlaufszeilen anzeigen (beim Hochscrollen).
    pub fn load_older(&self) {
        let changed = self.state.borrow_mut().buffer.load_older(OLDER_PAGE_SIZE);
        if changed {
            self.emit();
        }
    }

    pub fn reconnect(self: &Rc<Self>) {
        match self.channel() {
            Some(channel) => eventsub::client().resubscribe(&channel.id),
            None if !self.disposed() => {
                self.set(|s| s.error = None);
                spawn_local(self.clone().start());
            }
            None => {}
        }
    }

    fn dispose(&self) {
        let (channel, eventsub_listener, mod_listener, watching, live_listener, irc_listener) = {
            let mut s = self.state.borrow_mut();
            s.disposed = true;
            (s.channel.clone(), s.eventsub_listener.take(), s.mod_listener.take(), std::mem::take(&mut s.watching_stream), s.live_listener.take(), s.irc_listener.take())
        };
        let Some(channel) = channel else { return };
        if let Some(id) = eventsub_listener {
            eventsub::client().remove_channel(&channel.id, id);
        }
        if let Some(id) = mod_listener {
            md::unsubscribe_mod(id);
        }
        if watching {
            streams::unwatch_channel(&channel.id);
        }
        if let Some(id) = live_listener {
            streams::remove_live_listener(id);
        }
        if let Some(id) = irc_listener {
            irc_reader::reader().unwatch(&channel.login, id);
        }
    }

}

// ---- Liste der Sitzungen ----

#[derive(Default)]
struct Registry {
    /// In Reihenfolge des Öffnens (wie bisher die Map in TypeScript).
    sessions: Vec<(String, Rc<Session>)>,
    listeners: Vec<(u32, Listener)>,
    started: bool,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

fn emit_registry() {
    let listeners: Vec<Listener> = REGISTRY.with(|r| r.borrow().listeners.iter().map(|(_, l)| l.clone()).collect());
    for listener in listeners {
        listener();
    }
}

/// Offene Kanäle: nur angemeldet, aus den Kanal-Tabs (gleicher Kanal nur einmal).
fn wanted_logins() -> Vec<String> {
    if auth::own_user_id().is_none() {
        return Vec::new();
    }
    let mut logins: Vec<String> = Vec::new();
    for tab in workspace::state().tabs.iter().filter(|t| text(t, "kind") == "channel" && text(t, "platform") == "twitch") {
        let login = text(tab, "channel").to_string();
        if !logins.contains(&login) {
            logins.push(login);
        }
    }
    logins
}

/// Gleicht die Sitzungen mit den offenen Kanälen ab.
fn sync() {
    let wanted = wanted_logins();
    let (removed, added, empty) = REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        let (keep, removed): (Vec<_>, Vec<_>) = std::mem::take(&mut r.sessions).into_iter().partition(|(login, _)| wanted.contains(login));
        r.sessions = keep;
        let mut added = Vec::new();
        for login in &wanted {
            if !r.sessions.iter().any(|(l, _)| l == login) {
                let session = Session::new(login);
                r.sessions.push((login.clone(), session.clone()));
                added.push(session);
            }
        }
        (removed, added, r.sessions.is_empty())
    });
    for (_, session) in &removed {
        session.dispose();
    }
    for session in &added {
        spawn_local(session.clone().start());
    }
    if empty {
        eventsub::client().shutdown();
    }
    if !removed.is_empty() || !added.is_empty() {
        emit_registry();
    }
}

/// Verbindet Kanäle, solange angemeldet und Tabs offen sind; trennt alles beim Abmelden (einmal starten).
pub fn start() {
    if REGISTRY.with(|r| std::mem::replace(&mut r.borrow_mut().started, true)) {
        return;
    }
    workspace::subscribe(Rc::new(sync));
    auth::subscribe(Rc::new(sync));
    sync();
}

pub fn session(login: &str) -> Option<Rc<Session>> {
    REGISTRY.with(|r| r.borrow().sessions.iter().find(|(l, _)| l == login).map(|(_, s)| s.clone()))
}

pub fn logins() -> Vec<String> {
    REGISTRY.with(|r| r.borrow().sessions.iter().map(|(l, _)| l.clone()).collect())
}

pub fn subscribe_registry(listener: Listener) -> u32 {
    let id = next_id();
    REGISTRY.with(|r| r.borrow_mut().listeners.push((id, listener)));
    id
}

pub fn unsubscribe_registry(id: u32) {
    REGISTRY.with(|r| r.borrow_mut().listeners.retain(|(i, _)| *i != id));
}

/// Meldet jede neu eintreffende Zeile aller Kanäle (für Benachrichtigungen), nie den geladenen Verlauf.
pub fn on_live_item(listener: LiveListener) -> u32 {
    let id = next_id();
    RUST_LIVE_LISTENERS.with(|l| l.borrow_mut().push((id, listener)));
    id
}

/// Login des offenen Kanals mit dieser ID.
/// Systemmeldung im Chat (zum Beispiel „konnte nicht geladen werden“ oder 7TV-Änderungen): im Kanal `channel_id` oder
/// (None) in allen offenen Kanälen.
pub fn channel_notice(channel_id: Option<&str>, kind: &str, detail: &str) {
    let targets: Vec<Rc<Session>> = REGISTRY.with(|r| {
        r.borrow()
            .sessions
            .iter()
            .filter(|(_, s)| channel_id.is_none_or(|id| s.state.borrow().channel.as_ref().is_some_and(|c| c.id == id)))
            .map(|(_, s)| s.clone())
            .collect()
    });
    for session in targets.into_iter().filter(|s| !s.disposed()) {
        session.notice(kind, Some(detail));
    }
}

pub fn login_for_channel(channel_id: &str) -> Option<String> {
    REGISTRY.with(|r| {
        r.borrow().sessions.iter().find(|(_, s)| s.state.borrow().channel.as_ref().is_some_and(|c| c.id == channel_id)).map(|(login, _)| login.clone())
    })
}

/// Nachrichten der Person im Kanal aus dem geladenen Chat (für die Nutzerkarte), älteste zuerst, höchstens die
/// neuesten 50.
pub fn recent_messages(login: &str, user_id: &str) -> Vec<Value> {
    let Some(session) = session(login) else { return Vec::new() };
    let state = session.state.borrow();
    multiyap_core::chat_tools::recent_messages_of(
        state.buffer.items().iter().map(|line| &*line.value),
        user_id,
        multiyap_core::chat_tools::RECENT_MESSAGES_LIMIT,
    )
}

type Attached = Rc<RefCell<Option<(Rc<Session>, u32)>>>;

thread_local! {
    static WATCHES: RefCell<Vec<(u32, Attached)>> = const { RefCell::new(Vec::new()) };
}

/// Meldet Änderungen der Sitzung eines Kanals, auch wenn sie erst später entsteht oder wechselt. Gibt die Nummer zum
/// Abmelden zurück (`unwatch_login`).
pub fn watch_login(login: &str, listener: Listener) -> u32 {
    let attached: Attached = Rc::default();
    let attach = {
        let (attached, login, listener) = (attached.clone(), login.to_string(), listener.clone());
        Rc::new(move |notify: bool| {
            let next = session(&login);
            let same = match (attached.borrow().as_ref(), next.as_ref()) {
                (Some((current, _)), Some(next)) => Rc::ptr_eq(current, next),
                (None, None) => true,
                _ => false,
            };
            if same {
                return;
            }
            let previous = attached.borrow_mut().take();
            if let Some((old, id)) = previous {
                old.unsubscribe(id);
            }
            if let Some(next) = next {
                let id = next.subscribe(listener.clone());
                *attached.borrow_mut() = Some((next, id));
            }
            if notify {
                listener();
            }
        })
    };
    attach(false);
    let id = subscribe_registry(Rc::new(move || attach(true)));
    WATCHES.with(|w| w.borrow_mut().push((id, attached)));
    id
}

pub fn unwatch_login(id: u32) {
    unsubscribe_registry(id);
    let attached = WATCHES.with(|w| {
        let mut w = w.borrow_mut();
        let index = w.iter().position(|(i, _)| *i == id)?;
        Some(w.remove(index).1)
    });
    let previous = attached.and_then(|a| a.borrow_mut().take());
    if let Some((session, listener_id)) = previous {
        session.unsubscribe(listener_id);
    }
}

// ---- Für TypeScript (bis die übrigen Dienste in Rust sind) ----

