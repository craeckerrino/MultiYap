//! Programmstart: Die Seite ruft nach dem Laden von WebAssembly `start` auf; Rust zeichnet die App und startet
//! Speicher, Anmeldung und Dienste.

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn console_error(message: &str);
}

/// Startet MultiYap in `#root`. `redirect_href` und `expected_state`: ein Rücksprung von Twitch, den der Einstieg
/// vor dem Laden von WebAssembly erkannt und aus der Adresse entfernt hat (sonst undefined).
#[wasm_bindgen]
pub fn start(redirect_href: Option<String>, expected_state: Option<String>) -> Result<(), JsError> {
    // Fehler in Rust lesbar in der Konsole melden (statt nur „unreachable“).
    std::panic::set_hook(Box::new(|info| console_error(&info.to_string())));
    let root = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("root"))
        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
        .ok_or_else(|| JsError::new("Element #root fehlt."))?;
    multiyap_web::language::init();
    multiyap_ui::start(root);
    wasm_bindgen_futures::spawn_local(multiyap_web::startup::start(redirect_href.map(|href| (href, expected_state))));
    Ok(())
}
