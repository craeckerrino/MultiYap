//! IndexedDB-Schicht: Öffnen mit Migrationen, Transaktionen, Abgleich zwischen Browser-Tabs. Die Migrationen stehen in
//! `crates/core/src/persistence.rs`. In crates/web, damit auch Dienste in Rust lesen und schreiben können (siehe
//! „Zugriff aus Rust“). Die Datenbank der App öffnet der Programmstart in Rust (`open_app`, crates/web/src/startup.rs);
//! `openIdb` bleibt für die Tests in TypeScript.
//!
//! Wichtig: Innerhalb einer Transaktion nur auf deren eigene Anfragen warten. Die Zusagen (Promises) entstehen direkt
//! aus den Ereignissen der Anfragen, die Migrationen warten über Mikroaufgaben; so bleibt die Transaktion aktiv.

use multiyap_core::i18n::tr;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use multiyap_core::persistence::{self as p, Step};
use js_sys::{Array, Function, Object, Promise, Reflect, JSON};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{IdbDatabase, IdbFactory, IdbObjectStore, IdbOpenDbRequest, IdbRequest, IdbTransaction};

const CHANNEL_NAME: &str = "multiyap-db";

/// Name der Datenbank der App.
pub const DB_NAME: &str = "multiyap";

/// Eine Migration: eingebaut ("builtin") oder eigene (nur in Tests, Ergebnis evtl. eine Zusage).
pub type Migrate = Rc<dyn Fn(u32, &IdbDatabase, &IdbTransaction) -> Result<JsValue, JsValue>>;

fn get(value: &JsValue, key: &str) -> JsValue {
    Reflect::get(value, &JsValue::from_str(key)).unwrap_or(JsValue::UNDEFINED)
}

/// promise.then(on_ok, on_err) mit JavaScript-Funktionen.
fn then(promise: &Promise, on_ok: &JsValue, on_err: Option<&JsValue>) -> Promise {
    let Ok(function) = get(promise, "then").dyn_into::<Function>() else { return promise.clone() };
    let result = match on_err {
        Some(on_err) => function.call2(promise, on_ok, on_err),
        None => function.call1(promise, on_ok),
    };
    result.map(JsCast::unchecked_into).unwrap_or_else(|error| Promise::reject(&error))
}

/// Zusage aus einer IndexedDB-Anfrage (wie requestToPromise in TypeScript).
fn request_promise(request: &IdbRequest) -> Promise {
    let request = request.clone();
    Promise::new(&mut |resolve, reject| {
        let ok_request = request.clone();
        let on_success = Closure::<dyn FnMut()>::new(move || {
            let _ = resolve.call1(&JsValue::NULL, &ok_request.result().unwrap_or(JsValue::UNDEFINED));
        })
        .into_js_value();
        let err_request = request.clone();
        let on_error = Closure::<dyn FnMut()>::new(move || {
            let error = err_request.error().ok().flatten().map(JsValue::from).unwrap_or(JsValue::NULL);
            let _ = reject.call1(&JsValue::NULL, &error);
        })
        .into_js_value();
        request.set_onsuccess(Some(on_success.unchecked_ref()));
        request.set_onerror(Some(on_error.unchecked_ref()));
    })
}

async fn wait(request: Result<IdbRequest, JsValue>) -> Result<JsValue, JsValue> {
    JsFuture::from(request_promise(&request?)).await
}

/// Wert als JSON-Wert; undefined und nicht als JSON darstellbares als None. Felder mit undefined fallen weg (wie
/// beim Speichern als JSON), damit optionale Felder nicht zu null werden.
fn to_value(value: &JsValue) -> Option<serde_json::Value> {
    if value.is_undefined() {
        return None;
    }
    let text = JSON::stringify(value).ok()?.as_string()?;
    serde_json::from_str(&text).ok()
}

fn from_value(value: &serde_json::Value) -> Result<JsValue, JsValue> {
    JSON::parse(&value.to_string())
}

fn object_store(transaction: &IdbTransaction, name: &str) -> Result<IdbObjectStore, JsValue> {
    transaction.object_store(name)
}

/// Ein Migrationsschritt in der versionchange-Transaktion.
async fn run_step(db: &IdbDatabase, transaction: &IdbTransaction, step: &Step) -> Result<(), JsValue> {
    match *step {
        Step::CreateStore(name) => {
            db.create_object_store(name)?;
        }
        Step::DeleteRemovedSettings => {
            let store = object_store(transaction, p::SETTINGS)?;
            for key in p::REMOVED_SETTING_KEYS {
                wait(store.delete(&JsValue::from_str(key))).await?;
            }
        }
        Step::Update { store, key, change } => {
            let store = object_store(transaction, store)?;
            let current = wait(store.get(&JsValue::from_str(key))).await?;
            if let Some(next) = to_value(&current).as_ref().and_then(change) {
                wait(store.put_with_key(&from_value(&next)?, &JsValue::from_str(key))).await?;
            }
        }
        Step::CopyIfMissing { store, from, to, convert } => {
            let store = object_store(transaction, store)?;
            let old = wait(store.get(&JsValue::from_str(from))).await?;
            if let Some(next) = to_value(&old).as_ref().and_then(convert) {
                let existing = wait(store.get(&JsValue::from_str(to))).await?;
                if existing.is_undefined() {
                    wait(store.put_with_key(&from_value(&next)?, &JsValue::from_str(to))).await?;
                }
            }
        }
        Step::Move { store, from, to, convert } => {
            let store = object_store(transaction, store)?;
            let old = wait(store.get(&JsValue::from_str(from))).await?;
            if let Some(next) = to_value(&old).as_ref().and_then(convert) {
                wait(store.put_with_key(&from_value(&next)?, &JsValue::from_str(to))).await?;
                wait(store.delete(&JsValue::from_str(from))).await?;
            }
        }
    }
    Ok(())
}

/// Öffnet die Datenbank der App mit allen eingebauten Migrationen. `on_blocked`: ein anderer Tab mit älterer Version
/// hält sie offen. Fehler wie bei `openIdb` (VersionError bei neueren Daten).
pub async fn open_app(on_blocked: impl Fn() + 'static) -> Result<DbConnection, JsValue> {
    let migrate: Migrate = Rc::new(|_, _, _| Ok(JsValue::from_str("builtin")));
    open_named(DB_NAME, p::MIGRATIONS.len() as u32, migrate, Rc::new(on_blocked)).await
}

/// Öffnet eine Datenbank bis zur Version `target` (für Tests mit eigenem Namen und eigenen Migrationen).
pub async fn open_named(name: &str, target: u32, migrate: Migrate, on_blocked: Rc<dyn Fn()>) -> Result<DbConnection, JsValue> {
    let promise = open_raw(name, target, migrate, on_blocked).map_err(JsValue::from)?;
    let idb = JsFuture::from(promise).await?;
    Ok(DbConnection::new(idb.unchecked_into()))
}

/// Öffnen bis zur Version `target`; die Zusage liefert die rohe IndexedDB-Verbindung.
fn open_raw(name: &str, target: u32, migrate: Migrate, on_blocked: Rc<dyn Fn()>) -> Result<Promise, JsError> {
    let factory: IdbFactory = get(&js_sys::global(), "indexedDB").unchecked_into();
    let request: IdbOpenDbRequest = factory.open_with_u32(name, target).map_err(|_| JsError::new(tr("IndexedDB nicht verfügbar", "IndexedDB not available")))?;
    let migration_error: Rc<RefCell<Option<JsValue>>> = Rc::new(RefCell::new(None));

    let upgrade_request = request.clone();
    let upgrade_error = migration_error.clone();
    let on_upgrade = Closure::<dyn FnMut(JsValue)>::new(move |event: JsValue| {
        let old = get(&event, "oldVersion").as_f64().unwrap_or(0.0) as u32;
        let db: IdbDatabase = upgrade_request.result().unwrap_or(JsValue::UNDEFINED).unchecked_into();
        let Some(transaction) = upgrade_request.transaction() else { return };
        let migrate = migrate.clone();
        let error_slot = upgrade_error.clone();
        spawn_local(async move {
            let result: Result<(), JsValue> = async {
                for version in (old + 1)..=target {
                    let outcome = migrate(version, &db, &transaction)?;
                    if outcome.as_string().as_deref() == Some("builtin") {
                        let migration = p::MIGRATIONS
                            .get(version as usize - 1)
                            .ok_or_else(|| JsValue::from(js_sys::Error::new(&format!("Migration {version} fehlt"))))?;
                        for step in migration.steps {
                            run_step(&db, &transaction, step).await?;
                        }
                    } else if let Ok(promise) = outcome.dyn_into::<Promise>() {
                        JsFuture::from(promise).await?;
                    }
                }
                Ok(())
            }
            .await;
            if let Err(error) = result {
                *error_slot.borrow_mut() = Some(error);
                let _ = transaction.abort();
            }
        });
    })
    .into_js_value();
    request.set_onupgradeneeded(Some(on_upgrade.unchecked_ref()));
    let on_blocked_handler = Closure::<dyn FnMut()>::new(move || on_blocked()).into_js_value();
    request.set_onblocked(Some(on_blocked_handler.unchecked_ref()));

    let promise = Promise::new(&mut |resolve, reject| {
        let ok_request = request.clone();
        let on_success = Closure::<dyn FnMut()>::new(move || {
            let _ = resolve.call1(&JsValue::NULL, &ok_request.result().unwrap_or(JsValue::UNDEFINED));
        })
        .into_js_value();
        let err_request = request.clone();
        let error_slot = migration_error.clone();
        let on_error = Closure::<dyn FnMut()>::new(move || {
            let request_error: Option<JsValue> = err_request.error().ok().flatten().map(Into::into);
            let is_version_error = request_error.as_ref().is_some_and(|e| get(e, "name").as_string().as_deref() == Some("VersionError"));
            let error = if is_version_error { request_error } else { error_slot.borrow_mut().take().or(request_error) };
            let error = error.unwrap_or_else(|| js_sys::Error::new(tr("Datenbank konnte nicht geöffnet werden.", "Database could not be opened.")).into());
            let _ = reject.call1(&JsValue::NULL, &error);
        })
        .into_js_value();
        request.set_onsuccess(Some(on_success.unchecked_ref()));
        request.set_onerror(Some(on_error.unchecked_ref()));
    });
    Ok(promise)
}

struct Listeners {
    next: Cell<u32>,
    list: RefCell<Vec<(u32, Function)>>,
}

impl Listeners {
    fn new() -> Rc<Listeners> {
        Rc::new(Listeners { next: Cell::new(0), list: RefCell::new(Vec::new()) })
    }

    fn add(self: &Rc<Self>, listener: Function) -> JsValue {
        let id = self.next.get() + 1;
        self.next.set(id);
        self.list.borrow_mut().push((id, listener));
        let weak = Rc::downgrade(self);
        Closure::<dyn FnMut()>::new(move || {
            if let Some(listeners) = weak.upgrade() {
                listeners.list.borrow_mut().retain(|(i, _)| *i != id);
            }
        })
        .into_js_value()
    }

    fn emit(&self, value: &JsValue) {
        let list: Vec<Function> = self.list.borrow().iter().map(|(_, f)| f.clone()).collect();
        for listener in list {
            let _ = listener.call1(&JsValue::NULL, value);
        }
    }
}

struct Connection {
    idb: IdbDatabase,
    channel: Option<JsValue>,
    closed: Cell<bool>,
    changes: Rc<Listeners>,
    outdated: Rc<Listeners>,
}

impl Connection {
    fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.idb.close();
        if let Some(channel) = &self.channel {
            if let Ok(close) = get(channel, "close").dyn_into::<Function>() {
                let _ = close.call0(channel);
            }
        }
    }
}

fn change_event(stores: &JsValue, remote: bool) -> JsValue {
    let event = Object::new();
    let _ = Reflect::set(&event, &"stores".into(), stores);
    let _ = Reflect::set(&event, &"remote".into(), &JsValue::from_bool(remote));
    event.into()
}

/// Geöffnete Verbindung. Einzige Stelle, über die dauerhafte Daten gelesen und geschrieben werden.
pub struct DbConnection {
    inner: Rc<Connection>,
}

impl DbConnection {
    fn new(idb: IdbDatabase) -> DbConnection {
        let changes = Listeners::new();
        let outdated = Listeners::new();
        // Abgleich zwischen Browser-Tabs: jede abgeschlossene Schreib-Transaktion wird gemeldet.
        let channel = get(&js_sys::global(), "BroadcastChannel")
            .dyn_into::<Function>()
            .ok()
            .and_then(|ctor| Reflect::construct(&ctor, &Array::of1(&JsValue::from_str(&format!("{CHANNEL_NAME}:{}", idb.name())))).ok());
        if let Some(channel) = &channel {
            let remote_changes = changes.clone();
            let on_message = Closure::<dyn FnMut(JsValue)>::new(move |message: JsValue| {
                let stores = get(&get(&message, "data"), "stores");
                remote_changes.emit(&change_event(&stores, true));
            })
            .into_js_value();
            let _ = Reflect::set(channel, &"onmessage".into(), &on_message);
        }
        let inner = Rc::new(Connection { idb: idb.clone(), channel, closed: Cell::new(false), changes, outdated });
        // Ein anderer Tab will das Schema anheben: Verbindung schließen, damit er nicht blockiert wird.
        let weak = Rc::downgrade(&inner);
        let on_version_change = Closure::<dyn FnMut()>::new(move || {
            if let Some(inner) = weak.upgrade() {
                inner.close();
                inner.outdated.emit(&JsValue::UNDEFINED);
            }
        })
        .into_js_value();
        idb.set_onversionchange(Some(on_version_change.unchecked_ref()));
        DbConnection { inner }
    }
}

/// Roher Zugriff innerhalb einer Transaktion (ohne Prüfung; die Prüfung legt TypeScript darum).
fn raw_access(transaction: &IdbTransaction) -> Object {
    let access = Object::new();
    let method = |name: &str, function: JsValue| {
        let _ = Reflect::set(&access, &JsValue::from_str(name), &function);
    };
    let store = |transaction: &IdbTransaction, name: &JsValue| object_store(transaction, &name.as_string().unwrap_or_default());
    let promise_of = |result: Result<IdbRequest, JsValue>| -> Promise {
        match result {
            Ok(request) => request_promise(&request),
            Err(error) => Promise::reject(&error),
        }
    };
    let t = transaction.clone();
    method(
        "get",
        Closure::<dyn FnMut(JsValue, JsValue) -> Promise>::new(move |name: JsValue, key: JsValue| promise_of(store(&t, &name).and_then(|s| s.get(&key))))
            .into_js_value(),
    );
    let t = transaction.clone();
    method(
        "put",
        Closure::<dyn FnMut(JsValue, JsValue, JsValue) -> Promise>::new(move |name: JsValue, key: JsValue, value: JsValue| {
            promise_of(store(&t, &name).and_then(|s| s.put_with_key(&value, &key)))
        })
        .into_js_value(),
    );
    let t = transaction.clone();
    method(
        "delete",
        Closure::<dyn FnMut(JsValue, JsValue) -> Promise>::new(move |name: JsValue, key: JsValue| promise_of(store(&t, &name).and_then(|s| s.delete(&key))))
            .into_js_value(),
    );
    let t = transaction.clone();
    method(
        "clear",
        Closure::<dyn FnMut(JsValue) -> Promise>::new(move |name: JsValue| promise_of(store(&t, &name).and_then(|s| s.clear()))).into_js_value(),
    );
    let t = transaction.clone();
    method(
        "keys",
        Closure::<dyn FnMut(JsValue) -> Promise>::new(move |name: JsValue| promise_of(store(&t, &name).and_then(|s| s.get_all_keys())))
            .into_js_value(),
    );
    let t = transaction.clone();
    method(
        "entries",
        Closure::<dyn FnMut(JsValue) -> Promise>::new(move |name: JsValue| {
            let pair = store(&t, &name).and_then(|s| Ok((request_promise(&s.get_all_keys()?), request_promise(&s.get_all()?))));
            match pair {
                Ok((keys, values)) => then(
                    &Promise::all(&Array::of2(&keys, &values)),
                    &Closure::once_into_js(move |both: JsValue| -> JsValue {
                        let both: Array = both.unchecked_into();
                        let keys: Array = both.get(0).unchecked_into();
                        let values: Array = both.get(1).unchecked_into();
                        keys.iter().zip(values.iter()).map(|(k, v)| JsValue::from(Array::of2(&k, &v))).collect::<Array>().into()
                    }),
                    None,
                ),
                Err(error) => Promise::reject(&error),
            }
        })
        .into_js_value(),
    );
    access
}

impl DbConnection {
    pub fn schema_version(&self) -> u32 {
        self.inner.idb.version() as u32
    }

    /// Die IndexedDB-Verbindung (nur für Tests, die rohe Werte schreiben).
    pub fn idb(&self) -> IdbDatabase {
        self.inner.idb.clone()
    }

    pub fn is_closed(&self) -> bool {
        self.inner.closed.get()
    }

    /// Führt `work(access)` in genau einer Transaktion aus. Schlägt `work` fehl, wird alles verworfen. Schreibende
    /// Transaktionen mit durability "strict"; danach Meldung an diesen und andere Browser-Tabs.
    pub fn run(&self, stores: Array, write: bool, work: Function) -> Promise {
        if self.inner.closed.get() {
            return Promise::reject(&js_sys::Error::new(tr("Die Datenbankverbindung ist geschlossen.", "The database connection is closed.")).into());
        }
        let inner = self.inner.clone();
        Promise::new(&mut |resolve, reject| {
            let options = Object::new();
            let _ = Reflect::set(&options, &"durability".into(), &"strict".into());
            let mode = JsValue::from_str(if write { "readwrite" } else { "readonly" });
            let transaction = get(&inner.idb, "transaction")
                .dyn_into::<Function>()
                .and_then(|f| f.call3(&inner.idb, &stores, &mode, &options))
                .map(JsCast::unchecked_into::<IdbTransaction>);
            let transaction = match transaction {
                Ok(t) => t,
                Err(error) => {
                    let _ = reject.call1(&JsValue::NULL, &error);
                    return;
                }
            };
            let work_error: Rc<RefCell<Option<JsValue>>> = Rc::new(RefCell::new(None));
            let result: Rc<RefCell<JsValue>> = Rc::new(RefCell::new(JsValue::UNDEFINED));

            let (complete_error, complete_result, complete_inner, complete_stores) = (work_error.clone(), result.clone(), inner.clone(), stores.clone());
            let (resolve_ok, reject_ok) = (resolve.clone(), reject.clone());
            let on_complete = Closure::<dyn FnMut()>::new(move || {
                if let Some(error) = complete_error.borrow_mut().take() {
                    let _ = reject_ok.call1(&JsValue::NULL, &error);
                    return;
                }
                if write {
                    complete_inner.changes.emit(&change_event(&complete_stores, false));
                    if let Some(channel) = &complete_inner.channel {
                        let message = Object::new();
                        let _ = Reflect::set(&message, &"stores".into(), &complete_stores);
                        if let Ok(post) = get(channel, "postMessage").dyn_into::<Function>() {
                            let _ = post.call1(channel, &message);
                        }
                    }
                }
                let _ = resolve_ok.call1(&JsValue::NULL, &complete_result.borrow());
            })
            .into_js_value();
            let (abort_error, abort_transaction, reject_abort) = (work_error.clone(), transaction.clone(), reject.clone());
            let on_abort = Closure::<dyn FnMut()>::new(move || {
                let error = abort_error
                    .borrow_mut()
                    .take()
                    .or_else(|| abort_transaction.error().map(Into::into))
                    .unwrap_or_else(|| js_sys::Error::new(tr("Transaktion abgebrochen.", "Transaction aborted.")).into());
                let _ = reject_abort.call1(&JsValue::NULL, &error);
            })
            .into_js_value();
            transaction.set_oncomplete(Some(on_complete.unchecked_ref()));
            transaction.set_onabort(Some(on_abort.unchecked_ref()));

            let fail = {
                let (work_error, transaction) = (work_error.clone(), transaction.clone());
                move |error: JsValue| {
                    *work_error.borrow_mut() = Some(error);
                    // War die Transaktion schon beendet, meldet onabort oder oncomplete den Ausgang.
                    let _ = transaction.abort();
                }
            };
            match work.call1(&JsValue::NULL, &raw_access(&transaction)) {
                Ok(value) => {
                    let promise = Promise::resolve(&value);
                    let on_value = Closure::once_into_js(move |value: JsValue| {
                        *result.borrow_mut() = value;
                    });
                    let mut fail = Some(fail);
                    let on_error = Closure::<dyn FnMut(JsValue)>::new(move |error: JsValue| {
                        if let Some(fail) = fail.take() {
                            fail(error);
                        }
                    })
                    .into_js_value();
                    let _ = then(&promise, &on_value, Some(&on_error));
                }
                Err(error) => fail(error),
            }
        })
    }

    /// Meldet abgeschlossene Änderungen aus diesem und anderen Tabs: { stores, remote }. Gibt die Abmeldung zurück.
    pub fn on_change(&self, listener: Function) -> JsValue {
        self.inner.changes.add(listener)
    }

    /// Meldet, dass ein anderer Tab eine neuere Version geöffnet hat. Danach ist diese Verbindung geschlossen.
    pub fn on_outdated(&self, listener: Function) -> JsValue {
        self.inner.outdated.add(listener)
    }

    pub fn close(&self) {
        self.inner.close();
    }
}

// ---- Zugriff aus Rust ----

/// Eine Änderung innerhalb von `DbConnection::transact`.
pub enum WriteOp {
    Put(String, serde_json::Value),
    Delete(String),
}

impl Clone for DbConnection {
    fn clone(&self) -> Self {
        DbConnection { inner: self.inner.clone() }
    }
}

thread_local! {
    /// Die Datenbank der App (nicht die von Tests), gesetzt nach dem Öffnen.
    static MAIN: RefCell<Option<DbConnection>> = const { RefCell::new(None) };
}

/// Meldet die geöffnete Datenbank der App an, damit die Dienste in Rust sie nutzen.
pub fn set_main_database(connection: &DbConnection) {
    MAIN.with(|main| *main.borrow_mut() = Some(connection.clone()));
    // Einstellungen, die schon vorher gefragt waren, jetzt aus dem Speicher laden.
    crate::settings::database_ready();
}

/// Die Datenbank der App, sobald sie geöffnet ist.
pub fn main_database() -> Option<DbConnection> {
    MAIN.with(|main| main.borrow().clone())
}

/// Ruft `access[method](store, key, value?)` in der Transaktion auf und gibt die Zusage zurück.
fn access_call(access: &JsValue, method: &str, store: &str, key: &str, value: Option<&JsValue>) -> JsValue {
    let Ok(function) = get(access, method).dyn_into::<Function>() else { return JsValue::UNDEFINED };
    let (store, key) = (JsValue::from_str(store), JsValue::from_str(key));
    match value {
        Some(value) => function.call3(access, &store, &key, value),
        None => function.call2(access, &store, &key),
    }
    .unwrap_or_else(|error| Promise::reject(&error).into())
}

impl DbConnection {
    /// Ein Wert als JSON; None, wenn es ihn nicht gibt.
    pub async fn read_json(&self, store: &str, key: &str) -> Result<Option<serde_json::Value>, JsValue> {
        let (store_name, key) = (store.to_string(), key.to_string());
        let work = Closure::once_into_js(move |access: JsValue| -> JsValue { access_call(&access, "get", &store_name, &key, None) });
        let value = JsFuture::from(self.run(Array::of1(&JsValue::from_str(store)), false, work.unchecked_into())).await?;
        Ok(to_value(&value))
    }

    /// Schreibt einen Wert in einer eigenen Transaktion (danach Meldung an diesen und andere Browser-Tabs).
    pub async fn write_json(&self, store: &str, key: &str, value: &serde_json::Value) -> Result<(), JsValue> {
        let (store_name, key, value) = (store.to_string(), key.to_string(), from_value(value)?);
        let work = Closure::once_into_js(move |access: JsValue| -> JsValue { access_call(&access, "put", &store_name, &key, Some(&value)) });
        JsFuture::from(self.run(Array::of1(&JsValue::from_str(store)), true, work.unchecked_into())).await?;
        Ok(())
    }

    /// Liest einen Wert und schreibt den daraus berechneten in einer Transaktion (gleichzeitige Änderungen, auch aus
    /// anderen Browser-Tabs, gehen so nicht verloren). Gibt `change` einen Fehler zurück, bleibt alles unverändert.
    pub async fn update_json(
        &self,
        store: &str,
        key: &str,
        change: impl FnOnce(Option<serde_json::Value>) -> Result<serde_json::Value, String> + 'static,
    ) -> Result<(), JsValue> {
        let (store_name, key) = (store.to_string(), key.to_string());
        let work = Closure::once_into_js(move |access: JsValue| -> JsValue {
            let current: Promise = access_call(&access, "get", &store_name, &key, None).unchecked_into();
            let on_value = Closure::once_into_js(move |value: JsValue| -> JsValue {
                match change(to_value(&value)).map_err(|message| JsValue::from(js_sys::Error::new(&message))).and_then(|next| from_value(&next)) {
                    Ok(next) => access_call(&access, "put", &store_name, &key, Some(&next)),
                    Err(error) => Promise::reject(&error).into(),
                }
            });
            then(&current, &on_value, None).into()
        });
        JsFuture::from(self.run(Array::of1(&JsValue::from_str(store)), true, work.unchecked_into())).await?;
        Ok(())
    }

    /// Alle Einträge eines Speichers als (Schlüssel, Wert), in einer Transaktion gelesen. Schlüssel, die kein Text
    /// sind, und Werte, die kein JSON sind, fallen weg.
    pub async fn entries_json(&self, store: &str) -> Result<Vec<(String, serde_json::Value)>, JsValue> {
        let store_name = store.to_string();
        let work = Closure::once_into_js(move |access: JsValue| -> JsValue {
            match get(&access, "entries").dyn_into::<Function>() {
                Ok(function) => function.call1(&access, &JsValue::from_str(&store_name)).unwrap_or_else(|error| Promise::reject(&error).into()),
                Err(_) => JsValue::UNDEFINED,
            }
        });
        let pairs = JsFuture::from(self.run(Array::of1(&JsValue::from_str(store)), false, work.unchecked_into())).await?;
        let pairs: Array = pairs.dyn_into().unwrap_or_default();
        Ok(pairs
            .iter()
            .filter_map(|pair| {
                let pair: Array = pair.dyn_into().ok()?;
                Some((pair.get(0).as_string()?, to_value(&pair.get(1))?))
            })
            .collect())
    }

    /// Liest einen Wert und führt danach die daraus geplanten Änderungen aus, alles in einer Transaktion. Gibt `plan`
    /// einen Fehler zurück, bleibt alles unverändert.
    pub async fn transact(
        &self,
        store: &str,
        read_key: &str,
        plan: impl FnOnce(Option<serde_json::Value>) -> Result<Vec<WriteOp>, String> + 'static,
    ) -> Result<(), JsValue> {
        let (store_name, read_key) = (store.to_string(), read_key.to_string());
        let work = Closure::once_into_js(move |access: JsValue| -> JsValue {
            let current: Promise = access_call(&access, "get", &store_name, &read_key, None).unchecked_into();
            let on_value = Closure::once_into_js(move |value: JsValue| -> JsValue {
                let ops = match plan(to_value(&value)) {
                    Ok(ops) => ops,
                    Err(message) => return Promise::reject(&js_sys::Error::new(&message).into()).into(),
                };
                let mut pending = Array::new();
                for op in ops {
                    let promise = match op {
                        WriteOp::Put(key, value) => match from_value(&value) {
                            Ok(value) => access_call(&access, "put", &store_name, &key, Some(&value)),
                            Err(error) => return Promise::reject(&error).into(),
                        },
                        WriteOp::Delete(key) => access_call(&access, "delete", &store_name, &key, None),
                    };
                    pending = pending.concat(&Array::of1(&promise));
                }
                Promise::all(&pending).into()
            });
            then(&current, &on_value, None).into()
        });
        JsFuture::from(self.run(Array::of1(&JsValue::from_str(store)), true, work.unchecked_into())).await?;
        Ok(())
    }

    /// Liest alle Schlüssel eines Speichers und führt danach die daraus geplanten Änderungen aus, in einer Transaktion
    /// (zum Beispiel eine Höchstzahl prüfen, auch gegen einen zweiten Browser-Tab).
    pub async fn transact_keys(&self, store: &str, plan: impl FnOnce(Vec<String>) -> Result<Vec<WriteOp>, String> + 'static) -> Result<(), JsValue> {
        let store_name = store.to_string();
        let work = Closure::once_into_js(move |access: JsValue| -> JsValue {
            let keys = match get(&access, "keys").dyn_into::<Function>() {
                Ok(function) => function.call1(&access, &JsValue::from_str(&store_name)).unwrap_or_else(|error| Promise::reject(&error).into()),
                Err(_) => return Promise::reject(&js_sys::Error::new(tr("Kein Zugriff", "No access")).into()).into(),
            };
            let keys: Promise = keys.unchecked_into();
            let on_keys = Closure::once_into_js(move |list: JsValue| -> JsValue {
                let names: Vec<String> = list.dyn_into::<Array>().map(|a| a.iter().filter_map(|k| k.as_string()).collect()).unwrap_or_default();
                let ops = match plan(names) {
                    Ok(ops) => ops,
                    Err(message) => return Promise::reject(&js_sys::Error::new(&message).into()).into(),
                };
                let mut pending = Array::new();
                for op in ops {
                    let promise = match op {
                        WriteOp::Put(key, value) => match from_value(&value) {
                            Ok(value) => access_call(&access, "put", &store_name, &key, Some(&value)),
                            Err(error) => return Promise::reject(&error).into(),
                        },
                        WriteOp::Delete(key) => access_call(&access, "delete", &store_name, &key, None),
                    };
                    pending = pending.concat(&Array::of1(&promise));
                }
                Promise::all(&pending).into()
            });
            then(&keys, &on_keys, None).into()
        });
        JsFuture::from(self.run(Array::of1(&JsValue::from_str(store)), true, work.unchecked_into())).await?;
        Ok(())
    }

    /// Mehrere Speicher in genau einer Transaktion, mit Zugriff aus Rust (zum Beispiel für die Sicherung: alten Stand
    /// sichern, ersetzen). Schlägt `work` fehl, wird alles verworfen und der Fehler zurückgegeben.
    pub async fn transaction<T, F>(&self, stores: &[&str], write: bool, work: impl FnOnce(Access) -> F + 'static) -> Result<T, JsValue>
    where
        T: 'static,
        F: std::future::Future<Output = Result<T, JsValue>> + 'static,
    {
        let slot: Rc<RefCell<Option<T>>> = Rc::new(RefCell::new(None));
        let filled = slot.clone();
        let run = Closure::once_into_js(move |access: JsValue| -> JsValue {
            wasm_bindgen_futures::future_to_promise(async move {
                let value = work(Access(access)).await?;
                *filled.borrow_mut() = Some(value);
                Ok(JsValue::UNDEFINED)
            })
            .into()
        });
        let names: Array = stores.iter().map(|s| JsValue::from_str(s)).collect();
        JsFuture::from(self.run(names, write, run.unchecked_into())).await?;
        let value = slot.borrow_mut().take();
        value.ok_or_else(|| js_sys::Error::new(tr("Transaktion ohne Ergebnis.", "Transaction without result.")).into())
    }

    /// Meldet abgeschlossene Schreibvorgänge: betroffene Speicher und ob sie aus einem anderen Browser-Tab kamen.
    /// Gibt die Abmeldung zurück.
    pub fn watch(&self, listener: impl Fn(Vec<String>, bool) + 'static) -> JsValue {
        let function = Closure::<dyn Fn(JsValue)>::new(move |event: JsValue| {
            let stores = get(&event, "stores").dyn_into::<Array>().map(|a| a.iter().filter_map(|s| s.as_string()).collect()).unwrap_or_default();
            listener(stores, get(&event, "remote").as_bool().unwrap_or(false));
        })
        .into_js_value();
        self.inner.changes.add(function.unchecked_into())
    }
}

/// Zugriff innerhalb einer laufenden Transaktion (`DbConnection::transaction`). Schlüssel als JsValue, weil manche
/// Speicher Zahlen als Schlüssel haben (automatische Sicherungen).
pub struct Access(JsValue);

impl Access {
    async fn call(&self, method: &str, args: &[&JsValue]) -> Result<JsValue, JsValue> {
        let function: Function = get(&self.0, method).dyn_into()?;
        let list: Array = args.iter().map(|a| (*a).clone()).collect();
        let promise: Promise = function.apply(&self.0, &list)?.unchecked_into();
        JsFuture::from(promise).await
    }

    /// Alle Einträge eines Speichers als (Schlüssel, Wert); Werte, die kein JSON sind, fallen weg.
    pub async fn entries(&self, store: &str) -> Result<Vec<(JsValue, serde_json::Value)>, JsValue> {
        let pairs: Array = self.call("entries", &[&JsValue::from_str(store)]).await?.dyn_into().unwrap_or_default();
        Ok(pairs
            .iter()
            .filter_map(|pair| {
                let pair: Array = pair.dyn_into().ok()?;
                Some((pair.get(0), to_value(&pair.get(1))?))
            })
            .collect())
    }

    pub async fn keys(&self, store: &str) -> Result<Vec<JsValue>, JsValue> {
        let keys: Array = self.call("keys", &[&JsValue::from_str(store)]).await?.dyn_into().unwrap_or_default();
        Ok(keys.iter().collect())
    }

    /// Ein Wert als JSON; None, wenn es ihn nicht gibt.
    pub async fn get(&self, store: &str, key: &JsValue) -> Result<Option<serde_json::Value>, JsValue> {
        Ok(to_value(&self.call("get", &[&JsValue::from_str(store), key]).await?))
    }

    pub async fn put(&self, store: &str, key: &JsValue, value: &serde_json::Value) -> Result<(), JsValue> {
        self.call("put", &[&JsValue::from_str(store), key, &from_value(value)?]).await.map(|_| ())
    }

    pub async fn delete(&self, store: &str, key: &JsValue) -> Result<(), JsValue> {
        self.call("delete", &[&JsValue::from_str(store), key]).await.map(|_| ())
    }

    pub async fn clear(&self, store: &str) -> Result<(), JsValue> {
        self.call("clear", &[&JsValue::from_str(store)]).await.map(|_| ())
    }
}



// ---- Sicherung (crates/core/src/backup.rs) ----
