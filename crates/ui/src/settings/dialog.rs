//! Einstellungsfenster (settings-dialog.tsx, settings-search.ts): großes Fenster über der Oberfläche, oben Reiter
//! (Allgemein, Twitch, YouTube), links Suche und Bereiche, rechts die Optionen. Schließen per X, Esc oder Klick
//! daneben. Änderungen gelten sofort.
//!
//! Nachbau von shadcn Dialog, Tabs und ScrollArea: Das Fenster ist ein eingebautes `<dialog>` mit `showModal()`
//! (oberste Ebene, Rest der Seite gesperrt, Fokus bleibt drin). Der Bildlauf rechts nutzt den Bildlauf des Browsers
//! mit schmaler Leiste in der Farbe von ScrollArea (Daumen bg-border, rund) statt einer nachgebauten Leiste.

use multiyap_core::i18n::{t, tr};
use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use super::{
    account, backup, chat_behavior, colors, display, hotkeys, notifications, quick_actions, terms,
    third_party, updates,
};
use multiyap_core::search::matches_search;

use crate::components::button::button_class;
use crate::components::input::INPUT_CLASS;
use crate::components::ComingSoonBadge;

#[derive(Clone, Copy, PartialEq)]
enum Section {
    Terms,
    Notifications,
    ChatBehavior,
    Display,
    Colors,
    Hotkeys,
    Backup,
    Account,
    QuickActions,
    Shortcuts,
    UserCard,
    ThirdParty,
    Updates,
}

fn section_view(section: Section) -> AnyView {
    match section {
        Section::Terms => view! { <terms::TermCategoriesSection /> }.into_any(),
        Section::Notifications => view! { <notifications::NotificationsSection /> }.into_any(),
        Section::ChatBehavior => view! { <chat_behavior::ChatBehaviorSection /> }.into_any(),
        Section::Display => view! { <display::DisplaySection /> }.into_any(),
        Section::Colors => view! { <colors::ColorsSection /> }.into_any(),
        Section::Hotkeys => view! { <hotkeys::HotkeysSection /> }.into_any(),
        Section::Backup => view! { <backup::BackupSection /> }.into_any(),
        Section::Account => view! { <account::AccountSection /> }.into_any(),
        Section::QuickActions => view! { <quick_actions::QuickActionsSection /> }.into_any(),
        Section::Shortcuts => view! { <hotkeys::ShortcutsSection /> }.into_any(),
        Section::UserCard => view! { <quick_actions::UserCardSection /> }.into_any(),
        Section::ThirdParty => view! { <third_party::ThirdPartySection /> }.into_any(),
        Section::Updates => view! { <updates::UpdatesSection /> }.into_any(),
    }
}

struct Entry {
    id: &'static str,
    label: (&'static str, &'static str),
    /// Fehlt, solange der Bereich noch nicht umgesetzt ist („Coming Soon“).
    section: Option<Section>,
    /// Unterpunkt eines anderen Bereichs (eingerückt angezeigt).
    child: bool,
}

const fn entry(
    id: &'static str,
    label: (&'static str, &'static str),
    section: Option<Section>,
    child: bool,
) -> Entry {
    Entry {
        id,
        label,
        section,
        child,
    }
}

struct Group {
    id: &'static str,
    label: (&'static str, &'static str),
    entries: &'static [Entry],
}

/// Reiter oben: Allgemeines, alles nur für Twitch, alles nur für YouTube. Bereiche innerhalb eines Reiters in der
/// Reihenfolge aus.
const GROUPS: [Group; 3] = [
    Group {
        id: "general",
        label: ("Allgemein", "General"),
        entries: &[
            entry("channels", ("Kanäle", "Channels"), None, false),
            entry("terms", ("Filter", "Filters"), Some(Section::Terms), false),
            entry(
                "notifications",
                ("Benachrichtigungen", "Notifications"),
                Some(Section::Notifications),
                false,
            ),
            entry(
                "chat-behavior",
                ("Chat-Verhalten", "Chat behavior"),
                Some(Section::ChatBehavior),
                false,
            ),
            entry("display", ("Darstellung", "Appearance"), Some(Section::Display), false),
            // Eigener Unterpunkt, damit „Darstellung“ übersichtlich bleibt.
            entry("colors", ("Farben", "Colors"), Some(Section::Colors), true),
            // Allgemeine Tastenkürzel; die Mod-Tastenkürzel stehen unter Twitch.
            entry("hotkeys", ("Tastenkürzel", "Keyboard shortcuts"), Some(Section::Hotkeys), false),
            entry("backup", ("Sicherung", "Backup"), Some(Section::Backup), false),
            // Updates aus dem lokalen Versionsordner.
            entry("updates", ("Updates", "Updates"), Some(Section::Updates), false),
        ],
    },
    Group {
        id: "twitch",
        label: ("Twitch", "Twitch"),
        entries: &[
            entry(
                "account",
                ("Konto & Anmeldung", "Account & sign-in"),
                Some(Section::Account),
                false,
            ),
            entry(
                "quick-actions",
                ("Schnellaktionen", "Quick actions"),
                Some(Section::QuickActions),
                false,
            ),
            // Eigener Unterpunkt.
            entry("shortcuts", ("Tastenkürzel", "Keyboard shortcuts"), Some(Section::Shortcuts), true),
            entry("usercard", ("Nutzerkarte", "User card"), Some(Section::UserCard), false),
            entry(
                "third-party",
                ("Drittanbieter", "Third parties"),
                Some(Section::ThirdParty),
                false,
            ),
        ],
    },
    Group {
        id: "youtube",
        label: ("YouTube", "YouTube"),
        entries: &[
            entry("youtube-account", ("Konto & Anmeldung", "Account & sign-in"), None, false),
            entry("youtube-moderation", ("Moderation", "Moderation"), None, false),
        ],
    },
];

/// Ein Reiter öffnet mit dem ersten fertigen Bereich, nicht mit einem „Coming Soon“.
fn first_entry(group: &Group) -> &'static str {
    group
        .entries
        .iter()
        .find(|e| e.section.is_some())
        .unwrap_or(&group.entries[0])
        .id
}

fn group_by_id(id: &str) -> &'static Group {
    GROUPS.iter().find(|g| g.id == id).unwrap_or(&GROUPS[0])
}

// ---- Suche (settings-search.ts) ----

fn elements(root: &web_sys::Element, selector: &str) -> Vec<web_sys::HtmlElement> {
    let Ok(list) = root.query_selector_all(selector) else {
        return Vec::new();
    };
    (0..list.length())
        .filter_map(|i| list.item(i))
        .filter_map(|n| n.dyn_into().ok())
        .collect()
}

/// Blendet in einem gerenderten Bereich alles aus, was nicht zur Suche passt. Passt die Überschrift einer Gruppe,
/// bleibt die ganze Gruppe sichtbar; sonst nur die passenden Einträge. Arbeitet auf dem echten Inhalt, damit neue
/// Einstellungen ohne eigene Liste gefunden werden.
fn apply_search(root: &web_sys::Element, query: &str, section_label: &str) -> bool {
    let section_matches = matches_search(section_label, query);
    let mut any_visible = false;
    for group in elements(root, "[data-setting-group]") {
        let head = group
            .query_selector("[data-setting-group-head]")
            .ok()
            .flatten()
            .and_then(|h| h.text_content())
            .unwrap_or_default();
        let head_matches = section_matches || matches_search(&head, query);
        let mut group_visible = head_matches;
        if let Some(items) = group.query_selector("[data-setting-items]").ok().flatten() {
            let children = items.children();
            for i in 0..children.length() {
                let Some(item) = children
                    .item(i)
                    .and_then(|c| c.dyn_into::<web_sys::HtmlElement>().ok())
                else {
                    continue;
                };
                let visible =
                    head_matches || matches_search(&item.text_content().unwrap_or_default(), query);
                item.set_hidden(!visible);
                group_visible |= visible;
            }
        }
        group.set_hidden(!group_visible);
        any_visible |= group_visible;
    }
    any_visible
}

/// Alle Bereiche untereinander, nur Passendes sichtbar. Ein MutationObserver filtert nach, wenn Inhalte nachladen
/// (nur Inhalt beobachtet, keine Attribute, damit das Ausblenden keine Schleife auslöst).
#[component]
fn SearchResults(query: Signal<String>) -> impl IntoView {
    let root_ref = NodeRef::<leptos::html::Div>::new();
    let empty = RwSignal::new(false);
    let run = move || {
        let Some(root) = root_ref.get_untracked() else {
            return;
        };
        let query = query.get_untracked();
        let mut any = false;
        for block in elements(&root, "[data-search-section]") {
            let label = block
                .get_attribute("data-search-section")
                .unwrap_or_default();
            let has_groups = block
                .query_selector("[data-setting-group]")
                .ok()
                .flatten()
                .is_some();
            let visible = if has_groups {
                apply_search(&block, &query, &label)
            } else {
                matches_search(&label, &query)
            };
            block.set_hidden(!visible);
            any |= visible;
        }
        empty.set(!any);
    };
    let observer = StoredValue::new_local(None::<web_sys::MutationObserver>);
    Effect::new(move |_| {
        query.track();
        request_animation_frame(run);
        if observer.with_value(Option::is_none) {
            if let Some(root) = root_ref.get() {
                let callback = Closure::<dyn FnMut()>::new(move || request_animation_frame(run))
                    .into_js_value();
                if let Ok(watcher) = web_sys::MutationObserver::new(callback.unchecked_ref()) {
                    let options = web_sys::MutationObserverInit::new();
                    options.set_child_list(true);
                    options.set_subtree(true);
                    options.set_character_data(true);
                    let _ = watcher.observe_with_options(&root, &options);
                    observer.set_value(Some(watcher));
                }
            }
        }
    });
    on_cleanup(move || {
        if let Some(watcher) = observer.try_get_value().flatten() {
            watcher.disconnect();
        }
    });
    view! {
        <div node_ref=root_ref class="flex flex-col gap-10">
            {move || empty.get().then(|| view! { <p class="text-sm text-muted-foreground">{tr("Keine Einstellung gefunden.", "No setting found.")}</p> })}
            {GROUPS.iter().flat_map(|group| group.entries.iter().map(move |e| (group, e))).map(|(group, e)| {
                // Reitername vor dem Bereich, damit „Konto & Anmeldung“ von Twitch und YouTube unterscheidbar ist.
                let label = if group.id == "general" { t(e.label).to_string() } else { format!("{}: {}", t(group.label), t(e.label)) };
                let attribute = label.clone();
                view! {
                    <div data-search-section=attribute class="flex flex-col gap-6">
                        <h2 class="flex items-center gap-2 text-base font-semibold">
                            {label}
                            {e.section.is_none().then(|| view! { <ComingSoonBadge /> })}
                        </h2>
                        {e.section.map(section_view)}
                    </div>
                }
            }).collect_view()}
        </div>
    }
}

/// `open:flex` statt `flex`: Eine feste Anzeige-Klasse würde das geschlossene `<dialog>` sichtbar machen.
const CONTENT: &str = "fixed top-1/2 left-1/2 z-50 open:flex h-5/6 w-full max-w-[calc(100%-2rem)] -translate-x-1/2 -translate-y-1/2 flex-col gap-0 rounded-xl bg-popover p-0 text-sm text-popover-foreground ring-1 ring-foreground/10 duration-100 outline-none sm:max-w-5xl data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95 m-0 max-h-none border-0 backdrop:bg-black/10 backdrop:backdrop-blur-xs";
const TABS_TRIGGER: &str = "relative inline-flex h-[calc(100%-1px)] flex-1 items-center justify-center gap-1.5 rounded-md border border-transparent px-1.5 py-0.5 text-sm font-medium whitespace-nowrap text-foreground/60 transition-all group-data-vertical/tabs:w-full group-data-vertical/tabs:justify-start hover:text-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-1 focus-visible:outline-ring disabled:pointer-events-none disabled:opacity-50 has-data-[icon=inline-end]:pr-1 has-data-[icon=inline-start]:pl-1 dark:text-muted-foreground dark:hover:text-foreground group-data-[variant=default]/tabs-list:data-active:shadow-sm group-data-[variant=line]/tabs-list:data-active:shadow-none [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4 group-data-[variant=line]/tabs-list:bg-transparent group-data-[variant=line]/tabs-list:data-active:bg-transparent dark:group-data-[variant=line]/tabs-list:data-active:border-transparent dark:group-data-[variant=line]/tabs-list:data-active:bg-transparent data-active:bg-background data-active:text-foreground dark:data-active:border-input dark:data-active:bg-input/30 dark:data-active:text-foreground after:absolute after:bg-foreground after:opacity-0 after:transition-opacity group-data-horizontal/tabs:after:inset-x-0 group-data-horizontal/tabs:after:bottom-[-5px] group-data-horizontal/tabs:after:h-0.5 group-data-vertical/tabs:after:inset-y-0 group-data-vertical/tabs:after:-right-1 group-data-vertical/tabs:after:w-0.5 group-data-[variant=line]/tabs-list:data-active:after:opacity-100 px-3";
/// Gewählter Reiter: grüner Rand und heller grüner Hintergrund, wie die Auswahl im Dialog „Neuer Tab“.
const ACTIVE_GROUP: &str = "data-active:border-primary data-active:bg-primary/20 dark:data-active:border-primary dark:data-active:bg-primary/20";
/// Bildlauf wie ScrollArea: schmale Leiste, Daumen in bg-border, rund, ohne Spur.
const SCROLL: &str = "min-h-0 flex-1 overflow-y-auto [scrollbar-width:thin] [scrollbar-color:var(--border)_transparent]";

fn search_icon() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
            stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"
            class="lucide lucide-search pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground">
            <path d="m21 21-4.34-4.34"></path>
            <circle cx="11" cy="11" r="8"></circle>
        </svg>
    }
}

#[component]
pub fn SettingsDialog() -> impl IntoView {
    let dialog_ref = NodeRef::<leptos::html::Dialog>::new();
    let open = RwSignal::new(false);
    let group_id = RwSignal::new(GROUPS[0].id);
    let active = RwSignal::new(first_entry(&GROUPS[0]));
    let query = RwSignal::new(String::new());
    let searching = Signal::derive(move || !query.get().trim().is_empty());

    let show = move || {
        if let Some(dialog) = dialog_ref.get_untracked() {
            if !dialog.open() {
                open.set(true);
                let _ = dialog.show_modal();
            }
        }
    };
    let close = move || {
        if let Some(dialog) = dialog_ref.get_untracked() {
            dialog.close();
        }
    };

    // Zahnrad in der Kopfleiste und Tastenkürzel „Einstellungen öffnen“ (crates/web/src/app_commands.rs).
    let command = multiyap_web::app_commands::on("openSettings", std::rc::Rc::new(show));
    on_cleanup(move || multiyap_web::app_commands::off(command));

    let nav = move || {
        let group = group_by_id(group_id.get());
        group
            .entries
            .iter()
            .map(|e| {
                let id = e.id;
                let variant = if !searching.get() && active.get() == id { "secondary" } else { "ghost" };
                // Ohne tailwind-merge: justify-center aus den Grundklassen direkt ersetzen (wie cn() in React).
                let mut class = button_class(variant, "default", "").replacen("justify-center", "justify-between", 1);
                if e.section.is_none() {
                    class.push_str(" text-muted-foreground");
                }
                // Unterpunkte eingerückt und dünner als die Oberkategorie.
                if e.child {
                    class = class.replacen("font-medium", "font-normal", 1).replacen(
                        " px-2.5 ",
                        " pr-2.5 pl-6 ",
                        1,
                    );
                }
                view! {
                    <button type="button" data-slot="button" class=class on:click=move |_| {
                        query.set(String::new());
                        active.set(id);
                    }>
                        {t(e.label)}
                        {e.section.is_none().then(|| view! { <ComingSoonBadge /> })}
                    </button>
                }
            })
            .collect_view()
    };

    let content = move || {
        if searching.get() {
            return view! { <div class="p-6"><SearchResults query=Signal::derive(move || query.get()) /></div> }.into_any();
        }
        let group = group_by_id(group_id.get());
        let id = active.get();
        let e = group
            .entries
            .iter()
            .find(|e| e.id == id)
            .unwrap_or_else(|| {
                group
                    .entries
                    .iter()
                    .find(|e| e.id == first_entry(group))
                    .unwrap_or(&group.entries[0])
            });
        view! {
            <div class="flex flex-col gap-6 p-6">
                <h2 class="text-base font-semibold">{t(e.label)}</h2>
                {match e.section {
                    Some(section) => section_view(section),
                    None => view! { <p class="text-sm text-muted-foreground">{tr("Dieser Bereich kommt mit einer späteren Ausbaustufe.", "This section comes in a later stage.")}</p> }.into_any(),
                }}
            </div>
        }
        .into_any()
    };

    view! {
        <dialog
            node_ref=dialog_ref
            // Rolle nur, solange offen: Tastenkürzel und Nutzerkarte prüfen [role=dialog], um bei offenem Fenster zu ruhen.
            role=move || open.get().then_some("dialog")
            aria-labelledby="settings-title"
            aria-describedby="settings-description"
            data-slot="dialog-content"
            data-state=move || if open.get() { "open" } else { "closed" }
            class=CONTENT
            on:close=move |_| open.set(false)
            on:cancel=move |event: web_sys::Event| {
                // Esc leert zuerst die Suche, erst ein zweites Esc schließt das Fenster. Beim Wählen einer Taste
                // fängt das Wählen die Taste selbst ab (preventDefault), dann kommt hier nichts an.
                if !query.get_untracked().is_empty() {
                    event.prevent_default();
                    query.set(String::new());
                }
            }
            on:click=move |event: web_sys::MouseEvent| {
                // Klick daneben (auf den abgedunkelten Hintergrund) schließt.
                let Some(dialog) = dialog_ref.get_untracked() else { return };
                let on_dialog = event.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()).is_some_and(|t| t == **dialog);
                if on_dialog {
                    let rect = dialog.get_bounding_client_rect();
                    let (x, y) = (event.client_x() as f64, event.client_y() as f64);
                    if x < rect.left() || x > rect.right() || y < rect.top() || y > rect.bottom() {
                        close();
                    }
                }
            }
        >
            // Inhalt erst, wenn das Fenster offen ist (wie Radix: geschlossen ist nichts gerendert).
            {move || open.get().then(|| view! {
                <div data-slot="dialog-header" class="flex flex-row items-center gap-6 border-b p-4">
                    <h2 id="settings-title" data-slot="dialog-title" class="text-base leading-none font-medium">{tr("Einstellungen", "Settings")}</h2>
                    <div data-slot="tabs" data-orientation="horizontal" class="group/tabs flex gap-2 data-horizontal:flex-col">
                        <div role="tablist" aria-label=tr("Reiter", "Tabs") aria-orientation="horizontal" data-slot="tabs-list" data-variant="default"
                            class="group/tabs-list inline-flex w-fit items-center justify-center rounded-lg p-[3px] text-muted-foreground group-data-horizontal/tabs:h-8 group-data-vertical/tabs:h-fit group-data-vertical/tabs:flex-col data-[variant=line]:rounded-none">
                            {GROUPS.iter().map(|g| {
                                let id = g.id;
                                let selected = move || group_id.get() == id;
                                view! {
                                    <button type="button" role="tab" data-slot="tabs-trigger"
                                        aria-selected=move || selected().to_string()
                                        data-state=move || if selected() { "active" } else { "inactive" }
                                        tabindex=move || if selected() { "0" } else { "-1" }
                                        class=format!("{TABS_TRIGGER} {ACTIVE_GROUP}")
                                        on:mousedown=move |_| {
                                            query.set(String::new());
                                            group_id.set(id);
                                            active.set(first_entry(group_by_id(id)));
                                        }
                                        on:keydown=move |event: web_sys::KeyboardEvent| {
                                            let index = GROUPS.iter().position(|g| g.id == id).unwrap_or(0);
                                            let next = match event.key().as_str() {
                                                "ArrowRight" => (index + 1) % GROUPS.len(),
                                                "ArrowLeft" => (index + GROUPS.len() - 1) % GROUPS.len(),
                                                _ => return,
                                            };
                                            event.prevent_default();
                                            query.set(String::new());
                                            group_id.set(GROUPS[next].id);
                                            active.set(first_entry(&GROUPS[next]));
                                        }
                                    >
                                        {t(g.label)}
                                    </button>
                                }
                            }).collect_view()}
                        </div>
                    </div>
                    <p id="settings-description" data-slot="dialog-description" class="sr-only">{tr("Änderungen gelten sofort.", "Changes apply immediately.")}</p>
                </div>
                <div class="flex min-h-0 flex-1">
                    <nav class="flex w-64 shrink-0 flex-col gap-1 border-r p-2" aria-label=tr("Bereiche", "Sections")>
                        <div class="relative mb-1">
                            {search_icon()}
                            <input type="search" data-slot="input" class=format!("{INPUT_CLASS} pl-8")
                                placeholder=tr("Einstellungen durchsuchen", "Search settings") aria-label=tr("Einstellungen durchsuchen", "Search settings")
                                prop:value=move || query.get() on:input=move |event| query.set(event_target_value(&event)) />
                        </div>
                        {nav}
                    </nav>
                    <div data-slot="scroll-area" class=SCROLL>{content}</div>
                </div>
                <button type="button" data-slot="dialog-close" class=button_class("ghost", "icon-sm", "absolute top-2 right-2") on:click=move |_| close()>
                    {crate::components::icons::x("")}
                    <span class="sr-only">{tr("Schließen", "Close")}</span>
                </button>
            })}
        </dialog>
    }
}
