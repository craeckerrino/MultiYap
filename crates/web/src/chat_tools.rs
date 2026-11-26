//! Chat-Werkzeuge (bisher src/features/chat/chat-search.ts, jump.ts, reply-target.ts, message-actions.ts,
//! src/features/pins/pinned-store.ts und src/features/usercard/*.ts): Suche im Chat, Sprung zu einer Zeile, Antworten
//! und Eingabefelder je Kanal, angepinnte Nachricht, Aktionen aus dem Rechtsklick-Menü, Nutzerkarten als Fenster und
//! ihre Daten. Alles nur im Arbeitsspeicher. Moderatives nur mit Mod-Rechten über die zentrale Prüfung.
//! Reine Teile mit Tests in crates/core/src/chat_tools.rs.

use multiyap_core::i18n::tr;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use multiyap_core::chat_tools::{self as ct, CardWindows, InputRegistry, PinOutcome, PinnedMessage, SearchState};
use multiyap_core::model::{Badge, ChannelRef, UserRef};
use js_sys::{Function, Reflect};
use serde::Serialize;
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};

use crate::moderation::can_moderate;
use crate::net::global_function;

type Listener = Rc<dyn Fn()>;

#[derive(Default)]
struct Listeners {
    list: Vec<(u32, Listener)>,
    next: u32,
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

    fn all(&self) -> Vec<Listener> {
        self.list.iter().map(|(_, l)| l.clone()).collect()
    }
}

fn emit(listeners: Vec<Listener>) {
    for listener in listeners {
        listener();
    }
}

// ---- Kurze Meldungen (crates/web/src/toasts.rs) ----

/// Kurze Meldung unten im Fenster. `kind`: "error", "info", "success", "warning" oder "loading".
pub fn toast(kind: &str, text: &str) {
    crate::toasts::show(kind, text, None);
}

/// Kurze Meldung, die eine frühere mit derselben `id` ersetzt (zum Beispiel „lädt …“, dann „fertig“).
pub fn toast_with_id(kind: &str, text: &str, id: &str) {
    crate::toasts::show(kind, text, Some(id));
}

fn helix_status(error: &JsValue) -> Option<u16> {
    Reflect::get(error, &"helixStatus".into()).ok().and_then(|s| s.as_f64()).map(|s| s as u16)
}

fn error_text(error: &JsValue, fallback: &str) -> String {
    Reflect::get(error, &"message".into()).ok().and_then(|m| m.as_string()).filter(|m| !m.is_empty()).unwrap_or_else(|| fallback.to_string())
}

fn request_animation_frame(action: impl FnOnce() + 'static) {
    let run = Closure::once_into_js(action);
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

// ---- Suche im Chat ----

#[derive(Default)]
struct Search {
    states: HashMap<String, SearchState>,
    listeners: Listeners,
}

thread_local! {
    static SEARCH: RefCell<Search> = RefCell::new(Search::default());
}

fn set_search(tab_id: &str, change: impl FnOnce(&SearchState) -> SearchState) {
    let listeners = SEARCH.with(|s| {
        let mut s = s.borrow_mut();
        let next = change(&s.states.get(tab_id).cloned().unwrap_or_default());
        s.states.insert(tab_id.to_string(), next);
        s.listeners.all()
    });
    emit(listeners);
}

pub fn search_state(tab_id: &str) -> SearchState {
    SEARCH.with(|s| s.borrow().states.get(tab_id).cloned().unwrap_or_default())
}

/// Strg+F: öffnen oder, wenn schon offen, das Suchfeld wieder fokussieren.
pub fn open_search(tab_id: &str) {
    set_search(tab_id, SearchState::opened);
}

pub fn set_search_query(tab_id: &str, query: &str) {
    set_search(tab_id, |s| SearchState { query: query.to_string(), ..s.clone() });
}

pub fn close_search(tab_id: &str) {
    set_search(tab_id, SearchState::closed);
}

pub fn subscribe_search(listener: Listener) -> u32 {
    SEARCH.with(|s| s.borrow_mut().listeners.add(listener))
}

pub fn unsubscribe_search(id: u32) {
    SEARCH.with(|s| s.borrow_mut().listeners.remove(id));
}

// ---- Sprung zu einer Zeile (aus dem Hervorhebungen-Tab) ----

type JumpHandler = Rc<dyn Fn(&str) -> bool>;

thread_local! {
    static JUMP: RefCell<(Vec<(u32, JumpHandler)>, u32)> = RefCell::new((Vec::new(), 0));
}

/// Jede Chat-Liste meldet sich an; wer die Zeile hat, scrollt hin und meldet Erfolg.
pub fn on_jump_request(handler: JumpHandler) -> u32 {
    JUMP.with(|j| {
        let mut j = j.borrow_mut();
        j.1 += 1;
        let id = j.1;
        j.0.push((id, handler));
        id
    })
}

pub fn remove_jump_handler(id: u32) {
    JUMP.with(|j| j.borrow_mut().0.retain(|(i, _)| *i != id));
}

/// true, wenn mindestens eine Chat-Liste die Zeile gefunden hat.
pub fn request_jump(item_id: &str) -> bool {
    let handlers: Vec<JumpHandler> = JUMP.with(|j| j.borrow().0.iter().map(|(_, h)| h.clone()).collect());
    let mut found = false;
    for handler in handlers {
        if handler(item_id) {
            found = true;
        }
    }
    found
}

// ---- Antworten und Eingabefelder je Kanal ----

/// Nachricht, auf die im Eingabefeld eines Kanals geantwortet wird (Leiste „Antwort an …“ mit X).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyTarget {
    pub message_id: String,
    pub display_name: String,
    pub text: String,
}

/// Bedienung des Eingabefelds eines Kanals von außen: fokussieren und Text an der Schreibmarke einfügen.
#[derive(Clone)]
pub struct InputHandlers {
    pub focus: Rc<dyn Fn()>,
    pub insert: Rc<dyn Fn(&str)>,
}

#[derive(Default)]
struct Replies {
    targets: HashMap<String, ReplyTarget>,
    inputs: InputRegistry<InputHandlers>,
    listeners: Listeners,
}

thread_local! {
    static REPLIES: RefCell<Replies> = RefCell::new(Replies::default());
}

pub fn set_reply_target(channel_login: &str, target: Option<ReplyTarget>) {
    let login = channel_login.to_lowercase();
    let listeners = REPLIES.with(|r| {
        let mut r = r.borrow_mut();
        match target {
            Some(target) => {
                r.targets.insert(login, target);
            }
            None => {
                r.targets.remove(&login)?;
            }
        }
        Some(r.listeners.all())
    });
    emit(listeners.unwrap_or_default());
}

pub fn reply_target(channel_login: &str) -> Option<ReplyTarget> {
    REPLIES.with(|r| r.borrow().targets.get(&channel_login.to_lowercase()).cloned())
}

pub fn subscribe_reply_targets(listener: Listener) -> u32 {
    REPLIES.with(|r| r.borrow_mut().listeners.add(listener))
}

pub fn unsubscribe_reply_targets(id: u32) {
    REPLIES.with(|r| r.borrow_mut().listeners.remove(id));
}

/// Eingabefeld eines Kanals anmelden; gibt die Nummer zum Abmelden zurück.
pub fn register_input(channel_login: &str, handlers: InputHandlers) -> u32 {
    REPLIES.with(|r| r.borrow_mut().inputs.register(channel_login, handlers))
}

pub fn unregister_input(channel_login: &str, id: u32) {
    REPLIES.with(|r| r.borrow_mut().inputs.unregister(channel_login, id));
}

fn input(channel_login: &str) -> Option<InputHandlers> {
    REPLIES.with(|r| r.borrow().inputs.get(channel_login).cloned())
}

/// Zum Kanal-Tab wechseln (zum Beispiel aus dem Hervorhebungen-Tab) und danach etwas mit dessen Eingabefeld tun. Ist
/// der Kanal nicht offen, kommt ein kurzer Hinweis.
pub fn with_channel_input(channel_login: &str, action: impl FnOnce() + 'static) {
    let login = channel_login.to_lowercase();
    let state = crate::workspace::state();
    let visible: HashSet<String> = state.visible_tab_ids().into_iter().collect();
    let candidates: Vec<String> = state
        .tabs
        .iter()
        .filter(|t| t.get("kind").and_then(Value::as_str) == Some("channel") && t.get("channel").and_then(Value::as_str) == Some(login.as_str()))
        .filter_map(|t| t.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    let Some(tab) = candidates.iter().find(|id| visible.contains(*id)).or(candidates.first()).cloned() else {
        toast("info", tr("Der Kanal ist nicht offen.", "The channel is not open."));
        return;
    };
    spawn_local(async move {
        let _ = crate::workspace::activate_tab(&tab).await;
        request_animation_frame(action);
    });
}

/// Antworten (Rechtsklick-Menü und Antwort-Button): Leiste über dem Eingabefeld setzen, zum Kanal-Tab wechseln und das
/// Feld fokussieren.
pub fn start_reply(channel_login: &str, message_id: &str, display_name: &str, text: &str) {
    set_reply_target(channel_login, Some(ReplyTarget { message_id: message_id.into(), display_name: display_name.into(), text: text.into() }));
    let login = channel_login.to_string();
    with_channel_input(channel_login, move || {
        if let Some(handlers) = input(&login) {
            (handlers.focus)();
        }
    });
}

/// Rechtsklick auf einen Namen: „@Name“ in das Eingabefeld des Kanals einfügen.
pub fn mention_in_input(channel_login: &str, name: &str) {
    let (login, text) = (channel_login.to_string(), format!("@{name}"));
    with_channel_input(channel_login, move || {
        if let Some(handlers) = input(&login) {
            (handlers.insert)(&text);
        }
    });
}

// ---- Angepinnte Nachricht ----

#[derive(Default)]
struct Pins {
    pins: HashMap<String, Option<PinnedMessage>>,
    in_flight: HashSet<String>,
    listeners: Listeners,
}

thread_local! {
    static PINS: RefCell<Pins> = RefCell::new(Pins::default());
}

/// Jetzt abfragen (beim Öffnen, alle 30 Sekunden und nach eigenen Änderungen). Twitch liefert sie nur Broadcaster und
/// Mods und meldet Änderungen nicht über EventSub. Nicht erreichbar: alter Stand bleibt.
pub async fn refresh_pinned(channel_id: String) {
    if !can_moderate(&channel_id) || !PINS.with(|p| p.borrow_mut().in_flight.insert(channel_id.clone())) {
        return;
    }
    let result = crate::api::get_pinned_message(&channel_id).await;
    PINS.with(|p| p.borrow_mut().in_flight.remove(&channel_id));
    let Ok(raw) = result else { return };
    let next = raw.as_ref().and_then(ct::to_pinned_message);
    let listeners = PINS.with(|p| {
        let mut p = p.borrow_mut();
        if p.pins.get(&channel_id).cloned().flatten() == next {
            return None;
        }
        p.pins.insert(channel_id.clone(), next);
        Some(p.listeners.all())
    });
    emit(listeners.unwrap_or_default());
}

pub fn pinned(channel_id: &str) -> Option<PinnedMessage> {
    PINS.with(|p| p.borrow().pins.get(channel_id).cloned().flatten())
}

pub fn subscribe_pinned(listener: Listener) -> u32 {
    PINS.with(|p| p.borrow_mut().listeners.add(listener))
}

pub fn unsubscribe_pinned(id: u32) {
    PINS.with(|p| p.borrow_mut().listeners.remove(id));
}

// ---- Aktionen aus dem Chat (Rechtsklick-Menü, AutoMod-Zeile, Leiste der angepinnten Nachricht) ----

fn no_mod_rights() -> &'static str {
    tr("Keine Mod-Rechte in diesem Kanal.", "No mod rights in this channel.")
}
fn missing_scope() -> &'static str {
    tr("Dem Token fehlt die Berechtigung. Bitte einmal neu anmelden.", "The token is missing the permission. Please sign in again once.")
}

fn pin_seconds(seconds: Option<f64>) -> Option<f64> {
    seconds.filter(|s| s.is_finite())
}

/// Anpinnen aus dem Menü; ist die Nachricht schon angepinnt, wird stattdessen die Dauer geändert.
pub async fn pin_from_menu(channel_id: String, message_id: String, seconds: Option<f64>) {
    if !can_moderate(&channel_id) {
        toast("error", no_mod_rights());
        return;
    }
    let seconds = pin_seconds(seconds);
    let (pin_channel, pin_message) = (channel_id.clone(), message_id.clone());
    let result = ct::pin_or_update(
        crate::api::pin("PUT", &channel_id, &message_id, seconds),
        move || async move { crate::api::pin("PATCH", &pin_channel, &pin_message, seconds).await },
        helix_status,
    )
    .await;
    match result {
        Ok(outcome) => {
            toast("success", if outcome == PinOutcome::Pinned { tr("Nachricht angepinnt", "Message pinned") } else { tr("Pin-Dauer geändert", "Pin duration changed") });
            spawn_local(refresh_pinned(channel_id));
        }
        Err(error) => toast(
            "error",
            &match helix_status(&error) {
                Some(400) => tr("Die Pin-Dauer konnte nicht geändert werden.", "The pin duration could not be changed.").to_string(),
                Some(404) => tr("Die Nachricht wurde nicht gefunden (zu alt oder gelöscht).", "The message was not found (too old or deleted).").to_string(),
                Some(401) => missing_scope().to_string(),
                _ => error_text(&error, tr("Anpinnen nicht möglich.", "Pinning not possible.")),
            },
        ),
    }
}

/// Angepinnte Nachricht lösen.
pub async fn unpin_from_menu(channel_id: String, message_id: String) {
    if !can_moderate(&channel_id) {
        toast("error", no_mod_rights());
        return;
    }
    match ct::unpin(crate::api::unpin_message(&channel_id, &message_id), helix_status).await {
        Ok(true) => {
            toast("success", tr("Nachricht gelöst", "Message unpinned"));
            spawn_local(refresh_pinned(channel_id));
        }
        Ok(false) => toast("info", tr("Diese Nachricht ist nicht angepinnt.", "This message is not pinned.")),
        Err(error) if helix_status(&error) == Some(401) => toast("error", missing_scope()),
        Err(error) => toast("error", &error_text(&error, tr("Lösen nicht möglich.", "Unpinning not possible."))),
    }
}

fn pin_error_text(error: &JsValue) -> String {
    if helix_status(error) == Some(401) { missing_scope().to_string() } else { error_text(error, tr("Unbekannter Fehler", "Unknown error")) }
}

/// Dauer einer angepinnten Nachricht ändern (Leiste oben im Chat).
pub async fn change_pin_duration(channel_id: String, message_id: String, seconds: Option<f64>) {
    match crate::api::pin("PATCH", &channel_id, &message_id, pin_seconds(seconds)).await {
        Ok(()) => toast("success", tr("Pin-Dauer geändert", "Pin duration changed")),
        Err(error) => toast("error", &pin_error_text(&error)),
    }
    refresh_pinned(channel_id).await;
}

/// Angepinnte Nachricht lösen (Leiste oben im Chat).
pub async fn release_pin(channel_id: String, message_id: String) {
    match ct::unpin(crate::api::unpin_message(&channel_id, &message_id), helix_status).await {
        Ok(true) => toast("success", tr("Nachricht gelöst", "Message unpinned")),
        Ok(false) => {}
        Err(error) => toast("error", &pin_error_text(&error)),
    }
    refresh_pinned(channel_id).await;
}

/// Text in die Zwischenablage.
pub async fn copy_text(text: String) {
    let clipboard = Reflect::get(&js_sys::global(), &"navigator".into()).ok().and_then(|n| Reflect::get(&n, &"clipboard".into()).ok());
    let write = clipboard.as_ref().and_then(|c| Reflect::get(c, &"writeText".into()).ok()).and_then(|f| f.dyn_into::<Function>().ok());
    let promise = match (clipboard, write) {
        (Some(clipboard), Some(write)) => write.call1(&clipboard, &JsValue::from_str(&text)).ok().and_then(|p| p.dyn_into::<js_sys::Promise>().ok()),
        _ => None,
    };
    let ok = match promise {
        Some(promise) => JsFuture::from(promise).await.is_ok(),
        None => false,
    };
    if ok {
        toast("success", tr("Kopiert", "Copied"));
    } else {
        toast("error", tr("Kopieren nicht möglich", "Copying not possible"));
    }
}

/// AutoMod: zulassen oder ablehnen. Die Zeile aktualisiert sich über EventSub.
pub async fn resolve_automod(channel_id: String, message_id: String, allow: bool) {
    if !can_moderate(&channel_id) {
        return;
    }
    if let Err(error) = crate::api::resolve_automod_message(&message_id, allow).await {
        let text = if helix_status(&error) == Some(400) {
            tr("Die Nachricht wurde bereits bearbeitet oder ist abgelaufen.", "The message was already handled or has expired.").to_string()
        } else {
            error_text(&error, tr("Unbekannter Fehler", "Unknown error"))
        };
        toast("error", &format!("AutoMod: {text}"));
    }
}

// ---- Nutzerkarten ----

#[derive(Default)]
struct Cards {
    windows: CardWindows<Function>,
    listeners: Listeners,
}

thread_local! {
    static CARDS: RefCell<Cards> = RefCell::new(Cards::default());
}

/// Offene Karte ohne die Freigabe der Chat-Pause.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardView {
    pub id: String,
    pub channel: ChannelRef,
    pub user: UserRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub badges: Option<Vec<Badge>>,
    pub x: f64,
    pub y: f64,
    pub pinned: bool,
}

fn viewport() -> (f64, f64) {
    let size = |name: &str| Reflect::get(&js_sys::global(), &name.into()).ok().and_then(|v| v.as_f64());
    (size("innerWidth").unwrap_or(1920.0), size("innerHeight").unwrap_or(1080.0))
}

/// Ändert die Karten, gibt die Chat-Pausen frei und meldet die Änderung.
fn change_cards<T>(change: impl FnOnce(&mut CardWindows<Function>) -> (T, Vec<Function>, bool)) -> T {
    let (result, released, changed, listeners) = CARDS.with(|c| {
        let mut c = c.borrow_mut();
        let (result, released, changed) = change(&mut c.windows);
        (result, released, changed, c.listeners.all())
    });
    for release in released {
        let _ = release.call0(&JsValue::NULL);
    }
    if changed {
        emit(listeners);
    }
    result
}

/// Neue Karte öffnen; eine offene, nicht angepinnte wird ersetzt. `release` hebt die Chat-Pause wieder auf.
pub fn open_card(channel: ChannelRef, user: UserRef, badges: Option<Vec<Badge>>, x: f64, y: f64, release: Option<Function>) {
    change_cards(|cards| ((), cards.open(channel, user, badges, x, y, release, viewport()), true));
}

pub fn close_card(id: &str) {
    change_cards(|cards| {
        let before = cards.windows.len();
        let released = cards.close(id);
        ((), released, cards.windows.len() != before)
    });
}

/// Alle nicht angepinnten Karten schließen (Klick daneben, Esc).
pub fn close_unpinned_cards() {
    change_cards(|cards| match cards.close_unpinned() {
        Some(released) => ((), released, true),
        None => ((), Vec::new(), false),
    });
}

pub fn set_card_pinned(id: &str, pinned: bool) {
    change_cards(|cards| ((), cards.set_pinned(id, pinned), true));
}

pub fn move_card(id: &str, x: f64, y: f64) {
    change_cards(|cards| {
        cards.move_to(id, x, y, viewport());
        ((), Vec::new(), true)
    });
}

/// Zuletzt angefasste Karte nach vorne holen.
pub fn focus_card(id: &str) {
    change_cards(|cards| ((), Vec::new(), cards.focus(id)));
}

pub fn card_windows() -> Vec<CardView> {
    CARDS.with(|c| {
        c.borrow()
            .windows
            .windows
            .iter()
            .map(|w| CardView { id: w.id.clone(), channel: w.channel.clone(), user: w.user.clone(), badges: w.badges.clone(), x: w.x, y: w.y, pinned: w.pinned })
            .collect()
    })
}

pub fn subscribe_cards(listener: Listener) -> u32 {
    CARDS.with(|c| c.borrow_mut().listeners.add(listener))
}

pub fn unsubscribe_cards(id: u32) {
    CARDS.with(|c| c.borrow_mut().listeners.remove(id));
}

/// Daten der Nutzerkarte beim Öffnen; einzelne Fehler blenden nur den jeweiligen Wert aus. Folgedatum nur in
/// Mod-Kanälen, Ban-Status nur im eigenen Kanal (Twitch liefert ihn Mods nicht).
pub async fn load_card_data(channel_id: &str, user_id: &str, moderator: bool, broadcaster: bool) -> Value {
    use crate::api;
    let followed = async {
        if moderator {
            api::get_followed_at(channel_id, user_id).await.ok()
        } else {
            None
        }
    };
    let ban = async {
        if broadcaster {
            api::get_own_channel_ban(channel_id, user_id).await.ok().flatten()
        } else {
            None
        }
    };
    let (user, followers, live, followed_at, ban) =
        futures::join!(api::get_user_by_id(user_id), api::get_follower_total(user_id), api::is_user_live(user_id), followed, ban);
    let user = user.ok().flatten();
    let text = |v: &Value, name: &str| v.get(name).and_then(Value::as_str).unwrap_or_default().to_string();
    let ban = ban.map(|ban| {
        let until = ban.get("expires_at").and_then(Value::as_str).filter(|e| !e.is_empty()).and_then(multiyap_core::time::parse_rfc3339_ms);
        let optional = |name: &str| Some(text(&ban, name)).filter(|t| !t.is_empty());
        json!({ "type": if until.is_some() { "timeout" } else { "ban" }, "by": optional("moderator_name"), "reason": optional("reason"), "until": until })
    });
    json!({
        "user": user.as_ref().map(|u| json!({ "displayName": text(u, "display_name"), "profileImageUrl": text(u, "profile_image_url"), "createdAt": text(u, "created_at") })),
        "followers": followers.ok(),
        "followQueried": followed_at.is_some(),
        "followedAt": followed_at.flatten(),
        "live": live.unwrap_or(false),
        "ban": ban,
        "error": if user.is_some() { Value::Null } else { json!(tr("Nutzer konnte nicht geladen werden.", "User could not be loaded.")) },
    })
}

/// Twitchs eigene Nutzerkarte als Popup (Button „Twitch Usercard“, Befehl /user); dort ist die Mod-Historie sichtbar,
/// die über die API nicht abrufbar ist.
pub fn open_twitch_usercard(channel_login: &str, user_login: &str) {
    let url = format!(
        "https://www.twitch.tv/popout/{}/viewercard/{}",
        multiyap_core::twitch::url_component(channel_login),
        multiyap_core::twitch::url_component(user_login)
    );
    if let Some(window) = web_sys::window() {
        let _ = window.open_with_url_and_target_and_features(&url, "_blank", "popup,width=420,height=720,noopener,noreferrer");
    }
}

// ---- Für TypeScript (bis die übrigen Teile in Rust sind) ----

