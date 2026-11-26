//! Anmeldung bei Twitch (bisher core/auth/auth-store.ts). Konto und Token liegen im Speicher „auth“ (nie Teil der
//! Sicherung). Token werden nie geloggt und verlassen Rust nur für Anfragen an Twitch: Oberfläche und TypeScript
//! bekommen nur den Stand ohne Token (`AuthView`).
//!
//! Abläufe wie bisher: Prüfen beim Start und danach stündlich, Hinweis „Bitte neu anmelden“ bei ungültigem Token,
//! Anmeldung per Code mit Verlängern vor Ablauf (über alle Browser-Tabs nacheinander, weil ein Refresh-Token nur
//! einmal gilt), Anmeldung mit Rücksprung als Ausweichweg, eingefügtes Token, Abmelden mit Widerruf.

use multiyap_core::i18n::tr;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use multiyap_core::persistence::AUTH;
use multiyap_core::twitch::auth::{self as twitch, DevicePollResult, RefreshResult, ValidateResult, ValidatedToken};
use multiyap_core::twitch::config;
use js_sys::{Function, Promise, Reflect};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{future_to_promise, spawn_local, JsFuture};

use crate::db::{main_database, WriteOp};
use crate::net::{self, global_function, now_ms, sleep};

/// Twitch verlangt eine Prüfung beim Start und danach stündlich.
const VALIDATE_INTERVAL_MS: f64 = 60.0 * 60.0 * 1000.0;
/// Längste Wartezeit, die setTimeout zuverlässig kann.
const MAX_TIMEOUT_MS: f64 = 2_000_000_000.0;
/// Anmeldung per Code: so lange vor Ablauf des Tokens wird verlängert.
const REFRESH_MARGIN_MS: f64 = 10.0 * 60.0 * 1000.0;
/// Twitch beim Verlängern nicht erreichbar: erneuter Versuch nach dieser Zeit.
const REFRESH_RETRY_MS: f64 = 60.0 * 1000.0;
/// Schlüssel im Speicher „auth“.
const ACTIVE_KEY: &str = "active";
/// Zufallswert der Anmeldung mit Rücksprung (sessionStorage).
const STATE_KEY: &str = "multiyap:oauth-state";
const REFRESH_LOCK: &str = "multiyap:token-refresh";

fn account_key(user_id: &str) -> String {
    format!("account:{user_id}")
}

/// Ein angemeldetes Konto, wie im Speicher (geprüft mit dem Schema „account“ aus crates/core/src/stored.rs).
/// Absichtlich ohne Debug, damit Token nie in Ausgaben geraten.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub platform: String,
    pub user_id: String,
    pub login: String,
    /// Client-ID, zu der das Token gehört (bei eingefügten Token evtl. eine fremde App).
    pub client_id: String,
    pub access_token: String,
    pub scopes: Vec<String>,
    /// Zeitpunkt der Anmeldung (Millisekunden seit 1970).
    pub obtained_at: f64,
    /// Ablaufzeitpunkt laut Twitch, None ohne Ablauf.
    pub expires_at: Option<f64>,
    /// "login" (Rücksprung), "manual" (eingefügt) oder "device" (Anmeldung per Code).
    pub source: String,
    /// Nur bei Anmeldung per Code: zum Verlängern. Wie das Token geheim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

#[derive(Clone)]
enum AuthState {
    Loading,
    SignedOut { error: Option<String> },
    SignedIn { account: Account, missing_scopes: Vec<String> },
    /// Token ungültig oder abgelaufen: Hinweis „Bitte neu anmelden“.
    Expired { account: Account },
}

impl AuthState {
    fn account(&self) -> Option<&Account> {
        match self {
            AuthState::SignedIn { account, .. } | AuthState::Expired { account } => Some(account),
            _ => None,
        }
    }
}

/// Konto ohne Token für Oberfläche und TypeScript.
#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub platform: String,
    pub user_id: String,
    pub login: String,
    pub client_id: String,
    pub scopes: Vec<String>,
    pub obtained_at: f64,
    pub expires_at: Option<f64>,
    pub source: String,
}

/// Stand der Anmeldung ohne Token: status "loading", "signed-out", "signed-in" oder "expired".
#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthView {
    pub status: &'static str,
    pub account: Option<AccountInfo>,
    pub missing_scopes: Vec<String>,
    pub error: Option<String>,
}

/// Anmeldung per Code: Zustand für das Fenster mit dem Code.
#[derive(Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum DeviceLogin {
    Idle,
    Requesting,
    #[serde(rename_all = "camelCase")]
    Waiting {
        user_code: String,
        verification_uri: String,
        expires_at: f64,
    },
    Error {
        message: String,
    },
}

type Listener = Rc<dyn Fn()>;

struct Store {
    state: AuthState,
    listeners: Vec<(u32, Listener)>,
    device: DeviceLogin,
    device_listeners: Vec<(u32, Listener)>,
    next_listener: u32,
    validate_timer: Option<JsValue>,
    expiry_timer: Option<JsValue>,
    refresh_timer: Option<JsValue>,
    watching: bool,
}

thread_local! {
    static STORE: RefCell<Store> = RefCell::new(Store {
        state: AuthState::Loading,
        listeners: Vec::new(),
        device: DeviceLogin::Idle,
        device_listeners: Vec::new(),
        next_listener: 0,
        validate_timer: None,
        expiry_timer: None,
        refresh_timer: None,
        watching: false,
    });
    static DEVICE_GENERATION: Cell<u32> = const { Cell::new(0) };
}

fn with_store<R>(f: impl FnOnce(&mut Store) -> R) -> R {
    STORE.with(|s| f(&mut s.borrow_mut()))
}

fn current() -> AuthState {
    with_store(|s| s.state.clone())
}

fn set_state(next: AuthState) {
    let listeners: Vec<Listener> = with_store(|s| {
        s.state = next;
        s.listeners.iter().map(|(_, l)| l.clone()).collect()
    });
    for listener in listeners {
        listener();
    }
}

fn set_device(next: DeviceLogin) {
    let listeners: Vec<Listener> = with_store(|s| {
        s.device = next;
        s.device_listeners.iter().map(|(_, l)| l.clone()).collect()
    });
    for listener in listeners {
        listener();
    }
}

// ---- Zeitgeber ----

fn set_timer(kind: &str, callback: JsValue, ms: f64) -> Option<JsValue> {
    global_function(kind).ok()?.call2(&JsValue::NULL, &callback, &JsValue::from_f64(ms.clamp(0.0, MAX_TIMEOUT_MS))).ok()
}

fn clear_timer(kind: &str, timer: Option<JsValue>) {
    if let (Some(timer), Ok(clear)) = (timer, global_function(kind)) {
        let _ = clear.call1(&JsValue::NULL, &timer);
    }
}

fn clear_timers() {
    let (validate, expiry, refresh) = with_store(|s| (s.validate_timer.take(), s.expiry_timer.take(), s.refresh_timer.take()));
    clear_timer("clearInterval", validate);
    clear_timer("clearTimeout", expiry);
    clear_timer("clearTimeout", refresh);
}

fn schedule_checks(account: &Account) {
    clear_timers();
    let check = Closure::<dyn Fn()>::new(|| spawn_local(check_active_token())).into_js_value();
    let validate = set_timer("setInterval", check, VALIDATE_INTERVAL_MS);
    with_store(|s| s.validate_timer = validate);
    let Some(expires_at) = account.expires_at else { return };
    if account.refresh_token.is_some() {
        // Anmeldung per Code: rechtzeitig vor Ablauf verlängern.
        schedule_refresh(expires_at - now_ms() - REFRESH_MARGIN_MS);
        return;
    }
    let on_expiry = Closure::once_into_js(|| spawn_local(check_active_token()));
    let expiry = set_timer("setTimeout", on_expiry, expires_at - now_ms() + 1000.0);
    with_store(|s| s.expiry_timer = expiry);
}

fn schedule_refresh(delay: f64) {
    let previous = with_store(|s| s.refresh_timer.take());
    clear_timer("clearTimeout", previous);
    let on_refresh = Closure::once_into_js(|| {
        with_store(|s| s.refresh_timer = None);
        if let AuthState::SignedIn { account, .. } = current() {
            spawn_local(async move {
                renew_account(account).await;
            });
        }
    });
    let timer = set_timer("setTimeout", on_refresh, delay);
    with_store(|s| s.refresh_timer = timer);
}

// ---- Speicher ----

fn parse_account(value: &Value) -> Option<Account> {
    let parsed = multiyap_core::stored::named("account")?.parse(value).ok()?;
    serde_json::from_value(parsed).ok()
}

/// Das aktive Konto; Err bei beschädigten Daten (die Daten bleiben dann unangetastet).
async fn load_active_account() -> Result<Option<Account>, ()> {
    let db = main_database().ok_or(())?;
    let entries = db.entries_json(AUTH).await.map_err(|_| ())?;
    let Some(active) = entries.iter().find(|(key, _)| key == ACTIVE_KEY).and_then(|(_, v)| v.as_str().map(str::to_string)) else {
        return Ok(None);
    };
    match entries.iter().find(|(key, _)| *key == account_key(&active)) {
        None => Ok(None),
        Some((_, value)) => parse_account(value).map(Some).ok_or(()),
    }
}

/// Speichert ein Konto und macht es aktiv, in einer Transaktion. Solange nur ein Konto unterstützt wird, entfällt
/// dabei ein bisher aktives anderes Konto samt Token.
async fn save_active_account(account: &Account) -> Result<(), String> {
    let error: &'static str = tr("Die Anmeldung konnte nicht gespeichert werden.", "The sign-in could not be saved.");
    let value = serde_json::to_value(account).map_err(|_| error.to_string())?;
    let value = multiyap_core::stored::named("account").and_then(|s| s.parse(&value).ok()).ok_or_else(|| error.to_string())?;
    let db = main_database().ok_or_else(|| error.to_string())?;
    let user_id = account.user_id.clone();
    db.transact(AUTH, ACTIVE_KEY, move |previous| {
        let mut ops = Vec::new();
        if let Some(previous) = previous.as_ref().and_then(Value::as_str).filter(|id| *id != user_id) {
            ops.push(WriteOp::Delete(account_key(previous)));
        }
        ops.push(WriteOp::Put(account_key(&user_id), value));
        ops.push(WriteOp::Put(ACTIVE_KEY.into(), Value::String(user_id)));
        Ok(ops)
    })
    .await
    .map_err(|_| error.to_string())
}

async fn remove_account(user_id: &str) -> Result<(), JsValue> {
    let db = main_database().ok_or_else(|| JsValue::from_str("Die Datenbank ist nicht bereit."))?;
    let user_id = user_id.to_string();
    db.transact(AUTH, ACTIVE_KEY, move |active| {
        let mut ops = vec![WriteOp::Delete(account_key(&user_id))];
        if active.as_ref().and_then(Value::as_str) == Some(user_id.as_str()) {
            ops.push(WriteOp::Delete(ACTIVE_KEY.into()));
        }
        Ok(ops)
    })
    .await
}

// ---- Abläufe ----

fn account_from_token(token: &ValidatedToken, access_token: String, source: &str, refresh_token: Option<String>) -> Account {
    let now = now_ms();
    Account {
        platform: "twitch".into(),
        user_id: token.user_id.clone(),
        login: token.login.clone(),
        client_id: token.client_id.clone(),
        access_token,
        scopes: token.scopes.clone(),
        obtained_at: now,
        expires_at: (token.expires_in > 0.0).then_some(now + token.expires_in * 1000.0),
        source: source.into(),
        refresh_token,
    }
}

fn show_account(account: Account) {
    schedule_checks(&account);
    let missing_scopes = config::missing_scopes(&account.scopes);
    set_state(AuthState::SignedIn { account, missing_scopes });
}

fn mark_expired(account: Account) {
    clear_timers();
    set_state(AuthState::Expired { account });
}

enum RefreshOutcome {
    Ok(Box<Account>),
    Invalid,
    Unreachable,
}

/// Führt `task` über alle Browser-Tabs nacheinander aus (Web Locks); ohne Web Locks direkt.
async fn with_refresh_lock(task: impl std::future::Future<Output = RefreshOutcome> + 'static) -> RefreshOutcome {
    let request = Reflect::get(&js_sys::global(), &"navigator".into())
        .ok()
        .and_then(|navigator| Reflect::get(&navigator, &"locks".into()).ok())
        .filter(|locks| !locks.is_undefined() && !locks.is_null())
        .and_then(|locks| Reflect::get(&locks, &"request".into()).ok().and_then(|f| f.dyn_into::<Function>().ok()).map(|f| (locks, f)));
    let Some((locks, request)) = request else { return task.await };
    let result: Rc<RefCell<Option<RefreshOutcome>>> = Rc::default();
    let slot = result.clone();
    let mut task = Some(task);
    let callback = Closure::once_into_js(move |_lock: JsValue| -> Promise {
        let task = task.take();
        future_to_promise(async move {
            if let Some(task) = task {
                *slot.borrow_mut() = Some(task.await);
            }
            Ok(JsValue::UNDEFINED)
        })
    });
    let outcome = request.call2(&locks, &JsValue::from_str(REFRESH_LOCK), &callback).ok().and_then(|p| p.dyn_into::<Promise>().ok());
    if let Some(promise) = outcome {
        let _ = JsFuture::from(promise).await;
    }
    let outcome = result.borrow_mut().take();
    outcome.unwrap_or(RefreshOutcome::Unreachable)
}

async fn refresh_account(account: Account) -> RefreshOutcome {
    with_refresh_lock(async move {
        let stored = load_active_account().await.ok().flatten();
        // Inzwischen abgemeldet oder anderes Konto: nichts verlängern.
        let Some(stored) = stored.filter(|s| s.user_id == account.user_id) else { return RefreshOutcome::Invalid };
        // Ein anderer Tab hat schon verlängert.
        if stored.access_token != account.access_token && stored.expires_at.is_some_and(|at| at - now_ms() > REFRESH_MARGIN_MS) {
            return RefreshOutcome::Ok(Box::new(stored));
        }
        let Some(refresh_token) = stored.refresh_token.clone() else { return RefreshOutcome::Invalid };
        let token = match net::refresh_access_token(&stored.client_id, &refresh_token).await {
            RefreshResult::Token { token } => token,
            RefreshResult::Invalid => return RefreshOutcome::Invalid,
            RefreshResult::Unreachable => return RefreshOutcome::Unreachable,
        };
        // Das alte Refresh-Token ist jetzt verbraucht: das neue sofort speichern, auch wenn die Prüfung danach scheitert.
        let validated = net::validate_token(&token.access_token).await;
        if let ValidateResult::Valid { token: v } = &validated {
            if v.user_id != stored.user_id {
                return RefreshOutcome::Invalid;
            }
        }
        let now = now_ms();
        let next = Account {
            scopes: match validated {
                ValidateResult::Valid { token: v } => v.scopes,
                _ => stored.scopes.clone(),
            },
            access_token: token.access_token,
            refresh_token: Some(token.refresh_token),
            obtained_at: now,
            expires_at: (token.expires_in > 0.0).then_some(now + token.expires_in * 1000.0),
            ..stored
        };
        match save_active_account(&next).await {
            Ok(()) => RefreshOutcome::Ok(Box::new(next)),
            Err(_) => RefreshOutcome::Unreachable,
        }
    })
    .await
}

/// Verlängert die Anmeldung und passt den Zustand an. true: es gibt wieder ein gültiges Token.
async fn renew_account(account: Account) -> bool {
    let user_id = account.user_id.clone();
    let outcome = refresh_account(account.clone()).await;
    // Zwischenzeitlich abgemeldet oder Konto gewechselt: Ergebnis verwerfen.
    let state = current();
    if matches!(state, AuthState::SignedOut { .. }) || state.account().is_some_and(|a| a.user_id != user_id) {
        return false;
    }
    match outcome {
        RefreshOutcome::Ok(next) => {
            show_account(*next);
            true
        }
        RefreshOutcome::Invalid => {
            mark_expired(account);
            false
        }
        RefreshOutcome::Unreachable => {
            schedule_refresh(REFRESH_RETRY_MS);
            false
        }
    }
}

/// Für Twitch-Anfragen mit Antwort 401: bei Anmeldung per Code einmal verlängern, true heißt erneut versuchen.
/// Sonst wie bisher prüfen (meldet „Bitte neu anmelden“, falls das Token ungültig ist).
pub async fn recover_token() -> bool {
    let AuthState::SignedIn { account, .. } = current() else { return false };
    if account.refresh_token.is_none() {
        spawn_local(check_active_token());
        return false;
    }
    renew_account(account).await
}

/// Prüft das aktive Token bei Twitch und passt den Zustand an.
pub async fn check_active_token() {
    let AuthState::SignedIn { account, .. } = current() else { return };
    let result = net::validate_token(&account.access_token).await;
    // Zwischenzeitlich abgemeldet oder Konto gewechselt: Ergebnis verwerfen.
    match current() {
        AuthState::SignedIn { account: now, .. } if now.user_id == account.user_id => {}
        _ => return,
    }
    // Bei "unreachable" bleibt alles, wie es ist; die nächste Prüfung folgt.
    if !matches!(result, ValidateResult::Invalid) {
        return;
    }
    if account.refresh_token.is_some() {
        renew_account(account).await;
    } else {
        mark_expired(account);
    }
}

async fn adopt_token(access_token: String, source: &str, refresh_token: Option<String>) -> Option<String> {
    let token = match net::validate_token(&access_token).await {
        ValidateResult::Unreachable => return Some(tr("Twitch ist gerade nicht erreichbar. Bitte später erneut versuchen.", "Twitch is not reachable right now. Please try again later.").into()),
        ValidateResult::Invalid => return Some(tr("Das Token ist ungültig oder abgelaufen.", "The token is invalid or expired.").into()),
        ValidateResult::Valid { token } => token,
    };
    let account = account_from_token(&token, access_token, source, refresh_token);
    if let Err(message) = save_active_account(&account).await {
        return Some(message);
    }
    show_account(account);
    None
}

async fn reload_from_database(error: Option<String>) {
    // Beschädigte Anmeldedaten: als abgemeldet behandeln, Daten bleiben unangetastet.
    let Some(account) = load_active_account().await.ok().flatten() else {
        clear_timers();
        set_state(AuthState::SignedOut { error });
        return;
    };
    if matches!(net::validate_token(&account.access_token).await, ValidateResult::Invalid) {
        // Anmeldung per Code: abgelaufenes Token (zum Beispiel nach einer Pause) still verlängern.
        if account.refresh_token.is_some() && renew_account(account.clone()).await {
            return;
        }
        mark_expired(account);
        return;
    }
    show_account(account);
}

/// Startet die Anmeldung, sobald die Datenbank bereit ist. `redirect`: ein schon aus der Adresse entfernter
/// Rücksprung von Twitch (Adresse und erwarteter Zufallswert).
pub async fn init(redirect: Option<(String, Option<String>)>) {
    if let Some(db) = main_database() {
        if !with_store(|s| std::mem::replace(&mut s.watching, true)) {
            db.watch(|stores, remote| {
                // Anmeldung oder Abmeldung in einem anderen Browser-Tab übernehmen.
                if remote && stores.iter().any(|s| s == AUTH) {
                    spawn_local(reload_from_database(None));
                }
            });
        }
    }
    let redirect = redirect.and_then(|(href, expected)| twitch::interpret_redirect(&href, expected.as_deref()));
    match redirect {
        Some(twitch::RedirectResult::Token { access_token, .. }) => {
            if let Some(error) = adopt_token(access_token, "login", None).await {
                reload_from_database(Some(error)).await;
            }
        }
        Some(twitch::RedirectResult::Error { message }) => reload_from_database(Some(message)).await,
        None => reload_from_database(None).await,
    }
}

/// Anmeldung per Code möglich (eigene Public-App eingetragen)?
pub fn device_login_available() -> bool {
    !config::DEVICE_CLIENT_ID.is_empty()
}

/// Knopf „Mit Twitch anmelden“: per Code, falls eingerichtet, sonst mit Rücksprung.
pub fn sign_in() {
    if device_login_available() {
        spawn_local(start_device_login());
    } else {
        sign_in_with_redirect();
    }
}

/// Ausweichweg: Anmeldung mit Rücksprung (Implicit Grant). Der Zufallswert schützt vor untergeschobenen Rücksprüngen.
pub fn sign_in_with_redirect() {
    let Some(window) = web_sys::window() else { return };
    let state = window.crypto().ok().map(|c| c.random_uuid().replace('-', "")).unwrap_or_default();
    // Ohne sessionStorage kann der Rücksprung nicht geprüft werden und wird später abgelehnt.
    if let Ok(Some(storage)) = window.session_storage() {
        let _ = storage.set_item(STATE_KEY, &state);
    }
    // Bei erneuter Anmeldung nach Ablauf das Konto bestätigen lassen, damit ein Kontowechsel möglich ist.
    let force_verify = matches!(current(), AuthState::Expired { .. });
    let url = twitch::build_authorize_url(config::CLIENT_ID, config::REDIRECT_URI, &config::scopes(), &state, force_verify);
    let _ = window.location().assign(&url);
}

/// Fordert einen Code an und wartet, bis er auf twitch.tv/activate bestätigt ist.
pub async fn start_device_login() {
    let generation = DEVICE_GENERATION.with(|g| {
        g.set(g.get() + 1);
        g.get()
    });
    let still_current = move || DEVICE_GENERATION.with(Cell::get) == generation;
    set_device(DeviceLogin::Requesting);
    let scopes = config::scopes();
    let code = match net::request_device_code(config::DEVICE_CLIENT_ID, &scopes).await {
        Ok(code) => code,
        Err(message) => {
            if still_current() {
                set_device(DeviceLogin::Error { message });
            }
            return;
        }
    };
    if !still_current() {
        return;
    }
    let expires_at = now_ms() + code.expires_in * 1000.0;
    set_device(DeviceLogin::Waiting { user_code: code.user_code.clone(), verification_uri: code.verification_uri.clone(), expires_at });

    let mut interval = code.interval * 1000.0;
    loop {
        sleep(interval).await;
        if !still_current() {
            return;
        }
        if now_ms() > expires_at {
            set_device(DeviceLogin::Error { message: tr("Der Code ist abgelaufen. Bitte erneut anmelden.", "The code has expired. Please sign in again.").into() });
            return;
        }
        let result = net::poll_device_token(config::DEVICE_CLIENT_ID, &code.device_code, &scopes).await;
        if !still_current() {
            return;
        }
        match result {
            DevicePollResult::Pending | DevicePollResult::Unreachable => continue,
            DevicePollResult::SlowDown => {
                interval += 5000.0;
                continue;
            }
            DevicePollResult::Token { token } => {
                let error = adopt_token(token.access_token, "device", Some(token.refresh_token)).await;
                if still_current() {
                    set_device(match error {
                        Some(message) => DeviceLogin::Error { message },
                        None => DeviceLogin::Idle,
                    });
                }
                return;
            }
            DevicePollResult::Denied => {
                set_device(DeviceLogin::Error { message: tr("Die Anmeldung wurde bei Twitch abgelehnt.", "The sign-in was declined on Twitch.").into() });
                return;
            }
            DevicePollResult::Expired => {
                set_device(DeviceLogin::Error { message: tr("Der Code ist abgelaufen. Bitte erneut anmelden.", "The code has expired. Please sign in again.").into() });
                return;
            }
            DevicePollResult::Error { message } => {
                set_device(DeviceLogin::Error { message });
                return;
            }
        }
    }
}

/// Fenster mit dem Code schließen: Warten beenden.
pub fn cancel_device_login() {
    DEVICE_GENERATION.with(|g| g.set(g.get() + 1));
    set_device(DeviceLogin::Idle);
}

/// Ausweichweg: Token manuell einfügen. Gibt eine Fehlermeldung zurück oder None bei Erfolg.
pub async fn sign_in_with_token(input: &str) -> Option<String> {
    let token = twitch::normalize_pasted_token(input);
    if token.is_empty() {
        return Some(tr("Bitte ein Token einfügen.", "Please paste a token.").into());
    }
    adopt_token(token, "manual", None).await
}

/// Meldet ab: Token bei Twitch widerrufen und lokal löschen.
pub async fn sign_out() {
    let Some(account) = current().account().cloned() else { return };
    clear_timers();
    // Scheitert das Löschen, bleibt der Zustand wie bisher unverändert.
    if remove_account(&account.user_id).await.is_err() {
        return;
    }
    set_state(AuthState::SignedOut { error: None });
    // Laut Doku nimmt Twitch hier Zugriffstoken an; ob es das Refresh-Token mit ungültig macht, steht dort nicht.
    // Deshalb beide widerrufen (Sicherheitscheck, 16.11.2026). Fehler sind egal: lokal ist schon alles gelöscht.
    net::revoke_token(&account.client_id, &account.access_token).await;
    if let Some(refresh_token) = &account.refresh_token {
        net::revoke_token(&account.client_id, refresh_token).await;
    }
}

// ---- Lesen ----

/// Stand der Anmeldung ohne Token.
pub fn view() -> AuthView {
    let (status, missing_scopes, error) = match current() {
        AuthState::Loading => ("loading", Vec::new(), None),
        AuthState::SignedOut { error } => ("signed-out", Vec::new(), error),
        AuthState::SignedIn { missing_scopes, .. } => ("signed-in", missing_scopes, None),
        AuthState::Expired { .. } => ("expired", Vec::new(), None),
    };
    let account = current().account().map(|a| AccountInfo {
        platform: a.platform.clone(),
        user_id: a.user_id.clone(),
        login: a.login.clone(),
        client_id: a.client_id.clone(),
        scopes: a.scopes.clone(),
        obtained_at: a.obtained_at,
        expires_at: a.expires_at,
        source: a.source.clone(),
    });
    AuthView { status, account, missing_scopes, error }
}

/// Zugangsdaten für Helix: (Nutzer-ID, Client-ID, Token), nur wenn angemeldet.
pub(crate) fn helix_credentials() -> Option<(String, String, String)> {
    match current() {
        AuthState::SignedIn { account, .. } => Some((account.user_id, account.client_id, account.access_token)),
        _ => None,
    }
}

/// Token und Login für die nur lesende IRC-Verbindung, nur mit chat:read.
pub fn irc_credentials() -> Option<(String, String)> {
    match current() {
        AuthState::SignedIn { account, .. } if account.scopes.iter().any(|s| s == "chat:read") => Some((account.access_token, account.login)),
        _ => None,
    }
}

/// Nutzer-ID des angemeldeten Kontos.
pub fn own_user_id() -> Option<String> {
    match current() {
        AuthState::SignedIn { account, .. } => Some(account.user_id),
        _ => None,
    }
}

pub fn device_login() -> DeviceLogin {
    with_store(|s| s.device.clone())
}

pub fn subscribe(listener: Listener) -> u32 {
    with_store(|s| {
        let id = s.next_listener;
        s.next_listener += 1;
        s.listeners.push((id, listener));
        id
    })
}

pub fn unsubscribe(id: u32) {
    with_store(|s| s.listeners.retain(|(i, _)| *i != id));
}

pub fn subscribe_device(listener: Listener) -> u32 {
    with_store(|s| {
        let id = s.next_listener;
        s.next_listener += 1;
        s.device_listeners.push((id, listener));
        id
    })
}

pub fn unsubscribe_device(id: u32) {
    with_store(|s| s.device_listeners.retain(|(i, _)| *i != id));
}

// ---- Für TypeScript (bis die übrigen Dienste in Rust sind) ----

