//! Befehle an Teile der Oberfläche, die ihren Zustand selbst halten (bisher src/core/app-commands.ts): „newTab“ öffnet
//! das Fenster für einen neuen Tab, „openSettings“ das Einstellungsfenster. Ausgelöst von den allgemeinen Tastenkürzeln
//! (crates/web/src/hotkeys.rs) und dem Zahnrad in der Kopfleiste.

use std::cell::RefCell;
use std::rc::Rc;


type Listener = Rc<dyn Fn()>;

thread_local! {
    static LISTENERS: RefCell<Vec<(u32, String, Listener)>> = const { RefCell::new(Vec::new()) };
    static NEXT: RefCell<u32> = const { RefCell::new(0) };
}

/// Meldet sich für einen Befehl an; gibt die Nummer zum Abmelden zurück.
pub fn on(command: &str, listener: Listener) -> u32 {
    let id = NEXT.with(|n| {
        let mut n = n.borrow_mut();
        *n += 1;
        *n
    });
    LISTENERS.with(|l| l.borrow_mut().push((id, command.to_string(), listener)));
    id
}

pub fn off(id: u32) {
    LISTENERS.with(|l| l.borrow_mut().retain(|(i, _, _)| *i != id));
}

/// true, wenn jemand den Befehl angenommen hat (zum Beispiel ist die Kopfleiste erst nach dem Laden da).
pub fn run(command: &str) -> bool {
    let listeners: Vec<Listener> = LISTENERS.with(|l| l.borrow().iter().filter(|(_, c, _)| c == command).map(|(_, _, f)| f.clone()).collect());
    for listener in &listeners {
        listener();
    }
    !listeners.is_empty()
}

// ---- Für TypeScript (Arbeitsfläche in React) ----

