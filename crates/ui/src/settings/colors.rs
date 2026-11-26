//! Farben (colors-group.tsx): alle Farben von Hervorhebungen und Markierungen anpassbar und einzeln ausschaltbar, mit
//! Vorschau und „Standard wiederherstellen“. Änderungen gelten sofort. Fest bleiben bewusst die Chat-Ereignisse (Rosa),
//! die AutoMod-Knöpfe (Grün/Rot) und die Filter (nur ihre sechs festen Farben).

use multiyap_core::i18n::tr;
use std::collections::BTreeMap;

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::bridge::{save_setting, update_setting, use_setting};
use crate::components::overlay::AlertDialog;
use crate::components::{icons, Button, Select, SelectEntry, SettingGroup, Switch};

const COLORS: &str = "display.colors";

use multiyap_core::display::custom_colors;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct ColorEntry {
    color: Option<String>,
    enabled: bool,
}

type Colors = BTreeMap<String, ColorEntry>;

/// Ändert den Eintrag einer Farbe; ein Eintrag, der dem Standard entspricht, wird entfernt.
fn save_color(key: &'static str, entry: Option<ColorEntry>) {
    update_setting::<Colors>(COLORS, move |mut current| {
        match &entry {
            None => {
                current.remove(key);
            }
            Some(e) if e.enabled && e.color.is_none() => {
                current.remove(key);
            }
            Some(e) => {
                current.insert(key.into(), e.clone());
            }
        }
        current
    });
}

/// Feste Farbauswahl statt freier Farbwahl: Farben der Tailwind-Palette (Stufe 500), die zum dunklen Design passen.
/// Gespeichert wird weiter #rrggbb, ältere eigene Farben bleiben gültig.
fn palette() -> [(&'static str, &'static str, &'static str); 12] {
    [
    ("#ef4444", tr("Rot", "Red"), "bg-[#ef4444]"),
    ("#f97316", "Orange", "bg-[#f97316]"),
    ("#eab308", tr("Gelb", "Yellow"), "bg-[#eab308]"),
    ("#84cc16", tr("Hellgrün", "Light green"), "bg-[#84cc16]"),
    ("#22c55e", tr("Grün", "Green"), "bg-[#22c55e]"),
    ("#14b8a6", tr("Türkis", "Teal"), "bg-[#14b8a6]"),
    ("#0ea5e9", tr("Himmelblau", "Sky blue"), "bg-[#0ea5e9]"),
    ("#3b82f6", tr("Blau", "Blue"), "bg-[#3b82f6]"),
    ("#6366f1", "Indigo", "bg-[#6366f1]"),
    ("#8b5cf6", tr("Violett", "Violet"), "bg-[#8b5cf6]"),
    ("#ec4899", "Pink", "bg-[#ec4899]"),
    ("#71717a", tr("Grau", "Gray"), "bg-[#71717a]"),
]
}

/// Für die abwechselnden Zeilenhintergründe nur dunkle Grautöne (Tailwind, Stufen 800 und 900).
fn row_palette() -> [(&'static str, &'static str, &'static str); 4] {
    [
    ("#27272a", tr("Zink", "Zinc"), "bg-[#27272a]"),
    ("#262626", "Neutral", "bg-[#262626]"),
    ("#1e293b", tr("Schiefer", "Slate"), "bg-[#1e293b]"),
    ("#18181b", tr("Sehr dunkel", "Very dark"), "bg-[#18181b]"),
]
}

/// Einträge der Auswahl: „Standard“, die Palette und eine ältere eigene Farbe, solange sie gespeichert ist.
fn color_entries(key: &str, current: Option<&str>) -> Vec<SelectEntry> {
    let palette: &[(&str, &str, &str)] = if key == "row-alt" { &row_palette() } else { &palette() };
    let mut entries = vec![SelectEntry::item("", tr("Standard", "Default"))];
    entries.extend(palette.iter().map(|(hex, label, class)| SelectEntry::item(*hex, *label).swatch(*class)));
    if let Some(color) = current.filter(|c| !palette.iter().any(|(hex, _, _)| hex.eq_ignore_ascii_case(c))) {
        entries.push(SelectEntry::item(color, tr("Bisherige eigene Farbe", "Previous custom color")));
    }
    entries
}

/// Vorschau einer Farbe so, wie sie im Chat wirkt: Strich links, leichte Hinterlegung, Text in der Farbe.
fn swatch(key: &'static str) -> impl IntoView {
    if key == "row-alt" {
        view! { <span class="w-28 shrink-0 rounded-sm px-2 py-0.5 text-sm" style="background-color: var(--row-alt);">{tr("Beispiel", "Example")}</span> }.into_any()
    } else {
        let style = format!("border-left-color: var(--{key}); background-color: color-mix(in oklab, var(--{key}) 15%, transparent); color: var(--{key});");
        view! { <span class="w-28 shrink-0 rounded-sm border-l-2 px-2 py-0.5 text-sm" style=style>{tr("Beispiel", "Example")}</span> }.into_any()
    }
}

#[component]
fn ColorRow(key: &'static str, label: &'static str, colors: ReadSignal<Colors>) -> impl IntoView {
    let entry = Memo::new(move |_| colors.get().get(key).cloned());
    let enabled = Signal::derive(move || entry.get().is_none_or(|e| e.enabled));
    let current = Signal::derive(move || entry.get().and_then(|e| e.color).unwrap_or_default());
    let entries = Signal::derive(move || current.with(|c| color_entries(key, (!c.is_empty()).then_some(c.as_str()))));
    let pick = move |color: String| {
        let color = (!color.is_empty()).then_some(color);
        save_color(key, Some(ColorEntry { color, enabled: enabled.get_untracked() }));
    };
    view! {
        <div class="flex items-center gap-3 py-3">
            {swatch(key)}
            <span class="min-w-0 flex-1 text-sm font-medium">{label}</span>
            <Select
                id=format!("color-{key}")
                aria_label=multiyap_core::tf!("Farbe für {label}", "Color for {label}")
                value=current
                entries=entries
                disabled=Signal::derive(move || !enabled.get())
                class="w-40"
                on_change=Callback::new(pick)
            />
            <span title=tr("Standardfarbe", "Default color") class="contents">
                <Button variant="ghost" size="icon-sm" aria_label=multiyap_core::tf!("{label}: Standard", "{label}: default") disabled=Signal::derive(move || entry.get().is_none())
                    on_click=Callback::new(move |_| save_color(key, None))>
                    {icons::rotate_ccw("")}
                </Button>
            </span>
            <Switch
                id=format!("color-enabled-{key}")
                checked=enabled
                on_change=Callback::new(move |checked: bool| save_color(key, Some(ColorEntry { color: entry.get_untracked().and_then(|e| e.color), enabled: checked })))
            />
        </div>
    }
}

#[component]
pub fn ColorsSection() -> impl IntoView {
    let colors = use_setting(COLORS, Colors::new());
    view! {
        <div class="flex flex-col gap-8">
            <SettingGroup title=tr("Hervorhebungen und Markierungen", "Highlights and markers") description=tr("Aus: neutral grau. Chat-Ereignisse, AutoMod-Knöpfe und Filter haben feste Farben.", "Off: neutral gray. Chat events, AutoMod buttons and filters have fixed colors.")>
                {custom_colors().into_iter().map(|(key, label)| view! { <ColorRow key=key label=label colors=colors /> }).collect_view()}
                <div class="py-3">
                    <AlertDialog
                        title=tr("Alle Farben zurücksetzen?", "Reset all colors?")
                        description=tr("Alle eigenen Farben gehen verloren, ausgeschaltete Farben werden wieder eingeschaltet.", "All custom colors are lost, switched off colors are switched on again.")
                        action=tr("Zurücksetzen", "Reset")
                        trigger_disabled=Signal::derive(move || colors.get().is_empty())
                        on_confirm=Callback::new(|_| save_setting(COLORS, &Colors::new()))
                    >
                        {icons::rotate_ccw("")}
                        {tr("Standard wiederherstellen", "Restore default")}
                    </AlertDialog>
                </div>
            </SettingGroup>
        </div>
    }
}
