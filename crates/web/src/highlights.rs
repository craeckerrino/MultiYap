//! Hervorhebungen-Tab (bisher src/features/highlights-tab/feed.ts): sammelt passende Zeilen aus allen offenen Kanälen.
//! Nur im Arbeitsspeicher, nichts wird gespeichert. Ein Treffer bleibt auch dann sichtbar, wenn er im Kanal aus dem
//! Puffer fällt; Änderungen (gelöscht, AutoMod entschieden) werden übernommen, solange er im Kanal ist. Erwähnungen und
//! Filter erkennt dieselbe Erkennung wie bei Benachrichtigungen und in der Chat-Liste
//! (crates/web/src/notifications.rs); Zuordnung und Anzeige-Auswahl in crates/core/src/highlights.rs. YouTube-Kanäle
//! (crates/web/src/youtube.rs) laufen durch dieselbe Prüfung; ihr „Login“ ist der Kanal-Schlüssel.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use multiyap_core::highlights::{self as hl, HighlightMatch, MAX_FEED_ITEMS};
use multiyap_core::notify::Matcher;
use serde_json::Value;
use wasm_bindgen::prelude::*;

use crate::sessions::{self, Session};
use crate::{moderation, notifications, youtube};

/// Besucht alle Zeilen eines Kanals: Wert und JavaScript-Objekt bei Bedarf.
type Visit<'a> = dyn FnMut(&Rc<Value>, &dyn Fn() -> JsValue) + 'a;

type Listener = Rc<dyn Fn()>;

fn text<'a>(value: &'a Value, name: &str) -> &'a str {
    value.get(name).and_then(Value::as_str).unwrap_or_default()
}

fn channel_field<'a>(item: &'a Value, name: &str) -> &'a str {
    item.get("channel").map(|c| text(c, name)).unwrap_or_default()
}

/// Ein Treffer im Hervorhebungen-Tab.
pub struct Entry {
    pub value: Rc<Value>,
    /// Dasselbe Objekt wie im Kanal (für die Chat-Liste in React).
    pub js: JsValue,
    pub found: HighlightMatch,
    /// Laufende Nummer beim ersten Auftauchen (für den Zähler ungesehener Treffer).
    pub seq: u64,
}

impl Entry {
    fn timestamp(&self) -> f64 {
        self.value.get("timestamp").and_then(Value::as_f64).unwrap_or(0.0)
    }
}

#[derive(Default)]
struct Feed {
    entries: HashMap<String, Rc<Entry>>,
    /// Zuletzt geprüfte Zeilen je Kanal (Zeilen sind unveränderlich, eine Änderung ist ein neuer Wert).
    checked: HashMap<String, HashMap<String, Rc<Value>>>,
    attached: HashMap<String, (Rc<Session>, u32)>,
    snapshot: Rc<Vec<Rc<Entry>>>,
    next_seq: u64,
    /// Zuletzt gesehene laufende Nummer je Tab (nur für die Sitzung).
    seen_up_to: HashMap<String, u64>,
    listeners: Vec<(u32, Listener)>,
    next_listener: u32,
    version: u32,
    started: bool,
}

thread_local! {
    static FEED: RefCell<Feed> = RefCell::new(Feed { next_seq: 1, ..Feed::default() });
}

fn emit(rebuild: bool) {
    let listeners: Vec<Listener> = FEED.with(|f| {
        let mut f = f.borrow_mut();
        if rebuild {
            let mut list: Vec<Rc<Entry>> = f.entries.values().cloned().collect();
            list.sort_by(|a, b| a.timestamp().total_cmp(&b.timestamp()).then(a.seq.cmp(&b.seq)));
            f.snapshot = Rc::new(list);
        }
        f.version = f.version.wrapping_add(1);
        f.listeners.iter().map(|(_, l)| l.clone()).collect()
    });
    for listener in listeners {
        listener();
    }
}

fn classify(matcher: &Matcher, item: &Value) -> Option<HighlightMatch> {
    hl::classify(item, |message| {
        let is_moderator = moderation::can_moderate(channel_field(message, "id"));
        (matcher.is_mention(message), matcher.term_category(message, is_moderator).map(|c| text(c, "id").to_string()))
    })
}

impl Feed {
    /// Neue oder geänderte Zeilen eines Kanals übernehmen. Gibt zurück, ob sich etwas geändert hat.
    fn scan(&mut self, login: &str, session: &Session, matcher: &Matcher) -> bool {
        self.scan_lines(login, |visit| session.visit_lines(visit), matcher)
    }

    /// Wie `scan`, für einen YouTube-Kanal.
    fn scan_youtube(&mut self, key: &str, matcher: &Matcher) -> bool {
        self.scan_lines(key, |visit| youtube::visit_lines(key, visit), matcher)
    }

    fn scan_lines(&mut self, login: &str, lines: impl FnOnce(&mut Visit), matcher: &Matcher) -> bool {
        let previous = self.checked.remove(login).unwrap_or_default();
        let mut current = HashMap::with_capacity(previous.len());
        let mut changed = false;
        lines(&mut |value, js| {
            let id = text(value, "id");
            current.insert(id.to_string(), value.clone());
            if previous.get(id).is_some_and(|known| Rc::ptr_eq(known, value)) {
                return;
            }
            match classify(matcher, value) {
                None => changed |= self.entries.remove(id).is_some(),
                Some(found) => {
                    let seq = match self.entries.get(id) {
                        Some(existing) => existing.seq,
                        None => {
                            self.next_seq += 1;
                            self.next_seq - 1
                        }
                    };
                    self.entries.insert(id.to_string(), Rc::new(Entry { value: value.clone(), js: js(), found, seq }));
                    changed = true;
                }
            }
        });
        self.checked.insert(login.to_string(), current);
        if self.entries.len() > MAX_FEED_ITEMS {
            let mut oldest: Vec<(f64, u64, String)> = self.entries.iter().map(|(id, e)| (e.timestamp(), e.seq, id.clone())).collect();
            oldest.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            for (_, _, id) in oldest.into_iter().take(self.entries.len() - MAX_FEED_ITEMS) {
                self.entries.remove(&id);
            }
        }
        changed
    }
}

fn scan_session(login: &str) {
    let Some(session) = sessions::session(login) else { return };
    let matcher = notifications::matcher();
    if FEED.with(|f| f.borrow_mut().scan(login, &session, &matcher)) {
        emit(true);
    }
}

/// Sitzungen abgleichen: neue beobachten, Zeilen geschlossener Kanäle entfernen. YouTube-Kanäle werden jedes Mal
/// durchgesehen (unveränderte Zeilen kosten nur einen Zeigervergleich).
fn sync_sessions() {
    let twitch = sessions::logins();
    let youtube_keys = youtube::keys();
    let open: Vec<String> = twitch.iter().chain(youtube_keys.iter()).cloned().collect();
    let (detached, added, mut changed) = FEED.with(|f| {
        let mut f = f.borrow_mut();
        let closed: Vec<String> = f.attached.keys().filter(|login| !open.contains(login)).cloned().collect();
        let detached: Vec<(Rc<Session>, u32)> = closed.iter().filter_map(|login| f.attached.remove(login)).collect();
        f.checked.retain(|login, _| open.contains(login));
        let before = f.entries.len();
        f.entries.retain(|_, entry| open.iter().any(|login| login == channel_field(&entry.value, "login")));
        let added: Vec<String> = twitch.iter().filter(|login| !f.attached.contains_key(*login)).cloned().collect();
        (detached, added, f.entries.len() != before)
    });
    for (session, id) in detached {
        session.unsubscribe(id);
    }
    let matcher = notifications::matcher();
    for login in added {
        let Some(session) = sessions::session(&login) else { continue };
        let watched = login.clone();
        let id = session.subscribe(Rc::new(move || scan_session(&watched)));
        changed |= FEED.with(|f| {
            let mut f = f.borrow_mut();
            f.attached.insert(login.clone(), (session.clone(), id));
            f.scan(&login, &session, &matcher)
        });
    }
    changed |= FEED.with(|f| {
        let mut f = f.borrow_mut();
        youtube_keys.iter().fold(false, |changed, key| f.scan_youtube(key, &matcher) | changed)
    });
    if changed {
        emit(true);
    }
}

/// Einstellungen, Anmeldung oder Mod-Rechte haben sich geändert: alles neu bewerten.
fn rescan_all() {
    let matcher = notifications::matcher();
    let sessions: Vec<(String, Rc<Session>)> = FEED.with(|f| f.borrow().attached.iter().map(|(l, (s, _))| (l.clone(), s.clone())).collect());
    FEED.with(|f| {
        let mut f = f.borrow_mut();
        let ids: Vec<String> = f.entries.keys().cloned().collect();
        for id in ids {
            let Some(entry) = f.entries.get(&id).cloned() else { continue };
            match classify(&matcher, &entry.value) {
                None => {
                    f.entries.remove(&id);
                }
                Some(found) if found != entry.found => {
                    f.entries.insert(id, Rc::new(Entry { value: entry.value.clone(), js: entry.js.clone(), found, seq: entry.seq }));
                }
                Some(_) => {}
            }
        }
        f.checked.clear();
        for (login, session) in &sessions {
            f.scan(login, session, &matcher);
        }
        for key in youtube::keys() {
            f.scan_youtube(&key, &matcher);
        }
    });
    // Auch ohne geänderte Treffer neu zeichnen: Mod-Aktionen und AutoMod zeigt der Tab nur in Mod-Kanälen.
    emit(true);
}

/// Startet die Sammlung (einmal).
pub fn start() {
    if FEED.with(|f| std::mem::replace(&mut f.borrow_mut().started, true)) {
        return;
    }
    sessions::subscribe_registry(Rc::new(sync_sessions));
    youtube::subscribe(Rc::new(sync_sessions));
    notifications::subscribe_matcher(Rc::new(rescan_all));
    moderation::subscribe_mod(Rc::new(rescan_all));
    sync_sessions();
}

/// Alle Treffer, älteste zuerst.
pub fn entries() -> Rc<Vec<Rc<Entry>>> {
    FEED.with(|f| f.borrow().snapshot.clone())
}

pub fn version() -> u32 {
    FEED.with(|f| f.borrow().version)
}

pub fn subscribe(listener: Listener) -> u32 {
    FEED.with(|f| {
        let mut f = f.borrow_mut();
        f.next_listener += 1;
        let id = f.next_listener;
        f.listeners.push((id, listener));
        id
    })
}

pub fn unsubscribe(id: u32) {
    FEED.with(|f| f.borrow_mut().listeners.retain(|(i, _)| *i != id));
}

/// Gehört der Treffer in einen Tab mit dieser Anzeige-Auswahl? Mod-Aktionen und AutoMod wie im Chat nur aus Mod-Kanälen.
pub fn shown(entry: &Entry, show: &Value) -> bool {
    if matches!(entry.found, HighlightMatch::ModAction | HighlightMatch::Automod) && !moderation::can_moderate(channel_field(&entry.value, "id")) {
        return false;
    }
    hl::shown_in(&entry.found, channel_field(&entry.value, "login"), show)
}

/// Alles bis jetzt gilt in diesem Tab als gesehen.
pub fn mark_seen(tab_id: &str) {
    let changed = FEED.with(|f| {
        let mut f = f.borrow_mut();
        let latest = f.next_seq - 1;
        f.seen_up_to.insert(tab_id.to_string(), latest) != Some(latest)
    });
    if changed {
        emit(false);
    }
}

/// Zähler ungesehener Treffer. Treffer aus dem geladenen Verlauf zählen nicht. Ein Tab, der noch nie gesehen wurde,
/// beginnt beim aktuellen Stand.
pub fn unseen_count(tab_id: &str, show: &Value) -> u32 {
    let (seen, list) = FEED.with(|f| {
        let mut f = f.borrow_mut();
        let latest = f.next_seq - 1;
        (*f.seen_up_to.entry(tab_id.to_string()).or_insert(latest), f.snapshot.clone())
    });
    list.iter().filter(|e| e.seq > seen && !hl::from_history(&e.value) && shown(e, show)).count() as u32
}

// ---- Für TypeScript (Hervorhebungen-Tab und Tab-Eintrag sind noch React) ----

