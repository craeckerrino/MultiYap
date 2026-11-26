//! Live-Status und Stream-Info aller offenen Kanäle (bisher src/core/streams/stream-status.ts). Nichts davon wird
//! gespeichert.
//! - Sofort: EventSub stream.online (soweit das Kostenbudget von Twitch reicht).
//! - Abgleich: „Get Streams“ für bis zu 100 Kanäle in einer Anfrage, jede Minute und beim Öffnen.
//! - „Zusammen mit“: „Get Shared Chat Session“, nur für Kanäle, die live sind.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use multiyap_core::streams::{self as s, StreamInfo};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::net::global_function;

const POLL_INTERVAL_MS: f64 = 60_000.0;
/// Kurz sammeln, bevor abgeglichen wird (mehrere Kanäle öffnen sich oft gleichzeitig).
const POLL_DELAY_MS: f64 = 500.0;

type Listener = Rc<dyn Fn()>;
type LiveListener = Rc<dyn Fn(&str, bool)>;
type WentLiveListener = Rc<dyn Fn(&str)>;

#[derive(Default)]
struct Store {
    infos: HashMap<String, StreamInfo>,
    /// Kanäle, deren Status schon einmal ermittelt wurde (erster Stand löst kein Pulsieren aus).
    known: HashSet<String>,
    /// Offene Kanäle mit Anzahl der Beobachter.
    watched: HashMap<String, u32>,
    timer: Option<JsValue>,
    poll_queued: bool,
    listeners: Vec<(u32, Listener)>,
    live_listeners: Vec<(u32, LiveListener)>,
    went_live_listeners: Vec<(u32, WentLiveListener)>,
    next_listener: u32,
    started: bool,
}

thread_local! {
    static STORE: RefCell<Store> = RefCell::new(Store::default());
    static VERSION: Cell<u32> = const { Cell::new(0) };
}

fn with<R>(f: impl FnOnce(&mut Store) -> R) -> R {
    STORE.with(|s| f(&mut s.borrow_mut()))
}

fn emit() {
    VERSION.with(|v| v.set(v.get().wrapping_add(1)));
    let listeners: Vec<Listener> = with(|s| s.listeners.iter().map(|(_, l)| l.clone()).collect());
    for listener in listeners {
        listener();
    }
}

/// Stand eines Kanals; unbekannte Kanäle gelten als offline.
pub fn info(channel_id: &str) -> StreamInfo {
    with(|s| s.infos.get(channel_id).cloned()).unwrap_or_default()
}

fn update(channel_id: &str, next: StreamInfo) {
    let merged = with(|st| {
        let previous = st.infos.get(channel_id).cloned().unwrap_or_default();
        let merged = s::merge(&previous, st.known.contains(channel_id), next);
        st.known.insert(channel_id.to_string());
        if merged.changed {
            st.infos.insert(channel_id.to_string(), merged.info.clone());
        }
        merged
    });
    if !merged.changed {
        return;
    }
    emit();
    if merged.went_live {
        let listeners: Vec<WentLiveListener> = with(|s| s.went_live_listeners.iter().map(|(_, l)| l.clone()).collect());
        for listener in listeners {
            listener(channel_id);
        }
    }
    if merged.flipped {
        let listeners: Vec<LiveListener> = with(|s| s.live_listeners.iter().map(|(_, l)| l.clone()).collect());
        for listener in listeners {
            listener(channel_id, merged.info.live);
        }
    }
}

/// Pulsieren beenden, sobald man in den Tab klickt.
pub fn acknowledge_live(channel_id: &str) {
    let changed = with(|s| match s.infos.get_mut(channel_id) {
        Some(info) if info.went_live => {
            info.went_live = false;
            true
        }
        _ => false,
    });
    if changed {
        emit();
    }
}

/// Sofortige Meldung von EventSub: Kanal ist live. Details kommen mit dem nächsten Abgleich.
pub fn report_online(channel_id: &str, started_at: Option<&str>) {
    let previous = info(channel_id);
    let started_at = started_at.and_then(multiyap_core::time::parse_rfc3339_ms).or(previous.started_at);
    update(channel_id, StreamInfo { live: true, started_at, ..previous });
    schedule_poll();
}

pub fn report_offline(channel_id: &str) {
    update(channel_id, StreamInfo::default());
}

async fn poll() {
    with(|s| s.poll_queued = false);
    if crate::auth::own_user_id().is_none() {
        return;
    }
    let ids: Vec<String> = with(|s| s.watched.keys().cloned().collect());
    for batch in ids.chunks(100) {
        // Twitch nicht erreichbar: bisherigen Stand behalten.
        let Ok(streams) = crate::api::get_streams(batch).await else { continue };
        for id in batch {
            let stream = streams.iter().find(|st| st.get("user_id").and_then(|v| v.as_str()) == Some(id.as_str()));
            match stream {
                None => update(id, StreamInfo::default()),
                Some(stream) => {
                    let shared = crate::api::get_shared_chat_participants(id).await.unwrap_or_default();
                    update(id, s::info_from_stream(stream, shared));
                }
            }
        }
    }
}

fn schedule_poll() {
    if with(|s| std::mem::replace(&mut s.poll_queued, true)) {
        return;
    }
    let run = Closure::once_into_js(|| spawn_local(poll()));
    if let Ok(set_timeout) = global_function("setTimeout") {
        let _ = set_timeout.call2(&JsValue::NULL, &run, &JsValue::from_f64(POLL_DELAY_MS));
    }
}

/// Kanal beobachten, solange er offen ist (mit `unwatch_channel` beenden).
pub fn watch_channel(channel_id: &str) {
    with(|s| *s.watched.entry(channel_id.to_string()).or_insert(0) += 1);
    schedule_poll();
    if with(|s| s.timer.is_none()) {
        let tick = Closure::<dyn Fn()>::new(|| spawn_local(poll())).into_js_value();
        let timer = global_function("setInterval").ok().and_then(|f| f.call2(&JsValue::NULL, &tick, &JsValue::from_f64(POLL_INTERVAL_MS)).ok());
        with(|s| s.timer = timer);
    }
}

pub fn unwatch_channel(channel_id: &str) {
    let stop_timer = with(|s| {
        let count = s.watched.get(channel_id).copied().unwrap_or(1).saturating_sub(1);
        if count > 0 {
            s.watched.insert(channel_id.to_string(), count);
            return None;
        }
        s.watched.remove(channel_id);
        s.infos.remove(channel_id);
        s.known.remove(channel_id);
        Some(if s.watched.is_empty() { s.timer.take() } else { None })
    });
    let Some(timer) = stop_timer else { return };
    if let (Some(timer), Ok(clear)) = (timer, global_function("clearInterval")) {
        let _ = clear.call1(&JsValue::NULL, &timer);
    }
    emit();
}

/// Beim Abmelden alles vergessen, beim Anmelden abgleichen (einmal starten).
pub fn start() {
    if with(|s| std::mem::replace(&mut s.started, true)) {
        return;
    }
    crate::auth::subscribe(Rc::new(|| {
        if crate::auth::own_user_id().is_none() {
            with(|s| {
                s.infos.clear();
                s.known.clear();
            });
            emit();
        } else {
            schedule_poll();
        }
    }));
}

pub fn version() -> u32 {
    VERSION.with(Cell::get)
}

fn next_id(s: &mut Store) -> u32 {
    let id = s.next_listener;
    s.next_listener += 1;
    id
}

pub fn subscribe(listener: Listener) -> u32 {
    with(|s| {
        let id = next_id(s);
        s.listeners.push((id, listener));
        id
    })
}

pub fn unsubscribe(id: u32) {
    with(|s| s.listeners.retain(|(i, _)| *i != id));
}

/// Meldet jeden Wechsel zwischen live und offline eines offenen Kanals (für die Systemnachricht im Chat). Der erste
/// bekannte Stand meldet nichts.
pub fn on_live_change(listener: LiveListener) -> u32 {
    with(|s| {
        let id = next_id(s);
        s.live_listeners.push((id, listener));
        id
    })
}

/// Meldet, wenn ein offener Kanal live geht (für den Live-Ton). Der erste bekannte Stand meldet nichts.
pub fn on_went_live(listener: WentLiveListener) -> u32 {
    with(|s| {
        let id = next_id(s);
        s.went_live_listeners.push((id, listener));
        id
    })
}

pub fn remove_live_listener(id: u32) {
    with(|s| {
        s.live_listeners.retain(|(i, _)| *i != id);
        s.went_live_listeners.retain(|(i, _)| *i != id);
    });
}

// ---- Für TypeScript (bis die übrigen Dienste in Rust sind) ----

