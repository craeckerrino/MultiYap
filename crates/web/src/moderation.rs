//! Mod-Status, Rollen anderer Personen und verdächtige Nutzer je Kanal (bisher src/core/moderation/mod-status.ts,
//! channel-roles.ts und suspicious.ts). Alles nur im Arbeitsspeicher für die laufende Sitzung, nichts wird
//! gespeichert, keine Nachrichteninhalte.
//!
//! Mod-Status ist die zentrale Prüfung der Mod-Rechte: Jedes moderative Element fragt ausschließlich `can_moderate`,
//! nie verstreut. Quellen: eigener Kanal (Broadcaster), Twitch „Get Moderated Channels“ (beim Anmelden und alle 10
//! Minuten), eigene Abzeichen in eigenen Nachrichten (sofort bei Mod oder Demod).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use multiyap_core::roles::{self, KnownRole, OrderedMap};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::net::global_function;

const REFRESH_INTERVAL_MS: f64 = 10.0 * 60.0 * 1000.0;
/// Obergrenze der gemerkten Rollen je Kanal, damit sehr große Chats den Speicher nicht füllen.
const MAX_ROLES_PER_CHANNEL: usize = 50_000;
/// Höchstens so viele zuletzt entfernte Status verdächtiger Nutzer.
const MAX_REMOVED_SUSPICIOUS: usize = 200;

type Listener = Rc<dyn Fn()>;

#[derive(Default)]
struct Listeners {
    list: Vec<(u32, Listener)>,
    next: u32,
}

impl Listeners {
    fn add(&mut self, listener: Listener) -> u32 {
        let id = self.next;
        self.next += 1;
        self.list.push((id, listener));
        id
    }

    fn remove(&mut self, id: u32) {
        self.list.retain(|(i, _)| *i != id);
    }

    fn all(&self) -> Vec<Listener> {
        self.list.iter().map(|(_, l)| l.clone()).collect()
    }
}

// ---- Mod-Status ----

#[derive(Default)]
struct ModStatus {
    own_user_id: Option<String>,
    moderated: HashSet<String>,
    /// Erkenntnis aus eigenen Abzeichen, gilt bis zur nächsten Abfrage der Liste.
    overrides: HashMap<String, bool>,
    /// Kanäle (Login), in denen das eigene Konto VIP ist (aus IRC USERSTATE und eigenen Nachrichten).
    vip_logins: HashSet<String>,
    refresh_timer: Option<JsValue>,
    version: u32,
    listeners: Listeners,
    started: bool,
}

/// Suchschlüssel Kanal und Person.
fn key(channel_id: &str, user_id: &str) -> String {
    format!("{channel_id}:{user_id}")
}

/// Bekannter Status einer verdächtigen Person: "monitored" oder "restricted", dazu lesbare Gründe.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SuspiciousInfo {
    pub status: String,
    pub reasons: Vec<String>,
}

struct Suspicious {
    known: HashMap<String, SuspiciousInfo>,
    /// Zuletzt entfernter Status: Entfernt man ihn selbst, kommt die Meldung von Twitch erst danach und soll trotzdem
    /// sagen können, was aufgehoben wurde.
    removed: OrderedMap<String, String>,
    version: u32,
    listeners: Listeners,
}

thread_local! {
    static MOD: RefCell<ModStatus> = RefCell::new(ModStatus::default());
    static ROLES: RefCell<HashMap<String, OrderedMap<String, KnownRole>>> = RefCell::new(HashMap::new());
    static SUSPICIOUS: RefCell<Suspicious> = RefCell::new(Suspicious {
        known: HashMap::new(),
        removed: OrderedMap::new(MAX_REMOVED_SUSPICIOUS),
        version: 0,
        listeners: Listeners::default(),
    });
}

fn emit_mod() {
    let listeners = MOD.with(|m| {
        let mut m = m.borrow_mut();
        m.version = m.version.wrapping_add(1);
        m.listeners.all()
    });
    for listener in listeners {
        listener();
    }
}

/// Die zentrale Prüffunktion: darf das angemeldete Konto in diesem Kanal moderieren?
pub fn can_moderate(channel_id: &str) -> bool {
    MOD.with(|m| {
        let m = m.borrow();
        let Some(own) = &m.own_user_id else { return false };
        if channel_id.is_empty() {
            return false;
        }
        if channel_id == own {
            return true;
        }
        m.overrides.get(channel_id).copied().unwrap_or_else(|| m.moderated.contains(channel_id))
    })
}

/// Ist das angemeldete Konto der Broadcaster dieses Kanals?
pub fn is_broadcaster(channel_id: &str) -> bool {
    !channel_id.is_empty() && MOD.with(|m| m.borrow().own_user_id.as_deref() == Some(channel_id))
}

/// Ist das eigene Konto in diesem Kanal VIP? (Twitch liefert das nicht über die API.)
pub fn is_own_vip(channel_login: &str) -> bool {
    !channel_login.is_empty() && MOD.with(|m| m.borrow().vip_logins.contains(&channel_login.to_lowercase()))
}

/// Eigene Abzeichen in einem Kanal melden (aus IRC USERSTATE oder eigenen Nachrichten).
pub fn report_own_channel_badges(channel_login: &str, badge_set_ids: &[String]) {
    let login = channel_login.to_lowercase();
    let vip = badge_set_ids.iter().any(|b| b == "vip");
    let changed = MOD.with(|m| {
        let mut m = m.borrow_mut();
        if vip { m.vip_logins.insert(login) } else { m.vip_logins.remove(&login) }
    });
    if changed {
        emit_mod();
    }
}

/// Wird bei jeder eigenen Nachricht mit deren Abzeichen aufgerufen.
pub fn report_own_badges(channel_id: &str, badge_set_ids: &[String], channel_login: Option<&str>) {
    if let Some(login) = channel_login {
        report_own_channel_badges(login, badge_set_ids);
    }
    let is_mod = roles::has_mod_badge(badge_set_ids);
    if can_moderate(channel_id) == is_mod {
        return;
    }
    MOD.with(|m| m.borrow_mut().overrides.insert(channel_id.to_string(), is_mod));
    emit_mod();
}

/// Neu abfragen, zum Beispiel beim Öffnen eines Kanals. Twitch nicht erreichbar: bisheriger Stand bleibt.
pub async fn refresh() {
    let Some(user_id) = MOD.with(|m| m.borrow().own_user_id.clone()) else { return };
    let Ok(ids) = crate::api::get_moderated_channels(&user_id).await else { return };
    let applied = MOD.with(|m| {
        let mut m = m.borrow_mut();
        if m.own_user_id.as_deref() != Some(user_id.as_str()) {
            return false;
        }
        m.moderated = ids.into_iter().collect();
        m.overrides.clear();
        true
    });
    if applied {
        emit_mod();
    }
}

fn clear_refresh_timer() {
    let timer = MOD.with(|m| m.borrow_mut().refresh_timer.take());
    if let (Some(timer), Ok(clear)) = (timer, global_function("clearInterval")) {
        let _ = clear.call1(&JsValue::NULL, &timer);
    }
}

/// Folgt An- und Abmeldung: neues Konto setzt alles zurück und fragt die Liste ab.
fn sync_with_auth() {
    let user_id = crate::auth::own_user_id();
    if MOD.with(|m| m.borrow().own_user_id == user_id) {
        return;
    }
    clear_refresh_timer();
    MOD.with(|m| {
        let mut m = m.borrow_mut();
        m.own_user_id = user_id.clone();
        m.moderated.clear();
        m.overrides.clear();
        m.vip_logins.clear();
    });
    emit_mod();
    if user_id.is_some() {
        spawn_local(refresh());
        let tick = Closure::<dyn Fn()>::new(|| spawn_local(refresh())).into_js_value();
        let timer = global_function("setInterval").ok().and_then(|f| f.call2(&JsValue::NULL, &tick, &JsValue::from_f64(REFRESH_INTERVAL_MS)).ok());
        MOD.with(|m| m.borrow_mut().refresh_timer = timer);
    }
}

/// Startet die Erkennung und folgt An- und Abmeldung (einmal).
pub fn start() {
    if MOD.with(|m| std::mem::replace(&mut m.borrow_mut().started, true)) {
        return;
    }
    crate::auth::subscribe(Rc::new(sync_with_auth));
    sync_with_auth();
}

pub fn mod_version() -> u32 {
    MOD.with(|m| m.borrow().version)
}

pub fn subscribe_mod(listener: Listener) -> u32 {
    MOD.with(|m| m.borrow_mut().listeners.add(listener))
}

pub fn unsubscribe_mod(id: u32) {
    MOD.with(|m| m.borrow_mut().listeners.remove(id));
}

// ---- Rollen anderer Personen ----

/// Rolle setzen oder (None) entfernen. Neueste Angabe gilt.
pub fn note_role(channel_id: &str, user_id: &str, role: Option<KnownRole>) {
    ROLES.with(|r| {
        let mut r = r.borrow_mut();
        let map = r.entry(channel_id.to_string()).or_insert_with(|| OrderedMap::new(MAX_ROLES_PER_CHANNEL));
        match role {
            Some(role) => {
                map.insert(user_id.to_string(), role);
            }
            None => {
                map.remove(&user_id.to_string());
            }
        }
    });
}

pub fn note_badges(channel_id: &str, user_id: &str, badge_set_ids: &[String]) {
    note_role(channel_id, user_id, roles::role_from_badges(badge_set_ids));
}

/// Bekannte Mods und VIPs eines Kanals.
#[derive(Clone, Debug, Default, Serialize)]
pub struct KnownRoles {
    pub moderators: Vec<String>,
    pub vips: Vec<String>,
}

pub fn known_channel_roles(channel_id: &str) -> KnownRoles {
    ROLES.with(|r| {
        let mut known = KnownRoles::default();
        if let Some(map) = r.borrow().get(channel_id) {
            for (user, role) in map.iter() {
                match role {
                    KnownRole::Moderator => known.moderators.push(user.clone()),
                    KnownRole::Vip => known.vips.push(user.clone()),
                }
            }
        }
        known
    })
}

/// Rollen für die Chatter-Liste in fremden Kanälen: erkannte Rollen und die eigene Rolle.
pub fn known_chatter_roles(channel_id: &str, channel_login: &str) -> KnownRoles {
    let mut known = known_channel_roles(channel_id);
    if let Some(own) = crate::auth::own_user_id().filter(|own| own != channel_id) {
        if can_moderate(channel_id) {
            if !known.moderators.contains(&own) {
                known.moderators.push(own);
            }
        } else if is_own_vip(channel_login) && !known.vips.contains(&own) {
            known.vips.push(own);
        }
    }
    known
}

// ---- Verdächtige Nutzer ----

/// Setzt den bekannten Status; None entfernt ihn (und merkt sich, was entfernt wurde).
pub fn set_known_suspicious(channel_id: &str, user_id: &str, info: Option<SuspiciousInfo>) {
    let k = key(channel_id, user_id);
    let changed = SUSPICIOUS.with(|s| {
        let mut s = s.borrow_mut();
        match info {
            Some(info) => {
                s.known.insert(k.clone(), info);
                s.removed.remove(&k);
            }
            None => {
                let Some(previous) = s.known.remove(&k) else { return false };
                s.removed.insert(k, previous.status);
            }
        }
        s.version = s.version.wrapping_add(1);
        true
    });
    if changed {
        let listeners = SUSPICIOUS.with(|s| s.borrow().listeners.all());
        for listener in listeners {
            listener();
        }
    }
}

pub fn known_suspicious(channel_id: &str, user_id: &str) -> Option<SuspiciousInfo> {
    SUSPICIOUS.with(|s| s.borrow().known.get(&key(channel_id, user_id)).cloned())
}

/// Bekannter Status oder, falls gerade entfernt, der Status davor.
pub fn known_or_removed_status(channel_id: &str, user_id: &str) -> Option<String> {
    let k = key(channel_id, user_id);
    SUSPICIOUS.with(|s| {
        let s = s.borrow();
        s.known.get(&k).map(|i| i.status.clone()).or_else(|| s.removed.get(&k).cloned())
    })
}

pub fn suspicious_version() -> u32 {
    SUSPICIOUS.with(|s| s.borrow().version)
}

pub fn subscribe_suspicious(listener: Listener) -> u32 {
    SUSPICIOUS.with(|s| s.borrow_mut().listeners.add(listener))
}

pub fn unsubscribe_suspicious(id: u32) {
    SUSPICIOUS.with(|s| s.borrow_mut().listeners.remove(id));
}

// ---- Für TypeScript (bis die übrigen Dienste in Rust sind) ----

