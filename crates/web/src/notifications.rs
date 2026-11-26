//! Benachrichtigungen (bisher src/features/notifications/*.ts): Ton und Tab-Blinken für Live, Erwähnungen und Begriffe,
//! ungesehene Zeilen je Kanal, Stummschalten, Standard-Sound und eigene Sounds. Chat- Ereignisse und AutoMod lösen
//! keinen Ton aus, der geladene Verlauf auch nicht; stumme Kanäle bleiben still. Nichts davon wird gespeichert außer
//! den Einstellungen und den eigenen Sounds (Speicher „sounds“). Erkennung, Signal, Sperrzeit und Vorrang in
//! crates/core/src/notify.rs.

use multiyap_core::i18n::tr;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use multiyap_core::notify::{self as nt, Attention, Cooldown, MatchSettings, Matcher, Signal};
use js_sys::{Function, Promise, Reflect};
use serde::Serialize;
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{future_to_promise, spawn_local, JsFuture};

use crate::chat_tools::toast;
use crate::db::{main_database, WriteOp};
use crate::{auth, moderation, sessions, settings, streams};

const SOUNDS: &str = "sounds";
fn save_error() -> &'static str {
    tr("Die Einstellung konnte nicht gespeichert werden.", "The setting could not be saved.")
}
/// Höchstens so viele stumm geschaltete Kanäle.
const MAX_MUTED_CHANNELS: usize = 500;

type Listener = Rc<dyn Fn()>;

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

fn setting(key: &str) -> Value {
    settings::get(key).unwrap_or(Value::Null)
}

fn list_contains(key: &str, login: &str) -> bool {
    setting(key).as_array().is_some_and(|list| list.iter().any(|v| v.as_str() == Some(login)))
}

// ---- Erkennung (Erwähnungen und Filter), einmal je Stand der Einstellungen gebaut ----

const MATCH_KEYS: [&str; 7] = [
    "mentions.ownName",
    "mentions.extraNames",
    "highlights.scope",
    "highlights.categories",
    "highlights.exceptions",
    "highlights.exemptBroadcaster",
    "highlights.exemptMods",
];

thread_local! {
    static MATCHER: RefCell<Option<Rc<Matcher>>> = const { RefCell::new(None) };
    static MATCHER_LISTENERS: RefCell<Listeners> = RefCell::new(Listeners::default());
    static MATCHER_WATCHED: RefCell<bool> = const { RefCell::new(false) };
}

fn match_settings() -> MatchSettings {
    let flag = |key: &str| setting(key).as_bool().unwrap_or(false);
    let list = |key: &str| setting(key).as_array().cloned().unwrap_or_default();
    let view = auth::view();
    MatchSettings {
        own: view.account.filter(|_| view.status == "signed-in").map(|a| (a.user_id, a.login)),
        own_name: flag("mentions.ownName"),
        extra_names: list("mentions.extraNames").iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        scope: setting("highlights.scope").as_str().unwrap_or("mod").to_string(),
        categories: list("highlights.categories"),
        exceptions: list("highlights.exceptions"),
        exempt_broadcaster: flag("highlights.exemptBroadcaster"),
        exempt_mods: flag("highlights.exemptMods"),
    }
}

/// Beobachtet einmal die Einstellungen und die Anmeldung, von denen die Erkennung abhängt.
fn watch_matcher() {
    if MATCHER_WATCHED.with(|w| std::mem::replace(&mut *w.borrow_mut(), true)) {
        return;
    }
    for key in MATCH_KEYS {
        settings::subscribe(key, Rc::new(invalidate_matcher));
    }
    auth::subscribe(Rc::new(invalidate_matcher));
}

/// Erkennung mit den aktuellen Einstellungen (neu gebaut, wenn sich Einstellungen oder Anmeldung ändern). Gemeinsam für
/// Benachrichtigungen, Chat-Liste und Hervorhebungen-Tab, damit alle genau gleich entscheiden.
pub fn matcher() -> Rc<Matcher> {
    watch_matcher();
    if let Some(matcher) = MATCHER.with(|m| m.borrow().clone()) {
        return matcher;
    }
    let matcher = Rc::new(Matcher::new(match_settings()));
    MATCHER.with(|m| *m.borrow_mut() = Some(matcher.clone()));
    matcher
}

fn invalidate_matcher() {
    MATCHER.with(|m| *m.borrow_mut() = None);
    emit(&MATCHER_LISTENERS);
}

/// Meldet, wenn die Erkennung neu gebaut werden muss (Einstellungen oder Anmeldung geändert).
pub fn subscribe_matcher(listener: Listener) -> u32 {
    watch_matcher();
    MATCHER_LISTENERS.with(|l| l.borrow_mut().add(listener))
}

pub fn unsubscribe_matcher(id: u32) {
    MATCHER_LISTENERS.with(|l| l.borrow_mut().remove(id));
}

// ---- Tab-Blinken ----

thread_local! {
    static ATTENTION: RefCell<HashMap<String, Attention>> = RefCell::new(HashMap::new());
    static ATTENTION_LISTENERS: RefCell<Listeners> = RefCell::new(Listeners::default());
}

pub fn raise_attention(login: &str, next: Attention) {
    let changed = ATTENTION.with(|a| {
        let mut a = a.borrow_mut();
        if !nt::replaces_attention(a.get(login), &next) {
            return false;
        }
        a.insert(login.to_string(), next);
        true
    });
    if changed {
        emit(&ATTENTION_LISTENERS);
    }
}

/// Beenden, sobald man in den Tab klickt oder er sichtbar wird.
pub fn clear_attention(login: &str) {
    if ATTENTION.with(|a| a.borrow_mut().remove(login).is_some()) {
        emit(&ATTENTION_LISTENERS);
    }
}

pub fn attention(login: &str) -> Option<Attention> {
    ATTENTION.with(|a| a.borrow().get(login).cloned())
}

pub fn subscribe_attention(listener: Listener) -> u32 {
    ATTENTION_LISTENERS.with(|l| l.borrow_mut().add(listener))
}

pub fn unsubscribe_attention(id: u32) {
    ATTENTION_LISTENERS.with(|l| l.borrow_mut().remove(id));
}

// ---- Ungesehene Zeilen: Tab-Name weiß und fett, bis man den Tab ansieht ----

thread_local! {
    static UNREAD: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static UNREAD_LISTENERS: RefCell<Listeners> = RefCell::new(Listeners::default());
}

pub fn mark_unread(login: &str) {
    if UNREAD.with(|u| u.borrow_mut().insert(login.to_string())) {
        emit(&UNREAD_LISTENERS);
    }
}

/// Beenden, sobald der Tab sichtbar ist oder der Kanal stummgeschaltet wird.
pub fn clear_unread(login: &str) {
    if UNREAD.with(|u| u.borrow_mut().remove(login)) {
        emit(&UNREAD_LISTENERS);
    }
}

pub fn is_unread(login: &str) -> bool {
    UNREAD.with(|u| u.borrow().contains(login))
}

pub fn subscribe_unread(listener: Listener) -> u32 {
    UNREAD_LISTENERS.with(|l| l.borrow_mut().add(listener))
}

pub fn unsubscribe_unread(id: u32) {
    UNREAD_LISTENERS.with(|l| l.borrow_mut().remove(id));
}

// ---- Sichtbarkeit und Stummschalten ----

/// Kanäle, deren Tab gerade in einem Bereich sichtbar ist (dort kein Blinken).
fn visible_logins() -> HashSet<String> {
    let state = crate::workspace::state();
    let active: HashSet<String> = state.visible_tab_ids().into_iter().collect();
    nt::visible_logins(&state.tabs, &active)
}

fn is_muted(login: &str) -> bool {
    list_contains("notifications.mutedChannels", login)
}

fn toggled(current: Value, login: &str) -> Value {
    let mut list: Vec<Value> = current.as_array().cloned().unwrap_or_default();
    if list.iter().any(|v| v.as_str() == Some(login)) {
        list.retain(|v| v.as_str() != Some(login));
    } else {
        list.push(json!(login));
    }
    Value::Array(list)
}

/// Kanal stummschalten oder wieder freigeben: keine Töne, kein Blinken.
pub async fn toggle_muted(login: &str) {
    let owned = login.to_string();
    match settings::update("notifications.mutedChannels", move |current| toggled(current, &owned)).await {
        Ok(()) => {
            clear_attention(login);
            clear_unread(login);
        }
        Err(_) => toast("error", save_error()),
    }
}

/// Kanal direkt in den Einstellungen stummschalten, auch ohne offenen Tab. false: schon stumm oder die Liste ist voll.
pub async fn add_muted(login: &str) -> bool {
    let added = Rc::new(std::cell::Cell::new(false));
    let (flag, owned) = (added.clone(), login.to_string());
    let result = settings::update("notifications.mutedChannels", move |current| {
        let mut list: Vec<Value> = current.as_array().cloned().unwrap_or_default();
        if list.iter().any(|v| v.as_str() == Some(owned.as_str())) || list.len() >= MAX_MUTED_CHANNELS {
            return Value::Array(list);
        }
        flag.set(true);
        list.push(json!(owned));
        Value::Array(list)
    })
    .await;
    if result.is_err() {
        toast("error", save_error());
        return false;
    }
    if added.get() {
        clear_attention(login);
        clear_unread(login);
    }
    added.get()
}

/// Live-Benachrichtigung eines Kanals aus- oder wieder einschalten (Ton und Pulsieren beim Live-Gehen).
pub async fn toggle_live_notify(login: &str) {
    let owned = login.to_string();
    if settings::update("notifications.liveOffChannels", move |current| toggled(current, &owned)).await.is_err() {
        toast("error", save_error());
    }
}

// ---- Töne ----

thread_local! {
    static AUDIO: RefCell<Option<JsValue>> = const { RefCell::new(None) };
    static COOLDOWN: RefCell<Cooldown> = RefCell::new(Cooldown::default());
}

fn get(target: &JsValue, key: &str) -> JsValue {
    Reflect::get(target, &key.into()).unwrap_or(JsValue::UNDEFINED)
}

fn call(target: &JsValue, method: &str, args: &[JsValue]) -> Option<JsValue> {
    let function: Function = get(target, method).dyn_into().ok()?;
    let list: js_sys::Array = args.iter().collect();
    function.apply(target, &list).ok()
}

/// AudioContext des Browsers (einmal angelegt). Der Browser erlaubt Ton erst nach einer ersten Bedienung der Seite.
fn audio_context() -> Option<JsValue> {
    let context = AUDIO.with(|a| a.borrow().clone()).or_else(|| {
        let constructor: Function = get(&js_sys::global(), "AudioContext").dyn_into().ok()?;
        let context = Reflect::construct(&constructor, &js_sys::Array::new()).ok()?;
        AUDIO.with(|a| *a.borrow_mut() = Some(context.clone()));
        Some(context)
    })?;
    if get(&context, "state").as_string().as_deref() == Some("suspended") {
        call(&context, "resume", &[]);
    }
    Some(context)
}

/// Eingebauter Standard-Sound: ein kurzer, zweistufiger Klang, im Browser erzeugt (keine Audiodatei). `volume` 0 bis 1.
fn play_default_sound(volume: f64) {
    let Some(ctx) = audio_context().filter(|_| volume > 0.0) else { return };
    let start = get(&ctx, "currentTime").as_f64().unwrap_or_default();
    let destination = get(&ctx, "destination");
    for (frequency, offset) in [(880.0, 0.0), (1318.5, 0.12)] {
        let (Some(oscillator), Some(gain)) = (call(&ctx, "createOscillator", &[]), call(&ctx, "createGain", &[])) else { return };
        let _ = Reflect::set(&oscillator, &"type".into(), &"sine".into());
        let _ = Reflect::set(&get(&oscillator, "frequency"), &"value".into(), &frequency.into());
        let t = start + offset;
        let param = get(&gain, "gain");
        call(&param, "setValueAtTime", &[0.0.into(), t.into()]);
        call(&param, "linearRampToValueAtTime", &[(0.4 * volume).into(), (t + 0.01).into()]);
        call(&param, "exponentialRampToValueAtTime", &[0.0001.into(), (t + 0.35).into()]);
        if let Some(connected) = call(&oscillator, "connect", std::slice::from_ref(&gain)) {
            call(&connected, "connect", std::slice::from_ref(&destination));
        }
        call(&oscillator, "start", &[t.into()]);
        call(&oscillator, "stop", &[(t + 0.4).into()]);
    }
}

/// Ton eines Ereignisses, mit Sperrzeit je Ereignisart (`key`); Sound nach dem ersten gewählten Eintrag in
/// `choice_keys` (zum Beispiel weiterer Name vor Erwähnung allgemein), sonst der Standard-Sound.
fn play(key: &str, choice_keys: &[String], event_volume: f64) {
    if !COOLDOWN.with(|c| c.borrow_mut().allow(key, js_sys::Date::now(), nt::SOUND_COOLDOWN_MS)) {
        return;
    }
    let choice = nt::sound_choice(&setting("notifications.soundChoice"), choice_keys);
    let master = setting("notifications.volume").as_f64().unwrap_or(100.0);
    spawn_local(play_sound(choice, (master / 100.0) * (event_volume / 100.0)));
}

// ---- Eigene Sounds ----

/// Angaben zu einem eigenen Sound ohne die Audiodaten (für Listen und Auswahl).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SoundInfo {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size: f64,
    pub created_at: String,
}

thread_local! {
    static SOUND_INFOS: RefCell<Vec<SoundInfo>> = const { RefCell::new(Vec::new()) };
    static SOUND_LISTENERS: RefCell<Listeners> = RefCell::new(Listeners::default());
    /// Dekodierte Sounds für die Sitzung (Zusage auf einen AudioBuffer oder null); Schlüssel: Sound-ID.
    static DECODED: RefCell<HashMap<String, JsValue>> = RefCell::new(HashMap::new());
}

fn parse_sound(value: &Value) -> Option<Value> {
    multiyap_core::stored::named("customSound")?.parse(value).ok()
}

/// Liste neu laden; unlesbare Einträge werden übersprungen statt die ganze Liste zu verlieren.
async fn reload_sounds() {
    let Some(db) = main_database() else { return };
    let Ok(entries) = db.entries_json(SOUNDS).await else { return };
    let mut infos: Vec<SoundInfo> = entries
        .iter()
        .filter_map(|(_, value)| parse_sound(value))
        .map(|s| {
            let text = |name: &str| s.get(name).and_then(Value::as_str).unwrap_or_default().to_string();
            SoundInfo { id: text("id"), name: text("name"), mime_type: text("mimeType"), size: s.get("size").and_then(Value::as_f64).unwrap_or_default(), created_at: text("createdAt") }
        })
        .collect();
    infos.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    let ids: HashSet<String> = infos.iter().map(|i| i.id.clone()).collect();
    DECODED.with(|d| d.borrow_mut().retain(|id, _| ids.contains(id)));
    SOUND_INFOS.with(|s| *s.borrow_mut() = infos);
    emit(&SOUND_LISTENERS);
}

pub fn custom_sounds() -> Vec<SoundInfo> {
    SOUND_INFOS.with(|s| s.borrow().clone())
}

fn decode_bytes(bytes: &[u8]) -> Option<Promise> {
    let ctx = audio_context()?;
    let buffer = js_sys::Uint8Array::from(bytes).buffer();
    call(&ctx, "decodeAudioData", &[buffer.into()]).and_then(|p| p.dyn_into::<Promise>().ok())
}

/// Dekodierter Sound (AudioBuffer) oder None.
async fn decoded(id: &str) -> Option<JsValue> {
    let pending = DECODED.with(|d| d.borrow().get(id).cloned());
    let promise = match pending {
        Some(promise) => promise,
        None => {
            let owned = id.to_string();
            let promise: JsValue = future_to_promise(async move {
                let db = main_database().ok_or(JsValue::NULL)?;
                let sound = db.read_json(SOUNDS, &owned).await?.as_ref().and_then(parse_sound).ok_or(JsValue::NULL)?;
                let bytes = sound.get("data").and_then(Value::as_str).and_then(nt::base64_decode).ok_or(JsValue::NULL)?;
                JsFuture::from(decode_bytes(&bytes).ok_or(JsValue::NULL)?).await
            })
            .into();
            DECODED.with(|d| d.borrow_mut().insert(id.to_string(), promise.clone()));
            promise
        }
    };
    JsFuture::from(promise.unchecked_into::<Promise>()).await.ok().filter(|b| b.is_object())
}

/// Spielt einen eigenen Sound oder, ohne Auswahl oder wenn er fehlt, den Standard-Sound. `volume` von 0 bis 1.
pub async fn play_sound(sound_id: Option<String>, volume: f64) {
    if volume <= 0.0 {
        return;
    }
    let known = sound_id.filter(|id| SOUND_INFOS.with(|s| s.borrow().iter().any(|i| &i.id == id)));
    let buffer = match known {
        Some(id) => decoded(&id).await,
        None => None,
    };
    let (Some(buffer), Some(ctx)) = (buffer, audio_context()) else {
        play_default_sound(volume);
        return;
    };
    let (Some(source), Some(gain)) = (call(&ctx, "createBufferSource", &[]), call(&ctx, "createGain", &[])) else { return };
    let _ = Reflect::set(&source, &"buffer".into(), &buffer);
    let _ = Reflect::set(&get(&gain, "gain"), &"value".into(), &volume.into());
    if let Some(connected) = call(&source, "connect", &[gain]) {
        call(&connected, "connect", &[get(&ctx, "destination")]);
    }
    call(&source, "start", &[]);
}

/// Prüft und speichert eine hochgeladene Datei: Format (MP3, WAV, OGG), Größe, Anzahl und ob der Browser sie wirklich
/// abspielen kann. Erst danach wird geschrieben; die Anzahl wird in derselben Transaktion geprüft (zweiter Browser-Tab).
pub async fn add_custom_sound(file: JsValue) -> Result<SoundInfo, String> {
    let name = get(&file, "name").as_string().unwrap_or_default();
    let mime = nt::sound_mime_type(&name, &get(&file, "type").as_string().unwrap_or_default()).ok_or(tr("Nur MP3, WAV oder OGG.", "Only MP3, WAV or OGG."))?;
    let size = get(&file, "size").as_f64().unwrap_or_default() as usize;
    if size == 0 {
        return Err(tr("Die Datei ist leer.", "The file is empty.").into());
    }
    if size > nt::MAX_SOUND_BYTES {
        return Err(tr("Die Datei ist größer als 1 MB.", "The file is larger than 1 MB.").into());
    }
    let too_many = multiyap_core::tf!("Höchstens {} eigene Sounds.", "At most {} custom sounds.", nt::MAX_CUSTOM_SOUNDS);
    if custom_sounds().len() >= nt::MAX_CUSTOM_SOUNDS {
        return Err(too_many);
    }
    let buffer = call(&file, "arrayBuffer", &[]).and_then(|p| p.dyn_into::<Promise>().ok()).ok_or(tr("Die Datei lässt sich nicht lesen.", "The file cannot be read."))?;
    let buffer = JsFuture::from(buffer).await.map_err(|_| tr("Die Datei lässt sich nicht lesen.", "The file cannot be read."))?;
    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
    audio_context().ok_or(tr("Der Browser kann keine Töne abspielen.", "The browser cannot play sounds."))?;
    let check = decode_bytes(&bytes).ok_or(tr("Die Datei lässt sich nicht abspielen.", "The file cannot be played."))?;
    JsFuture::from(check.clone()).await.map_err(|_| tr("Die Datei lässt sich nicht abspielen.", "The file cannot be played."))?;

    let base = name.rsplit_once('.').map_or(name.as_str(), |(b, _)| b).trim();
    let display = multiyap_core::twitch::helix_api::truncate_utf16(base, 60);
    let display = if display.trim().is_empty() { "Sound".to_string() } else { display.trim().to_string() };
    let id = web_sys::window().and_then(|w| w.crypto().ok()).map(|c| c.random_uuid()).unwrap_or_default();
    let created_at = js_sys::Date::new_0().to_iso_string().as_string().unwrap_or_default();
    let sound = json!({ "id": id, "name": display, "mimeType": mime, "size": size, "data": nt::base64_encode(&bytes), "createdAt": created_at });
    let sound = parse_sound(&sound).ok_or(tr("Der Sound konnte nicht gespeichert werden.", "The sound could not be saved."))?;
    let db = main_database().ok_or(tr("Der Sound konnte nicht gespeichert werden.", "The sound could not be saved."))?;
    let key = id.clone();
    let value = sound.clone();
    let limit = too_many.clone();
    db.transact_keys(SOUNDS, move |keys| {
        if keys.len() >= nt::MAX_CUSTOM_SOUNDS {
            return Err(limit);
        }
        Ok(vec![WriteOp::Put(key, value)])
    })
    .await
    .map_err(|error| Reflect::get(&error, &"message".into()).ok().and_then(|m| m.as_string()).filter(|m| *m == too_many).unwrap_or_else(|| tr("Der Sound konnte nicht gespeichert werden.", "The sound could not be saved.").into()))?;
    DECODED.with(|d| d.borrow_mut().insert(id.clone(), check.into()));
    Ok(SoundInfo { id, name: display, mime_type: mime.into(), size: size as f64, created_at })
}

pub async fn rename_custom_sound(id: &str, name: &str) -> Result<(), String> {
    let trimmed = multiyap_core::twitch::helix_api::truncate_utf16(name.trim(), 60);
    if trimmed.is_empty() {
        return Ok(());
    }
    let db = main_database().ok_or(save_error())?;
    let key = id.to_string();
    db.transact(SOUNDS, id, move |current| {
        let Some(mut sound) = current.as_ref().and_then(parse_sound) else { return Ok(Vec::new()) };
        sound["name"] = json!(trimmed);
        Ok(vec![WriteOp::Put(key, sound)])
    })
    .await
    .map_err(|_| tr("Der Name konnte nicht gespeichert werden.", "The name could not be saved.").to_string())
}

/// Entfernt einen Sound. Ereignisse, die ihn nutzen, spielen danach wieder den Standard-Sound.
pub async fn remove_custom_sound(id: &str) -> Result<(), String> {
    let db = main_database().ok_or(save_error())?;
    let key = id.to_string();
    db.transact_keys(SOUNDS, move |_| Ok(vec![WriteOp::Delete(key)])).await.map_err(|_| tr("Der Sound konnte nicht entfernt werden.", "The sound could not be removed.").to_string())?;
    let owned = id.to_string();
    settings::update("notifications.soundChoice", move |current| {
        let map = current.as_object().cloned().unwrap_or_default();
        Value::Object(map.into_iter().filter(|(_, v)| v.as_str() != Some(owned.as_str())).collect())
    })
    .await
    .map_err(|_| tr("Der Sound konnte nicht entfernt werden.", "The sound could not be removed.").to_string())?;
    DECODED.with(|d| d.borrow_mut().remove(id));
    Ok(())
}

pub fn subscribe_sounds(listener: Listener) -> u32 {
    SOUND_LISTENERS.with(|l| l.borrow_mut().add(listener))
}

pub fn unsubscribe_sounds(id: u32) {
    SOUND_LISTENERS.with(|l| l.borrow_mut().remove(id));
}

// ---- Start ----

fn on_live_item(item: &Value) {
    let Some(login) = item.get("channel").and_then(|c| c.get("login")).and_then(Value::as_str).map(str::to_string) else { return };
    let matcher = matcher();
    // Ungesehene Zeilen: schon markiert, dann nichts weiter prüfen (bei vollen Chats laufen hier viele Zeilen durch).
    if !is_unread(&login) && nt::counts_as_unread(item, matcher.is_own(item)) && !is_muted(&login) && !visible_logins().contains(&login) {
        mark_unread(&login);
    }
    if item.get("kind").and_then(Value::as_str) != Some("message") || is_muted(&login) {
        return;
    }
    let channel_id = item.get("channel").and_then(|c| c.get("id")).and_then(Value::as_str).unwrap_or_default();
    let Some(signal) = matcher.signal(item, moderation::can_moderate(channel_id)) else { return };
    // Jede Begriffs-Kategorie ist eine eigene Ereignisart mit eigener Sperrzeit.
    let (settings, key, choice_keys, attention) = match &signal {
        // Ein weiterer Name kann einen eigenen Sound haben, sonst gilt der Sound der Erwähnung. Sperrzeit und Ton
        // an/aus gelten für alle Erwähnungen gemeinsam.
        Signal::Mention { name } => (
            serde_json::from_value(setting("notifications.mention")).unwrap_or(nt::NotifyEvent { sound: true, blink: true, volume: 100.0 }),
            "mention".to_string(),
            name.iter().map(|n| nt::mention_sound_key(n)).chain(["mention".to_string()]).collect::<Vec<_>>(),
            Attention { kind: "mention".into(), class_name: "bg-mention-blink/25".into() },
        ),
        Signal::Term { category_id, class_name } => (
            nt::term_notify_for(&setting("notifications.termCategories"), category_id),
            format!("term:{category_id}"),
            vec![format!("term:{category_id}")],
            Attention { kind: "term".into(), class_name: class_name.clone() },
        ),
    };
    if settings.sound {
        play(&key, &choice_keys, settings.volume);
    }
    if settings.blink && !visible_logins().contains(&login) {
        raise_attention(&login, attention);
    }
}

fn on_went_live(channel_id: &str) {
    let Some(login) = sessions::login_for_channel(channel_id) else { return };
    went_live(&login);
}

/// Live-Ton für einen Kanal (Twitch-Login oder YouTube-Schlüssel).
fn went_live(login: &str) {
    // Live-Benachrichtigung für diesen Kanal ausgeschaltet.
    if is_muted(login) || list_contains("notifications.liveOffChannels", login) {
        return;
    }
    let live = setting("notifications.live");
    if live.get("sound").and_then(Value::as_bool) == Some(true) {
        play("live", &["live".to_string()], live.get("volume").and_then(Value::as_f64).unwrap_or(100.0));
    }
}

thread_local! {
    static STARTED: RefCell<bool> = const { RefCell::new(false) };
}

/// Startet Benachrichtigungen, ungesehene Zeilen und die Liste der eigenen Sounds (einmal).
pub fn start() {
    if STARTED.with(|s| std::mem::replace(&mut *s.borrow_mut(), true)) {
        return;
    }
    watch_matcher();
    // Alle benötigten Einstellungen gleich laden, damit schon die erste Nachricht mit den gespeicherten Werten geprüft wird.
    for key in ["notifications.volume", "notifications.live", "notifications.liveOffChannels", "notifications.mention", "notifications.termCategories", "notifications.mutedChannels", "notifications.soundChoice"] {
        let _ = settings::get(key);
    }
    sessions::on_live_item(Rc::new(on_live_item));
    streams::on_went_live(Rc::new(on_went_live));
    // YouTube wie Twitch: Erwähnungen, Filter, ungesehene Zeilen und Live-Ton.
    crate::youtube::on_live_item(Rc::new(on_live_item));
    crate::youtube::on_went_live(Rc::new(went_live));
    if let Some(db) = main_database() {
        db.watch(|stores, _| {
            if stores.iter().any(|s| s == SOUNDS) {
                spawn_local(reload_sounds());
            }
        });
    }
    spawn_local(reload_sounds());
}

// ---- Für TypeScript (Tab-Einträge sind noch React) ----

