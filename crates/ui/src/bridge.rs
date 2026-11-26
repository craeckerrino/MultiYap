//! Gemeinsame Helfer der Oberfläche für die Dienste in crates/web: Einstellungen, Anmeldung, Meldungen. Der Name stammt
//! aus der Zeit der Brücke zu TypeScript; seitdem läuft alles direkt in Rust.

use leptos::prelude::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use wasm_bindgen::prelude::*;

/// Wert über JSON-Text (null bleibt null).
fn from_js<T: DeserializeOwned>(value: &JsValue) -> Option<T> {
    let text = js_sys::JSON::stringify(value).ok()?.as_string()?;
    serde_json::from_str(&text).ok()
}

/// Wert über JSON-Text an JavaScript (null bleibt null, fehlende Felder fallen weg).
pub fn to_js_value<T: Serialize>(value: &T) -> JsValue {
    to_js(value)
}

fn to_js<T: Serialize>(value: &T) -> JsValue {
    serde_json::to_string(value).ok().and_then(|text| js_sys::JSON::parse(&text).ok()).unwrap_or(JsValue::NULL)
}

/// Zufällige ID (crypto.randomUUID) für neue Einträge.
pub fn random_id() -> String {
    web_sys::window().and_then(|w| w.crypto().ok()).map(|c| c.random_uuid()).unwrap_or_else(|| format!("{}", js_sys::Math::random()))
}

/// Kurze Meldung unten im Fenster (crates/ui/src/toaster.rs). `kind`: "error", "info" oder "success".
pub fn toast(kind: &str, text: &str) {
    multiyap_web::toasts::show(kind, text, None);
}

/// Anmeldestand ohne Token (Direkt aus crates/web/src/auth.rs): status, login, source, expiresAt, missingScopes
/// (Anzahl), error.
pub fn auth_info() -> serde_json::Value {
    let view = multiyap_web::auth::view();
    serde_json::json!({
        "status": view.status,
        "login": view.account.as_ref().map(|a| a.login.clone()),
        "source": view.account.as_ref().map(|a| a.source.clone()),
        "expiresAt": view.account.as_ref().and_then(|a| a.expires_at),
        "missingScopes": view.missing_scopes.len(),
        "error": view.error,
    })
}

/// Anmeldestand als Signal, das sich bei jeder Änderung erneuert.
pub fn use_auth<T>(fallback: T) -> ReadSignal<T>
where
    T: DeserializeOwned + Clone + Send + Sync + 'static,
{
    let read = || serde_json::from_value::<T>(auth_info()).ok();
    let (value, set_value) = signal(read().unwrap_or(fallback));
    let id = multiyap_web::auth::subscribe(std::rc::Rc::new(move || {
        if let Some(next) = read() {
            set_value.try_set(next);
        }
    }));
    on_cleanup(move || multiyap_web::auth::unsubscribe(id));
    value
}

/// Zustand der Anmeldung per Code als Signal.
pub fn use_device_login<T>(fallback: T) -> ReadSignal<T>
where
    T: DeserializeOwned + Clone + Send + Sync + 'static,
{
    let read = || serde_json::to_value(multiyap_web::auth::device_login()).ok().and_then(|v| serde_json::from_value::<T>(v).ok());
    let (value, set_value) = signal(read().unwrap_or(fallback));
    let id = multiyap_web::auth::subscribe_device(std::rc::Rc::new(move || {
        if let Some(next) = read() {
            set_value.try_set(next);
        }
    }));
    on_cleanup(move || multiyap_web::auth::unsubscribe_device(id));
    value
}

/// Nutzer nach Login (Helix, für die Sitzung zwischengespeichert in crates/web/src/api.rs): { userId, login,
/// displayName } oder None, wenn es ihn nicht gibt. Err bei Fehlern der Anfrage.
pub async fn user_by_login(login: &str) -> Result<Option<serde_json::Value>, JsValue> {
    let user = multiyap_web::api::get_user_by_login(login).await?;
    Ok(user.map(|u| {
        serde_json::json!({ "userId": u["id"], "login": u["login"], "displayName": u["display_name"] })
    }))
}

/// Wert aus crates/web in einen eigenen Typ der Oberfläche (gleiche Felder über JSON).
pub fn convert<T: DeserializeOwned>(value: &impl Serialize) -> Option<T> {
    serde_json::to_value(value).ok().and_then(|v| serde_json::from_value(v).ok())
}

/// Abmelden: Token widerrufen und lokal löschen.
pub fn sign_out() {
    leptos::task::spawn_local(multiyap_web::auth::sign_out());
}

/// Ändert eine Einstellung auf Grundlage des gespeicherten Stands, Lesen und Schreiben in einer Transaktion.
/// Schlägt es fehl, eine Meldung; der gespeicherte Stand bleibt dann unverändert.
pub fn update_setting<T>(key: &str, change: impl Fn(T) -> T + 'static)
where
    T: DeserializeOwned + Serialize + Default + 'static,
{
    let key = key.to_string();
    leptos::task::spawn_local(async move {
        let result = multiyap_web::settings::update(&key, move |current| {
            serde_json::to_value(change(serde_json::from_value(current).unwrap_or_default())).unwrap_or(serde_json::Value::Null)
        })
        .await;
        if let Err(message) = result {
            toast("error", &message);
        }
    });
}

/// Wert über JSON aus JavaScript lesen.
pub fn read_value<T: DeserializeOwned>(value: &JsValue) -> Option<T> {
    from_js(value)
}

/// Aktueller Wert einer Einstellung (bis zum Laden der Standardwert).
pub fn get_setting<T: DeserializeOwned>(key: &str) -> Option<T> {
    multiyap_web::settings::get(key).and_then(|value| serde_json::from_value(value).ok())
}

/// Speichert eine Einstellung sofort (geprüft in Rust); bei Fehlern eine Meldung.
pub fn save_setting<T: Serialize>(key: &str, value: &T) {
    let Ok(value) = serde_json::to_value(value) else {
        toast("error", multiyap_web::settings::save_error());
        return;
    };
    let key = key.to_string();
    leptos::task::spawn_local(async move {
        if let Err(message) = multiyap_web::settings::save(&key, &value).await {
            toast("error", &message);
        }
    });
}

/// Einstellung als Signal, das sich bei jeder Änderung (auch aus anderen Browser-Tabs) erneuert.
pub fn use_setting<T>(key: &'static str, fallback: T) -> ReadSignal<T>
where
    T: DeserializeOwned + Clone + Send + Sync + 'static,
{
    let (value, set_value) = signal(get_setting::<T>(key).unwrap_or(fallback));
    let listener = std::rc::Rc::new(move || {
        if let Some(next) = get_setting::<T>(key) {
            set_value.try_set(next);
        }
    });
    if let Some(id) = multiyap_web::settings::subscribe(key, listener) {
        on_cleanup(move || multiyap_web::settings::unsubscribe(key, id));
    }
    value
}
