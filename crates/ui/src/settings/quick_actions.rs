//! Schnellaktionen (quick-actions-section.tsx) und Nutzerkarte (usercard-section.tsx, Abschnitt 9). Änderungen gelten
//! sofort.

use multiyap_core::i18n::tr;
use multiyap_core::{hotkeys, shortcuts};
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::model::{action_label, QuickButton, ReasonDialog, ReasonTemplate, ReasonTemplates, ALL_ACTIONS, MAX_REASON_LENGTH};
use super::widgets::DurationInput;
use crate::bridge::{random_id, save_setting, use_setting};
use crate::components::input::{merge_width, INPUT_CLASS};
use crate::components::{icons, Button, CommitInput, Select, SelectEntry, SettingGroup, SettingRow, Switch};

const QUICK_BUTTONS: &str = "moderation.quickButtons";
const IN_CHAT: &str = "moderation.quickButtonsInChat";
const REASON_DIALOG: &str = "moderation.reasonDialog";
const TEMPLATES: &str = "moderation.reasonTemplates";
const NO_REASON_HOTKEY: &str = "moderation.noReasonHotkey";
const CARD_ACTIONS: &str = "usercard.actions";

fn is_hotkey(key: &str) -> bool {
    key.len() == 1 && key.chars().all(|c| c.is_ascii_digit() || c.is_ascii_lowercase())
}

/// Dauern für neue Timeout-Buttons, die erste freie wird genommen: 10 Minuten, dann gängige Stufen.
const FREE_TIMEOUTS: [i64; 10] = [600, 60, 300, 1800, 3600, 7200, 21_600, 43_200, 86_400, 604_800];

/// Liste von Mod-Knöpfen bearbeiten: Reihenfolge, Timeout-Dauer, fester Grund (wenn das Grund-Fenster für die Aktion
/// aus ist), hinzufügen und entfernen. Die Einträge sind direkte Kinder, damit die Suche sie einzeln findet.
#[component]
fn ButtonListEditor(setting: &'static str, actions: &'static [&'static str]) -> impl IntoView {
    let buttons = use_setting(setting, Vec::<QuickButton>::new());
    let dialog = use_setting(REASON_DIALOG, ReasonDialog { timeout: true, ban: true });
    let new_action = RwSignal::new("timeout".to_string());
    let save = move |next: Vec<QuickButton>| save_setting(setting, &next);
    let update = move |id: String, change: Box<dyn Fn(&mut QuickButton)>| {
        let mut next = buttons.get_untracked();
        if let Some(button) = next.iter_mut().find(|b| b.id == id) {
            change(button);
        }
        save(next);
    };
    let move_by = move |id: String, delta: i32| {
        let mut next = buttons.get_untracked();
        let Some(index) = next.iter().position(|b| b.id == id) else { return };
        let target = index as i32 + delta;
        if target < 0 || target >= next.len() as i32 {
            return;
        }
        next.swap(index, target as usize);
        save(next);
    };
    // Ziehen mit der Maus: nur am Griff, Ablegen vor oder hinter einem Eintrag.
    let armed = RwSignal::new(None::<String>);
    let dragged = RwSignal::new(None::<String>);
    let over = RwSignal::new(None::<(String, bool)>);
    let move_to = move |source: String, target: String, after: bool| {
        let mut next = buttons.get_untracked();
        let Some(from) = next.iter().position(|b| b.id == source) else { return };
        let item = next.remove(from);
        // Auf sich selbst abgelegt: nichts ändern.
        let Some(to) = next.iter().position(|b| b.id == target) else { return };
        next.insert(if after { to + 1 } else { to }, item);
        save(next);
    };
    let end_drag = move || {
        armed.set(None);
        dragged.set(None);
        over.set(None);
    };
    let ids = Memo::new(move |_| buttons.get().into_iter().map(|b| b.id).collect::<Vec<_>>());
    // Jeden Button nur einmal: Aktionen außer Timeout höchstens einmal, Timeouts je Dauer höchstens einmal.
    let used_actions = Memo::new(move |_| buttons.with(|list| list.iter().filter(|b| b.action != "timeout").map(|b| b.action.clone()).collect::<Vec<_>>()));
    let free_timeout = Memo::new(move |_| {
        buttons.with(|list| {
            let taken = list.iter().filter(|b| b.action == "timeout").filter_map(|b| b.duration_seconds).collect::<Vec<_>>();
            FREE_TIMEOUTS.iter().copied().find(|d| !taken.contains(d))
        })
    });
    // Ist die gewählte Aktion schon vergeben, die erste freie vorschlagen.
    Effect::new(move |_| {
        let used = used_actions.get();
        let current = new_action.get_untracked();
        if current != "timeout" && used.contains(&current) {
            if let Some(free) = actions.iter().find(|a| **a == "timeout" || !used.iter().any(|u| u == *a)) {
                new_action.set((*free).to_string());
            }
        }
    });

    view! {
        <For each=move || ids.get() key=|id| id.clone() let:id>
            {
                let id_for = id.clone();
                let button = Memo::new(move |_| buttons.get().into_iter().find(|b| b.id == id_for));
                let position = {
                    let id = id.clone();
                    Memo::new(move |_| (ids.get().iter().position(|i| *i == id).unwrap_or(0), ids.get().len()))
                };
                let (up, down, remove, duration_id, reason_id) = (id.clone(), id.clone(), id.clone(), id.clone(), id.clone());
                let (row_class_id, draggable_id, start_id, over_id, grip_id) = (id.clone(), id.clone(), id.clone(), id.clone(), id.clone());
                view! {
                    <div
                        class=move || {
                            let base = "flex flex-wrap items-start gap-3 py-3";
                            match over.get() {
                                Some((target, after)) if target == row_class_id && dragged.with(|d| d.as_ref() != Some(&target)) => {
                                    if after { format!("{base} shadow-[inset_0_-2px_0_0_var(--color-primary)]") } else { format!("{base} shadow-[inset_0_2px_0_0_var(--color-primary)]") }
                                }
                                _ if dragged.with(|d| d.as_ref() == Some(&row_class_id)) => format!("{base} opacity-50"),
                                _ => base.to_string(),
                            }
                        }
                        draggable=move || (armed.with(|a| a.as_ref() == Some(&draggable_id))).to_string()
                        on:dragstart=move |event: web_sys::DragEvent| {
                            if let Some(data) = event.data_transfer() {
                                let _ = data.set_data("text/plain", &start_id);
                                data.set_effect_allowed("move");
                            }
                            dragged.set(Some(start_id.clone()));
                        }
                        on:dragend=move |_| end_drag()
                        on:dragover=move |event: web_sys::DragEvent| {
                            if dragged.with_untracked(Option::is_none) {
                                return;
                            }
                            event.prevent_default();
                            let after = event
                                .current_target()
                                .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                                .map(|row| {
                                    let rect = row.get_bounding_client_rect();
                                    f64::from(event.client_y()) > rect.top() + rect.height() / 2.0
                                })
                                .unwrap_or(false);
                            let next = Some((over_id.clone(), after));
                            if over.with_untracked(|o| *o != next) {
                                over.set(next);
                            }
                        }
                        on:drop=move |event: web_sys::DragEvent| {
                            event.prevent_default();
                            if let (Some(source), Some((target, after))) = (dragged.get_untracked(), over.get_untracked()) {
                                move_to(source, target, after);
                            }
                            end_drag();
                        }
                    >
                        <span
                            class="flex h-8 cursor-grab items-center text-muted-foreground active:cursor-grabbing"
                            title=tr("Ziehen zum Verschieben", "Drag to move")
                            aria-hidden="true"
                            on:pointerdown=move |_| armed.set(Some(grip_id.clone()))
                            on:pointerup=move |_| armed.set(None)
                        >
                            {icons::grip_vertical("size-4")}
                        </span>
                        <span class="w-24 pt-1.5 text-sm font-medium">
                            {move || {
                                button.get().map(|b| {
                                    let suffix = match (b.action.as_str(), b.duration_seconds) {
                                        ("timeout", Some(d)) => format!(" {}", hotkeys::format_duration(d)),
                                        _ => String::new(),
                                    };
                                    format!("{}{suffix}", action_label(&b.action))
                                })
                            }}
                        </span>
                        {move || {
                            (button.get().is_some_and(|b| b.action == "timeout")).then(|| {
                                let duration_id = duration_id.clone();
                                let own = duration_id.clone();
                                let taken = Signal::derive(move || {
                                    buttons.with(|list| list.iter().filter(|b| b.action == "timeout" && b.id != own).filter_map(|b| b.duration_seconds).collect::<Vec<_>>())
                                });
                                view! {
                                    <DurationInput
                                        taken=taken
                                        seconds=Signal::derive(move || button.get().and_then(|b| b.duration_seconds).unwrap_or(600))
                                        on_change=Callback::new(move |seconds: i64| update(duration_id.clone(), Box::new(move |b| b.duration_seconds = Some(seconds))))
                                    />
                                }
                            })
                        }}
                        {move || {
                            let fixed = button.get().is_some_and(|b| {
                                let d = dialog.get();
                                (b.action == "timeout" && !d.timeout) || (b.action == "ban" && !d.ban)
                            });
                            let reason_id = reason_id.clone();
                            fixed.then(|| view! {
                                <CommitInput
                                    class="w-56"
                                    value=Signal::derive(move || button.get().and_then(|b| b.fixed_reason).unwrap_or_default())
                                    max_length=MAX_REASON_LENGTH
                                    placeholder=tr("Fester Grund (optional)", "Fixed reason (optional)")
                                    aria_label=tr("Fester Grund", "Fixed reason")
                                    on_commit=Callback::new(move |value: String| {
                                        let trimmed = value.trim().to_string();
                                        update(reason_id.clone(), Box::new(move |b| b.fixed_reason = (!trimmed.is_empty()).then(|| trimmed.clone())));
                                        true
                                    })
                                />
                            })
                        }}
                        <div class="ml-auto flex gap-1">
                            <Button variant="ghost" size="icon-sm" aria_label=tr("Nach oben", "Move up") disabled=Signal::derive(move || position.get().0 == 0) on_click=Callback::new(move |_| move_by(up.clone(), -1))>
                                {icons::arrow_up("")}
                            </Button>
                            <Button variant="ghost" size="icon-sm" aria_label=tr("Nach unten", "Move down") disabled=Signal::derive(move || { let (i, n) = position.get(); i + 1 >= n }) on_click=Callback::new(move |_| move_by(down.clone(), 1))>
                                {icons::arrow_down("")}
                            </Button>
                            <Button variant="ghost" size="icon-sm" aria_label=tr("Entfernen", "Remove") on_click=Callback::new(move |_| save(buttons.get_untracked().into_iter().filter(|b| b.id != remove).collect()))>
                                {icons::trash("")}
                            </Button>
                        </div>
                    </div>
                }
            }
        </For>
        <div class="flex items-center gap-2 py-3">
            <Select
                id=format!("new-action-{setting}")
                class="w-40"
                aria_label=tr("Aktion für neuen Button", "Action for new button")
                value=new_action
                entries=Signal::derive(move || {
                    let used = used_actions.get();
                    actions.iter().map(|a| SelectEntry::item(*a, action_label(a)).disabled(used.iter().any(|u| u == *a))).collect::<Vec<_>>()
                })
                on_change=Callback::new(move |action: String| new_action.set(action))
            />
            <Button
                variant="secondary"
                size="sm"
                disabled=Signal::derive(move || {
                    let action = new_action.get();
                    buttons.with(Vec::len) >= 20 || if action == "timeout" { free_timeout.get().is_none() } else { used_actions.with(|u| u.contains(&action)) }
                })
                on_click=Callback::new(move |_| {
                    let action = new_action.get_untracked();
                    let duration = if action == "timeout" {
                        let Some(free) = free_timeout.get_untracked() else { return };
                        Some(free)
                    } else if used_actions.with_untracked(|u| u.contains(&action)) {
                        return;
                    } else {
                        None
                    };
                    let mut next = buttons.get_untracked();
                    next.push(QuickButton { id: random_id(), action, duration_seconds: duration, fixed_reason: None });
                    save(next);
                })
            >
                {icons::plus("")}
                {tr("Button hinzufügen", "Add button")}
            </Button>
        </div>
    }
}

#[component]
fn QuickButtonsEditor() -> impl IntoView {
    let in_chat = use_setting(IN_CHAT, true);
    view! {
        <SettingGroup title=tr("Schnellbuttons", "Quick buttons") description=tr("Vor dem Namen, in dieser Reihenfolge.", "Before the name, in this order.")>
            <SettingRow label=tr("Im Chat anzeigen", "Show in chat") description=tr("Aus: keine Schnellbuttons, auch in Mod-Kanälen.", "Off: no quick buttons, also in mod channels.") html_for="quick-buttons-in-chat">
                <Switch id="quick-buttons-in-chat" checked=in_chat on_change=Callback::new(|checked: bool| save_setting(IN_CHAT, &checked)) />
            </SettingRow>
            <ButtonListEditor setting=QUICK_BUTTONS actions=&ALL_ACTIONS />
        </SettingGroup>
    }
}

#[component]
fn ReasonDialogEditor() -> impl IntoView {
    let dialog = use_setting(REASON_DIALOG, ReasonDialog { timeout: true, ban: true });
    let no_reason = use_setting(NO_REASON_HOTKEY, "0".to_string());
    let templates = use_setting(TEMPLATES, ReasonTemplates::default());
    let hotkey_error = RwSignal::new(None::<&'static str>);
    let save_dialog = move |change: fn(&mut ReasonDialog, bool)| {
        Callback::new(move |checked: bool| {
            let mut next = dialog.get_untracked();
            change(&mut next, checked);
            save_setting(REASON_DIALOG, &next);
        })
    };
    view! {
        <SettingGroup title=tr("Grund-Fenster", "Reason window") description=tr("Fenster für den Grund. Verwarnen hat es immer.", "Window for the reason. Warning always has it.")>
            <SettingRow label=tr("Bei Timeout", "On timeout") description=tr("Aus: sofort ausführen.", "Off: run immediately.") html_for="dialog-timeout">
                <Switch id="dialog-timeout" checked=Signal::derive(move || dialog.get().timeout) on_change=save_dialog(|d, v| d.timeout = v) />
            </SettingRow>
            <SettingRow label=tr("Bei Ban", "On ban") description=tr("Aus: sofort ausführen.", "Off: run immediately.") html_for="dialog-ban">
                <Switch id="dialog-ban" checked=Signal::derive(move || dialog.get().ban) on_change=save_dialog(|d, v| d.ban = v) />
            </SettingRow>
            <SettingRow label=tr("Hotkey für „Kein Grund“", "Hotkey for “No reason”") description=tr("Ein Buchstabe oder eine Ziffer (Timeout und Ban).", "A letter or a digit (timeout and ban).") html_for="no-reason-hotkey">
                <div class="flex flex-col items-end gap-1">
                    <CommitInput
                        id="no-reason-hotkey"
                        class="w-14 text-center"
                        max_length=1
                        value=no_reason
                        on_commit=Callback::new(move |value: String| {
                            let key = value.to_lowercase();
                            let t = templates.get_untracked();
                            let clash = t.timeout.iter().chain(t.ban.iter()).any(|t| t.hotkey.as_deref() == Some(key.as_str()));
                            if !is_hotkey(&key) || clash {
                                hotkey_error.set(Some(if clash { {tr("Schon von einer Vorlage belegt.", "Already used by a template.")} } else { {tr("Ein Buchstabe oder eine Ziffer.", "A letter or a digit.")} }));
                                return false;
                            }
                            hotkey_error.set(None);
                            save_setting(NO_REASON_HOTKEY, &key);
                            true
                        })
                    />
                    {move || hotkey_error.get().map(|text| view! { <p class="text-xs text-destructive">{text}</p> })}
                </div>
            </SettingRow>
        </SettingGroup>
    }
}

fn reason_actions() -> [(&'static str, &'static str); 3] {
    [("warn", tr("Verwarnen", "Warn")), ("timeout", "Timeout"), ("ban", "Ban")]
}

#[component]
fn TemplatesEditor() -> impl IntoView {
    let templates = use_setting(TEMPLATES, ReasonTemplates::default());
    let no_reason = use_setting(NO_REASON_HOTKEY, "0".to_string());
    let action = RwSignal::new("timeout");
    let new_text = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let list = Memo::new(move |_| templates.get().list(action.get()).clone());

    let save = move |next: Vec<ReasonTemplate>| {
        let mut all = templates.get_untracked();
        *all.list_mut(action.get_untracked()) = next;
        save_setting(TEMPLATES, &all);
    };
    let set_hotkey = move |id: String, value: String| -> bool {
        let key = value.to_lowercase();
        let current = list.get_untracked();
        if key.is_empty() {
            save(current.into_iter().map(|t| if t.id == id { ReasonTemplate { hotkey: None, ..t } } else { t }).collect());
            return true;
        }
        if !is_hotkey(&key) {
            error.set(Some(tr("Hotkey: ein Buchstabe oder eine Ziffer.", "Hotkey: a letter or a digit.").into()));
            return false;
        }
        if current.iter().any(|t| t.id != id && t.hotkey.as_deref() == Some(key.as_str())) || (action.get_untracked() != "warn" && key == no_reason.get_untracked()) {
            error.set(Some(multiyap_core::tf!("Hotkey „{key}“ ist schon belegt.", "Hotkey “{key}” is already used.")));
            return false;
        }
        error.set(None);
        save(current.into_iter().map(|t| if t.id == id { ReasonTemplate { hotkey: Some(key.clone()), ..t } } else { t }).collect());
        true
    };
    let add = move || {
        let text = new_text.get_untracked().trim().to_string();
        if text.is_empty() {
            return;
        }
        let mut next = list.get_untracked();
        let hotkey = shortcuts::next_free_hotkey(next.iter().map(|t| t.hotkey.as_deref()), &no_reason.get_untracked());
        next.push(ReasonTemplate { id: random_id(), text, hotkey });
        save(next);
        new_text.set(String::new());
    };

    view! {
        <SettingGroup title=tr("Vorlagen", "Templates") description=tr("Eigene Liste je Aktion, Klick oder Taste schickt ab.", "A list per action, a click or key sends it.")>
            <div class="flex gap-2 py-3">
                {reason_actions().into_iter().map(|(id, label)| {
                    let id: &'static str = id;
                    view! {
                        <Button size="sm" variant=Signal::derive(move || if action.get() == id { "secondary" } else { "outline" })
                            on_click=Callback::new(move |_| action.set(id))>
                            {label}
                        </Button>
                    }
                    .into_any()
                }).collect_view()}
            </div>
            {move || list.get().is_empty().then(|| view! { <p class="py-3 text-sm text-muted-foreground">{tr("Noch keine Vorlagen.", "No templates yet.")}</p> })}
            <For each=move || list.get() key=|t| (t.id.clone(), t.text.clone(), t.hotkey.clone()) let:template>
                {
                    let (text_id, key_id, remove_id) = (template.id.clone(), template.id.clone(), template.id.clone());
                    view! {
                        <div class="flex items-center gap-2 py-2">
                            <CommitInput
                                class="flex-1"
                                value=template.text.clone()
                                max_length=MAX_REASON_LENGTH
                                aria_label=tr("Text der Vorlage", "Text of the template")
                                on_commit=Callback::new(move |value: String| {
                                    let text = value.trim().to_string();
                                    if text.is_empty() {
                                        return false;
                                    }
                                    save(list.get_untracked().into_iter().map(|t| if t.id == text_id { ReasonTemplate { text: text.clone(), ..t } } else { t }).collect());
                                    true
                                })
                            />
                            <CommitInput
                                class="w-14 text-center"
                                max_length=1
                                value=template.hotkey.clone().unwrap_or_default()
                                aria_label="Hotkey"
                                on_commit=Callback::new(move |value: String| set_hotkey(key_id.clone(), value))
                            />
                            <Button variant="ghost" size="icon-sm" aria_label=tr("Vorlage entfernen", "Remove template")
                                on_click=Callback::new(move |_| save(list.get_untracked().into_iter().filter(|t| t.id != remove_id).collect()))>
                                {icons::trash("")}
                            </Button>
                        </div>
                    }
                }
            </For>
            {move || error.get().map(|text| view! { <p class="py-1 text-xs text-destructive">{text}</p> })}
            <form class="flex items-center gap-2 py-3" on:submit=move |event| { event.prevent_default(); add(); }>
                <input
                    data-slot="input"
                    class=merge_width(INPUT_CLASS, "flex-1")
                    maxlength=MAX_REASON_LENGTH
                    placeholder=tr("Neue Vorlage", "New template")
                    aria-label=tr("Neue Vorlage", "New template")
                    prop:value=move || new_text.get()
                    on:input=move |event| new_text.set(event_target_value(&event))
                />
                <Button submit=true variant="secondary" size="sm" disabled=Signal::derive(move || new_text.get().trim().is_empty() || list.get().len() >= 50)>
                    {icons::plus("")}
                    {tr("Hinzufügen", "Add")}
                </Button>
            </form>
        </SettingGroup>
    }
}

#[component]
pub fn QuickActionsSection() -> impl IntoView {
    view! {
        <div class="flex flex-col gap-8">
            <QuickButtonsEditor />
            <ReasonDialogEditor />
            <TemplatesEditor />
        </div>
    }
}

/// Nutzerkarte: eigene Aktionsliste, unabhängig von den Schnellbuttons. Ohne Löschen: die Karte gehört zu einer Person,
/// nicht zu einer Nachricht.
#[component]
pub fn UserCardSection() -> impl IntoView {
    view! {
        <SettingGroup title=tr("Aktionen", "Actions") description=tr("Knöpfe in der Nutzerkarte, in dieser Reihenfolge, nur in Mod-Kanälen.", "Buttons in the user card, in this order, only in mod channels.")>
            <ButtonListEditor setting=CARD_ACTIONS actions=&["warn", "timeout", "ban", "unban"] />
        </SettingGroup>
    }
}
