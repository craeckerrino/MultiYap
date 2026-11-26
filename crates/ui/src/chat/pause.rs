//! Pausiert einen Chat, solange etwas darin offen ist (bisher chat-pause.tsx; zum Beispiel das Grund-Fenster, die
//! Nutzerkarte oder die Maus über dem Chat). Mehrere Auslöser werden gezählt; der Chat läuft weiter, wenn alle
//! aufgehoben sind. Je Tab eine Pause.

use std::cell::Cell;

use leptos::prelude::*;
use wasm_bindgen::prelude::*;

#[derive(Clone, Copy)]
pub struct ChatPause(RwSignal<u32>);

impl Default for ChatPause {
    fn default() -> Self {
        ChatPause(RwSignal::new(0))
    }
}

impl ChatPause {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn paused(&self) -> bool {
        self.0.get() > 0
    }

    /// Hält den Chat an. Die zurückgegebene JavaScript-Funktion hebt diese Pause wieder auf (nur der erste Aufruf
    /// zählt); sie geht auch an die Nutzerkarte in crates/web.
    pub fn hold(&self) -> JsValue {
        let holders = self.0;
        holders.update(|count| *count += 1);
        let released = Cell::new(false);
        Closure::<dyn Fn()>::new(move || {
            if !released.replace(true) {
                holders.try_update(|count| *count = count.saturating_sub(1));
            }
        })
        .into_js_value()
    }
}
