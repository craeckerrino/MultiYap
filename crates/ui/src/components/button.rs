//! Nachbau von src/components/ui/button.tsx (shadcn/ui): gleiche Grundklassen, Varianten und Größen.

use leptos::prelude::*;

const BASE: &str = "group/button inline-flex shrink-0 items-center justify-center rounded-lg border border-transparent bg-clip-padding text-sm font-medium whitespace-nowrap transition-all outline-none select-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 active:not-aria-[haspopup]:translate-y-px disabled:pointer-events-none disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 dark:aria-invalid:border-destructive/50 dark:aria-invalid:ring-destructive/40 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4";

fn variant_class(variant: &str) -> &'static str {
    match variant {
        "outline" => "border-border bg-background hover:bg-muted hover:text-foreground aria-expanded:bg-muted aria-expanded:text-foreground dark:border-input dark:bg-input/30 dark:hover:bg-input/50",
        "secondary" => "bg-secondary text-secondary-foreground hover:bg-[color-mix(in_oklch,var(--secondary),var(--foreground)_5%)] aria-expanded:bg-secondary aria-expanded:text-secondary-foreground",
        "ghost" => "hover:bg-muted hover:text-foreground aria-expanded:bg-muted aria-expanded:text-foreground dark:hover:bg-muted/50",
        "destructive" => "bg-destructive/10 text-destructive hover:bg-destructive/20 focus-visible:border-destructive/40 focus-visible:ring-destructive/20 dark:bg-destructive/20 dark:hover:bg-destructive/30 dark:focus-visible:ring-destructive/40",
        "link" => "text-primary underline-offset-4 hover:underline",
        // Ohne eigene Farben: der Aufrufer gibt Hintergrund und Text vor (zum Beispiel AutoMod). Ohne Zusammenführen
        // der Klassen gewinnt sonst je nach Reihenfolge im CSS bg-primary.
        "plain" => "",
        _ => "bg-primary text-primary-foreground hover:bg-primary/80",
    }
}

fn size_class(size: &str) -> &'static str {
    match size {
        "xs" => "h-6 gap-1 rounded-[min(var(--radius-md),10px)] px-2 text-xs in-data-[slot=button-group]:rounded-lg has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 [&_svg:not([class*='size-'])]:size-3",
        "sm" => "h-7 gap-1 rounded-[min(var(--radius-md),12px)] px-2.5 text-[0.8rem] in-data-[slot=button-group]:rounded-lg has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 [&_svg:not([class*='size-'])]:size-3.5",
        "lg" => "h-9 gap-1.5 px-2.5 has-data-[icon=inline-end]:pr-2 has-data-[icon=inline-start]:pl-2",
        "icon" => "size-8",
        "icon-xs" => "size-6 rounded-[min(var(--radius-md),10px)] in-data-[slot=button-group]:rounded-lg [&_svg:not([class*='size-'])]:size-3",
        "icon-sm" => "size-7 rounded-[min(var(--radius-md),12px)] in-data-[slot=button-group]:rounded-lg",
        "icon-lg" => "size-9",
        _ => "h-8 gap-1.5 px-2.5 has-data-[icon=inline-end]:pr-2 has-data-[icon=inline-start]:pl-2",
    }
}

/// Klassen eines Knopfs (für Elemente, die wie ein Knopf aussehen sollen).
pub fn button_class(variant: &str, size: &str, extra: &str) -> String {
    format!("{BASE} {} {} {extra}", variant_class(variant), size_class(size)).trim_end().to_string()
}

/// Knopf. `variant`: default, outline, secondary, ghost, destructive, link, plain. `size`: default, xs, sm, lg, icon, icon-xs,
/// icon-sm, icon-lg. Ein Klick ruft `on_click`, bei type="submit" sendet er das Formular ab.
#[component]
pub fn Button(
    #[prop(optional, into)] variant: Signal<&'static str>,
    #[prop(optional)] size: Option<&'static str>,
    #[prop(optional, into)] class: Signal<String>,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(optional, into)] aria_label: Option<String>,
    #[prop(optional)] submit: bool,
    #[prop(optional, into)] on_click: Option<Callback<()>>,
    children: Children,
) -> impl IntoView {
    let size = size.unwrap_or("default");
    view! {
        <button
            type=if submit { "submit" } else { "button" }
            data-slot="button"
            data-variant=move || match variant.get() { "" => "default", v => v }
            data-size=size
            aria-label=aria_label
            disabled=move || disabled.get()
            class=move || button_class(variant.get(), size, &class.get())
            on:click=move |_| {
                if let Some(on_click) = on_click {
                    on_click.run(());
                }
            }
        >
            {children()}
        </button>
    }
}
