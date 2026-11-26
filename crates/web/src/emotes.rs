//! Emote-Dienste (bisher src/emotes/twitch-emotes.ts, third-party/store.ts, cheermotes.ts und groups.ts):
//! - Twitch-Emotes, die das angemeldete Konto benutzen kann (Doku geprüft am 15.11.2026): „Get User Emotes“ (alle
//!   Seiten) und „Get Global Emotes“ einmal pro Sitzung, Follower-Emotes eines Kanals einmal pro Kanal.
//! - Emotes von 7TV, BTTV und FFZ: je Quelle einmal pro Sitzung (global einmal, Kanal einmal je Kanal). Ein
//!   abgeschalteter Dienst wird nie abgefragt. Bei gleichen Namen gilt: Kanal vor global, innerhalb dessen 7TV vor BTTV
//!   vor FFZ.
//! - Bits-Bilder (Get Cheermotes) einmal je Kanal und Sitzung.
//! - Zuletzt benutzte Emotes (Einstellung „emotes.recent“, nur Name und Bild).
//!
//! Umwandeln und Prüfen in crates/core/src/emotes.rs; Anfragen an Twitch über crates/web/src/api.rs, an die
//! Drittanbieter ohne Zugangsdaten über `net::fetch_json`.

use multiyap_core::i18n::tr;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use multiyap_core::emotes as ce;
use multiyap_core::model::Emote;
use futures::future::{FutureExt, LocalBoxFuture, Shared};
use serde::Serialize;
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::net::{fetch_json, helix_error};

type Listener = Rc<dyn Fn()>;
type SharedList = Shared<LocalBoxFuture<'static, Result<Rc<Vec<Emote>>, JsValue>>>;

#[derive(Default)]
struct Listeners {
    list: Vec<(u32, Listener)>,
    next: u32,
    version: u32,
}

impl Listeners {
    fn add(&mut self, listener: Listener) -> u32 {
        self.next += 1;
        self.list.push((self.next, listener));
        self.next
    }

    fn remove(&mut self, id: u32) {
        self.list.retain(|(i, _)| *i != id);
    }
}

fn emit(cell: &'static std::thread::LocalKey<RefCell<Listeners>>) {
    let listeners: Vec<Listener> = cell.with(|l| {
        let mut l = l.borrow_mut();
        l.version = l.version.wrapping_add(1);
        l.list.iter().map(|(_, f)| f.clone()).collect()
    });
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

// ---- Twitch-Emotes ----

#[derive(Default)]
struct TwitchEmotes {
    /// Nutzer-ID und die Liste aus Abo- und globalen Emotes.
    base: Option<(String, SharedList)>,
    follower: HashMap<String, SharedList>,
}

thread_local! {
    static TWITCH: RefCell<TwitchEmotes> = RefCell::new(TwitchEmotes::default());
    static TWITCH_LISTENERS: RefCell<Listeners> = RefCell::new(Listeners::default());
}

fn convert_twitch(list: &[Value], from_global_list: bool) -> Vec<Emote> {
    let dark = crate::dark_theme();
    list.iter().filter_map(|raw| ce::twitch_api_emote(raw, from_global_list, dark)).collect()
}

fn base_list(user_id: &str) -> SharedList {
    if let Some(shared) = TWITCH.with(|t| t.borrow().base.as_ref().filter(|(u, _)| u == user_id).map(|(_, s)| s.clone())) {
        return shared;
    }
    let user = user_id.to_string();
    let shared = async move {
        let (own, global) = futures::join!(crate::api::get_user_emotes(&user, None), crate::api::get_global_emotes());
        let mut list = convert_twitch(&own?, false);
        list.extend(convert_twitch(&global?, true));
        Ok(Rc::new(list))
    }
    .boxed_local()
    .shared();
    TWITCH.with(|t| {
        let mut t = t.borrow_mut();
        t.base = Some((user_id.to_string(), shared.clone()));
        t.follower.clear();
    });
    shared
}

fn follower_list(user_id: &str, channel_id: &str) -> SharedList {
    if let Some(shared) = TWITCH.with(|t| t.borrow().follower.get(channel_id).cloned()) {
        return shared;
    }
    let (user, channel) = (user_id.to_string(), channel_id.to_string());
    let shared = async move {
        // Fehler sind nicht schlimm: dann eben ohne Follower-Emotes.
        let list = crate::api::get_user_emotes(&user, Some(&channel)).await.unwrap_or_default();
        let follower: Vec<Value> = list
            .into_iter()
            .filter(|e| e.get("emote_type").and_then(Value::as_str) == Some("follower") && e.get("owner_id").and_then(Value::as_str) == Some(channel.as_str()))
            .collect();
        Ok(Rc::new(convert_twitch(&follower, false)))
    }
    .boxed_local()
    .shared();
    TWITCH.with(|t| t.borrow_mut().follower.insert(channel_id.to_string(), shared.clone()));
    shared
}

/// Alle in diesem Kanal nutzbaren Twitch-Emotes, ohne doppelte Namen (Follower-Emotes zuerst).
pub async fn usable_twitch_emotes(channel_id: &str) -> Result<Vec<Emote>, JsValue> {
    let user_id = crate::auth::own_user_id().ok_or_else(|| helix_error(401, tr("Nicht angemeldet.", "Not signed in.")))?;
    let base = base_list(&user_id);
    let follower = follower_list(&user_id, channel_id);
    let (base, follower) = futures::join!(base, follower);
    let base = match base {
        Ok(base) => base,
        Err(error) => {
            // Beim nächsten Bedarf neu versuchen.
            TWITCH.with(|t| t.borrow_mut().base = None);
            return Err(error);
        }
    };
    let follower = follower.unwrap_or_default();
    let mut seen = HashSet::new();
    Ok(follower.iter().chain(base.iter()).filter(|e| seen.insert(e.name.clone())).cloned().collect())
}

/// Zwischengespeicherte Listen verwerfen (Tastenkürzel „Emotes neu laden“). Beim nächsten Bedarf wird neu abgefragt;
/// Eingabefelder laden über die Meldung nach.
pub fn reset_twitch_emotes() {
    TWITCH.with(|t| *t.borrow_mut() = TwitchEmotes::default());
    emit(&TWITCH_LISTENERS);
}

pub fn subscribe_twitch(listener: Listener) -> u32 {
    TWITCH_LISTENERS.with(|l| l.borrow_mut().add(listener))
}

pub fn unsubscribe_twitch(id: u32) {
    TWITCH_LISTENERS.with(|l| l.borrow_mut().remove(id));
}

// ---- 7TV, BTTV, FFZ ----

const SERVICES: [&str; 3] = ["seventv", "bttv", "ffz"];

fn service_label(service: &str) -> &'static str {
    match service {
        "seventv" => "7TV",
        "bttv" => "BTTV",
        _ => "FFZ",
    }
}

/// Dienst eingeschaltet? (7TV: Emotes, BTTV und FFZ: an/aus; Standard: an)
fn enabled(service: &str) -> bool {
    match service {
        "seventv" => crate::settings::get("thirdParty.seventv").and_then(|v| v.get("emotes").and_then(Value::as_bool)).unwrap_or(true),
        "bttv" => crate::settings::get("thirdParty.bttv").and_then(|v| v.as_bool()).unwrap_or(true),
        _ => crate::settings::get("thirdParty.ffz").and_then(|v| v.as_bool()).unwrap_or(true),
    }
}

fn twitch_id(id: &str) -> String {
    multiyap_core::twitch::url_component(id)
}

/// Lädt die Emotes eines Dienstes: global (`channel` None) oder für einen Kanal.
async fn load_service(service: &str, channel: Option<&str>) -> Result<Vec<Emote>, JsValue> {
    Ok(match (service, channel) {
        ("bttv", None) => ce::bttv_emotes(fetch_json("https://api.betterttv.net/3/cached/emotes/global").await?.as_ref()),
        ("bttv", Some(id)) => ce::bttv_channel(fetch_json(&format!("https://api.betterttv.net/3/cached/users/twitch/{}", twitch_id(id))).await?.as_ref()),
        ("ffz", None) => ce::ffz_global(fetch_json("https://api.frankerfacez.com/v1/set/global").await?.as_ref()),
        ("ffz", Some(id)) => ce::ffz_channel(fetch_json(&format!("https://api.frankerfacez.com/v1/room/id/{}", twitch_id(id))).await?.as_ref()),
        (_, None) => {
            let json = fetch_json("https://7tv.io/v3/emote-sets/global").await?;
            ce::seventv_emotes(json.as_ref().and_then(|j| j.get("emotes")))
        }
        (_, Some(id)) => {
            let json = fetch_json(&format!("https://7tv.io/v3/users/twitch/{}", twitch_id(id))).await?;
            // Änderungen am Emote-Set live mitbekommen (crates/web/src/seventv.rs).
            if let Some(set_id) = json.as_ref().and_then(|j| j.get("emote_set")).and_then(|s| s.get("id")).and_then(Value::as_str).filter(|s| !s.is_empty()) {
                crate::seventv::watch_emote_set(id, set_id);
            }
            ce::seventv_channel(json.as_ref())
        }
    })
}

#[derive(Default)]
struct ThirdParty {
    /// Fertig geladene Listen; Schlüssel "dienst:global" oder "dienst:kanal-id".
    results: HashMap<String, Vec<Emote>>,
    pending: HashSet<String>,
    channels: Vec<String>,
    /// Je Kanal: Emotes ohne doppelte Namen in Vorrang-Reihenfolge.
    lists: HashMap<String, Rc<Vec<Emote>>>,
    watching: bool,
}

thread_local! {
    static THIRD: RefCell<ThirdParty> = RefCell::new(ThirdParty::default());
    static THIRD_LISTENERS: RefCell<Listeners> = RefCell::new(Listeners::default());
}

fn key(service: &str, scope: Option<&str>) -> String {
    format!("{service}:{}", scope.unwrap_or("global"))
}

/// Alle eingeschalteten Quellen (global und je angemeldetem Kanal) als (Dienst, Kanal).
fn targets() -> Vec<(&'static str, Option<String>)> {
    let channels = THIRD.with(|t| t.borrow().channels.clone());
    let mut list = Vec::new();
    for service in SERVICES.into_iter().filter(|s| enabled(s)) {
        list.push((service, None));
        list.extend(channels.iter().map(|c| (service, Some(c.clone()))));
    }
    list
}

fn load(service: &'static str, channel: Option<String>) {
    let k = key(service, channel.as_deref());
    let start = THIRD.with(|t| {
        let mut t = t.borrow_mut();
        !t.results.contains_key(&k) && t.pending.insert(k.clone())
    });
    if !start {
        return;
    }
    spawn_local(async move {
        let result = load_service(service, channel.as_deref()).await;
        THIRD.with(|t| t.borrow_mut().pending.remove(&k));
        match result {
            Ok(emotes) => {
                THIRD.with(|t| t.borrow_mut().results.insert(k, emotes));
                rebuild();
            }
            // Ein Ausfall eines Dienstes stört den Chat nicht, steht aber mit Grund im Chat.
            Err(error) => report_failure(service, channel.as_deref(), &error),
        }
    });
}

fn js_message(error: &JsValue) -> String {
    js_sys::Reflect::get(error, &"message".into()).ok().and_then(|m| m.as_string()).or_else(|| error.as_string()).unwrap_or_default()
}

/// Verständlicher Grund für einen Ladefehler.
fn failure_reason(error: &JsValue) -> String {
    let name = js_sys::Reflect::get(error, &"name".into()).ok().and_then(|n| n.as_string()).unwrap_or_default();
    let message = js_message(error);
    if name == "AbortError" {
        tr("keine Antwort innerhalb der Frist", "no response in time").into()
    } else if name == "TypeError" {
        tr("Dienst nicht erreichbar", "service not reachable").into()
    } else if let Some(status) = message.split("HTTP ").nth(1) {
        multiyap_core::tf!("Dienst antwortet mit Fehler {status}", "service responds with error {status}")
    } else if message.is_empty() {
        tr("unbekannter Fehler", "unknown error").into()
    } else {
        message
    }
}

/// Meldet einen fehlgeschlagenen Abruf in der Konsole und im Chat (global: in allen offenen Kanälen).
fn report_failure(service: &str, channel: Option<&str>, error: &JsValue) {
    let scope = if channel.is_some() { tr("Kanal", "channel") } else { "global" };
    let detail = format!("{}-Emotes ({scope}): {}", service_label(service), failure_reason(error));
    console_warn(&multiyap_core::tf!("Emotes konnten nicht geladen werden: {detail}", "Emotes could not be loaded: {detail}"));
    crate::sessions::channel_notice(channel, "load-failed", &detail);
}

/// 7TV meldet eine Änderung am Emote-Set eines Kanals: geladene Liste direkt anpassen (ohne neue Abfrage).
pub fn apply_seventv_change(channel_id: &str, change: &multiyap_core::seventv::EmoteSetChange) {
    let k = key("seventv", Some(channel_id));
    let changed = THIRD.with(|t| {
        let mut t = t.borrow_mut();
        let Some(list) = t.results.get_mut(&k) else { return false };
        multiyap_core::seventv::apply_emote_set_change(list, change);
        true
    });
    if changed {
        rebuild();
    }
}

fn load_enabled() {
    for (service, channel) in targets() {
        load(service, channel);
    }
}

fn rebuild() {
    THIRD.with(|t| {
        let mut t = t.borrow_mut();
        let mut lists = HashMap::new();
        for channel in &t.channels {
            let mut seen = HashSet::new();
            let mut list = Vec::new();
            for scope in [Some(channel.as_str()), None] {
                for service in SERVICES.into_iter().filter(|s| enabled(s)) {
                    for emote in t.results.get(&key(service, scope)).into_iter().flatten() {
                        if seen.insert(emote.name.clone()) {
                            list.push(emote.clone());
                        }
                    }
                }
            }
            lists.insert(channel.clone(), Rc::new(list));
        }
        t.lists = lists;
    });
    emit(&THIRD_LISTENERS);
}

/// Kanal anmelden (beim Beitreten); lädt die eingeschalteten Dienste für ihn.
pub fn ensure_third_party(channel_id: &str) {
    if !THIRD.with(|t| std::mem::replace(&mut t.borrow_mut().watching, true)) {
        for setting in ["thirdParty.seventv", "thirdParty.bttv", "thirdParty.ffz"] {
            crate::settings::subscribe(
                setting,
                Rc::new(|| {
                    load_enabled();
                    rebuild();
                }),
            );
        }
    }
    let added = THIRD.with(|t| {
        let mut t = t.borrow_mut();
        let added = !t.channels.iter().any(|c| c == channel_id);
        if added {
            t.channels.push(channel_id.to_string());
        }
        added
    });
    if added {
        rebuild();
    }
    load_enabled();
}

/// Alles neu abfragen, was eingeschaltet ist (Tastenkürzel „Emotes neu laden“). Bis die neuen Listen da sind, bleiben
/// die alten sichtbar; schlägt ein Dienst fehl, bleibt seine alte Liste. Gibt die Zahl der fehlgeschlagenen Abfragen
/// zurück.
pub async fn reload_third_party() -> usize {
    let jobs: Vec<_> = targets()
        .into_iter()
        .filter_map(|(service, channel)| {
            let k = key(service, channel.as_deref());
            THIRD.with(|t| t.borrow_mut().pending.insert(k.clone())).then_some((service, channel, k))
        })
        .map(|(service, channel, k)| async move {
            let result = load_service(service, channel.as_deref()).await;
            THIRD.with(|t| t.borrow_mut().pending.remove(&k));
            match result {
                Ok(emotes) => {
                    THIRD.with(|t| t.borrow_mut().results.insert(k, emotes));
                    true
                }
                Err(error) => {
                    report_failure(service, channel.as_deref(), &error);
                    false
                }
            }
        })
        .collect();
    let done = futures::future::join_all(jobs).await;
    rebuild();
    done.into_iter().filter(|ok| !ok).count()
}

/// Emotes für einen Kanal ohne doppelte Namen (leer, solange nichts geladen ist).
pub fn third_party_emotes(channel_id: &str) -> Rc<Vec<Emote>> {
    THIRD.with(|t| t.borrow().lists.get(channel_id).cloned()).unwrap_or_default()
}

#[derive(Clone, Serialize)]
pub struct EmoteGroup {
    pub id: String,
    pub label: String,
    pub emotes: Vec<Emote>,
}

/// Emotes je Dienst, getrennt nach Kanal und global (für das Emote-Fenster). Nur eingeschaltete Dienste.
pub fn third_party_groups(channel_id: &str) -> Vec<EmoteGroup> {
    THIRD.with(|t| {
        let t = t.borrow();
        let mut groups = Vec::new();
        for service in SERVICES.into_iter().filter(|s| enabled(s)) {
            for (scope, label, suffix) in [(Some(channel_id), tr("Kanal", "channel"), "channel"), (None, "global", "global")] {
                let emotes = t.results.get(&key(service, scope)).cloned().unwrap_or_default();
                if !emotes.is_empty() {
                    groups.push(EmoteGroup { id: format!("{service}-{suffix}"), label: format!("{}: {label}", service_label(service)), emotes });
                }
            }
        }
        groups
    })
}

pub fn third_party_version() -> u32 {
    THIRD_LISTENERS.with(|l| l.borrow().version)
}

pub fn subscribe_third_party(listener: Listener) -> u32 {
    THIRD_LISTENERS.with(|l| l.borrow_mut().add(listener))
}

pub fn unsubscribe_third_party(id: u32) {
    THIRD_LISTENERS.with(|l| l.borrow_mut().remove(id));
}

// ---- Bits (Cheermotes) ----

#[derive(Clone)]
struct Tier {
    min_bits: f64,
    color: String,
    url: String,
    url2x: String,
}

/// Bild und Farbe einer Bits-Stufe.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CheerImage {
    pub url: String,
    pub url2x: String,
    pub color: String,
}

#[derive(Default)]
struct Cheermotes {
    /// Kanal-ID, dann Präfix in Kleinbuchstaben, dann Stufen (höchste zuerst).
    by_channel: HashMap<String, HashMap<String, Vec<Tier>>>,
    pending: HashSet<String>,
}

thread_local! {
    static CHEER: RefCell<Cheermotes> = RefCell::new(Cheermotes::default());
    static CHEER_LISTENERS: RefCell<Listeners> = RefCell::new(Listeners::default());
}

fn tiers_of(cheermotes: &[Value]) -> HashMap<String, Vec<Tier>> {
    let mut map = HashMap::new();
    for cheermote in cheermotes {
        let prefix = cheermote.get("prefix").and_then(Value::as_str).unwrap_or_default().to_lowercase();
        let mut tiers: Vec<Tier> = cheermote
            .get("tiers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|tier| {
                let images = tier.get("images").and_then(|i| i.get("dark")).and_then(|d| d.get("animated"));
                let image = |size: &str| images.and_then(|i| i.get(size)).and_then(Value::as_str).map(str::to_string);
                let url = image("1").unwrap_or_default();
                Tier {
                    min_bits: tier.get("min_bits").and_then(Value::as_f64).unwrap_or_default(),
                    color: tier.get("color").and_then(Value::as_str).unwrap_or_default().to_string(),
                    url2x: image("2").unwrap_or_else(|| url.clone()),
                    url,
                }
            })
            .collect();
        tiers.sort_by(|a, b| b.min_bits.total_cmp(&a.min_bits));
        map.insert(prefix, tiers);
    }
    map
}

/// Bits-Bilder eines Kanals laden (Get Cheermotes, Doku geprüft am 15.11.2026), einmal je Kanal und Sitzung. Ohne
/// Bilder bleiben Bits als Text mit Anzahl sichtbar; beim nächsten Öffnen wird erneut versucht.
pub fn ensure_cheermotes(channel_id: &str) {
    let start = CHEER.with(|c| {
        let mut c = c.borrow_mut();
        !c.by_channel.contains_key(channel_id) && c.pending.insert(channel_id.to_string())
    });
    if !start {
        return;
    }
    let channel = channel_id.to_string();
    spawn_local(async move {
        let result = crate::api::get_cheermotes(&channel).await;
        CHEER.with(|c| c.borrow_mut().pending.remove(&channel));
        if let Ok(list) = result {
            CHEER.with(|c| c.borrow_mut().by_channel.insert(channel, tiers_of(&list)));
            emit(&CHEER_LISTENERS);
        }
    });
}

/// Bild und Farbe zur Stufe (höchste Stufe, deren Mindestanzahl erreicht ist), animiert im dunklen Design.
pub fn cheer_image(channel_id: &str, prefix: &str, bits: f64) -> Option<CheerImage> {
    CHEER.with(|c| {
        let c = c.borrow();
        let tiers = c.by_channel.get(channel_id)?.get(&prefix.to_lowercase())?;
        let tier = tiers.iter().find(|t| bits >= t.min_bits).or_else(|| tiers.last())?;
        Some(CheerImage { url: tier.url.clone(), url2x: tier.url2x.clone(), color: tier.color.clone() })
    })
}

pub fn cheermotes_version() -> u32 {
    CHEER_LISTENERS.with(|l| l.borrow().version)
}

pub fn subscribe_cheermotes(listener: Listener) -> u32 {
    CHEER_LISTENERS.with(|l| l.borrow_mut().add(listener))
}

pub fn unsubscribe_cheermotes(id: u32) {
    CHEER_LISTENERS.with(|l| l.borrow_mut().remove(id));
}

// ---- Zuletzt benutzte Emotes ----

/// Merkt sich ein benutztes Emote (zuletzt benutzte oben im Emote-Fenster). Fehler sind egal.
pub fn remember(emote: &Emote) {
    let entry = json!({
        "source": emote.source,
        "id": emote.id,
        "name": emote.name,
        "imageUrl": emote.image_url,
        "imageUrl2x": emote.image_url2x,
    });
    spawn_local(async move {
        let _ = crate::settings::update("emotes.recent", move |current| ce::add_recent(&current, &entry)).await;
    });
}

// ---- Beim Beitreten eines Kanals ----

/// Lädt im Hintergrund vor, was ein Kanal braucht: Twitch-Emotes für das Autofill (Twitch liefert sie seitenweise,
/// das dauert), 7TV, BTTV und FFZ, Bits-Bilder sowie 7TV-Abzeichen und Namensfarben (nur eingeschaltete Dienste).
pub fn preload_channel(channel_id: &str) {
    let channel = channel_id.to_string();
    spawn_local(async move {
        let _ = usable_twitch_emotes(&channel).await;
    });
    ensure_third_party(channel_id);
    ensure_cheermotes(channel_id);
    crate::seventv::ensure_channel(channel_id);
}

// ---- Für TypeScript (bis die übrigen Teile in Rust sind) ----

