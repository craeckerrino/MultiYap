//! Speicher (crates/web/src/db.rs) mit IndexedDB: Öffnen und Migrationen, Transaktionen, mehrere Fenster. Vorher in
//! TypeScript (src/core/persistence/*.test.ts) hier.

#![cfg(target_arch = "wasm32")]

mod common;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use common::{builtin_count, migrations, open, sleep, unique_name};
use multiyap_core::persistence::{SETTINGS, WORKSPACE};
use multiyap_web::db;
use serde_json::json;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

#[wasm_bindgen(inline_js = "
export function breakMigration(db, transaction, key) {
  db.createObjectStore('neu');
  transaction.objectStore('settings').put('kaputt', key);
  throw new Error('Absichtlicher Fehler');
}
export function doubleSetting(transaction, key) {
  return new Promise((resolve, reject) => {
    const store = transaction.objectStore('settings');
    const read = store.get(key);
    read.onerror = () => reject(read.error);
    read.onsuccess = () => {
      const write = store.put(read.result * 2, key);
      write.onerror = () => reject(write.error);
      write.onsuccess = () => resolve();
    };
  });
}
")]
extern "C" {
    #[wasm_bindgen(js_name = breakMigration, catch)]
    fn break_migration(db: &web_sys::IdbDatabase, transaction: &web_sys::IdbTransaction, key: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = doubleSetting)]
    fn double_setting(transaction: &web_sys::IdbTransaction, key: &str) -> js_sys::Promise;
}

fn message(error: &JsValue) -> String {
    js_sys::Reflect::get(error, &"message".into()).ok().and_then(|m| m.as_string()).unwrap_or_default()
}

fn name_of(error: &JsValue) -> String {
    js_sys::Reflect::get(error, &"name".into()).ok().and_then(|m| m.as_string()).unwrap_or_default()
}

#[wasm_bindgen_test]
async fn neue_datenbank_hat_aktuelle_schema_version() {
    let conn = open(&unique_name("neu"), builtin_count()).await.unwrap();
    assert_eq!(conn.schema_version(), builtin_count());
    conn.close();
}

#[wasm_bindgen_test]
async fn fehlgeschlagene_migration_wird_vollstaendig_verworfen() {
    let name = unique_name("verworfen");
    let first = open(&name, builtin_count()).await.unwrap();
    first.write_json(SETTINGS, "test.migration.keep", &json!("wichtig")).await.unwrap();
    first.close();

    let broken = migrations(|_, idb, transaction| break_migration(idb, transaction, "test.migration.keep"));
    let error = db::open_named(&name, builtin_count() + 1, broken, Rc::new(|| {})).await.err().expect("Migration muss fehlschlagen");
    assert_eq!(message(&error), "Absichtlicher Fehler");

    let again = open(&name, builtin_count()).await.unwrap();
    assert_eq!(again.schema_version(), builtin_count());
    assert_eq!(again.read_json(SETTINGS, "test.migration.keep").await.unwrap(), Some(json!("wichtig")));
    again.close();
}

#[wasm_bindgen_test]
async fn daten_migration_laeuft_in_der_upgrade_transaktion() {
    let name = unique_name("daten");
    let first = open(&name, builtin_count()).await.unwrap();
    first.write_json(SETTINGS, "test.migration.data", &json!(21)).await.unwrap();
    first.close();

    let doubled = migrations(|_, _, transaction| Ok(double_setting(transaction, "test.migration.data").into()));
    let migrated = db::open_named(&name, builtin_count() + 1, doubled, Rc::new(|| {})).await.unwrap();
    assert_eq!(migrated.schema_version(), builtin_count() + 1);
    assert_eq!(migrated.read_json(SETTINGS, "test.migration.data").await.unwrap(), Some(json!(42)));
    migrated.close();
}

#[wasm_bindgen_test]
async fn daten_einer_neueren_version_werden_abgelehnt() {
    let name = unique_name("neuer");
    let newer = open(&name, builtin_count() + 1).await.unwrap();
    newer.write_json(SETTINGS, "test.newer", &json!("bleibt")).await.unwrap();
    newer.close();

    let error = open(&name, builtin_count()).await.err().expect("neuere Daten müssen abgelehnt werden");
    assert_eq!(name_of(&error), "VersionError");

    let check = open(&name, builtin_count() + 1).await.unwrap();
    assert_eq!(check.read_json(SETTINGS, "test.newer").await.unwrap(), Some(json!("bleibt")));
    check.close();
}

#[wasm_bindgen_test]
async fn transaktion_schreibt_ganz_oder_gar_nicht() {
    let conn = open(&unique_name("tx"), builtin_count()).await.unwrap();
    conn.write_json(SETTINGS, "test.tx.a", &json!("vorher")).await.unwrap();

    let result: Result<(), JsValue> = conn
        .transaction(&[SETTINGS], true, |access| async move {
            access.put(SETTINGS, &JsValue::from_str("test.tx.a"), &json!("nachher")).await?;
            access.put(SETTINGS, &JsValue::from_str("test.tx.b"), &json!(2)).await?;
            Err(js_sys::Error::new("Abbruch").into())
        })
        .await;
    assert_eq!(message(&result.expect_err("muss abbrechen")), "Abbruch");
    assert_eq!(conn.read_json(SETTINGS, "test.tx.a").await.unwrap(), Some(json!("vorher")));
    assert_eq!(conn.read_json(SETTINGS, "test.tx.b").await.unwrap(), None);

    conn.transaction(&[SETTINGS], true, |access| async move {
        access.put(SETTINGS, &JsValue::from_str("test.tx.a"), &json!("a2")).await?;
        access.put(SETTINGS, &JsValue::from_str("test.tx.b"), &json!(2)).await
    })
    .await
    .unwrap();
    assert_eq!(conn.read_json(SETTINGS, "test.tx.a").await.unwrap(), Some(json!("a2")));
    assert_eq!(conn.read_json(SETTINGS, "test.tx.b").await.unwrap(), Some(json!(2)));
    conn.close();
}

#[wasm_bindgen_test]
async fn aenderungen_gehen_an_andere_verbindungen() {
    let name = unique_name("tabs");
    let first = open(&name, builtin_count()).await.unwrap();
    let second = open(&name, builtin_count()).await.unwrap();
    let local: Rc<RefCell<Option<bool>>> = Rc::new(RefCell::new(None));
    let remote: Rc<RefCell<Option<bool>>> = Rc::new(RefCell::new(None));
    let (l, r) = (local.clone(), remote.clone());
    first.watch(move |stores, is_remote| {
        assert_eq!(stores, vec![SETTINGS.to_string()]);
        *l.borrow_mut() = Some(is_remote);
    });
    second.watch(move |_, is_remote| {
        *r.borrow_mut() = Some(is_remote);
    });
    first.write_json(SETTINGS, "test.tabs.sync", &json!("neu")).await.unwrap();
    for _ in 0..50 {
        if remote.borrow().is_some() {
            break;
        }
        sleep(10).await;
    }
    assert_eq!(*local.borrow(), Some(false));
    assert_eq!(*remote.borrow(), Some(true));
    assert_eq!(second.read_json(SETTINGS, "test.tabs.sync").await.unwrap(), Some(json!("neu")));
    first.close();
    second.close();
}

#[wasm_bindgen_test]
async fn neuere_version_in_anderem_fenster_schliesst_die_alte_verbindung() {
    let name = unique_name("veraltet");
    let old = open(&name, builtin_count()).await.unwrap();
    let outdated = Rc::new(Cell::new(false));
    let flag = outdated.clone();
    let listener = Closure::<dyn FnMut()>::new(move || flag.set(true)).into_js_value();
    old.on_outdated(listener.unchecked_into());

    let upgraded = open(&name, builtin_count() + 1).await.unwrap();
    assert!(outdated.get());
    assert!(old.is_closed());
    assert_eq!(upgraded.schema_version(), builtin_count() + 1);
    upgraded.close();
}

#[wasm_bindgen_test]
async fn migration_6_loescht_entfernte_einstellungen() {
    let name = unique_name("entfernt");
    let v5 = open(&name, 5).await.unwrap();
    v5.write_json(SETTINGS, "chat.pauseDelay", &json!(3)).await.unwrap();
    v5.write_json(SETTINGS, "chat.timeFormat", &json!("HH:MM:SS")).await.unwrap();
    v5.close();

    let conn = open(&name, builtin_count()).await.unwrap();
    let keys: Vec<String> = conn.entries_json(SETTINGS).await.unwrap().into_iter().map(|(key, _)| key).collect();
    assert_eq!(keys, vec!["chat.timeFormat".to_string()]);
    conn.close();
}

#[wasm_bindgen_test]
async fn alte_tab_liste_wird_zum_layout_mit_einem_bereich() {
    let name = unique_name("layout");
    let tab = |id: &str| json!({ "id": id, "kind": "channel", "platform": "twitch", "channel": id, "nickname": null });
    let v4 = open(&name, 4).await.unwrap();
    v4.write_json(WORKSPACE, "tabs", &json!({ "tabs": [tab("a"), tab("b")], "activeTabId": "b" })).await.unwrap();
    v4.close();

    let conn = open(&name, builtin_count()).await.unwrap();
    let layout = conn.read_json(WORKSPACE, multiyap_core::persistence::LAYOUT_KEY).await.unwrap().expect("Layout fehlt");
    assert_eq!(layout["root"]["type"], json!("area"));
    assert_eq!(layout["root"]["tabIds"], json!(["a", "b"]));
    assert_eq!(layout["root"]["activeTabId"], json!("b"));
    conn.close();
}

#[wasm_bindgen_test]
async fn einstellungen_ungueltige_werte_werden_nicht_geschrieben() {
    let conn = open(&unique_name("einstellungen"), builtin_count()).await.unwrap();
    db::set_main_database(&conn);
    multiyap_web::settings::save("layout.locked", &json!(true)).await.unwrap();
    assert!(multiyap_web::settings::save("layout.locked", &json!("kein Wahrheitswert")).await.is_err());
    assert_eq!(conn.read_json(SETTINGS, "layout.locked").await.unwrap(), Some(json!(true)));
    conn.close();
}
