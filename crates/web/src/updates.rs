//! Updates aus zwei Quellen:
//!
//! - Website: `https://multiyap.app/download/latest.json` (Einstellung „updates.fromWebsite“, Standard an), beim
//!   Start und alle 6 Stunden. Installiert wird im Programmfenster, das die Adresse `/update-download?version=…`
//!   abfängt, den Installer selbst lädt und Größe und Prüfsumme prüft (src-tauri/src/web_update.rs).
//! - Lokaler Versionsordner (Einstellung „updates.folder“): der lokale Server prüft den Ordner
//!   (crates/server/src/update.rs); installiert wird über `/update-install?…` (src-tauri/src/main.rs).
//!
//! Angeboten wird die höchste Version; gibt es sie in beiden Quellen, die aus dem Ordner (kein Download nötig).

use multiyap_core::i18n::tr;
use std::cell::RefCell;
use std::rc::Rc;

use multiyap_core::updates as core_updates;
use serde_json::Value;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::net::fetch_json;

const FOLDER: &str = "updates.folder";
const FROM_WEBSITE: &str = "updates.fromWebsite";
/// Abstand der Prüfungen auf der Website.
const WEBSITE_INTERVAL_MS: i32 = 6 * 60 * 60 * 1000;
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

type Listener = Rc<dyn Fn()>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Folder,
    Website,
}

/// Stand einer Quelle.
#[derive(Default)]
struct Check {
    checking: bool,
    /// Neuere Version in dieser Quelle.
    available: Option<String>,
    error: Option<String>,
    /// Laufende Prüfung; ältere Ergebnisse werden verworfen.
    run: u32,
}

#[derive(Default)]
struct State {
    started: bool,
    folder: Check,
    website: Check,
    /// Zuletzt per Meldung angekündigte Version (jede nur einmal).
    announced: Option<String>,
    listeners: Vec<(u32, Listener)>,
    next: u32,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn emit() {
    let listeners: Vec<Listener> = STATE.with(|s| s.borrow().listeners.iter().map(|(_, l)| l.clone()).collect());
    for listener in listeners {
        listener();
    }
}

fn folder() -> String {
    crate::settings::get(FOLDER).and_then(|v| v.as_str().map(|s| s.trim().to_string())).unwrap_or_default()
}

fn from_website() -> bool {
    crate::settings::get(FROM_WEBSITE).and_then(|v| v.as_bool()).unwrap_or(true)
}

fn encode(text: &str) -> String {
    js_sys::encode_uri_component(text).as_string().unwrap_or_default()
}

fn check_of(state: &mut State, source: Source) -> &mut Check {
    match source {
        Source::Folder => &mut state.folder,
        Source::Website => &mut state.website,
    }
}

/// Beim Start einmal prüfen, bei jeder Änderung der Einstellungen erneut, die Website zusätzlich alle 6 Stunden.
pub fn start() {
    if STATE.with(|s| std::mem::replace(&mut s.borrow_mut().started, true)) {
        return;
    }
    crate::settings::subscribe(FOLDER, Rc::new(check_folder));
    crate::settings::subscribe(FROM_WEBSITE, Rc::new(check_website));
    if let Some(window) = web_sys::window() {
        let tick = Closure::<dyn FnMut()>::new(check_website).into_js_value();
        let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(tick.unchecked_ref(), WEBSITE_INTERVAL_MS);
    }
    check();
}

/// Prüft alle eingeschalteten Quellen (auch über „Jetzt prüfen“).
pub fn check() {
    check_folder();
    check_website();
}

/// Beginnt eine Prüfung; `None`, wenn die Quelle aus ist (dann wird ihr Stand geleert).
fn begin(source: Source, enabled: bool) -> Option<u32> {
    let run = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let check = check_of(&mut s, source);
        check.run = check.run.wrapping_add(1);
        check.checking = enabled;
        if !enabled {
            check.available = None;
            check.error = None;
        }
        check.run
    });
    emit();
    enabled.then_some(run)
}

/// Übernimmt ein Ergebnis, kündigt eine neue Version einmal an.
fn finish(source: Source, run: u32, available: Option<String>, error: Option<String>) {
    let announce = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let check = check_of(&mut s, source);
        if check.run != run {
            return None;
        }
        check.checking = false;
        check.available = available.filter(|v| core_updates::is_newer(v, CURRENT_VERSION));
        check.error = error;
        let best = best(&s).map(|(v, _)| v);
        let new = best.filter(|v| s.announced.as_ref() != Some(v));
        if new.is_some() {
            s.announced = new.clone();
        }
        new
    });
    if let Some(version) = announce {
        crate::toasts::show("info", &multiyap_core::tf!("Update auf Version {version} bereit. Installieren über den Knopf oben rechts.", "Update to version {version} is ready. Install it with the button at the top right."), Some("update-available"));
    }
    emit();
}

fn check_folder() {
    let folder = folder();
    let Some(run) = begin(Source::Folder, !folder.is_empty()) else { return };
    spawn_local(async move {
        let (available, error) = match fetch_json(&format!("/api/update?folder={}&lang={}", encode(&folder), crate::language::code())).await {
            Ok(Some(json)) => (
                json.get("version").and_then(Value::as_str).map(str::to_string),
                json.get("error").and_then(Value::as_str).map(str::to_string),
            ),
            Ok(None) | Err(_) => (None, Some(tr("Prüfung nicht möglich.", "Check not possible.").to_string())),
        };
        finish(Source::Folder, run, available, error);
    });
}

fn check_website() {
    let Some(run) = begin(Source::Website, from_website()) else { return };
    spawn_local(async move {
        let (available, error) = match fetch_json(core_updates::LATEST_URL).await {
            Ok(Some(json)) => match core_updates::parse_release(&json) {
                Some(release) => (Some(release.version), None),
                None => (None, Some(tr("Angaben auf der Website sind ungültig.", "The details on the website are invalid.").to_string())),
            },
            Ok(None) => (None, None),
            Err(_) => (None, Some(tr("Website nicht erreichbar.", "Website not reachable.").to_string())),
        };
        finish(Source::Website, run, available, error);
    });
}

/// Höchste angebotene Version samt Quelle; bei Gleichstand der Ordner.
fn best(state: &State) -> Option<(String, Source)> {
    let folder = state.folder.available.clone().map(|v| (v, Source::Folder));
    let website = state.website.available.clone().map(|v| (v, Source::Website));
    match (folder, website) {
        (Some(f), Some(w)) => Some(if core_updates::is_newer(&w.0, &f.0) { w } else { f }),
        (f, w) => f.or(w),
    }
}

pub fn available() -> Option<String> {
    STATE.with(|s| best(&s.borrow()).map(|(v, _)| v))
}

pub fn source() -> Option<Source> {
    STATE.with(|s| best(&s.borrow()).map(|(_, source)| source))
}

/// Fehler der Quellen, zum Anzeigen in den Einstellungen.
pub fn error() -> Option<String> {
    STATE.with(|s| {
        let s = s.borrow();
        let parts: Vec<String> = [(tr("Ordner", "Folder"), &s.folder.error), ("Website", &s.website.error)]
            .into_iter()
            .filter_map(|(label, error)| error.as_ref().map(|e| format!("{label}: {e}")))
            .collect();
        (!parts.is_empty()).then(|| parts.join(" "))
    })
}

pub fn checking() -> bool {
    STATE.with(|s| {
        let s = s.borrow();
        s.folder.checking || s.website.checking
    })
}

/// Startet die Installation. Aus dem Ordner: das Programmfenster prüft erneut, startet den Installer und beendet
/// MultiYap. Von der Website: das Programmfenster lädt und prüft den Installer erst (kann etwas dauern).
pub fn install() {
    let Some((version, source)) = STATE.with(|s| best(&s.borrow())) else { return };
    let url = match source {
        Source::Folder => format!("/update-install?folder={}&version={}&lang={}", encode(&folder()), encode(&version), crate::language::code()),
        Source::Website => {
            crate::toasts::show("info", &multiyap_core::tf!("Update {version} wird von multiyap.app geladen und geprüft …", "Downloading and verifying update {version} from multiyap.app …"), Some("update-available"));
            format!("/update-download?version={}&lang={}", encode(&version), crate::language::code())
        }
    };
    if let Some(window) = web_sys::window() {
        let _ = window.open_with_url_and_target_and_features(&url, "_blank", "noopener,noreferrer");
    }
}

pub fn subscribe(listener: Listener) -> u32 {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.next += 1;
        let id = s.next;
        s.listeners.push((id, listener));
        id
    })
}

pub fn unsubscribe(id: u32) {
    STATE.with(|s| s.borrow_mut().listeners.retain(|(i, _)| *i != id));
}
