//! Kurze Meldungen unten rechts (Nachbau von shadcn/ui „Sonner“ in Rust): Farben aus den Tokens von Popover, Rundung
//! aus `--radius`, Symbol je Art. Höchstens drei Meldungen sind sichtbar, die neueste unten. Eine Meldung verschwindet
//! nach vier Sekunden („lädt“ erst, wenn sie ersetzt wird); solange der Zeiger über den Meldungen ist, läuft die Zeit
//! nicht weiter. Die Liste liegt in crates/web/src/toasts.rs.

use multiyap_core::i18n::tr;
use std::time::Duration;

use leptos::prelude::*;

use multiyap_web::toasts::{self, Toast};

use crate::components::icons;

/// Anzeigedauer einer Meldung (wie bei sonner).
const DURATION_MS: f64 = 4000.0;
/// Höchstzahl sichtbarer Meldungen (wie bei sonner).
const VISIBLE: usize = 3;

fn kind_icon(kind: &str) -> AnyView {
    match kind {
        "success" => icons::circle_check("size-4").into_any(),
        "warning" => icons::triangle_alert("size-4").into_any(),
        "error" => icons::octagon_x("size-4").into_any(),
        "loading" => icons::loader_circle("size-4 animate-spin").into_any(),
        _ => icons::info("size-4").into_any(),
    }
}

#[component]
fn ToastItem(toast: Toast, paused: ReadSignal<bool>) -> impl IntoView {
    let key = toast.key;
    if toast.kind != "loading" {
        let remaining = StoredValue::new(DURATION_MS);
        let started = StoredValue::new(0.0_f64);
        let timer = StoredValue::new(None::<TimeoutHandle>);
        Effect::new(move |_| {
            if paused.get() {
                if let Some(handle) = timer.try_update_value(Option::take).flatten() {
                    handle.clear();
                    remaining.update_value(|r| *r -= js_sys::Date::now() - started.get_value());
                }
            } else if timer.with_value(Option::is_none) {
                started.set_value(js_sys::Date::now());
                let wait = Duration::from_millis(remaining.get_value().max(0.0) as u64);
                timer.set_value(set_timeout_with_handle(move || toasts::dismiss(key), wait).ok());
            }
        });
        on_cleanup(move || {
            if let Some(handle) = timer.try_update_value(Option::take).flatten() {
                handle.clear();
            }
        });
    }
    let live = if toast.kind == "error" { "assertive" } else { "polite" };
    view! {
        <li data-slot="toast" data-type=toast.kind.clone() role="status" aria-live=live aria-atomic="true"
            class="pointer-events-auto flex w-full items-center gap-1.5 rounded-lg border bg-popover p-4 text-[13px] text-popover-foreground shadow-lg animate-in fade-in-0 slide-in-from-bottom-4">
            <span class="flex shrink-0 items-center">{kind_icon(&toast.kind)}</span>
            <span class="min-w-0 font-medium break-words">{toast.text}</span>
        </li>
    }
}

/// Bereich für die Meldungen; einmal in der App eingebunden.
#[component]
pub fn Toaster() -> impl IntoView {
    let list = RwSignal::new(toasts::list());
    let subscription = toasts::subscribe(std::rc::Rc::new(move || {
        list.try_set(toasts::list());
    }));
    on_cleanup(move || toasts::unsubscribe(subscription));
    let (paused, set_paused) = signal(false);
    // Verschwindet die letzte Meldung unter dem Zeiger, kommt kein pointerleave: Pause dann aufheben.
    Effect::new(move |_| {
        if list.with(Vec::is_empty) {
            set_paused.set(false);
        }
    });
    let visible = move || {
        let all = list.get();
        let skip = all.len().saturating_sub(VISIBLE);
        all.into_iter().skip(skip).collect::<Vec<_>>()
    };
    view! {
        <section aria-label=tr("Meldungen", "Notifications") class="pointer-events-none fixed right-6 bottom-6 z-100 w-89 max-w-[calc(100vw-3rem)]">
            <ol class="flex flex-col gap-2" on:pointerenter=move |_| set_paused.set(true) on:pointerleave=move |_| set_paused.set(false)>
                <For each=visible key=|toast| toast.key let:toast>
                    <ToastItem toast=toast paused=paused />
                </For>
            </ol>
        </section>
    }
}
