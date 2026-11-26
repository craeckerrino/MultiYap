//! Sicherung (bisher src/core/backup/backup.ts):
//! - Export als Datei mit Schema-Version und Prüfsumme (SHA-256). Zugangsdaten sind nie enthalten.
//! - Import: Datei vollständig prüfen (Datei und jeder Wert), bevor etwas geändert wird; dann den aktuellen Stand
//!   automatisch sichern und alles in EINER Transaktion übernehmen. Bei Fehler bleibt der alte Stand.
//! - Automatische Sicherungen (die letzten 5) lassen sich wie eine Datei geprüft wiederherstellen.
//!
//! Prüfungen, Prüfsumme und Dateiname mit Tests in crates/core/src/backup.rs.

use multiyap_core::i18n::tr;
use std::cell::RefCell;

use multiyap_core::backup::{self as bk, Entry, BACKUP_STORES};
use multiyap_core::persistence as p;
use multiyap_core::stored;
use serde::Serialize;
use serde_json::{json, Map, Value};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::chat_tools::toast;
use crate::db::{main_database, Access, DbConnection};

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
fn not_ready() -> &'static str {
    tr("Der Speicher ist nicht bereit.", "Storage is not ready.")
}

thread_local! {
    /// Geprüfte Sicherung, die auf „Einspielen“ wartet.
    static PENDING: RefCell<Option<Vec<Entry>>> = const { RefCell::new(None) };
}

/// Zusammenfassung einer geprüften Sicherung für die Anzeige.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub created_at: String,
    pub app_version: String,
    pub count: usize,
}

/// Eine automatische Sicherung in der Liste.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoBackupInfo {
    pub key: f64,
    pub created_at: String,
}

fn database() -> Result<DbConnection, String> {
    main_database().ok_or_else(|| not_ready().to_string())
}

fn iso_now() -> String {
    js_sys::Date::new_0().to_iso_string().as_string().unwrap_or_default()
}

/// Bereiche der Sicherung mit allen Einträgen, deren Schlüssel Text ist.
async fn read_current(access: &Access) -> Result<Value, JsValue> {
    let mut data = Map::new();
    for store in BACKUP_STORES {
        let values: Map<String, Value> = access.entries(store).await?.into_iter().filter_map(|(key, value)| Some((key.as_string()?, value))).collect();
        data.insert(store.to_string(), Value::Object(values));
    }
    Ok(Value::Object(data))
}

fn auto_backup(value: Option<Value>) -> Option<Value> {
    stored::named("autoBackup")?.parse(&value?).ok()
}

fn text(value: &Value, name: &str) -> String {
    value.get(name).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// Gibt den Text als Datei zum Speichern (Download im Programmfenster).
fn download(text: &str, name: &str) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str(not_ready()))?;
    let document = window.document().ok_or_else(|| JsValue::from_str(not_ready()))?;
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("application/json");
    let parts = js_sys::Array::of1(&JsValue::from_str(text));
    let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &options)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)?;
    let link: web_sys::HtmlAnchorElement = document.create_element("a")?.unchecked_into();
    link.set_href(&url);
    link.set_download(name);
    link.click();
    web_sys::Url::revoke_object_url(&url)
}

fn file_name_now() -> String {
    let date = js_sys::Date::new_0();
    bk::file_name(date.get_full_year(), date.get_month() + 1, date.get_date(), date.get_hours(), date.get_minutes())
}

/// Erstellt die Sicherungsdatei des aktuellen Stands und gibt sie zum Speichern.
pub async fn export() {
    let result: Result<(), JsValue> = async {
        let db = database().map_err(|m| JsValue::from_str(&m))?;
        let data = db.transaction(&BACKUP_STORES, false, |access| async move { read_current(&access).await }).await?;
        let file = bk::backup_file(APP_VERSION, db.schema_version(), &iso_now(), data);
        let text = serde_json::to_string_pretty(&file).map_err(|e| JsValue::from_str(&e.to_string()))?;
        download(&text, &file_name_now())
    }
    .await;
    if result.is_err() {
        toast("error", tr("Die Sicherung konnte nicht erstellt werden.", "The backup could not be created."));
    }
}

/// Prüft eine Sicherungsdatei vollständig (ändert nichts). Bei Erfolg wartet sie auf `apply_pending`.
pub fn check(text: &str) -> Result<Summary, Vec<String>> {
    PENDING.with(|p| *p.borrow_mut() = None);
    let db = database().map_err(|m| vec![m])?;
    let prepared = bk::prepare_file(text, db.schema_version())?;
    let values = bk::validate_entries(prepared.entries)?;
    let summary = Summary { created_at: prepared.created_at, app_version: prepared.app_version, count: values.len() };
    PENDING.with(|p| *p.borrow_mut() = Some(values));
    Ok(summary)
}

/// Verwirft die geprüfte Sicherung.
pub fn cancel() {
    PENDING.with(|p| *p.borrow_mut() = None);
}

/// Sichert den aktuellen Stand automatisch und ersetzt ihn durch die geprüften Werte, alles in einer Transaktion:
/// entweder vollständig oder gar nicht.
async fn apply(values: Vec<Entry>) -> Result<(), JsValue> {
    let db = database().map_err(|m| JsValue::from_str(&m))?;
    let schema_version = db.schema_version();
    let mut stores: Vec<&str> = BACKUP_STORES.to_vec();
    stores.push(p::BACKUPS);
    db.transaction(&stores, true, move |access| async move {
        // 1. Aktuellen Stand sichern; nur die neuesten automatischen Sicherungen behalten.
        let current = read_current(&access).await?;
        let now = js_sys::Date::now();
        let created_at = js_sys::Date::new(&JsValue::from_f64(now)).to_iso_string().as_string().unwrap_or_default();
        let auto = auto_backup(Some(json!({ "createdAt": created_at, "appVersion": APP_VERSION, "schemaVersion": schema_version, "data": current })))
            .ok_or_else(|| JsValue::from_str("Der aktuelle Stand konnte nicht gesichert werden."))?;
        access.put(p::BACKUPS, &JsValue::from_f64(now), &auto).await?;
        let keys: Vec<f64> = access.keys(p::BACKUPS).await?.iter().filter_map(JsValue::as_f64).collect();
        for key in bk::surplus_auto_backups(keys) {
            access.delete(p::BACKUPS, &JsValue::from_f64(key)).await?;
        }
        // 2. Bisherigen Stand ersetzen.
        for store in BACKUP_STORES {
            access.clear(store).await?;
        }
        for entry in &values {
            access.put(&entry.store, &JsValue::from_str(&entry.key), &entry.value).await?;
        }
        Ok(())
    })
    .await
}

/// Spielt ein und lädt danach neu, damit alle Bereiche den neuen Stand übernehmen.
async fn apply_and_reload(values: Vec<Entry>) -> bool {
    if apply(values).await.is_err() {
        toast("error", tr("Einspielen fehlgeschlagen. Der bisherige Stand ist unverändert.", "Restore failed. The previous state is unchanged."));
        return false;
    }
    toast("success", tr("Sicherung eingespielt. MultiYap wird neu geladen.", "Backup restored. MultiYap is reloading."));
    if let Some(window) = web_sys::window() {
        let reload = Closure::once_into_js(|| {
            if let Some(window) = web_sys::window() {
                let _ = window.location().reload();
            }
        });
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(reload.unchecked_ref(), 800);
    }
    true
}

/// Spielt die geprüfte Sicherung ein. false, wenn keine wartet oder das Einspielen fehlschlägt.
pub async fn apply_pending() -> bool {
    let Some(values) = PENDING.with(|p| p.borrow_mut().take()) else { return false };
    apply_and_reload(values).await
}

/// Liste der automatischen Sicherungen, neueste zuerst.
pub async fn auto_list() -> Vec<AutoBackupInfo> {
    let Ok(db) = database() else { return Vec::new() };
    let list = db
        .transaction(&[p::BACKUPS], false, |access| async move {
            let mut list = Vec::new();
            for key in access.keys(p::BACKUPS).await? {
                let Some(number) = key.as_f64() else { continue };
                if let Some(backup) = auto_backup(access.get(p::BACKUPS, &key).await.ok().flatten()) {
                    list.push(AutoBackupInfo { key: number, created_at: text(&backup, "createdAt") });
                }
            }
            Ok(list)
        })
        .await;
    let mut list = list.unwrap_or_default();
    list.sort_by(|a, b| b.key.total_cmp(&a.key));
    list
}

/// Prüft eine automatische Sicherung wie eine Datei und spielt sie ein.
pub async fn restore(key: f64) -> Result<bool, Vec<String>> {
    let db = database().map_err(|m| vec![m])?;
    let stored = db.transaction(&[p::BACKUPS], false, move |access| async move { access.get(p::BACKUPS, &JsValue::from_f64(key)).await }).await;
    let backup = auto_backup(stored.ok().flatten()).ok_or_else(|| vec![tr("Die automatische Sicherung wurde nicht gefunden.", "The automatic backup was not found.").to_string()])?;
    let schema_version = backup.get("schemaVersion").and_then(Value::as_f64).unwrap_or(0.0) as u32;
    let entries = bk::prepare_data(schema_version, &backup["data"], db.schema_version())?;
    let values = bk::validate_entries(entries)?;
    Ok(apply_and_reload(values).await)
}
