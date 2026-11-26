//! Nachbau von SettingRow und SettingGroup (src/features/settings/setting-row.tsx): einheitliche Zeile im
//! Einstellungsfenster, Beschriftung links, Bedienelement rechts. Die data-Attribute nutzt die Suche im
//! Einstellungsfenster (settings-search.ts).

use leptos::prelude::*;

/// Zeile: Beschriftung, optionale Erklärung, Bedienelement.
#[component]
pub fn SettingRow(
    #[prop(into)] label: String,
    #[prop(optional, into)] description: Option<String>,
    #[prop(optional, into)] html_for: Option<String>,
    children: Children,
) -> impl IntoView {
    view! {
        <div class="flex items-center justify-between gap-6 py-3">
            <div class="flex flex-col gap-1">
                // Bleibt für Screenreader mit dem Bedienelement verknüpft, ein Klick auf den Text schaltet aber nichts
                // (Nur der Schalter selbst soll schalten).
                <label for=html_for class="text-sm font-medium" on:click=|event| event.prevent_default()>
                    {label}
                </label>
                {description.filter(|t| !t.is_empty()).map(|text| view! { <p class="text-sm text-muted-foreground">{text}</p> })}
            </div>
            <div class="shrink-0">{children()}</div>
        </div>
    }
}

/// Überschrift eines Abschnitts innerhalb eines Bereichs; jedes direkte Kind ist ein Eintrag für die Suche.
#[component]
pub fn SettingGroup(
    /// Leer: Gruppe ohne Überschrift, etwa für einzelne Einstellungen (Keine Zwischenüberschrift über nur einem Punkt).
    #[prop(optional, into)] title: String,
    #[prop(optional, into)] description: Option<String>,
    children: Children,
) -> impl IntoView {
    let head = (!title.is_empty() || description.is_some()).then(|| {
        view! {
            <div data-setting-group-head="" class="flex flex-col gap-1">
                {(!title.is_empty()).then(|| view! { <h3 class="text-sm font-semibold">{title}</h3> })}
                {description.map(|text| view! { <p class="text-sm text-muted-foreground">{text}</p> })}
            </div>
        }
    });
    view! {
        <section class="flex flex-col gap-1" data-setting-group="">
            {head}
            <div class="flex flex-col divide-y" data-setting-items="">
                {children()}
            </div>
        </section>
    }
}
