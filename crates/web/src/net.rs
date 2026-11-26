//! Netzwerkzugriffe aus Rust mit den Mitteln des Browsers (in diesem Crate, damit die Anmeldung sie direkt nutzt).
//!
//! `fetch` und `setTimeout` kommen vom globalen Objekt, nicht von `window`: So läuft es im Browser und in den Tests
//! unter Node gleich. Token stehen nie in Fehlermeldungen und werden nie ausgegeben.

use multiyap_core::i18n::tr;
use std::cell::RefCell;

use multiyap_core::net;
use multiyap_core::twitch::auth;
use js_sys::{Function, Object, Promise, Reflect};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Headers, Request, RequestInit, Response};

const HELIX_URL: &str = "https://api.twitch.tv/helix";

pub fn global_function(name: &str) -> Result<Function, JsValue> {
    Reflect::get(&js_sys::global(), &JsValue::from_str(name))?
        .dyn_into::<Function>()
        .map_err(|_| JsValue::from_str(&format!("{name} fehlt")))
}

/// Wartet `ms` Millisekunden (setTimeout).
pub async fn sleep(ms: f64) {
    let promise = Promise::new(&mut |resolve, _reject| {
        if let Ok(set_timeout) = global_function("setTimeout") {
            let _ = set_timeout.call2(&JsValue::NULL, &resolve, &JsValue::from_f64(ms));
        }
    });
    let _ = JsFuture::from(promise).await;
}

/// Jetzige Zeit in Millisekunden seit 1970.
pub fn now_ms() -> f64 {
    js_sys::Date::now()
}

/// Sendet eine Anfrage mit dem fetch des Browsers.
pub async fn fetch(url: &str, init: &RequestInit) -> Result<Response, JsValue> {
    let request = Request::new_with_str_and_init(url, init)?;
    let fetch = global_function("fetch")?;
    let promise: Promise = fetch.call1(&js_sys::global(), &request)?.dyn_into()?;
    JsFuture::from(promise).await?.dyn_into::<Response>()
}

/// Antwort als JSON, oder None, wenn sie keins ist.
pub async fn json_of(response: &Response) -> Option<JsValue> {
    let promise = response.json().ok()?;
    JsFuture::from(promise).await.ok()
}

/// Antwort als JSON-Wert für die Auswertung in Rust.
pub async fn json_value(response: &Response) -> Option<serde_json::Value> {
    json_of(response).await.and_then(|v| serde_wasm_bindgen::from_value(v).ok())
}

/// Fehlerobjekt für TypeScript: { helixStatus, message } (src/platforms/twitch/helix.ts macht daraus HelixError).
pub fn helix_error(status: u16, message: &str) -> JsValue {
    let error = Object::new();
    let _ = Reflect::set(&error, &"helixStatus".into(), &JsValue::from(status));
    let _ = Reflect::set(&error, &"message".into(), &JsValue::from_str(message));
    error.into()
}

/// Zugangsdaten, die Tests vorgeben (configureHelix); sonst gilt die Anmeldung in Rust.
struct HelixOverride {
    /// () => { userId, clientId, accessToken }; wirft, wenn niemand angemeldet ist.
    get_credentials: Function,
    /// () => Promise<boolean>: Token verlängern, true heißt erneut versuchen.
    recover_token: Function,
}

thread_local! {
    static OVERRIDE: RefCell<Option<HelixOverride>> = const { RefCell::new(None) };
}

/// Nur für Tests: Zugangsdaten und Verlängern aus TypeScript statt aus der Anmeldung.
pub fn set_helix_override(get_credentials: Function, recover_token: Function) {
    OVERRIDE.with(|cell| *cell.borrow_mut() = Some(HelixOverride { get_credentials, recover_token }));
}

/// (Nutzer-ID, Client-ID, Token) des angemeldeten Kontos.
fn credentials() -> Result<(String, String, String), JsValue> {
    let get = OVERRIDE.with(|cell| cell.borrow().as_ref().map(|c| c.get_credentials.clone()));
    if let Some(get) = get {
        let value = get.call0(&JsValue::NULL)?;
        let field = |name: &str| Reflect::get(&value, &name.into()).ok().and_then(|v| v.as_string()).unwrap_or_default();
        return Ok((field("userId"), field("clientId"), field("accessToken")));
    }
    crate::auth::helix_credentials().ok_or_else(|| helix_error(401, tr("Nicht angemeldet.", "Not signed in.")))
}

/// Nutzer-ID des angemeldeten Kontos; None, wenn niemand angemeldet ist.
pub fn helix_user_id() -> Option<String> {
    credentials().ok().map(|(user_id, _, _)| user_id).filter(|id| !id.is_empty())
}

async fn recover() -> bool {
    let recover = OVERRIDE.with(|cell| cell.borrow().as_ref().map(|c| c.recover_token.clone()));
    let Some(recover) = recover else {
        return crate::auth::recover_token().await;
    };
    let Ok(result) = recover.call0(&JsValue::NULL) else {
        return false;
    };
    let Ok(promise) = result.dyn_into::<Promise>() else {
        return false;
    };
    JsFuture::from(promise).await.ok().and_then(|v| v.as_bool()).unwrap_or(false)
}

/// Helix-Anfrage. `body`: JSON als Text. Fehler als { helixStatus, message } oder Netzwerkfehler. Bei Rate-Limit
/// wird einmal geordnet bis zum Reset gewartet, bei 401 einmal verlängert.
pub async fn helix_call(path: &str, method: &str, query: &[(String, Vec<String>)], body: Option<JsValue>) -> Result<JsValue, JsValue> {
    let (_, mut client_id, mut token) = credentials()?;
    let url = net::build_url(HELIX_URL, path, query);
    let mut renewed = false;
    let mut attempt = 0;

    while attempt < 2 {
        let headers = Headers::new()?;
        headers.append("Authorization", &format!("Bearer {token}"))?;
        headers.append("Client-Id", &client_id)?;
        let init = RequestInit::new();
        init.set_method(method);
        if let Some(body) = &body {
            headers.append("Content-Type", "application/json")?;
            init.set_body(body);
        }
        init.set_headers(&headers);
        let response = fetch(&url, &init).await?;
        let status = response.status();

        if status == 429 && attempt == 0 {
            let reset = response.headers().get("Ratelimit-Reset").ok().flatten();
            sleep(net::rate_limit_delay_ms(reset.as_deref(), now_ms())).await;
            attempt += 1;
            continue;
        }
        // Token abgelaufen: bei Anmeldung per Code einmal verlängern und die Anfrage wiederholen, sonst prüfen
        // lassen (Hinweis „Bitte neu anmelden“).
        if status == 401 && !renewed {
            renewed = true;
            if recover().await {
                (_, client_id, token) = credentials()?;
                continue;
            }
        }
        if !response.ok() {
            let data = json_value(&response).await;
            return Err(helix_error(status, &net::error_message(status, data.as_ref())));
        }
        if status == 204 {
            return Ok(JsValue::UNDEFINED);
        }
        return json_of(&response).await.ok_or_else(|| helix_error(status, tr("Antwort von Twitch ist kein JSON.", "Response from Twitch is not JSON.")));
    }
    Err(helix_error(429, tr("Zu viele Anfragen an Twitch. Bitte kurz warten.", "Too many requests to Twitch. Please wait a moment.")))
}

// ---- Drittanbieter ohne Zugangsdaten (7TV, BTTV, FFZ) ----

/// Höchstens so lange auf einen Dienst warten.
const THIRD_PARTY_TIMEOUT_MS: i32 = 15_000;

/// Lädt JSON von einem Drittanbieter ohne Zugangsdaten (keine Cookies, kein Token). 404 heißt: für diesen Kanal
/// gibt es beim Dienst nichts (None).
pub async fn fetch_json(url: &str) -> Result<Option<serde_json::Value>, JsValue> {
    let controller = web_sys::AbortController::new()?;
    let init = RequestInit::new();
    init.set_credentials(web_sys::RequestCredentials::Omit);
    init.set_signal(Some(&controller.signal()));
    // Nach der Frist abbrechen.
    let abort = Closure::once_into_js({
        let controller = controller.clone();
        move || controller.abort()
    });
    let set_timeout = global_function("setTimeout")?;
    let timer = set_timeout.call2(&JsValue::NULL, &abort, &JsValue::from(THIRD_PARTY_TIMEOUT_MS))?;
    let result = fetch(url, &init).await;
    if let Ok(clear_timeout) = global_function("clearTimeout") {
        let _ = clear_timeout.call1(&JsValue::NULL, &timer);
    }
    let response = result?;
    if response.status() == 404 {
        return Ok(None);
    }
    if !response.ok() {
        let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
        return Err(js_sys::Error::new(&format!("{host}: HTTP {}", response.status())).into());
    }
    Ok(json_value(&response).await)
}

// ---- Verlauf über recent-messages (Auswertung in crates/core/src/history.rs) ----

/// Lädt Verlaufszeilen ohne Zugangsdaten (keine Cookies, kein Token). Fehler als kurze Meldung.
pub async fn fetch_recent_messages(login: &str, after: Option<f64>, hide_moderated: bool) -> Result<Vec<String>, String> {
    let url = multiyap_core::history::history_url(login, after, hide_moderated);
    let init = RequestInit::new();
    init.set_credentials(web_sys::RequestCredentials::Omit);
    let response = fetch(&url, &init).await.map_err(|_| tr("Dienst nicht erreichbar", "Service not reachable").to_string())?;
    if !response.ok() {
        return Err(multiyap_core::tf!("Antwort {}", "Response {}", response.status()));
    }
    let body = json_value(&response).await;
    multiyap_core::history::history_lines(body.as_ref())
}

// ---- Anmeldung bei Twitch (id.twitch.tv); Auswertung in crates/core/src/twitch/auth.rs ----

const VALIDATE_URL: &str = "https://id.twitch.tv/oauth2/validate";
const REVOKE_URL: &str = "https://id.twitch.tv/oauth2/revoke";
const DEVICE_URL: &str = "https://id.twitch.tv/oauth2/device";
const TOKEN_URL: &str = "https://id.twitch.tv/oauth2/token";

async fn post_form(url: &str, pairs: &[(&str, &str)]) -> Result<Response, JsValue> {
    let headers = Headers::new()?;
    headers.append("Content-Type", "application/x-www-form-urlencoded")?;
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_headers(&headers);
    init.set_body(&JsValue::from_str(&auth::form_body(pairs)));
    fetch(url, &init).await
}

/// Prüft ein Token bei Twitch (Pflicht beim Start und danach stündlich).
pub async fn validate_token(access_token: &str) -> auth::ValidateResult {
    let result = async {
        let headers = Headers::new()?;
        headers.append("Authorization", &format!("OAuth {access_token}"))?;
        let init = RequestInit::new();
        init.set_headers(&headers);
        fetch(VALIDATE_URL, &init).await
    }
    .await;
    match result {
        Ok(response) => {
            let body = if response.ok() { json_value(&response).await } else { None };
            auth::interpret_validate(response.status(), response.ok(), body.as_ref())
        }
        Err(_) => auth::ValidateResult::Unreachable,
    }
}

/// Widerruft ein Token bei Twitch. Fehler werden ignoriert: lokal wird trotzdem abgemeldet.
pub async fn revoke_token(client_id: &str, token: &str) {
    let _ = post_form(REVOKE_URL, &[("client_id", client_id), ("token", token)]).await;
}

/// Fordert einen Code für die Anmeldung per Code an. Fehler als verständliche Meldung.
pub async fn request_device_code(client_id: &str, scopes: &[String]) -> Result<auth::DeviceCode, String> {
    let response = post_form(DEVICE_URL, &[("client_id", client_id), ("scopes", &scopes.join(" "))])
        .await
        .map_err(|_| tr("Twitch ist gerade nicht erreichbar. Bitte später erneut versuchen.", "Twitch is not reachable right now. Please try again later.").to_string())?;
    let body = json_value(&response).await;
    auth::interpret_device_code(response.ok(), body.as_ref())
}

/// Fragt einmal nach dem Token zu einem Code.
pub async fn poll_device_token(client_id: &str, device_code: &str, scopes: &[String]) -> auth::DevicePollResult {
    let scopes = scopes.join(" ");
    let pairs = [
        ("client_id", client_id),
        ("scopes", scopes.as_str()),
        ("device_code", device_code),
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
    ];
    match post_form(TOKEN_URL, &pairs).await {
        Ok(response) => {
            let body = json_value(&response).await;
            auth::interpret_device_poll(response.status(), response.ok(), body.as_ref())
        }
        Err(_) => auth::DevicePollResult::Unreachable,
    }
}

/// Verlängert die Anmeldung mit dem Refresh-Token (Public-App, ohne Client-Secret).
pub async fn refresh_access_token(client_id: &str, refresh_token: &str) -> auth::RefreshResult {
    let pairs = [("client_id", client_id), ("grant_type", "refresh_token"), ("refresh_token", refresh_token)];
    match post_form(TOKEN_URL, &pairs).await {
        Ok(response) => {
            let body = if response.ok() { json_value(&response).await } else { None };
            auth::interpret_refresh(response.status(), response.ok(), body.as_ref())
        }
        Err(_) => auth::RefreshResult::Unreachable,
    }
}
