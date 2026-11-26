//! Kurze Meldungen unten rechts im Fenster (In Rust, vorher sonner in TypeScript). Hier nur die Liste; Anzeige,
//! Ablaufzeit und Schließen übernimmt `Toaster` in crates/ui/src/toaster.rs.

use std::cell::RefCell;
use std::rc::Rc;

type Listener = Rc<dyn Fn()>;

/// Eine Meldung. `key` ändert sich, wenn eine Meldung mit derselben `id` ersetzt wird (dann beginnt ihre Zeit neu).
#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub key: u32,
    pub id: Option<String>,
    /// "error", "info", "success", "warning" oder "loading".
    pub kind: String,
    pub text: String,
}

#[derive(Default)]
struct State {
    next: u32,
    list: Vec<Toast>,
    listeners: Vec<(u32, Listener)>,
    next_listener: u32,
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

/// Zeigt eine Meldung. Mit `id` ersetzt sie eine frühere mit derselben `id` an deren Stelle.
pub fn show(kind: &str, text: &str, id: Option<&str>) {
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        state.next += 1;
        let toast = Toast { key: state.next, id: id.map(str::to_string), kind: kind.to_string(), text: text.to_string() };
        match id.and_then(|id| state.list.iter().position(|t| t.id.as_deref() == Some(id))) {
            Some(index) => state.list[index] = toast,
            None => state.list.push(toast),
        }
    });
    emit();
}

/// Entfernt eine Meldung (abgelaufen oder geschlossen).
pub fn dismiss(key: u32) {
    let removed = STATE.with(|s| {
        let mut state = s.borrow_mut();
        let before = state.list.len();
        state.list.retain(|t| t.key != key);
        state.list.len() != before
    });
    if removed {
        emit();
    }
}

/// Alle Meldungen, älteste zuerst.
pub fn list() -> Vec<Toast> {
    STATE.with(|s| s.borrow().list.clone())
}

pub fn subscribe(listener: Listener) -> u32 {
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        state.next_listener += 1;
        let id = state.next_listener;
        state.listeners.push((id, listener));
        id
    })
}

pub fn unsubscribe(id: u32) {
    STATE.with(|s| s.borrow_mut().listeners.retain(|(i, _)| *i != id));
}
