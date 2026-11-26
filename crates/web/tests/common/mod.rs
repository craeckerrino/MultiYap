//! Gemeinsame Hilfen der Tests für WebAssembly (laufen mit wasm-bindgen-test-runner unter Node). Die IndexedDB kommt
//! aus der Nachbildung `fake-indexeddb` (npm, nur für Tests).

#![allow(dead_code)]

use std::cell::Cell;
use std::rc::Rc;

use multiyap_web::db::{self, DbConnection, Migrate};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(inline_js = "
export function installIndexedDb() {
  if (globalThis.indexedDB) return;
  const module = globalThis.process.getBuiltinModule('module');
  module.createRequire(globalThis.process.cwd() + '/')('fake-indexeddb/auto');
}
export function sleep(ms) { return new Promise((resolve) => setTimeout(resolve, ms)); }
")]
extern "C" {
    #[wasm_bindgen(js_name = installIndexedDb)]
    fn install_indexed_db();
    #[wasm_bindgen(js_name = sleep)]
    fn sleep_promise(ms: u32) -> js_sys::Promise;
}

/// Wartet `ms` Millisekunden (zum Beispiel auf Meldungen zwischen Verbindungen).
pub async fn sleep(ms: u32) {
    let _ = JsFuture::from(sleep_promise(ms)).await;
}

thread_local! {
    static COUNTER: Cell<u32> = const { Cell::new(0) };
}

/// Eindeutiger Name für eine Test-Datenbank.
pub fn unique_name(prefix: &str) -> String {
    install_indexed_db();
    let n = COUNTER.with(|c| {
        c.set(c.get() + 1);
        c.get()
    });
    format!("{prefix}-{}-{n}", js_sys::Date::now())
}

/// Anzahl der eingebauten Migrationen (aktuelle Schema-Version).
pub fn builtin_count() -> u32 {
    multiyap_core::persistence::MIGRATIONS.len() as u32
}

/// Eingebaute Migrationen bis `target`; darüber hinaus `extra(version, db, transaction)`.
pub fn migrations(extra: impl Fn(u32, &web_sys::IdbDatabase, &web_sys::IdbTransaction) -> Result<JsValue, JsValue> + 'static) -> Migrate {
    let builtin = builtin_count();
    Rc::new(move |version, idb, transaction| if version <= builtin { Ok(JsValue::from_str("builtin")) } else { extra(version, idb, transaction) })
}

/// Öffnet `name` bis Version `target`, nur mit eingebauten Migrationen.
pub async fn open(name: &str, target: u32) -> Result<DbConnection, JsValue> {
    db::open_named(name, target, migrations(|_, _, _| Ok(JsValue::UNDEFINED)), Rc::new(|| {})).await
}
