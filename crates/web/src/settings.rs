//! Gemeinsamer Zwischenspeicher der Einstellungen (bisher core/persistence/use-setting.ts). Jede Einstellung wird
//! einmal gelesen und bei Änderungen (auch aus anderen Browser-Tabs) neu geladen; Oberfläche in Rust und die
//! verbliebenen Teile in TypeScript lesen denselben Stand. Bis zum Laden gilt der Standardwert. Schemas und
//! Standardwerte stehen in crates/core/src/settings.rs.

use multiyap_core::i18n::tr;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use multiyap_core::persistence::SETTINGS;
use multiyap_core::settings;
use serde_json::Value;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::db::main_database;

/// Meldung an alle, wenn das Speichern fehlschlägt.
pub fn save_error() -> &'static str {
    tr("Die Einstellung konnte nicht gespeichert werden.", "The setting could not be saved.")
}

type Listener = Rc<dyn Fn()>;

struct Entry {
    value: Value,
    requested: bool,
    listeners: Vec<(u32, Listener)>,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<String, Entry>,
    watching: bool,
    next_listener: u32,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = warn)]
    fn console_warn(message: &str);
}

/// Gespeicherter Wert, geprüft. Fehlt er, der Standardwert; ist er ungültig, ebenfalls der Standardwert, der
/// gespeicherte Wert bleibt aber unangetastet, bis die Einstellung bewusst neu geschrieben wird.
fn checked(key: &str, stored: Option<Value>, default: &Value) -> Value {
    match stored {
        None => default.clone(),
        Some(value) => settings::parse(key, &value).unwrap_or_else(|_| {
            console_warn(&format!("Gespeicherte Einstellung „{key}“ ist ungültig; es gilt der Standardwert."));
            default.clone()
        }),
    }
}

fn notify(key: &str) {
    let listeners: Vec<Listener> =
        CACHE.with(|c| c.borrow().entries.get(key).map(|e| e.listeners.iter().map(|(_, l)| l.clone()).collect()).unwrap_or_default());
    for listener in listeners {
        listener();
    }
}

/// Liest eine Einstellung neu aus dem Speicher und meldet sie, wenn sie sich geändert hat.
fn reload(key: String) {
    let Some(db) = main_database() else { return };
    let Some(setting) = settings::find(&key) else { return };
    spawn_local(async move {
        let value = match db.read_json(SETTINGS, &key).await {
            Ok(stored) => checked(&key, stored, &setting.default),
            Err(_) => {
                console_warn(&format!("Einstellung „{key}“ konnte nicht gelesen werden."));
                return;
            }
        };
        let changed = CACHE.with(|c| match c.borrow_mut().entries.get_mut(&key) {
            Some(entry) if entry.value != value => {
                entry.value = value;
                true
            }
            _ => false,
        });
        if changed {
            notify(&key);
        }
    });
}

/// Beobachtet den Speicher der Einstellungen einmal: jede abgeschlossene Änderung lädt die gefragten neu.
fn watch() {
    let Some(db) = main_database() else { return };
    if CACHE.with(|c| std::mem::replace(&mut c.borrow_mut().watching, true)) {
        return;
    }
    db.watch(|stores, _remote| {
        if !stores.iter().any(|s| s == SETTINGS) {
            return;
        }
        let keys: Vec<String> = CACHE.with(|c| c.borrow().entries.iter().filter(|(_, e)| e.requested).map(|(k, _)| k.clone()).collect());
        keys.into_iter().for_each(reload);
    });
}

/// Legt den Eintrag an (mit Standardwert) und lädt ihn, sobald die Datenbank bereit ist. None bei unbekannter Einstellung.
fn ensure(key: &str) -> Option<()> {
    let setting = settings::find(key)?;
    let ready = main_database().is_some();
    let load = CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        let entry = cache.entries.entry(key.to_string()).or_insert_with(|| Entry { value: setting.default.clone(), requested: false, listeners: Vec::new() });
        let load = ready && !entry.requested;
        if load {
            entry.requested = true;
        }
        load
    });
    if load {
        watch();
        reload(key.to_string());
    }
    Some(())
}

/// Die Datenbank ist geöffnet: alle bisher gefragten Einstellungen laden.
pub(crate) fn database_ready() {
    let keys: Vec<String> = CACHE.with(|c| c.borrow().entries.keys().cloned().collect());
    for key in keys {
        ensure(&key);
    }
}

/// Aktueller Wert (bis zum Laden der Standardwert); None bei unbekannter Einstellung.
pub fn get(key: &str) -> Option<Value> {
    ensure(key)?;
    CACHE.with(|c| c.borrow().entries.get(key).map(|e| e.value.clone()))
}

/// Meldet Änderungen einer Einstellung; gibt die Nummer zum Abmelden zurück (None bei unbekannter Einstellung).
pub fn subscribe(key: &str, listener: Listener) -> Option<u32> {
    ensure(key)?;
    CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        let id = cache.next_listener;
        cache.next_listener += 1;
        cache.entries.get_mut(key).map(|entry| {
            entry.listeners.push((id, listener));
            id
        })
    })
}

pub fn unsubscribe(key: &str, id: u32) {
    CACHE.with(|c| {
        if let Some(entry) = c.borrow_mut().entries.get_mut(key) {
            entry.listeners.retain(|(i, _)| *i != id);
        }
    });
}

/// Schreibt eine Einstellung sofort (kein „Speichern“). Ungültige Werte werden abgelehnt; gespeichert wird der
/// geprüfte Wert. Der Zwischenspeicher übernimmt ihn über die Meldung der Datenbank.
pub async fn save(key: &str, value: &Value) -> Result<(), String> {
    let parsed = settings::parse(key, value).map_err(|_| save_error().to_string())?;
    let db = main_database().ok_or_else(|| save_error().to_string())?;
    db.write_json(SETTINGS, key, &parsed).await.map_err(|_| save_error().to_string())
}

/// Ändert eine Einstellung auf Grundlage des gespeicherten Stands, Lesen und Schreiben in einer Transaktion. So gehen
/// gleichzeitige Änderungen (zum Beispiel aus einem zweiten Browser-Tab) nicht verloren. Ist der gespeicherte Wert
/// ungültig, bleibt er unangetastet und die Änderung schlägt fehl.
pub async fn update(key: &str, change: impl FnOnce(Value) -> Value + 'static) -> Result<(), String> {
    let setting = settings::find(key).ok_or_else(|| save_error().to_string())?;
    let db = main_database().ok_or_else(|| save_error().to_string())?;
    let owned_key = key.to_string();
    db.update_json(SETTINGS, key, move |stored| {
        let current = match stored {
            None => setting.default.clone(),
            Some(value) => settings::parse(&owned_key, &value).map_err(|_| save_error().to_string())?,
        };
        settings::parse(&owned_key, &change(current)).map_err(|_| save_error().to_string())
    })
    .await
    .map_err(|_| save_error().to_string())
}

// ---- Für TypeScript (bis die übrigen Dienste in Rust sind) ----

