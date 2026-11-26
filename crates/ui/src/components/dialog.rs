//! Nachbau von shadcn Dialog (src/components/ui/dialog.tsx) mit dem eingebauten `<dialog>` und `showModal()`: oberste
//! Ebene, Rest der Seite gesperrt, Fokus bleibt drin. Esc und Klick daneben schließen; X oben rechts. Inhalte werden
//! nur gerendert, solange das Fenster offen ist (wie Radix).

use multiyap_core::i18n::tr;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::button::button_class;
use super::icons;

/// `open:grid` statt `grid`: Eine feste Anzeige-Klasse würde das geschlossene `<dialog>` sichtbar machen.
const CONTENT: &str = "fixed top-1/2 left-1/2 z-50 open:grid w-full max-w-[calc(100%-2rem)] -translate-x-1/2 -translate-y-1/2 gap-4 rounded-xl bg-popover p-4 text-sm text-popover-foreground ring-1 ring-foreground/10 duration-100 outline-none data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95 m-0 max-h-none border-0 backdrop:bg-black/10 backdrop:backdrop-blur-xs";
pub const DIALOG_HEADER: &str = "flex flex-col gap-2";
pub const DIALOG_FOOTER: &str = "-mx-4 -mb-4 flex flex-col-reverse gap-2 rounded-b-xl border-t bg-muted/50 p-4 sm:flex-row sm:justify-end";
pub const DIALOG_TITLE: &str = "text-base leading-none font-medium";
pub const DIALOG_DESCRIPTION: &str = "text-sm text-muted-foreground *:[a]:underline *:[a]:underline-offset-3 *:[a]:hover:text-foreground";
pub const LABEL: &str = "flex items-center gap-2 text-sm leading-none font-medium select-none group-data-[disabled=true]:pointer-events-none group-data-[disabled=true]:opacity-50 peer-disabled:cursor-not-allowed peer-disabled:opacity-50";

/// Fenster mit Titel und Beschreibung. `max_width`: zum Beispiel "sm:max-w-md" (Standard sm:max-w-sm).
#[component]
pub fn Dialog(
    open: RwSignal<bool>,
    #[prop(into)] title: String,
    #[prop(into)] description: Signal<String>,
    #[prop(optional, into)] max_width: Option<String>,
    children: ChildrenFn,
) -> impl IntoView {
    let dialog_ref = NodeRef::<leptos::html::Dialog>::new();
    let id = crate::bridge::random_id();
    let (title_id, description_id) = (format!("{id}-title"), format!("{id}-description"));
    Effect::new(move |_| {
        let Some(dialog) = dialog_ref.get() else { return };
        if open.get() {
            if !dialog.open() {
                let _ = dialog.show_modal();
            }
        } else if dialog.open() {
            dialog.close();
        }
    });
    let class = format!("{CONTENT} {}", max_width.unwrap_or_else(|| "sm:max-w-sm".into()));
    let (labelled, described) = (title_id.clone(), description_id.clone());
    view! {
        <dialog node_ref=dialog_ref class=class data-slot="dialog-content"
            role=move || open.get().then_some("dialog")
            aria-labelledby=labelled aria-describedby=described
            data-state=move || if open.get() { "open" } else { "closed" }
            on:close=move |_| open.set(false)
            on:click=move |event: web_sys::MouseEvent| {
                // Klick daneben (auf den abgedunkelten Hintergrund) schließt.
                let Some(dialog) = dialog_ref.get_untracked() else { return };
                let on_dialog = event.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()).is_some_and(|t| t == **dialog);
                if on_dialog {
                    let rect = dialog.get_bounding_client_rect();
                    let (x, y) = (f64::from(event.client_x()), f64::from(event.client_y()));
                    if x < rect.left() || x > rect.right() || y < rect.top() || y > rect.bottom() {
                        open.set(false);
                    }
                }
            }>
            {move || open.get().then(|| view! {
                <div data-slot="dialog-header" class=DIALOG_HEADER>
                    <h2 id=title_id.clone() data-slot="dialog-title" class=DIALOG_TITLE>{title.clone()}</h2>
                    <p id=description_id.clone() data-slot="dialog-description" class=DIALOG_DESCRIPTION>{move || description.get()}</p>
                </div>
                {children()}
                <button type="button" data-slot="dialog-close" class=button_class("ghost", "icon-sm", "absolute top-2 right-2") on:click=move |_| open.set(false)>
                    {icons::x("")}
                    <span class="sr-only">{tr("Schließen", "Close")}</span>
                </button>
            })}
        </dialog>
    }
}
