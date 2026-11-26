//! Sprache der Oberfläche: Einstellung `app.language` („auto“, „de“, „en“), „auto“ folgt der Sprache des Systems
//! (WebView2 meldet die Sprache von Windows). Die Texte wählt `multiyap_core::i18n`. Wechselt die Sprache, zeichnet die
//! Oberfläche sich neu (crates/ui/src/lib.rs); Chats, Anmeldung und Dienste laufen dabei weiter.

use std::cell::RefCell;
use std::rc::Rc;

use multiyap_core::i18n::{self, Lang};

pub const KEY: &str = "app.language";

type Listener = Rc<dyn Fn()>;

thread_local! {
    static LISTENERS: RefCell<Vec<Listener>> = const { RefCell::new(Vec::new()) };
}

/// Sprache des Systems laut Browser, zum Beispiel „de-DE“.
pub fn system() -> String {
    web_sys::window().and_then(|w| w.navigator().language()).unwrap_or_default()
}

fn setting() -> String {
    crate::settings::get(KEY).and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "auto".to_string())
}

/// Setzt die Sprache nach der Einstellung; true, wenn sie sich geändert hat.
fn apply() -> bool {
    let lang = Lang::from_setting(&setting(), &system());
    let changed = lang != i18n::current();
    i18n::set(lang);
    if let Some(root) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.document_element()) {
        let _ = root.set_attribute("lang", lang.code());
    }
    changed
}

/// Vor dem ersten Zeichnen: Sprache festlegen (bis die Einstellung geladen ist: die des Systems) und Änderungen folgen.
pub fn init() {
    apply();
    crate::settings::subscribe(
        KEY,
        Rc::new(|| {
            if apply() {
                let listeners: Vec<Listener> = LISTENERS.with(|l| l.borrow().clone());
                listeners.iter().for_each(|listener| listener());
            }
        }),
    );
}

/// Meldet einen Wechsel der Sprache (nach dem Setzen).
pub fn on_change(listener: Listener) {
    LISTENERS.with(|l| l.borrow_mut().push(listener));
}

/// Aktuelle Sprache, zum Weitergeben an das Programm (Adressen mit `lang=`).
pub fn code() -> &'static str {
    i18n::current().code()
}
