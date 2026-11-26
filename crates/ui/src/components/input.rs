//! Nachbauten von src/components/ui/input.tsx und textarea.tsx (shadcn/ui) sowie CommitInput
//! (src/features/settings/commit-input.tsx): Textfeld, das beim Verlassen oder mit Enter speichert, damit der Cursor
//! beim Tippen nicht springt und trotzdem kein „Speichern“-Knopf nötig ist.

use leptos::prelude::*;

pub const INPUT_CLASS: &str = "h-8 w-full min-w-0 rounded-lg border border-input bg-transparent px-2.5 py-1 text-base transition-colors outline-none file:inline-flex file:h-6 file:border-0 file:bg-transparent file:text-sm file:font-medium file:text-foreground placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 disabled:pointer-events-none disabled:cursor-not-allowed disabled:bg-input/50 disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 md:text-sm dark:bg-input/30 dark:disabled:bg-input/80 dark:aria-invalid:border-destructive/50 dark:aria-invalid:ring-destructive/40";

pub const TEXTAREA_CLASS: &str = "flex field-sizing-content min-h-16 w-full rounded-lg border border-input bg-transparent px-2.5 py-2 text-base transition-colors outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:bg-input/50 disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 md:text-sm dark:bg-input/30 dark:disabled:bg-input/80 dark:aria-invalid:border-destructive/50 dark:aria-invalid:ring-destructive/40";

/// Klassen zusammenführen wie `cn` für die Fälle hier: eine eigene Breite ersetzt `w-full`.
pub fn merge_width(base: &str, extra: &str) -> String {
    if extra.split_whitespace().any(|c| c.starts_with("w-")) {
        format!("{} {extra}", base.replacen("w-full ", "", 1))
    } else {
        format!("{base} {extra}")
    }
}

/// Textfeld mit Entwurf: speichert beim Verlassen oder mit Enter, Escape stellt den gespeicherten Wert wieder her.
/// Gibt `on_commit` false zurück, wird der alte Wert wiederhergestellt.
#[component]
pub fn CommitInput(
    #[prop(into)] value: Signal<String>,
    #[prop(into)] on_commit: Callback<String, bool>,
    #[prop(optional, into)] id: Option<String>,
    #[prop(optional, into)] class: String,
    #[prop(optional, into)] placeholder: Option<String>,
    #[prop(optional, into)] aria_label: Option<String>,
    #[prop(optional)] max_length: Option<usize>,
    #[prop(optional, into)] disabled: Signal<bool>,
) -> impl IntoView {
    let draft = RwSignal::new(value.get_untracked());
    // Gespeicherter Wert hat sich geändert (auch aus einem anderen Tab): Entwurf übernimmt ihn.
    Effect::new(move |_| draft.set(value.get()));
    let commit = move || {
        let next = draft.get_untracked();
        if next == value.get_untracked() {
            return;
        }
        if !on_commit.run(next) {
            draft.set(value.get_untracked());
        }
    };
    view! {
        <input
            data-slot="input"
            id=id
            class=merge_width(INPUT_CLASS, &class)
            placeholder=placeholder
            aria-label=aria_label
            maxlength=max_length
            disabled=move || disabled.get()
            prop:value=move || draft.get()
            on:input=move |event| draft.set(event_target_value(&event))
            on:blur=move |_| commit()
            on:keydown=move |event| match event.key().as_str() {
                "Enter" => {
                    event.prevent_default();
                    commit();
                }
                "Escape" => draft.set(value.get_untracked()),
                _ => {}
            }
        />
    }
}
