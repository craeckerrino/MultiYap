//! Oberfläche von MultiYap in Leptos. Zeichnet Rust die ganze App (`start`); React gibt es nicht mehr.
//!
//! Gestaltung: eigene Komponenten, die shadcn/ui 1:1 nachbauen (gleiche Klassen, Tokens, Rundungen, Abstände und
//! Größen). Die Klassen stehen ausgeschrieben im Code, damit Tailwind sie findet.

pub mod app;
pub mod bridge;
pub mod chat;
pub mod chatters;
pub mod components;
pub mod layout;
pub mod settings;
pub mod show;
pub mod sign_in;
pub mod store;
pub mod tab_views;
pub mod tabs;
pub mod toaster;
pub mod usercard;
pub mod workspace;

use std::cell::RefCell;

type Mounted = leptos::mount::UnmountHandle<leptos::tachys::view::any_view::AnyViewState>;

thread_local! {
    static MOUNTED: RefCell<Option<Mounted>> = const { RefCell::new(None) };
}

fn mount(root: &web_sys::HtmlElement) {
    let handle = leptos::mount::mount_to(root.clone(), || leptos::prelude::IntoAny::into_any(app::App()));
    MOUNTED.with(|m| *m.borrow_mut() = Some(handle));
}

/// Zeichnet die App in `root` (Programmstart, crates/wasm). Wechselt die Sprache (Einstellungen → Allgemein), wird
/// die Oberfläche abgebaut und in der neuen Sprache neu gezeichnet; Dienste und Verbindungen laufen weiter.
pub fn start(root: web_sys::HtmlElement) {
    mount(&root);
    multiyap_web::language::on_change(std::rc::Rc::new(move || {
        MOUNTED.with(|m| drop(m.borrow_mut().take()));
        mount(&root);
    }));
}
