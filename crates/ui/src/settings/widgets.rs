//! Bausteine der Einstellungen: Eingabe einer Timeout-Dauer (duration-input.tsx) und Taste wählen
//! (key-capture.tsx). Logik in crates/core/src/shortcuts.rs und hotkeys.rs.

use multiyap_core::i18n::tr;
use multiyap_core::{hotkeys, shortcuts};
use leptos::prelude::*;

use crate::components::button::button_class;
use crate::components::input::{merge_width, INPUT_CLASS};
use crate::components::{Select, SelectEntry};

/// Eingabe einer Timeout-Dauer mit Einheit (1 Sekunde bis 2 Wochen). Speichert nur gültige Werte.
#[component]
pub fn DurationInput(
    #[prop(into)] seconds: Signal<i64>,
    #[prop(into)] on_change: Callback<i64>,
    /// Dauern, die schon vergeben sind (zum Beispiel von anderen Timeout-Buttons): werden abgelehnt.
    #[prop(optional, into)]
    taken: Option<Signal<Vec<i64>>>,
) -> impl IntoView {
    let initial = shortcuts::split_duration(seconds.get_untracked());
    let amount = RwSignal::new(initial.0.to_string());
    let unit = RwSignal::new(initial.1.to_string());
    let error = RwSignal::new(None::<&'static str>);
    Effect::new(move |_| {
        let (a, u) = shortcuts::split_duration(seconds.get());
        amount.set(a.to_string());
        unit.set(u.to_string());
    });
    let apply = move |next_amount: String, next_unit: String| {
        let parsed = next_amount.trim().parse::<f64>().unwrap_or(f64::NAN);
        match shortcuts::to_seconds(parsed, &next_unit) {
            Err(message) => error.set(Some(message)),
            Ok(result) if result != seconds.get_untracked() && taken.is_some_and(|t| t.with_untracked(|t| t.contains(&result))) => {
                error.set(Some(tr("Diese Dauer gibt es schon.", "This duration already exists.")));
            }
            Ok(result) => {
                error.set(None);
                if result != seconds.get_untracked() {
                    on_change.run(result);
                }
            }
        }
    };
    let entries = shortcuts::duration_units().into_iter().map(|(id, label, _)| SelectEntry::item(id, label)).collect::<Vec<_>>();
    view! {
        <div class="flex flex-col gap-1">
            <div class="flex gap-2">
                <input
                    data-slot="input"
                    class=merge_width(INPUT_CLASS, "w-20")
                    inputmode="numeric"
                    aria-label=tr("Dauer", "Duration")
                    aria-invalid=move || error.get().map(|_| "true")
                    prop:value=move || amount.get()
                    on:input=move |event| amount.set(event_target_value(&event))
                    on:blur=move |_| apply(amount.get_untracked(), unit.get_untracked())
                    on:keydown=move |event| {
                        if event.key() == "Enter" {
                            apply(amount.get_untracked(), unit.get_untracked());
                        }
                    }
                />
                <Select
                    id=format!("einheit-{}", crate::bridge::random_id())
                    class="w-32"
                    aria_label=tr("Einheit", "Unit")
                    value=unit
                    entries=Signal::stored(entries)
                    on_change=Callback::new(move |next: String| {
                        unit.set(next.clone());
                        apply(amount.get_untracked(), next);
                    })
                />
            </div>
            {move || error.get().map(|text| view! { <p class="text-xs text-destructive">{text}</p> })}
        </div>
    }
}

/// Taste wählen: Klick, dann die gewünschte Taste drücken (Frei wählbar). Esc oder Klick daneben bricht ab. Gibt
/// `on_change` false zurück (zum Beispiel weil die Taste schon belegt ist), bleibt die alte Taste. Mit `combo` zählen
/// Strg, Alt, Umschalt und Win mit (allgemeine Tastenkürzel), sonst nur die Taste.
#[component]
pub fn KeyCapture(
    #[prop(into)] value: Signal<Option<String>>,
    #[prop(into)] on_change: Callback<String, bool>,
    #[prop(into)] label: String,
    #[prop(optional)] combo: bool,
    #[prop(optional, into)] disabled: Signal<bool>,
) -> impl IntoView {
    let listening = RwSignal::new(false);
    // Wird die Leertaste belegt, löst ihr Loslassen sonst einen neuen Klick aus und startet das Warten erneut.
    let swallow_space_up = RwSignal::new(false);
    let class = move || {
        let width = if combo { "w-36" } else { "w-28" };
        let base = button_class("outline", "sm", width);
        if listening.get() { format!("{} text-primary", base.replacen("border-border ", "border-primary ", 1)) } else { base }
    };
    let text = move || {
        if listening.get() {
            return tr("Taste drücken", "Press a key").to_string();
        }
        match value.get() {
            Some(key) if combo => hotkeys::combo_label(&key),
            Some(key) => hotkeys::shortcut_key_label(&key),
            None => tr("Taste wählen", "Choose a key").to_string(),
        }
    };
    view! {
        <button
            type="button"
            data-slot="button"
            data-variant="outline"
            data-size="sm"
            aria-label=label
            // Das Einstellungsfenster lässt Esc dann zum Abbrechen hier, statt sich zu schließen.
            data-key-capture=move || listening.get().then_some("listening")
            disabled=move || disabled.get()
            class=class
            on:click=move |_| listening.set(true)
            on:blur=move |_| listening.set(false)
            on:keydown=move |event| {
                if !listening.get_untracked() || event.key() == "Tab" {
                    return;
                }
                event.prevent_default();
                event.stop_propagation();
                if event.key() == "Escape" {
                    listening.set(false);
                    return;
                }
                if !combo && (event.ctrl_key() || event.alt_key() || event.meta_key()) {
                    return;
                }
                let key = if combo {
                    hotkeys::combo_from_event(&event.key(), event.ctrl_key(), event.alt_key(), event.shift_key(), event.meta_key())
                } else {
                    hotkeys::normalize_shortcut_key(&event.key())
                };
                let Some(key) = key else { return };
                listening.set(false);
                if event.key() == " " {
                    swallow_space_up.set(true);
                }
                on_change.run(key);
            }
            on:keyup=move |event| {
                if event.key() == " " && swallow_space_up.get_untracked() {
                    event.prevent_default();
                    swallow_space_up.set(false);
                }
            }
        >
            {text}
        </button>
    }
}
