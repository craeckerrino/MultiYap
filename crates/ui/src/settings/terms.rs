//! Filter (terms-section.tsx): Geltungsbereich und frei anlegbare Kategorien mit Farbe, Wörtern und Phrasen, dazu die
//! Ausnahmen für Benutzer (exceptions-group.tsx, Abschnitt 10.2).

use multiyap_core::i18n::tr;
use multiyap_core::terms::parse_terms;
use leptos::prelude::*;

use leptos::task::spawn_local;
use serde::Deserialize;

use super::model::{term_swatch, TermCategory, TermException, term_colors};
use crate::bridge::{random_id, save_setting, toast, update_setting, use_setting};
use crate::components::input::{merge_width, INPUT_CLASS, TEXTAREA_CLASS};
use crate::components::overlay::{DropdownMenu, MenuEntry};
use crate::components::{icons, Button, CommitInput, Select, SelectEntry, SettingGroup, SettingRow, Switch};

const CATEGORIES: &str = "highlights.categories";
const SCOPE: &str = "highlights.scope";

fn scope_labels() -> [(&'static str, &'static str); 3] {
    [("all", tr("In allen Kanälen", "In all channels")), ("mod", tr("Nur in Mod-Kanälen", "Only in mod channels")), ("off", tr("Aus", "Off"))]
}

/// Wörter und Phrasen, eine pro Zeile; gespeichert beim Verlassen.
#[component]
fn TermsField(category: Memo<Option<TermCategory>>, on_save: Callback<Vec<String>>) -> impl IntoView {
    let joined = move || category.get().map(|c| c.terms.join("\n")).unwrap_or_default();
    let draft = RwSignal::new(joined());
    Effect::new(move |_| draft.set(joined()));
    let name = move || category.get().map(|c| c.name).unwrap_or_default();
    view! {
        <textarea
            data-slot="textarea"
            class=TEXTAREA_CLASS
            rows="3"
            placeholder=tr("Ein Wort oder eine Phrase pro Zeile", "One word or phrase per line")
            aria-label=move || multiyap_core::tf!("Wörter und Phrasen der Kategorie {}", "Words and phrases of the category {}", name())
            prop:value=move || draft.get()
            on:input=move |event| draft.set(event_target_value(&event))
            on:blur=move |_| {
                let terms = parse_terms(&draft.get_untracked());
                if terms.join("\n") != joined() { on_save.run(terms) } else { draft.set(joined()) }
            }
        ></textarea>
    }
}

/// Kleiner Schalter mit Text daneben (Klick auf den Text schaltet nicht).
#[component]
fn InlineSwitch(id: String, label: &'static str, checked: Signal<bool>, on_change: Callback<bool>) -> impl IntoView {
    view! {
        <div class="flex items-center gap-2">
            <Switch id=id.clone() checked=checked on_change=on_change />
            <label for=id class="text-sm" on:click=|event| event.prevent_default()>{label}</label>
        </div>
    }
}

#[component]
pub fn TermCategoriesSection() -> impl IntoView {
    let categories = use_setting(CATEGORIES, Vec::<TermCategory>::new());
    let scope = use_setting(SCOPE, "mod".to_string());
    let save = move |next: Vec<TermCategory>| save_setting(CATEGORIES, &next);
    let update = move |id: String, change: Box<dyn Fn(&mut TermCategory)>| {
        let mut next = categories.get_untracked();
        if let Some(category) = next.iter_mut().find(|c| c.id == id) {
            change(category);
        }
        save(next);
    };
    let move_by = move |id: String, delta: i32| {
        let mut next = categories.get_untracked();
        let Some(index) = next.iter().position(|c| c.id == id) else { return };
        let target = index as i32 + delta;
        if target < 0 || target >= next.len() as i32 {
            return;
        }
        next.swap(index, target as usize);
        save(next);
    };
    let add = move || {
        let mut next = categories.get_untracked();
        let used: Vec<String> = next.iter().map(|c| c.color.clone()).collect();
        let color = term_colors().iter().map(|(c, _)| *c).find(|c| !used.iter().any(|u| u == c)).unwrap_or(term_colors()[next.len() % term_colors().len()].0);
        next.push(TermCategory { id: random_id(), name: tr("Neue Kategorie", "New category").into(), color: color.into(), terms: Vec::new(), regex: false, scroll_marker: true });
        save(next);
    };
    let ids = Memo::new(move |_| categories.get().into_iter().map(|c| c.id).collect::<Vec<_>>());
    let mark_first_message = use_setting(MARK_FIRST_MESSAGE, true);
    let mark_links = use_setting(MARK_LINKS, true);
    let color_entries = term_colors().iter().map(|(c, l)| SelectEntry::item(*c, *l).swatch(term_swatch(c))).collect::<Vec<_>>();

    view! {
        <div class="flex flex-col gap-8">
            <SettingGroup title=tr("Markierungen", "Markers")>
                <SettingRow label=tr("Filter anwenden", "Apply filters") html_for="highlights-scope">
                    <Select
                        id="highlights-scope"
                        class="w-56"
                        value=scope
                        entries=Signal::stored(scope_labels().iter().map(|(v, l)| SelectEntry::item(*v, *l)).collect::<Vec<_>>())
                        on_change=Callback::new(|value: String| save_setting(SCOPE, &value))
                    />
                </SettingRow>
                <SettingRow label=tr("Erste Nachrichten markieren", "Mark first messages") description=tr("Hinterlegung und Hinweis „Erste Nachricht“.", "Background and the note “First message”.") html_for="mark-first-message">
                    <Switch id="mark-first-message" checked=mark_first_message on_change=Callback::new(|checked: bool| save_setting(MARK_FIRST_MESSAGE, &checked)) />
                </SettingRow>
                <SettingRow label=tr("Nachrichten mit Links markieren", "Mark messages with links") description=tr("Leichte Hinterlegung.", "Light background.") html_for="mark-links">
                    <Switch id="mark-links" checked=mark_links on_change=Callback::new(|checked: bool| save_setting(MARK_LINKS, &checked)) />
                </SettingRow>
            </SettingGroup>
            <SettingGroup
                title=tr("Kategorien", "Categories")
                description=tr("Treffer werden in der Farbe der Kategorie hinterlegt, ganze Wörter. „Regex“ erkennt auch Varianten wie spaaam, s.p.a.m, sp4m oder spamming. Die erste passende Kategorie gilt.", "Matches get the category’s color as background, whole words. “Regex” also finds variants like spaaam, s.p.a.m, sp4m or spamming. The first matching category applies.")
            >
                {move || ids.get().is_empty().then(|| view! { <p class="py-3 text-sm text-muted-foreground">{tr("Noch keine Kategorien.", "No categories yet.")}</p> })}
                <For each=move || ids.get() key=|id| id.clone() let:id>
                    {
                        let lookup = id.clone();
                        let category = Memo::new(move |_| categories.get().into_iter().find(|c| c.id == lookup));
                        let position = {
                            let id = id.clone();
                            Memo::new(move |_| (ids.get().iter().position(|i| *i == id).unwrap_or(0), ids.get().len()))
                        };
                        let field = |f: fn(&TermCategory) -> bool| Signal::derive(move || category.get().is_some_and(|c| f(&c)));
                        let (name_id, color_id, marker_id, regex_id, terms_id, up, down, remove) =
                            (id.clone(), id.clone(), id.clone(), id.clone(), id.clone(), id.clone(), id.clone(), id.clone());
                        view! {
                            <div class="flex flex-col gap-2 py-4">
                                <div class="flex flex-wrap items-center gap-2">
                                    <span class=move || format!("size-3 shrink-0 rounded-full {}", term_swatch(&category.get().map(|c| c.color).unwrap_or_default())) aria-hidden="true"></span>
                                    <CommitInput
                                        class="w-48"
                                        value=Signal::derive(move || category.get().map(|c| c.name).unwrap_or_default())
                                        max_length=40
                                        aria_label=tr("Name der Kategorie", "Name of the category")
                                        on_commit=Callback::new(move |value: String| {
                                            let name = value.trim().to_string();
                                            if name.is_empty() {
                                                return false;
                                            }
                                            update(name_id.clone(), Box::new(move |c| c.name = name.clone()));
                                            true
                                        })
                                    />
                                    <Select
                                        id=format!("color-{id}")
                                        class="w-32"
                                        aria_label=tr("Farbe", "Color")
                                        value=Signal::derive(move || category.get().map(|c| c.color).unwrap_or_default())
                                        entries=Signal::stored(color_entries.clone())
                                        on_change=Callback::new(move |color: String| update(color_id.clone(), Box::new(move |c| c.color = color.clone())))
                                    />
                                    <InlineSwitch
                                        id=format!("marker-{id}")
                                        label=tr("Striche auf der Scroll-Leiste", "Marks on the scroll bar")
                                        checked=field(|c| c.scroll_marker)
                                        on_change=Callback::new(move |checked: bool| update(marker_id.clone(), Box::new(move |c| c.scroll_marker = checked)))
                                    />
                                    <InlineSwitch
                                        id=format!("regex-{id}")
                                        label="Regex"
                                        checked=field(|c| c.regex)
                                        on_change=Callback::new(move |checked: bool| update(regex_id.clone(), Box::new(move |c| c.regex = checked)))
                                    />
                                    <super::notifications::TermSoundPicker id=id.clone() />
                                    <div class="ml-auto flex gap-1">
                                        <Button variant="ghost" size="icon-sm" aria_label=tr("Nach oben", "Move up") disabled=Signal::derive(move || position.get().0 == 0) on_click=Callback::new(move |_| move_by(up.clone(), -1))>
                                            {icons::arrow_up("")}
                                        </Button>
                                        <Button variant="ghost" size="icon-sm" aria_label=tr("Nach unten", "Move down") disabled=Signal::derive(move || { let (i, n) = position.get(); i + 1 >= n }) on_click=Callback::new(move |_| move_by(down.clone(), 1))>
                                            {icons::arrow_down("")}
                                        </Button>
                                        <Button variant="ghost" size="icon-sm" aria_label=tr("Kategorie entfernen", "Remove category") on_click=Callback::new(move |_| {
                                            let next: Vec<TermCategory> = categories.get_untracked().into_iter().filter(|c| c.id != remove).collect();
                                            let keep = next.iter().map(|c| c.id.clone()).collect();
                                            save(next);
                                            super::notifications::prune_sound_choices("term:", keep);
                                        })>
                                            {icons::trash("")}
                                        </Button>
                                    </div>
                                </div>
                                <TermsField category=category on_save=Callback::new(move |terms: Vec<String>| update(terms_id.clone(), Box::new(move |c| c.terms = terms.clone()))) />
                            </div>
                        }
                    }
                </For>
                <div class="py-3">
                    <Button variant="secondary" size="sm" disabled=Signal::derive(move || ids.get().len() >= 30) on_click=Callback::new(move |_| add())>
                        {icons::plus("")}
                        {tr("Kategorie hinzufügen", "Add category")}
                    </Button>
                </div>
            </SettingGroup>
            <ExceptionsGroup />
            <super::chat_behavior::SpamGroup />
        </div>
    }
}

const EXCEPTIONS: &str = "highlights.exceptions";
const MARK_FIRST_MESSAGE: &str = "chat.markFirstMessage";
const MARK_LINKS: &str = "chat.markLinks";
const EXEMPT_BROADCASTER: &str = "highlights.exemptBroadcaster";
const EXEMPT_MODS: &str = "highlights.exemptMods";

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExceptionUser {
    pub user_id: String,
    pub login: String,
    pub display_name: String,
}

/// Legt eine Ausnahme an, ändert sie oder entfernt sie. Bleibt keine Kategorie übrig, wird sie entfernt. Liest den
/// aktuellen Stand direkt vor dem Schreiben (Reihenfolge bleibt erhalten).
pub(crate) fn save_exception(user: ExceptionUser, selection: Option<(bool, Vec<String>)>) {
    update_setting::<Vec<TermException>>(EXCEPTIONS, move |current| apply_exception(current, &user, selection.clone()));
}

/// Neue Liste nach dem Anlegen, Ändern oder Entfernen einer Ausnahme (Reihenfolge bleibt erhalten).
pub(crate) fn apply_exception(current: Vec<TermException>, user: &ExceptionUser, selection: Option<(bool, Vec<String>)>) -> Vec<TermException> {
    let remove = selection.as_ref().is_none_or(|(all, ids)| !*all && ids.is_empty());
    if remove {
        return current.into_iter().filter(|e| e.user_id != user.user_id).collect();
    }
    let (all, ids) = selection.unwrap_or_default();
    let entry = TermException {
        user_id: user.user_id.clone(),
        login: user.login.clone(),
        display_name: user.display_name.clone(),
        all_categories: all,
        category_ids: if all { Vec::new() } else { ids },
    };
    let mut next = current;
    match next.iter().position(|e| e.user_id == user.user_id) {
        Some(index) => next[index] = entry,
        None => next.push(entry),
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exception(user_id: &str) -> TermException {
        TermException { user_id: user_id.into(), login: user_id.into(), display_name: user_id.into(), all_categories: true, category_ids: Vec::new() }
    }

    #[test]
    fn anlegen_aendern_entfernen() {
        let user = ExceptionUser { user_id: "u1".into(), login: "neu".into(), display_name: "Neu".into() };
        let ids = |list: &[TermException]| list.iter().map(|e| e.user_id.clone()).collect::<Vec<_>>();
        let list = apply_exception(vec![exception("u0")], &user, Some((true, Vec::new())));
        assert_eq!(ids(&list), ["u0", "u1"]);
        let list = apply_exception(list, &user, Some((false, vec!["a".into()])));
        assert_eq!((list[1].all_categories, list[1].category_ids.clone(), list[1].login.as_str()), (false, vec!["a".to_string()], "neu"));
        // Leere Auswahl entfernt die Ausnahme.
        assert_eq!(ids(&apply_exception(list.clone(), &user, Some((false, Vec::new())))), ["u0"]);
        assert_eq!(ids(&apply_exception(list, &user, None)), ["u0"]);
    }
}

fn describe_exception(exception: &TermException, categories: &[TermCategory]) -> String {
    if exception.all_categories {
        return "Alle Kategorien".into();
    }
    let names: Vec<String> = categories.iter().filter(|c| exception.category_ids.contains(&c.id)).map(|c| c.name.clone()).collect();
    if names.is_empty() { "Keine Kategorie".into() } else { names.join(", ") }
}

fn user_of(exception: &TermException) -> ExceptionUser {
    ExceptionUser { user_id: exception.user_id.clone(), login: exception.login.clone(), display_name: exception.display_name.clone() }
}

/// Auswahl der Kategorien einer Ausnahme: „Alle Kategorien“ oder einzelne (exception-menu.tsx). Ohne bestehende
/// Ausnahme (Nutzerkarte) ist nichts gewählt und „Ausnahme entfernen“ fehlt.
pub(crate) fn exception_menu(user: ExceptionUser, exception: Option<TermException>, categories: Vec<TermCategory>) -> Vec<MenuEntry> {
    let exists = exception.is_some();
    let all = exception.as_ref().is_some_and(|e| e.all_categories);
    let selected: Vec<String> =
        if all { categories.iter().map(|c| c.id.clone()).collect() } else { exception.map(|e| e.category_ids).unwrap_or_default() };
    let mut entries = vec![MenuEntry::Label("Ausnehmen von".into())];
    entries.push(MenuEntry::Checkbox {
        label: "Alle Kategorien".into(),
        swatch: None,
        checked: all,
        on_toggle: Callback::new({
            let user = user.clone();
            move |checked: bool| save_exception(user.clone(), checked.then(|| (true, Vec::new())))
        }),
    });
    if !categories.is_empty() {
        entries.push(MenuEntry::Separator);
    }
    for category in &categories {
        let (user, selected, order) = (user.clone(), selected.clone(), categories.iter().map(|c| c.id.clone()).collect::<Vec<_>>());
        let id = category.id.clone();
        entries.push(MenuEntry::Checkbox {
            label: category.name.clone(),
            swatch: Some(term_swatch(&category.color).into()),
            checked: selected.contains(&category.id),
            on_toggle: Callback::new(move |checked: bool| {
                let mut next = selected.clone();
                if checked { next.push(id.clone()) } else { next.retain(|i| *i != id) }
                let ids = order.iter().filter(|i| next.contains(i)).cloned().collect();
                save_exception(user.clone(), Some((false, ids)));
            }),
        });
    }
    if exists {
        entries.push(MenuEntry::Separator);
        entries.push(MenuEntry::Item { label: "Ausnahme entfernen".into(), on_select: Callback::new(move |_| save_exception(user.clone(), None)) });
    }
    entries
}

/// Ausnahmen für Benutzer: vorerst global für alle Kanäle.
#[component]
fn ExceptionsGroup() -> impl IntoView {
    let categories = use_setting(CATEGORIES, Vec::<TermCategory>::new());
    let exceptions = use_setting(EXCEPTIONS, Vec::<TermException>::new());
    let exempt_broadcaster = use_setting(EXEMPT_BROADCASTER, true);
    let exempt_mods = use_setting(EXEMPT_MODS, false);
    let login = RwSignal::new(String::new());
    let busy = RwSignal::new(false);

    let add = move || {
        let name = login.get_untracked().trim().trim_start_matches('@').to_string();
        if name.is_empty() {
            return;
        }
        busy.set(true);
        spawn_local(async move {
            match crate::bridge::user_by_login(&name).await {
                Err(_) => toast("error", "Nutzer konnte nicht abgefragt werden"),
                Ok(value) => match value.and_then(|v| serde_json::from_value::<ExceptionUser>(v).ok()) {
                    None => toast("error", &format!("Nutzer „{name}“ nicht gefunden")),
                    Some(user) if exceptions.get_untracked().iter().any(|e| e.user_id == user.user_id) => {
                        toast("info", &format!("{} ist schon ausgenommen", user.display_name));
                        login.set(String::new());
                    }
                    Some(user) => {
                        save_exception(user, Some((true, Vec::new())));
                        login.set(String::new());
                    }
                },
            }
            busy.set(false);
        });
    };

    view! {
        <SettingGroup title="Ausnahmen für Benutzer" description="Keine Markierung durch die gewählten Kategorien, in allen Kanälen. Erwähnungen bleiben.">
            <SettingRow label="Broadcaster automatisch ausnehmen" html_for="exempt-broadcaster">
                <Switch id="exempt-broadcaster" checked=exempt_broadcaster on_change=Callback::new(|checked: bool| save_setting(EXEMPT_BROADCASTER, &checked)) />
            </SettingRow>
            <SettingRow label="Mods automatisch ausnehmen" html_for="exempt-mods">
                <Switch id="exempt-mods" checked=exempt_mods on_change=Callback::new(|checked: bool| save_setting(EXEMPT_MODS, &checked)) />
            </SettingRow>
            <For each=move || exceptions.get() key=|e| (e.user_id.clone(), e.all_categories, e.category_ids.clone()) let:exception>
                {
                    let menu_exception = exception.clone();
                    let user = user_of(&exception);
                    let summary = exception.clone();
                    view! {
                        <div class="flex items-center gap-2 py-3">
                            <div class="flex min-w-0 flex-1 flex-col">
                                <span class="truncate text-sm font-medium">{exception.display_name.clone()}</span>
                                <span class="truncate text-xs text-muted-foreground">{exception.login.clone()}</span>
                            </div>
                            <DropdownMenu
                                align_end=true
                                width_class="w-56"
                                trigger_class="max-w-56"
                                entries=Signal::derive(move || exception_menu(user_of(&menu_exception), Some(menu_exception.clone()), categories.get()))
                            >
                                <span class="truncate">{move || describe_exception(&summary, &categories.get())}</span>
                                {icons::chevron_down("")}
                            </DropdownMenu>
                            <Button variant="ghost" size="icon-sm" aria_label=format!("Ausnahme für {} entfernen", exception.display_name)
                                on_click=Callback::new(move |_| save_exception(user.clone(), None))>
                                {icons::trash("")}
                            </Button>
                        </div>
                    }
                }
            </For>
            <form class="flex items-center gap-2 py-3" on:submit=move |event| { event.prevent_default(); add(); }>
                <input data-slot="input" class=merge_width(INPUT_CLASS, "w-56") maxlength="50" placeholder="Login-Name" aria-label="Login-Name der Person"
                    prop:value=move || login.get() on:input=move |event| login.set(event_target_value(&event)) />
                <Button submit=true variant="secondary" size="sm" disabled=Signal::derive(move || busy.get() || login.get().trim().is_empty() || exceptions.get().len() >= 2000)>
                    {icons::plus("")}
                    "Ausnehmen"
                </Button>
            </form>
        </SettingGroup>
    }
}
