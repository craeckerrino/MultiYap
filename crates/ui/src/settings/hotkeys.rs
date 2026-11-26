//! Tastenkürzel: allgemeine (hotkeys-section.tsx) und Mod-Tastenkürzel (shortcuts-section.tsx). Änderungen gelten
//! sofort.

use multiyap_core::i18n::tr;
use std::collections::{BTreeMap, BTreeSet};

use multiyap_core::hotkeys::{self as hk, hotkey_actions};
use multiyap_core::shortcuts;
use leptos::prelude::*;

use super::model::{action_label, Hotkeys, ShortcutEntry, ShortcutList, ALL_ACTIONS, MAX_REASON_LENGTH};
use super::widgets::{DurationInput, KeyCapture};
use crate::bridge::{random_id, save_setting, use_setting};
use crate::components::{icons, Button, CommitInput, Select, SelectEntry, SettingGroup, SettingRow, Switch};

const HOTKEYS: &str = "hotkeys.general";
const SHORTCUTS: &str = "moderation.shortcutList";

fn empty_list() -> ShortcutList {
    ShortcutList { enabled: false, entries: Vec::new() }
}

/// Belegte Mod-Tasten mit Beschreibung, unabhängig davon, ob die Mod-Tastenkürzel eingeschaltet sind.
fn mod_keys(list: &ShortcutList) -> BTreeMap<String, String> {
    list.entries
        .iter()
        .filter_map(|e| e.key.clone().map(|k| (k, hk::describe_shortcut(&e.action, e.duration_seconds, e.reason.as_deref()))))
        .collect()
}

#[component]
pub fn HotkeysSection() -> impl IntoView {
    let hotkeys = use_setting(HOTKEYS, Hotkeys::new());
    let mod_list = use_setting(SHORTCUTS, empty_list());
    let error = RwSignal::new(None::<(&'static str, String)>);

    let assign = move |action: &'static str, combo: Option<String>| -> bool {
        let current = hotkeys.get_untracked();
        if let Some(combo) = &combo {
            if let Some(clash) = hk::find_hotkey_clash(&current, action, combo, &mod_keys(&mod_list.get_untracked())) {
                error.set(Some((action, clash)));
                return false;
            }
        }
        error.set(None);
        let mut next = current;
        next.insert(action.to_string(), combo);
        save_setting(HOTKEYS, &next);
        true
    };

    view! {
        <SettingGroup
            title=tr("Tastenkürzel", "Keyboard shortcuts")
            description=tr("Gelten überall, auch beim Tippen (außer einfache Buchstaben). Taste anklicken und Kombination drücken, Esc bricht ab.", "Work everywhere, also while typing (except single letters). Click the key and press the combination, Esc cancels.")
        >
            {hotkey_actions()
                .into_iter()
                .map(|(action, label)| {
                    let action: &'static str = action;
                    let value = Signal::derive(move || hotkeys.get().get(action).cloned().flatten());
                    view! {
                        <div class="flex flex-col">
                            <SettingRow label=label>
                                <div class="flex items-center gap-1">
                                    <KeyCapture
                                        combo=true
                                        value=value
                                        label=multiyap_core::tf!("Taste für {label}", "Key for {label}")
                                        on_change=Callback::new(move |combo: String| assign(action, Some(combo)))
                                    />
                                    <Button
                                        variant="ghost"
                                        size="icon-sm"
                                        aria_label=multiyap_core::tf!("Taste für {label} entfernen", "Remove key for {label}")
                                        disabled=Signal::derive(move || value.get().is_none())
                                        on_click=Callback::new(move |_| {
                                            assign(action, None);
                                        })
                                    >
                                        {icons::x("")}
                                    </Button>
                                </div>
                            </SettingRow>
                            {move || {
                                error.get().filter(|(a, _)| *a == action).map(|(_, text)| view! { <p class="pb-3 text-xs text-destructive">{text}</p> })
                            }}
                        </div>
                    }
                })
                .collect_view()}
        </SettingGroup>
    }
}

/// Grund nur, wo Twitch einen nimmt: Verwarnen (Pflicht), Timeout und Ban (optional).
fn takes_reason(action: &str) -> bool {
    matches!(action, "warn" | "timeout" | "ban")
}

/// Aktionen, die es nur einmal gibt (Timeouts je Dauer, Verwarnen je Grund).
const SINGLE_ACTIONS: [&str; 3] = ["delete", "ban", "unban"];

#[component]
pub fn ShortcutsSection() -> impl IntoView {
    let list = use_setting(SHORTCUTS, empty_list());
    let hotkeys = use_setting(HOTKEYS, Hotkeys::new());
    // Hinweis am betroffenen Eintrag, zum Beispiel bei doppelter Taste oder Aktion.
    let error = RwSignal::new(None::<(String, String)>);
    // Ändert sich nach einer abgelehnten Änderung, damit die Felder wieder den gespeicherten Wert zeigen.
    let reset = RwSignal::new(0u32);
    let off = Signal::derive(move || !list.get().enabled);

    // Immer sortiert speichern, damit neue und geänderte Einträge an ihren Platz rutschen.
    let save = move |entries: Vec<ShortcutEntry>| {
        let mut next = list.get_untracked();
        next.entries = shortcuts::sort_entries(&entries);
        save_setting(SHORTCUTS, &next);
    };

    // Übernimmt eine Änderung, außer Taste oder Aktion liegen schon woanders (jede Aktion nur eine Taste).
    let apply = move |next: ShortcutEntry| -> bool {
        let current = list.get_untracked();
        let reject = |text: String| {
            error.set(Some((next.id.clone(), text)));
            reset.update(|r| *r += 1);
            false
        };
        if let Some(key) = &next.key {
            if let Some(general) = shortcuts::hotkey_using_mod_key(&hotkeys.get_untracked(), key) {
                return reject(multiyap_core::tf!("„{}“ ist schon für „{}“ belegt (Allgemein, Tastenkürzel).", "“{}” is already used for “{}” (General, Keyboard shortcuts).",
                    hk::shortcut_key_label(key),
                    hk::hotkey_label(general)
                ));
            }
        }
        if let Some((kind, other)) = shortcuts::find_clash(&current.entries, &next) {
            let key = hk::shortcut_key_label(other.key.as_deref().unwrap_or(""));
            let describe = |e: &ShortcutEntry| hk::describe_shortcut(&e.action, e.duration_seconds, e.reason.as_deref());
            return reject(if kind == "key" {
                multiyap_core::tf!("„{key}“ ist schon für {} belegt.", "“{key}” is already used for {}.", describe(other))
            } else {
                multiyap_core::tf!("{} liegt schon auf „{key}“. Jede Aktion nur eine Taste.", "{} is already on “{key}”. Only one key per action.", describe(&next))
            });
        }
        error.set(None);
        save(current.entries.iter().map(|e| if e.id == next.id { next.clone() } else { e.clone() }).collect());
        true
    };

    let groups = Memo::new(move |_| {
        shortcuts::group_entries(&list.get().entries)
    });

    view! {
        <div class="flex flex-col gap-8">
            <SettingGroup>
                <SettingRow label=tr("Tastenkürzel einschalten", "Turn on keyboard shortcuts") description=tr("Maus über eine Nachricht und Taste drücken, die Aktion läuft sofort. Nur in Mod-Kanälen, jede Taste und Aktion nur einmal.", "Hover over a message and press the key, the action runs immediately. Only in mod channels, each key and action only once.") html_for="shortcuts-enabled">
                    <Switch
                        id="shortcuts-enabled"
                        checked=Signal::derive(move || list.get().enabled)
                        on_change=Callback::new(move |enabled: bool| {
                            let mut next = list.get_untracked();
                            next.enabled = enabled;
                            save_setting(SHORTCUTS, &next);
                        })
                    />
                </SettingRow>
            </SettingGroup>
            // Nach Aktion gebündelt, neue Einträge ohne Taste ganz unten.
            <For each=move || groups.get() key=|(id, _, entries)| (*id, entries.iter().map(|e| e.id.clone()).collect::<Vec<_>>()) let:group>
                <SettingGroup title=group.1>
                    {group
                        .2
                        .into_iter()
                        .map(|entry| view! { <EntryRow entry=entry list=list off=off error=error reset=reset apply=apply save=save /> })
                        .collect_view()}
                </SettingGroup>
            </For>
            {move || groups.get().is_empty().then(|| view! { <p class="text-sm text-muted-foreground">{tr("Noch keine Tastenkürzel.", "No keyboard shortcuts yet.")}</p> })}
            <div class="flex flex-col items-start gap-2">
                <Button
                    variant="secondary"
                    size="sm"
                    disabled=Signal::derive(move || off.get() || list.get().entries.len() >= 30)
                    on_click=Callback::new(move |_| {
                        let mut entries = list.get_untracked().entries;
                        entries.push(ShortcutEntry { id: random_id(), key: None, action: "timeout".into(), duration_seconds: Some(600), reason: None });
                        save(entries);
                    })
                >
                    {icons::plus("")}
                    {tr("Tastenkürzel hinzufügen", "Add keyboard shortcut")}
                </Button>
            </div>
        </div>
    }
}

#[component]
fn EntryRow(
    entry: ShortcutEntry,
    list: ReadSignal<ShortcutList>,
    off: Signal<bool>,
    error: RwSignal<Option<(String, String)>>,
    reset: RwSignal<u32>,
    apply: impl Fn(ShortcutEntry) -> bool + Copy + Send + Sync + 'static,
    save: impl Fn(Vec<ShortcutEntry>) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let id = entry.id.clone();
    // Aktueller Stand dieses Eintrags (nach dem Speichern aus der Einstellung).
    let current = {
        let id = id.clone();
        let fallback = entry.clone();
        Memo::new(move |_| list.get().entries.into_iter().find(|e| e.id == id).unwrap_or_else(|| fallback.clone()))
    };
    // Löschen, Ban und Unban, die schon auf einer anderen Taste liegen, in der Auswahl ausgrauen.
    let taken = {
        let id = id.clone();
        Memo::new(move |_| {
            list.get()
                .entries
                .iter()
                .filter(|e| e.id != id && e.key.is_some() && SINGLE_ACTIONS.contains(&e.action.as_str()))
                .map(|e| e.action.clone())
                .collect::<BTreeSet<_>>()
        })
    };
    let entries = Signal::derive(move || {
        let own = current.get().action;
        ALL_ACTIONS.iter().map(|a| SelectEntry::item(*a, action_label(a)).disabled(*a != own && taken.get().contains(*a))).collect::<Vec<_>>()
    });
    let error_text = {
        let id = id.clone();
        move || error.get().filter(|(e, _)| *e == id).map(|(_, text)| view! { <p class="text-xs text-destructive">{text}</p> })
    };
    let remove_id = id.clone();
    view! {
        <div class="flex flex-col gap-2 py-3">
            <div class="flex flex-wrap items-center gap-3">
                <KeyCapture
                    value=Signal::derive(move || current.get().key)
                    disabled=off
                    label=Signal::derive(move || multiyap_core::tf!("Taste für {}", "Key for {}", action_label(&current.get().action))).get_untracked()
                    on_change=Callback::new(move |key: String| {
                        let mut next = current.get_untracked();
                        next.key = Some(key);
                        apply(next)
                    })
                />
                <Select
                    id=format!("shortcut-action-{id}")
                    class="w-32"
                    aria_label=tr("Aktion", "Action")
                    value=Signal::derive(move || current.get().action)
                    entries=entries
                    disabled=off
                    on_change=Callback::new(move |action: String| {
                        let old = current.get_untracked();
                        let duration = (action == "timeout").then(|| old.duration_seconds.unwrap_or(600));
                        let reason = if takes_reason(&action) { old.reason.clone() } else { None };
                        apply(ShortcutEntry { id: old.id, key: old.key, action, duration_seconds: duration, reason });
                    })
                />
                {move || {
                    reset.track();
                    (current.get().action == "timeout").then(|| view! {
                        <DurationInput
                            seconds=Signal::derive(move || current.get().duration_seconds.unwrap_or(600))
                            on_change=Callback::new(move |seconds: i64| {
                                let mut next = current.get_untracked();
                                next.duration_seconds = Some(seconds);
                                apply(next);
                            })
                        />
                    })
                }}
                <Button
                    variant="ghost"
                    size="icon-sm"
                    class="ml-auto"
                    disabled=off
                    aria_label=tr("Tastenkürzel entfernen", "Remove keyboard shortcut")
                    on_click=Callback::new(move |_| save(list.get_untracked().entries.into_iter().filter(|e| e.id != remove_id).collect()))
                >
                    {icons::trash("")}
                </Button>
            </div>
            {move || {
                let e = current.get();
                takes_reason(&e.action).then(|| view! {
                    <CommitInput
                        value=Signal::derive(move || current.get().reason.unwrap_or_default())
                        max_length=MAX_REASON_LENGTH
                        disabled=off
                        aria_label=tr("Grund", "Reason")
                        placeholder=if e.action == "warn" { {tr("Grund (nötig)", "Reason (required)")} } else { {tr("Grund (optional)", "Reason (optional)")} }
                        on_commit=Callback::new(move |value: String| {
                            let mut next = current.get_untracked();
                            let trimmed = value.trim().to_string();
                            next.reason = (!trimmed.is_empty()).then_some(trimmed);
                            apply(next)
                        })
                    />
                })
            }}
            {move || {
                let e = current.get();
                (e.action == "warn" && e.reason.as_deref().is_none_or(|r| r.trim().is_empty())).then(|| view! {
                    <p class="text-xs text-destructive">{tr("Verwarnen braucht einen Grund, sonst passiert bei dieser Taste nichts.", "Warning needs a reason, otherwise this key does nothing.")}</p>
                })
            }}
            {error_text}
        </div>
    }
}
