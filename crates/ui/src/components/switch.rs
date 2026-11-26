//! Nachbau von src/components/ui/switch.tsx (shadcn/ui, Radix Switch): Knopf mit role="switch", aria-checked und
//! data-state wie Radix, damit die Varianten data-checked und data-unchecked aus shadcn/tailwind.css greifen.

use leptos::prelude::*;

const ROOT_CLASS: &str = "peer group/switch relative inline-flex shrink-0 items-center rounded-full border border-transparent transition-all outline-none group-has-[:focus-visible]/field-label:border-transparent group-has-[:focus-visible]/field-label:ring-0 after:absolute after:-inset-x-3 after:-inset-y-2 focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 data-[size=default]:h-[18.4px] data-[size=default]:w-[32px] data-[size=sm]:h-[14px] data-[size=sm]:w-[24px] dark:aria-invalid:border-destructive/50 dark:aria-invalid:ring-destructive/40 data-checked:bg-primary data-unchecked:bg-input dark:data-unchecked:bg-input/80 data-disabled:cursor-not-allowed data-disabled:opacity-50";

const THUMB_CLASS: &str = "pointer-events-none block rounded-full bg-background ring-0 transition-transform group-data-[size=default]/switch:size-4 group-data-[size=sm]/switch:size-3 group-data-[size=default]/switch:data-checked:translate-x-[calc(100%-2px)] group-data-[size=sm]/switch:data-checked:translate-x-[calc(100%-2px)] dark:data-checked:bg-primary-foreground group-data-[size=default]/switch:data-unchecked:translate-x-0 group-data-[size=sm]/switch:data-unchecked:translate-x-0 dark:data-unchecked:bg-foreground";

/// Schalter. `size`: "default" oder "sm".
#[component]
pub fn Switch(
    #[prop(into)] id: String,
    #[prop(into)] checked: Signal<bool>,
    #[prop(into)] on_change: Callback<bool>,
    #[prop(optional, into)] size: Option<String>,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(optional, into)] aria_label: Option<String>,
) -> impl IntoView {
    let state = move || if checked.get() { "checked" } else { "unchecked" };
    view! {
        <button
            type="button"
            role="switch"
            id=id
            aria-label=aria_label
            value="on"
            data-slot="switch"
            data-size=size.unwrap_or_else(|| "default".into())
            aria-checked=move || checked.get().to_string()
            data-state=state
            data-disabled=move || disabled.get().then_some("")
            disabled=move || disabled.get()
            class=ROOT_CLASS
            on:click=move |_| on_change.run(!checked.get_untracked())
        >
            <span data-slot="switch-thumb" data-state=state class=THUMB_CLASS></span>
        </button>
    }
}
