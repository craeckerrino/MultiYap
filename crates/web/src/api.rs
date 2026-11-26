//! Ausgewählte Helix-Endpunkte (bisher src/platforms/twitch/api.ts und moderation.ts; am 15.11.2026 in der
//! offiziellen Doku geprüft). Anfragen über `net::helix_call` mit den Zugangsdaten aus der Anmeldung in Rust.
//! Fehler bleiben wie bisher { helixStatus, message } (TypeScript macht daraus HelixError) oder Netzwerkfehler.
//! Antworten von Twitch gehen unverändert als JSON weiter, wo TypeScript sie bisher unverändert nutzte.

use multiyap_core::i18n::tr;
use std::cell::RefCell;
use std::collections::HashMap;

use multiyap_core::twitch::helix_api::{self as h, MAX_CHATTER_PAGES, MAX_REASON_LENGTH};
use futures::future::{FutureExt, LocalBoxFuture, Shared};
use serde::Serialize;
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

use crate::net::{helix_call, helix_error, helix_user_id};

type Pairs = Vec<(String, Vec<String>)>;
type HelixResult<T> = Result<T, JsValue>;
type SharedSets = Shared<LocalBoxFuture<'static, HelixResult<Value>>>;

fn pairs(list: &[(&str, &str)]) -> Pairs {
    list.iter().map(|(k, v)| (k.to_string(), vec![v.to_string()])).collect()
}

/// Helix-Anfrage mit JSON-Körper; Antwort als JSON (ohne Inhalt: Null).
async fn request(path: &str, method: &str, query: Pairs, body: Option<Value>) -> HelixResult<Value> {
    let body = body.map(|b| JsValue::from_str(&b.to_string()));
    let result = helix_call(path, method, &query, body).await?;
    if result.is_undefined() || result.is_null() {
        return Ok(Value::Null);
    }
    serde_wasm_bindgen::from_value(result).map_err(|_| helix_error(500, tr("Antwort von Twitch ist kein JSON.", "Response from Twitch is not JSON.")))
}

async fn get(path: &str, query: Pairs) -> HelixResult<Value> {
    request(path, "GET", query, None).await
}

fn first(response: &Value) -> Option<Value> {
    response.get("data").and_then(|d| d.get(0)).cloned()
}

fn data(response: &Value) -> Vec<Value> {
    response.get("data").and_then(Value::as_array).cloned().unwrap_or_default()
}

fn cursor(response: &Value) -> Option<String> {
    response.get("pagination").and_then(|p| p.get("cursor")).and_then(Value::as_str).filter(|c| !c.is_empty()).map(str::to_string)
}

/// Eigene Nutzer-ID; ohne Anmeldung der Fehler „Nicht angemeldet.“ (wie bisher getCredentials).
fn own_id() -> HelixResult<String> {
    helix_user_id().ok_or_else(|| helix_error(401, tr("Nicht angemeldet.", "Not signed in.")))
}

/// broadcaster_id und moderator_id (eigenes Konto).
fn ids(broadcaster_id: &str) -> HelixResult<Pairs> {
    Ok(pairs(&[("broadcaster_id", broadcaster_id), ("moderator_id", &own_id()?)]))
}

fn with(mut query: Pairs, key: &str, value: &str) -> Pairs {
    query.push((key.into(), vec![value.into()]));
    query
}

#[derive(Default)]
struct Caches {
    /// Nutzer für die Sitzung: nach Login (klein geschrieben) und nach ID.
    users_by_login: HashMap<String, Value>,
    users_by_id: HashMap<String, Value>,
    global_badges: Option<SharedSets>,
    channel_badges: HashMap<String, SharedSets>,
}

thread_local! {
    static CACHES: RefCell<Caches> = RefCell::new(Caches::default());
}

// ---- Nutzer, Follower, Streams ----

pub async fn get_user_by_login(login: &str) -> HelixResult<Option<Value>> {
    let key = login.to_lowercase();
    if let Some(user) = CACHES.with(|c| c.borrow().users_by_login.get(&key).cloned()) {
        return Ok(Some(user));
    }
    let user = first(&get("/users", pairs(&[("login", &key)])).await?);
    if let Some(user) = &user {
        CACHES.with(|c| c.borrow_mut().users_by_login.insert(key, user.clone()));
    }
    Ok(user)
}

pub async fn get_user_by_id(id: &str) -> HelixResult<Option<Value>> {
    if let Some(user) = CACHES.with(|c| c.borrow().users_by_id.get(id).cloned()) {
        return Ok(Some(user));
    }
    let user = first(&get("/users", pairs(&[("id", id)])).await?);
    if let Some(user) = &user {
        let login = user.get("login").and_then(Value::as_str).unwrap_or_default().to_string();
        CACHES.with(|c| {
            let mut caches = c.borrow_mut();
            caches.users_by_id.insert(id.to_string(), user.clone());
            caches.users_by_login.insert(login, user.clone());
        });
    }
    Ok(user)
}

/// Namensfarben von bis zu 100 Personen in einer Anfrage (Get User Chat Color): Nutzer-ID und Farbe als Hex-Text,
/// leer, wenn die Person keine Farbe gewählt hat.
pub async fn get_chat_colors(user_ids: &[String]) -> HelixResult<Vec<(String, String)>> {
    let response = get("/chat/color", vec![("user_id".to_string(), user_ids.to_vec())]).await?;
    Ok(data(&response)
        .iter()
        .filter_map(|u| Some((u.get("user_id")?.as_str()?.to_string(), u.get("color").and_then(Value::as_str).unwrap_or_default().to_string())))
        .collect())
}

/// Follower-Anzahl einer Person (ohne Mod-Rechte liefert Twitch nur die Gesamtzahl).
pub async fn get_follower_total(user_id: &str) -> HelixResult<f64> {
    let response = get("/channels/followers", pairs(&[("broadcaster_id", user_id), ("first", "1")])).await?;
    Ok(response.get("total").and_then(Value::as_f64).unwrap_or(0.0))
}

/// Seit wann folgt die Person dem Kanal? Nur mit Mod-Rechten im Kanal. None: folgt nicht.
pub async fn get_followed_at(channel_id: &str, user_id: &str) -> HelixResult<Option<String>> {
    let response = get("/channels/followers", pairs(&[("broadcaster_id", channel_id), ("user_id", user_id)])).await?;
    Ok(first(&response).and_then(|f| f.get("followed_at").and_then(Value::as_str).map(str::to_string)))
}

/// Ist die Person gerade live?
pub async fn is_user_live(user_id: &str) -> HelixResult<bool> {
    let response = get("/streams", pairs(&[("user_id", user_id), ("first", "1")])).await?;
    Ok(!data(&response).is_empty())
}

/// Laufende Streams von bis zu 100 Kanälen in einer Anfrage (Get Streams).
pub async fn get_streams(user_ids: &[String]) -> HelixResult<Vec<Value>> {
    let query = vec![("user_id".to_string(), user_ids.to_vec()), ("first".to_string(), vec!["100".to_string()])];
    Ok(data(&get("/streams", query).await?))
}

/// Andere Kanäle einer laufenden Shared-Chat-Sitzung (Get Shared Chat Session), ohne den Kanal selbst.
pub async fn get_shared_chat_participants(channel_id: &str) -> HelixResult<Vec<String>> {
    let response = get("/shared_chat/session", pairs(&[("broadcaster_id", channel_id)])).await?;
    Ok(multiyap_core::streams::shared_with(&response, channel_id))
}

// ---- Emotes und Bits ----

/// Get User Emotes: ohne Kanal alle Seiten (höchstens 50, gegen Endlosschleifen bei fehlerhaften Antworten), mit Kanal
/// nur die erste Seite (für dessen Follower-Emotes).
pub async fn get_user_emotes(user_id: &str, broadcaster_id: Option<&str>) -> HelixResult<Vec<Value>> {
    let mut all = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..50 {
        let mut query = pairs(&[("user_id", user_id)]);
        if let Some(broadcaster) = broadcaster_id {
            query = with(query, "broadcaster_id", broadcaster);
        }
        if let Some(after) = &after {
            query = with(query, "after", after);
        }
        let response = get("/chat/emotes/user", query).await?;
        all.extend(data(&response));
        after = cursor(&response);
        if after.is_none() || broadcaster_id.is_some() {
            break;
        }
    }
    Ok(all)
}

/// Get Global Emotes.
pub async fn get_global_emotes() -> HelixResult<Vec<Value>> {
    Ok(data(&get("/chat/emotes/global", Vec::new()).await?))
}

/// Get Cheermotes eines Kanals (globale und eigene, mit Stufen, Farbe und Bildern).
pub async fn get_cheermotes(broadcaster_id: &str) -> HelixResult<Vec<Value>> {
    Ok(data(&get("/bits/cheermotes", pairs(&[("broadcaster_id", broadcaster_id)])).await?))
}

// ---- Bans, Chatter, Rollen ----

/// Aktueller Ban oder Timeout. Laut Doku nur im eigenen Kanal (broadcaster_id = eigenes Konto).
pub async fn get_own_channel_ban(broadcaster_id: &str, user_id: &str) -> HelixResult<Option<Value>> {
    Ok(first(&get("/moderation/banned", pairs(&[("broadcaster_id", broadcaster_id), ("user_id", user_id)])).await?))
}

/// Aktuelle Bans und Timeouts mehrerer Personen im eigenen Kanal (Get Banned Users, Doku geprüft am 15.11.2026:
/// broadcaster_id muss das eigene Konto sein, user_id bis zu 100 Mal, Scope moderator:manage:banned_users).
/// Abgelaufene Timeouts und aufgehobene Bans sind nicht enthalten.
pub async fn get_own_channel_bans(broadcaster_id: &str, user_ids: &[String]) -> HelixResult<Vec<Value>> {
    let mut result = Vec::new();
    for chunk in user_ids.chunks(100) {
        let query = vec![("broadcaster_id".to_string(), vec![broadcaster_id.to_string()]), ("user_id".to_string(), chunk.to_vec())];
        result.extend(data(&get("/moderation/banned", query).await?));
    }
    Ok(result)
}

#[derive(Serialize)]
pub struct Chatters {
    pub chatters: Vec<Value>,
    pub total: f64,
    pub truncated: bool,
}

/// Wer gerade im Chat ist (Get Chatters, Doku geprüft am 15.11.2026: Broadcaster oder Mod, moderator_id = eigenes
/// Konto, Scope moderator:read:chatters, bis 1000 je Seite). Twitch aktualisiert die Liste mit Verzögerung.
pub async fn get_chatters(broadcaster_id: &str, moderator_id: &str) -> HelixResult<Chatters> {
    let mut chatters = Vec::new();
    let mut total = 0.0;
    let mut after: Option<String> = None;
    for _ in 0..MAX_CHATTER_PAGES {
        let mut query = pairs(&[("broadcaster_id", broadcaster_id), ("moderator_id", moderator_id), ("first", "1000")]);
        if let Some(after) = &after {
            query = with(query, "after", after);
        }
        let response = get("/chat/chatters", query).await?;
        chatters.extend(data(&response));
        total = response.get("total").and_then(Value::as_f64).unwrap_or(0.0);
        after = cursor(&response);
        if after.is_none() {
            return Ok(Chatters { chatters, total, truncated: false });
        }
    }
    Ok(Chatters { chatters, total, truncated: true })
}

/// Alle IDs einer seitenweisen Liste mit user_id (bis 100 je Seite).
async fn all_user_ids(path: &str, broadcaster_id: &str) -> HelixResult<Vec<String>> {
    let mut ids = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let mut query = pairs(&[("broadcaster_id", broadcaster_id), ("first", "100")]);
        if let Some(after) = &after {
            query = with(query, "after", after);
        }
        let response = get(path, query).await?;
        for entry in data(&response) {
            if let Some(id) = entry.get("user_id").and_then(Value::as_str) {
                if !ids.iter().any(|known| known == id) {
                    ids.push(id.to_string());
                }
            }
        }
        after = cursor(&response);
        if after.is_none() {
            return Ok(ids);
        }
    }
}

/// Kanäle, in denen das Konto Mod ist (Get Moderated Channels, Scope user:read:moderated_channels, bis 100 je Seite).
pub async fn get_moderated_channels(user_id: &str) -> HelixResult<Vec<String>> {
    let mut ids = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let mut query = pairs(&[("user_id", user_id), ("first", "100")]);
        if let Some(after) = &after {
            query = with(query, "after", after);
        }
        let response = get("/moderation/channels", query).await?;
        ids.extend(data(&response).iter().filter_map(|e| e.get("broadcaster_id").and_then(Value::as_str).map(str::to_string)));
        after = cursor(&response);
        if after.is_none() {
            return Ok(ids);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChannelRoles {
    pub moderators: Vec<String>,
    pub vips: Vec<String>,
}

/// Mods und VIPs des eigenen Kanals (Get Moderators, Get VIPs; Doku geprüft am 15.11.2026: nur der Broadcaster,
/// hier über channel:manage:moderators und channel:manage:vips). In fremden Kanälen liefert Twitch das nicht.
pub async fn get_own_channel_roles(broadcaster_id: &str) -> HelixResult<ChannelRoles> {
    let (moderators, vips) = futures::join!(all_user_ids("/moderation/moderators", broadcaster_id), all_user_ids("/channels/vips", broadcaster_id));
    Ok(ChannelRoles { moderators: moderators?, vips: vips? })
}

// ---- Abzeichen (einmal je Sitzung, gleichzeitige Anfragen teilen sich eine) ----

fn badge_sets(path: &'static str, query: Pairs) -> SharedSets {
    async move { get(path, query).await.map(|response| Value::Array(data(&response))) }.boxed_local().shared()
}

/// Globale Abzeichen als Liste von { set_id, versions }. Schlägt das Laden fehl, wird beim nächsten Mal neu geladen.
pub async fn get_global_badges() -> HelixResult<Value> {
    let shared = CACHES.with(|c| c.borrow_mut().global_badges.get_or_insert_with(|| badge_sets("/chat/badges/global", Vec::new())).clone());
    let result = shared.await;
    if result.is_err() {
        CACHES.with(|c| c.borrow_mut().global_badges = None);
    }
    result
}

/// Abzeichen eines Kanals, einmal je Kanal und Sitzung.
pub async fn get_channel_badges(broadcaster_id: &str) -> HelixResult<Value> {
    let key = broadcaster_id.to_string();
    let shared = CACHES.with(|c| {
        c.borrow_mut().channel_badges.entry(key.clone()).or_insert_with(|| badge_sets("/chat/badges", pairs(&[("broadcaster_id", &key)]))).clone()
    });
    let result = shared.await;
    if result.is_err() {
        CACHES.with(|c| c.borrow_mut().channel_badges.remove(&key));
    }
    result
}

// ---- Chat ----

#[derive(Serialize)]
pub struct SendResult {
    pub sent: bool,
    /// Grund von Twitch, falls die Nachricht nicht gesendet wurde.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Send Chat Message: höchstens 500 Zeichen. Antwort: reply_parent_message_id laut Doku (geprüft am 15.11.2026).
pub async fn send_chat_message(broadcaster_id: &str, message: &str, reply_to_id: Option<&str>) -> HelixResult<SendResult> {
    let mut body = json!({ "broadcaster_id": broadcaster_id, "sender_id": own_id()?, "message": message });
    if let Some(reply) = reply_to_id.filter(|r| !r.is_empty()) {
        body["reply_parent_message_id"] = json!(reply);
    }
    let response = request("/chat/messages", "POST", Vec::new(), Some(body)).await?;
    let (sent, reason) = h::send_result(&response).ok_or_else(|| helix_error(500, tr("Unerwartete Antwort von Twitch.", "Unexpected response from Twitch.")))?;
    Ok(SendResult { sent, reason })
}

/// Get Chat Settings (Doku geprüft am 15.11.2026). Mit moderator_id kommen auch die Verzögerungsfelder.
pub async fn get_chat_settings(broadcaster_id: &str, moderator_id: Option<&str>) -> HelixResult<Option<Value>> {
    let mut query = pairs(&[("broadcaster_id", broadcaster_id)]);
    if let Some(moderator) = moderator_id {
        query = with(query, "moderator_id", moderator);
    }
    Ok(first(&get("/chat/settings", query).await?).map(|s| h::chat_modes_from_settings(&s)))
}

/// Update Chat Settings mit einer Teiländerung der Chat-Modi. Nur mit Mod-Rechten.
pub async fn update_chat_settings(broadcaster_id: &str, patch: &Value) -> HelixResult<Option<Value>> {
    let response = request("/chat/settings", "PATCH", ids(broadcaster_id)?, Some(h::chat_settings_body(patch))).await?;
    Ok(first(&response).map(|s| h::chat_modes_from_settings(&s)))
}

// ---- Moderation ----

/// Ban (ohne Dauer) oder Timeout (mit Dauer). Leerer Grund wird nicht mitgeschickt.
pub async fn ban_user(broadcaster_id: &str, user_id: &str, duration_seconds: Option<f64>, reason: Option<&str>) -> HelixResult<()> {
    request("/moderation/bans", "POST", ids(broadcaster_id)?, Some(h::ban_body(user_id, duration_seconds, reason))).await.map(drop)
}

/// Hebt Ban und Timeout auf.
pub async fn unban_user(broadcaster_id: &str, user_id: &str) -> HelixResult<()> {
    request("/moderation/bans", "DELETE", with(ids(broadcaster_id)?, "user_id", user_id), None).await.map(drop)
}

/// Löscht eine Nachricht (höchstens 6 Stunden alt, nicht vom Broadcaster oder anderen Mods).
pub async fn delete_message(broadcaster_id: &str, message_id: &str) -> HelixResult<()> {
    request("/moderation/chat", "DELETE", with(ids(broadcaster_id)?, "message_id", message_id), None).await.map(drop)
}

/// Zurückgehaltene AutoMod-Nachricht zulassen oder ablehnen.
pub async fn resolve_automod_message(message_id: &str, allow: bool) -> HelixResult<()> {
    let body = json!({ "user_id": own_id()?, "msg_id": message_id, "action": if allow { "ALLOW" } else { "DENY" } });
    request("/moderation/automod/message", "POST", Vec::new(), Some(body)).await.map(drop)
}

/// Verwarnen: Twitch verlangt einen Grund.
pub async fn warn_user(broadcaster_id: &str, user_id: &str, reason: &str) -> HelixResult<()> {
    request("/moderation/warnings", "POST", ids(broadcaster_id)?, Some(h::warn_body(user_id, reason))).await.map(drop)
}

/// Überwachen (ACTIVE_MONITORING) oder Einschränken (RESTRICTED). Antwort: { status, types } oder None.
pub async fn set_suspicious_status(broadcaster_id: &str, user_id: &str, status: &str) -> HelixResult<Option<Value>> {
    let response = request("/moderation/suspicious_users", "POST", ids(broadcaster_id)?, Some(json!({ "user_id": user_id, "status": status }))).await?;
    Ok(first(&response))
}

/// Status eines verdächtigen Nutzers entfernen.
pub async fn remove_suspicious_status(broadcaster_id: &str, user_id: &str) -> HelixResult<()> {
    request("/moderation/suspicious_users", "DELETE", with(ids(broadcaster_id)?, "user_id", user_id), None).await.map(drop)
}

/// Chat leeren: „Delete Chat Messages“ ohne message_id.
pub async fn clear_chat(broadcaster_id: &str) -> HelixResult<()> {
    request("/moderation/chat", "DELETE", ids(broadcaster_id)?, None).await.map(drop)
}

/// Ankündigung (Doku geprüft am 15.11.2026): höchstens 500 Zeichen, eine alle 2 Sekunden.
pub async fn send_announcement(broadcaster_id: &str, message: &str, color: &str) -> HelixResult<()> {
    let body = json!({ "message": h::truncate_utf16(message, MAX_REASON_LENGTH), "color": color });
    request("/chat/announcements", "POST", ids(broadcaster_id)?, Some(body)).await.map(drop)
}

/// Shoutout (Doku geprüft am 15.11.2026): alle 2 Minuten, denselben Kanal höchstens einmal pro Stunde, nur live.
pub async fn send_shoutout(from_broadcaster_id: &str, to_broadcaster_id: &str) -> HelixResult<()> {
    let query = pairs(&[("from_broadcaster_id", from_broadcaster_id), ("to_broadcaster_id", to_broadcaster_id), ("moderator_id", &own_id()?)]);
    request("/chat/shoutouts", "POST", query, None).await.map(drop)
}

/// Mods und VIPs (Doku geprüft am 15.11.2026): nur der Broadcaster selbst, höchstens 10 Änderungen in 10 Sekunden.
/// Mods: /moderation/moderators (channel:manage:moderators), VIPs: /channels/vips (channel:manage:vips).
pub async fn change_role(path: &str, method: &str, broadcaster_id: &str, user_id: &str) -> HelixResult<()> {
    request(path, method, pairs(&[("broadcaster_id", broadcaster_id), ("user_id", user_id)]), None).await.map(drop)
}

/// Nachricht anpinnen (PUT) oder die Dauer ändern (PATCH), Doku geprüft am 15.11.2026: nur Broadcaster oder Mods,
/// Dauer 30 bis 1800 Sekunden oder ohne Dauer bis zum Streamende. Ein neuer Pin ersetzt den alten.
pub async fn pin(method: &str, broadcaster_id: &str, message_id: &str, duration_seconds: Option<f64>) -> HelixResult<()> {
    let mut query = with(ids(broadcaster_id)?, "message_id", message_id);
    if let Some(duration) = duration_seconds {
        query = with(query, "duration_seconds", &h::number(duration).to_string());
    }
    request("/chat/pins", method, query, None).await.map(drop)
}

/// Angepinnte Nachricht lösen (DELETE /chat/pins, 404 wenn nicht angepinnt).
pub async fn unpin_message(broadcaster_id: &str, message_id: &str) -> HelixResult<()> {
    request("/chat/pins", "DELETE", with(ids(broadcaster_id)?, "message_id", message_id), None).await.map(drop)
}

/// GET /chat/pins: nur Broadcaster und Mods. None heißt: nichts angepinnt.
pub async fn get_pinned_message(broadcaster_id: &str) -> HelixResult<Option<Value>> {
    Ok(first(&get("/chat/pins", ids(broadcaster_id)?).await?))
}

// ---- Für TypeScript (bis die übrigen Dienste in Rust sind) ----

