//! Stände der Dienste in crates/web als Signale der Oberfläche: Die Dienste melden Änderungen über Zuhörer, die
//! Oberfläche liest den Stand dann neu.

use std::rc::Rc;

use leptos::prelude::*;

type Listener = Rc<dyn Fn()>;

/// Zähler, der bei jeder Meldung des Dienstes steigt. Wer ihn verfolgt, liest den Stand des Dienstes neu.
pub fn watch_version(subscribe: fn(Listener) -> u32, unsubscribe: fn(u32)) -> ReadSignal<u32> {
    let version = RwSignal::new(0u32);
    let id = subscribe(Rc::new(move || {
        version.try_update(|v| *v = v.wrapping_add(1));
    }));
    on_cleanup(move || unsubscribe(id));
    version.read_only()
}

/// Zähler für die Sitzung eines Kanals: steigt bei jeder Änderung, auch wenn die Sitzung erst später entsteht, wechselt
/// oder der Kanal ein anderer wird.
pub fn session_version(login: Signal<String>) -> ReadSignal<u32> {
    use multiyap_web::sessions;
    let version = RwSignal::new(0u32);
    let watch = StoredValue::new(None::<u32>);
    Effect::new(move |_| {
        let login = login.get();
        if let Some(id) = watch.get_value() {
            sessions::unwatch_login(id);
        }
        let id = sessions::watch_login(
            &login,
            Rc::new(move || {
                version.try_update(|v| *v = v.wrapping_add(1));
            }),
        );
        watch.set_value(Some(id));
        version.update(|v| *v = v.wrapping_add(1));
    });
    on_cleanup(move || {
        if let Some(id) = watch.try_get_value().flatten() {
            sessions::unwatch_login(id);
        }
    });
    version.read_only()
}

/// Öffnet eine Adresse in einem neuen Fenster (ohne Zugriff zurück auf MultiYap).
pub fn open_url(url: &str, features: &str) {
    if let Some(window) = web_sys::window() {
        let _ = window.open_with_url_and_target_and_features(url, "_blank", features);
    }
}

/// Teil einer Adresse sicher kodiert.
pub fn encode(text: &str) -> String {
    js_sys::encode_uri_component(text).as_string().unwrap_or_default()
}
