//! Eingabefeld unten im Kanal-Tab in Leptos (Nachbau von chat-input.tsx, suggestion-list.tsx, emote-picker.tsx). Enter
//! schickt ab. Autofill: Namen mit „@“ sofort, ohne „@“ per Tab; Emotes mit „:“ plus 2 Zeichen; „/“-Befehle. Pfeil hoch
//! und runter holt zuletzt gesendete Nachrichten. Emote-Fenster mit Gruppen, Suche und zuletzt benutzten Emotes.
//!
//! Stellen im Text sind wie im Browser UTF-16-Einheiten; die Logik in crates/core arbeitet mit Byte-Stellen.

use multiyap_core::i18n::tr;
use std::cell::RefCell;
use std::rc::Rc;

use multiyap_core::autofill::{self, EmoteIndex, Mixed, Name, Trigger};
use multiyap_core::commands::{self, byte_to_utf16_index, utf16_to_byte_index, CommandContext, OwnRoles};
use multiyap_core::model::Emote;
use multiyap_core::sent_history::{self, SentHistory};
use multiyap_web::moderation;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Deserialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{HtmlElement, KeyboardEvent};

use super::layer::{dismiss_on_outside, place, show_popover, Side, POPOVER_CONTENT, POPOVER_RESET};

use crate::bridge::{toast, use_setting};
use crate::components::button::button_class;
use crate::components::input::INPUT_CLASS;
use crate::components::{icons, ComingSoonBadge};

/// Höchstlänge einer Chatnachricht bei Twitch (UTF-16-Einheiten wie im Browser).
const MAX_MESSAGE_LENGTH: usize = 500;
/// Pro Gruppe im Emote-Fenster zunächst so viele Emotes; der Rest auf Klick oder über die Suche.
const GROUP_PREVIEW: usize = 120;
const SOURCE_LABELS: [(&str, &str); 4] = [("twitch", "Twitch"), ("seventv", "7TV"), ("bttv", "BTTV"), ("ffz", "FFZ")];

thread_local! {
    static SENT: RefCell<SentHistory> = RefCell::new(SentHistory::default());
}

fn source_label(source: &str) -> &str {
    SOURCE_LABELS.iter().find(|(id, _)| *id == source).map_or(source, |(_, label)| label)
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Kürzt auf höchstens `max` UTF-16-Einheiten, ohne ein Zeichen zu zerteilen.
fn truncate_utf16(text: &str, max: usize) -> String {
    let mut count = 0;
    text.chars()
        .take_while(|c| {
            count += c.len_utf16();
            count <= max
        })
        .collect()
}

/// Wort vor der Schreibmarke: Anfang und Ende (UTF-16) und das Wort.
fn word_at(text: &str, caret: usize) -> (usize, usize, String) {
    let byte = utf16_to_byte_index(text, caret);
    let (start, word) = autofill::word_at_caret(text, byte);
    (byte_to_utf16_index(text, start), byte_to_utf16_index(text, byte), word.to_string())
}

#[derive(Clone, Debug, PartialEq)]
enum Suggestion {
    Name(Name),
    Emote(Emote),
    Command(&'static str),
}

/// Leute, die im Chat geschrieben haben (aus den Zeilen des Kanals), zuletzt aktive zuerst.
/// Zuletzt aktive Chatter des Kanals aus dem geladenen Chat (für das Autofill der Namen).
fn recent_chatters(channel_login: &str) -> Vec<Name> {
    let mut senders: Vec<Name> = Vec::new();
    if let Some(session) = multiyap_web::sessions::session(channel_login) {
        session.visit_lines(|line, _| {
            if line.get("kind").and_then(serde_json::Value::as_str) == Some("message") {
                let sender = &line["sender"];
                let read = |key: &str| sender.get(key).and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
                senders.push(Name { login: read("login"), display_name: read("displayName") });
            }
        });
    }
    #[derive(Deserialize)]
    struct Auth {
        login: Option<String>,
    }
    let own = serde_json::from_value::<Auth>(crate::bridge::auth_info()).ok().and_then(|a| a.login);
    autofill::recent_chatters(&senders, own.as_deref()).into_iter().filter_map(|i| senders.get(i).cloned()).collect()
}

#[derive(Clone, Copy, Default, Deserialize)]
struct ModStatus {
    moderator: bool,
    broadcaster: bool,
}

/// Vorschlagsliste über dem Eingabefeld. Bedienung über Pfeiltasten, Enter/Tab, Esc (im Eingabefeld); Klick
/// übernimmt ebenfalls. Ist ein Emote markiert, zeigt es eine größere Vorschau.
fn suggestion_list(items: Vec<Suggestion>, selected: RwSignal<usize>, on_pick: Callback<usize>) -> impl IntoView {
    let list_ref = NodeRef::<leptos::html::Div>::new();
    Effect::new(move |_| {
        let index = selected.get();
        if let Some(list) = list_ref.get() {
            if let Ok(Some(item)) = list.query_selector(&format!("[data-index=\"{index}\"]")) {
                let options = web_sys::ScrollIntoViewOptions::new();
                options.set_block(web_sys::ScrollLogicalPosition::Nearest);
                item.scroll_into_view_with_scroll_into_view_options(&options);
            }
        }
    });
    let has_emotes = items.iter().any(|i| matches!(i, Suggestion::Emote(_)));
    let preview_items = items.clone();
    view! {
        <div class="absolute right-0 bottom-full left-0 mb-1 flex gap-2 rounded-lg border bg-popover p-1 text-popover-foreground shadow-md">
            <div node_ref=list_ref role="listbox" aria-label=tr("Vorschläge", "Suggestions") class="flex max-h-72 min-w-0 flex-1 flex-col overflow-y-auto">
                {items.into_iter().enumerate().map(|(index, item)| {
                    let content = match &item {
                        Suggestion::Command(name) => {
                            let info = commands::find_command(name);
                            let soon = info.is_some_and(|i| i.coming_soon);
                            view! {
                                <span class=if soon { "flex min-w-0 flex-1 flex-col opacity-50" } else { "flex min-w-0 flex-1 flex-col" }>
                                    <span class="flex items-center gap-2">
                                        <span class="font-medium">{format!("/{name}")}</span>
                                        {soon.then(|| view! { <ComingSoonBadge /> })}
                                    </span>
                                    <span class="truncate text-xs text-muted-foreground">
                                        {info.map(|i| format!("{}: {}", multiyap_core::i18n::t(i.usage), multiyap_core::i18n::t(i.description)))}
                                    </span>
                                </span>
                            }.into_any()
                        }
                        Suggestion::Emote(emote) => view! {
                            <img src=emote.image_url.clone() alt="" class="h-7 w-auto shrink-0" loading="lazy" />
                            <span class="truncate">{emote.name.clone()}</span>
                            <span class="ml-auto shrink-0 text-xs text-muted-foreground">{source_label(&emote.source).to_string()}</span>
                        }.into_any(),
                        Suggestion::Name(name) => view! {
                            <span class="truncate">{name.display_name.clone()}</span>
                            <span class="ml-auto shrink-0 text-xs text-muted-foreground">"Name"</span>
                        }.into_any(),
                    };
                    view! {
                        <button type="button" role="option" aria-selected=move || (selected.get() == index).to_string() data-index=index
                            class=move || if selected.get() == index {
                                "flex min-h-9 items-center gap-2 rounded-md px-2 py-1 text-left text-sm bg-accent text-accent-foreground"
                            } else {
                                "flex min-h-9 items-center gap-2 rounded-md px-2 py-1 text-left text-sm hover:bg-accent/50"
                            }
                            // Fokus im Eingabefeld behalten.
                            on:mousedown=|event: web_sys::MouseEvent| event.prevent_default()
                            on:mouseenter=move |_| selected.set(index)
                            on:click=move |_| on_pick.run(index)>
                            {content}
                        </button>
                    }
                }).collect_view()}
            </div>
            {has_emotes.then(|| view! {
                <div class="flex w-24 shrink-0 flex-col items-center justify-center gap-1 border-l p-2">
                    {move || match preview_items.get(selected.get()) {
                        Some(Suggestion::Emote(emote)) => Some(view! {
                            <img src=emote.image_url2x.clone().unwrap_or_else(|| emote.image_url.clone()) alt=emote.name.clone() class="h-14 w-auto" />
                            <span class="max-w-full truncate text-xs text-muted-foreground">{emote.name.clone()}</span>
                        }),
                        _ => None,
                    }}
                </div>
            })}
        </div>
    }
}

// ---- Emote-Fenster ----

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct EmoteGroup {
    id: String,
    label: String,
    emotes: Vec<Emote>,
}

/// Twitch-Emotes in Gruppen: „dieser Kanal“ (gehört dem Kanal), „Abo-Emotes“ (der Rest aus „Get User Emotes“), „global“.
fn group_twitch(emotes: &[Emote], channel_id: &str) -> Vec<EmoteGroup> {
    let (mut channel, mut own, mut global) = (Vec::new(), Vec::new(), Vec::new());
    for emote in emotes {
        if emote.owner_id.as_deref() == Some(channel_id) {
            channel.push(emote.clone());
        } else if emote.global == Some(true) {
            global.push(emote.clone());
        } else {
            own.push(emote.clone());
        }
    }
    [("twitch-channel", tr("Twitch: dieser Kanal", "Twitch: this channel"), channel), ("twitch-own", tr("Twitch: Abo-Emotes", "Twitch: subscriber emotes"), own), ("twitch-global", "Twitch: global", global)]
        .into_iter()
        .filter(|(_, _, list)| !list.is_empty())
        .map(|(id, label, emotes)| EmoteGroup { id: id.into(), label: label.into(), emotes })
        .collect()
}

/// Gruppen auf eine Suche einschränken (Teil des Namens, Groß- und Kleinschreibung egal).
fn filter_groups(groups: Vec<EmoteGroup>, query: &str) -> Vec<EmoteGroup> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return groups;
    }
    groups
        .into_iter()
        .map(|g| EmoteGroup { emotes: g.emotes.into_iter().filter(|e| e.name.to_lowercase().contains(&needle)).collect(), ..g })
        .filter(|g| !g.emotes.is_empty())
        .collect()
}

fn emote_button(emote: Emote, on_pick: Callback<Emote>) -> impl IntoView {
    let srcset = emote.image_url2x.as_ref().map(|x2| format!("{} 1x, {x2} 2x", emote.image_url));
    view! {
        <button type="button" data-slot="button" class=button_class("ghost", "icon-lg", "")
            title=format!("{} ({})", emote.name, source_label(&emote.source)) aria-label=emote.name.clone()
            on:click={
                let emote = emote.clone();
                move |_| on_pick.run(emote.clone())
            }>
            <img src=emote.image_url.clone() srcset=srcset alt="" class="max-h-7 max-w-full object-contain" loading="lazy" decoding="async" />
        </button>
    }
}

fn emote_group(group: EmoteGroup, on_pick: Callback<Emote>) -> impl IntoView {
    let expanded = RwSignal::new(false);
    let total = group.emotes.len();
    let emotes = group.emotes;
    view! {
        <section class="flex flex-col gap-1">
            <h3 class="px-1 text-xs font-semibold text-muted-foreground">{format!("{} ({total})", group.label)}</h3>
            <div class="flex flex-wrap">
                {move || {
                    let count = if expanded.get() { total } else { total.min(GROUP_PREVIEW) };
                    emotes[..count].iter().cloned().map(|e| emote_button(e, on_pick)).collect_view()
                }}
            </div>
            {move || (!expanded.get() && total > GROUP_PREVIEW).then(|| view! {
                <button type="button" data-slot="button" class=button_class("ghost", "sm", "self-start") on:click=move |_| expanded.set(true)>
                    {multiyap_core::tf!("Alle {total} anzeigen", "Show all {total}")}
                </button>
            })}
        </section>
    }
}

/// Emote-Fenster am Eingabefeld: gruppiert nach Twitch (dieser Kanal, eigene Abo-Emotes, global) sowie 7TV, BTTV und
/// FFZ, mit Suchfeld und den zuletzt benutzten Emotes oben. Ein Klick fügt das Emote ein; das Fenster bleibt für
/// weitere Emotes offen. Solange es offen ist, steht die Chat-Anzeige still.
#[component]
fn EmotePicker(
    channel_id: Signal<Option<String>>,
    disabled: Signal<bool>,
    size: Signal<&'static str>,
    hold: Callback<(), JsValue>,
    third_party_version: RwSignal<u32>,
    on_pick: Callback<Emote>,
) -> impl IntoView {
    let open = RwSignal::new(false);
    let query = RwSignal::new(String::new());
    let twitch = RwSignal::new(None::<Vec<Emote>>);
    let failed = RwSignal::new(false);
    let recent = use_setting("emotes.recent", Vec::<Emote>::new());
    let trigger_ref = NodeRef::<leptos::html::Button>::new();
    let content_ref = NodeRef::<leptos::html::Div>::new();
    let close = move || {
        open.set(false);
        query.set(String::new());
    };
    dismiss_on_outside(move || open.get(), close);

    // Chat anhalten, solange offen.
    let release = StoredValue::new_local(None::<js_sys::Function>);
    Effect::new(move |_| {
        if open.get() {
            if release.with_value(Option::is_none) {
                release.set_value(hold.run(()).dyn_into::<js_sys::Function>().ok());
            }
        } else if let Some(r) = release.try_update_value(Option::take).flatten() {
            let _ = r.call0(&JsValue::NULL);
        }
    });
    on_cleanup(move || {
        if let Some(r) = release.try_update_value(Option::take).flatten() {
            let _ = r.call0(&JsValue::NULL);
        }
    });

    // Twitch-Emotes erst laden, wenn das Fenster geöffnet wird (meist schon vorgeladen).
    Effect::new(move |_| {
        let (true, Some(id)) = (open.get(), channel_id.get()) else { return };
        failed.set(false);
        spawn_local(async move {
            match multiyap_web::emotes::usable_twitch_emotes(&id).await {
                Ok(list) => {
                    twitch.try_set(Some(list));
                }
                Err(_) => {
                    failed.try_set(true);
                }
            }
        });
    });
    Effect::new(move |_| {
        if !open.get() {
            return;
        }
        if let (Some(content), Some(trigger)) = (content_ref.get(), trigger_ref.get()) {
            let content: HtmlElement = content.into();
            show_popover(&content);
            place(&content, super::layer::Rect::of(&trigger), Side::AboveEnd, 4.0);
            if let Ok(Some(input)) = content.query_selector("input") {
                if let Ok(input) = input.dyn_into::<HtmlElement>() {
                    let _ = input.focus();
                }
            }
        }
    });

    let groups = move || {
        third_party_version.track();
        let Some(id) = channel_id.get() else { return Vec::new() };
        let mut all = group_twitch(twitch.get().as_deref().unwrap_or_default(), &id);
        all.extend(multiyap_web::emotes::third_party_groups(&id).into_iter().map(|g| EmoteGroup { id: g.id, label: g.label, emotes: g.emotes }));
        filter_groups(all, &query.get())
    };

    let content = move || {
        let groups = groups();
        let recent_group = (query.with(|q| q.trim().is_empty()) && !recent.with(Vec::is_empty))
            .then(|| EmoteGroup { id: "recent".into(), label: tr("Zuletzt benutzt", "Recently used").into(), emotes: recent.get() });
        let nothing = twitch.with(Option::is_some) && groups.is_empty() && recent_group.is_none();
        view! {
            {recent_group.map(|g| emote_group(g, on_pick))}
            {groups.into_iter().map(|g| emote_group(g, on_pick)).collect_view()}
            {(twitch.with(Option::is_none) && !failed.get()).then(|| view! {
                <div class="flex items-center gap-2 px-1 text-sm text-muted-foreground">
                    {icons::loader_circle("size-4 animate-spin")}
                    {tr("Twitch-Emotes werden geladen…", "Loading Twitch emotes…")}
                </div>
            })}
            {failed.get().then(|| view! { <p class="px-1 text-sm text-destructive">{tr("Twitch-Emotes konnten nicht geladen werden.", "Twitch emotes could not be loaded.")}</p> })}
            {nothing.then(|| view! { <p class="px-1 text-sm text-muted-foreground">{tr("Keine Emotes gefunden.", "No emotes found.")}</p> })}
        }
    };

    let popover_class = format!("{POPOVER_RESET} {}", POPOVER_CONTENT.replacen("flex w-72", "flex h-96 w-96", 1).replacen("gap-2.5", "gap-2", 1).replacen(" p-2.5 ", " p-2 ", 1));
    view! {
        <button node_ref=trigger_ref type="button" data-slot="button" data-chat-overlay=""
            class=move || button_class("ghost", size.get(), "") disabled=move || disabled.get()
            aria-label="Emotes" title="Emotes" aria-expanded=move || open.get().to_string()
            on:click=move |_| if open.get_untracked() { close() } else { open.set(true) }>
            <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
                stroke-linecap="round" stroke-linejoin="round" class="lucide lucide-smile" aria-hidden="true">
                <circle cx="12" cy="12" r="10"></circle>
                <path d="M8 14s1.5 2 4 2 4-2 4-2"></path>
                <line x1="9" x2="9.01" y1="9" y2="9"></line>
                <line x1="15" x2="15.01" y1="9" y2="9"></line>
            </svg>
        </button>
        {move || open.get().then(|| view! {
            <div node_ref=content_ref popover="manual" data-chat-overlay="" data-slot="popover-content" data-state="open" data-side="top"
                class=popover_class.clone() style="opacity: 0;">
                <input data-slot="input" class=INPUT_CLASS placeholder=tr("Emotes suchen", "Search emotes") aria-label=tr("Emotes suchen", "Search emotes")
                    prop:value=move || query.get() on:input=move |event| query.set(event_target_value(&event)) />
                <div data-slot="scroll-area" class="min-h-0 flex-1 overflow-y-auto [scrollbar-width:thin] [scrollbar-color:var(--border)_transparent]">
                    <div class="flex flex-col gap-3 pr-2">{content}</div>
                </div>
            </div>
        })}
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplyTarget {
    message_id: String,
    display_name: String,
    text: String,
}

#[derive(Deserialize)]
struct SendResult {
    sent: bool,
    reason: Option<String>,
}

/// Eingabefeld unten im Kanal-Tab mit Autofill für Namen, Emotes und Befehle. Enter schickt ab.
#[component]
pub fn ChatInput(
    #[prop(into)] channel_id: Signal<Option<String>>,
    #[prop(into)] channel_name: Signal<String>,
    #[prop(into)] channel_login: Signal<String>,
    #[prop(into)] enabled: Signal<bool>,
    pause: super::pause::ChatPause,
) -> impl IntoView {
    // Höhe und Schriftgröße des Eingabefelds (Einstellungen → Darstellung → Eingabefeld).
    let input_size = use_setting("display.inputSize", "normal".to_string());
    let input_font_size = use_setting("display.inputFontSize", 14.0f64);

    let input_ref = NodeRef::<leptos::html::Input>::new();
    let text = RwSignal::new(String::new());
    let caret = RwSignal::new(0usize);
    let sending = RwSignal::new(false);
    // Über Tab geöffnet (Namen und Emotes ohne „@“ oder „:“).
    let tab_triggered = RwSignal::new(false);
    let dismissed = RwSignal::new(false);
    let selected = RwSignal::new(0usize);
    // Stelle im Verlauf gesendeter Nachrichten; None ist der eigene Entwurf.
    let history_index = RwSignal::new(None::<usize>);
    let draft = StoredValue::new(String::new());

    let set_cursor = move |position: usize| {
        request_animation_frame(move || {
            if let Some(input) = input_ref.get_untracked() {
                let position = position as u32;
                let _ = input.set_selection_range(position, position);
            }
        });
    };

    // ---- Emotes für das Autofill: Twitch (beim ersten Bedarf geladen) und 7TV, BTTV, FFZ ----
    let twitch_version = RwSignal::new(0u32);
    let third_party_version = RwSignal::new(0u32);
    {
        use multiyap_web::emotes as em;
        let twitch_id = em::subscribe_twitch(Rc::new(move || twitch_version.update(|v| *v += 1)));
        let third_id = em::subscribe_third_party(Rc::new(move || third_party_version.update(|v| *v += 1)));
        on_cleanup(move || {
            em::unsubscribe_twitch(twitch_id);
            em::unsubscribe_third_party(third_id);
        });
    }
    let twitch_emotes = RwSignal::new(Vec::<Emote>::new());
    let emotes_loading = RwSignal::new(false);
    let loaded_for = StoredValue::new(None::<String>);
    let emotes = Memo::new(move |_| {
        third_party_version.track();
        let mut list = twitch_emotes.get();
        if let Some(id) = channel_id.get() {
            let extra = multiyap_web::emotes::third_party_emotes(&id);
            // Twitch zuerst, gleiche Namen nur einmal.
            let seen: std::collections::HashSet<String> = list.iter().map(|e| e.name.clone()).collect();
            list.extend(extra.iter().filter(|e| !seen.contains(&e.name)).cloned());
        }
        list
    });
    let emote_index = StoredValue::new_local(None::<(usize, Rc<EmoteIndex>)>);
    let index_for = move |list: &[Emote]| {
        let key = list.as_ptr() as usize ^ list.len();
        if let Some((k, index)) = emote_index.get_value() {
            if k == key {
                return index;
            }
        }
        let index = Rc::new(EmoteIndex::new(list.iter().map(|e| e.name.clone()).collect()));
        emote_index.set_value(Some((key, index.clone())));
        index
    };

    let word = Memo::new(move |_| text.with(|t| word_at(t, caret.get())));
    let trigger = Memo::new(move |_| if dismissed.get() { None } else { autofill::detect_trigger(&word.get().2, tab_triggered.get()) });
    let command_context = Memo::new(move |_| {
        if dismissed.get() {
            return None;
        }
        text.with(|t| commands::detect_command_context(t, utf16_to_byte_index(t, caret.get())))
    });
    let needs_emotes = Memo::new(move |_| matches!(trigger.get(), Some(Trigger::Emote { .. }) | Some(Trigger::Tab { .. })));
    Effect::new(move |_| {
        let (true, Some(id)) = (needs_emotes.get(), channel_id.get()) else { return };
        let key = format!("{id}:{}", twitch_version.get());
        if loaded_for.get_value().as_deref() == Some(key.as_str()) {
            return;
        }
        loaded_for.set_value(Some(key));
        emotes_loading.set(true);
        spawn_local(async move {
            match multiyap_web::emotes::usable_twitch_emotes(&id).await {
                Ok(list) => {
                    twitch_emotes.try_set(list);
                }
                Err(_) => loaded_for.set_value(None),
            }
            emotes_loading.try_set(false);
        });
    });

    // „/“-Befehle: Liste am Befehlswort, danach Namen für das Namensargument.
    let suggestions = Memo::new(move |_| -> Option<Vec<Suggestion>> {
        let chatters = || recent_chatters(&channel_login.get_untracked());
        let names = |query: &str| {
            let list = chatters();
            autofill::suggest_names(&list, query).into_iter().filter_map(|i| list.get(i).cloned().map(Suggestion::Name)).collect::<Vec<_>>()
        };
        let list = match command_context.get() {
            Some(CommandContext::Command { query }) => {
                let id = channel_id.get_untracked().unwrap_or_default();
                // Zentrale Prüfung der Mod-Rechte in Rust (crates/web/src/moderation.rs).
                let status = ModStatus { moderator: moderation::can_moderate(&id), broadcaster: moderation::is_broadcaster(&id) };
                let vip = moderation::is_own_vip(&channel_login.get_untracked());
                let roles = OwnRoles { moderator: status.moderator, broadcaster: status.moderator && status.broadcaster, vip };
                commands::suggest_commands(&query, roles).into_iter().map(|c| Suggestion::Command(c.name)).collect()
            }
            Some(CommandContext::CommandUser { query, .. }) => names(&query),
            None => match trigger.get()? {
                Trigger::Name { query, .. } => names(&query),
                Trigger::Emote { query } => emotes.with(|list| {
                    index_for(list).suggest(&query).into_iter().filter_map(|i| list.get(i).cloned().map(Suggestion::Emote)).collect()
                }),
                Trigger::Tab { query } => {
                    let people = chatters();
                    emotes.with(|list| {
                        autofill::suggest_mixed(&people, &index_for(list), &query)
                            .into_iter()
                            .filter_map(|m| match m {
                                Mixed::Name(i) => people.get(i).cloned().map(Suggestion::Name),
                                Mixed::Emote(i) => list.get(i).cloned().map(Suggestion::Emote),
                            })
                            .collect()
                    })
                }
            },
        };
        (!list.is_empty()).then_some(list)
    });
    let suggestion_key = Memo::new(move |_| (command_context.get(), trigger.get()));
    Effect::new(move |_| {
        suggestion_key.track();
        selected.set(0);
    });

    let update_text = move |value: String, position: usize| {
        // Ein per Tab geöffnete Namensliste bleibt offen, solange am selben Wort weitergetippt wird.
        if word_at(&value, position).2.is_empty() {
            tab_triggered.set(false);
        }
        text.set(value);
        caret.set(position);
        dismissed.set(false);
        // Geänderter Text ist ab jetzt der Entwurf, auch wenn er aus dem Verlauf kam.
        history_index.set(None);
    };

    let pick = move |index: usize| {
        let Some(item) = suggestions.get_untracked().and_then(|list| list.get(index).cloned()) else { return };
        let with_at = match (command_context.get_untracked(), trigger.get_untracked()) {
            (Some(CommandContext::CommandUser { with_at, .. }), _) => with_at,
            (None, Some(Trigger::Name { with_at, .. })) => with_at,
            _ => false,
        };
        let replacement = match &item {
            Suggestion::Command(name) => format!("/{name}"),
            Suggestion::Name(name) => autofill::name_text(&name.login, &name.display_name, with_at),
            Suggestion::Emote(emote) => {
                multiyap_web::emotes::remember(emote);
                emote.name.clone()
            }
        };
        let (start, end, _) = word.get_untracked();
        let current = text.get_untracked();
        let (next, next_caret) = autofill::replace_word(&current, utf16_to_byte_index(&current, start), utf16_to_byte_index(&current, end), &replacement);
        let position = byte_to_utf16_index(&next, next_caret);
        text.set(next);
        caret.set(position);
        history_index.set(None);
        tab_triggered.set(false);
        dismissed.set(false);
        set_cursor(position);
    };

    // Wort an der Schreibmarke einfügen, mit Leerzeichen davor (falls nötig) und danach: Emotes aus dem
    // Emote-Fenster und „@Name“ per Rechtsklick auf einen Namen.
    let insert_word = move |insert: String| {
        let current = text.get_untracked();
        let position = caret.get_untracked().min(utf16_len(&current));
        let byte = utf16_to_byte_index(&current, position);
        let (before, after) = current.split_at(byte);
        let space = if !before.is_empty() && !before.ends_with(char::is_whitespace) { " " } else { "" };
        let addition = format!("{space}{insert} ");
        let value = truncate_utf16(&format!("{before}{addition}{after}"), MAX_MESSAGE_LENGTH);
        let next_caret = (utf16_len(before) + utf16_len(&addition)).min(utf16_len(&value));
        text.set(value);
        caret.set(next_caret);
        history_index.set(None);
        dismissed.set(true);
        request_animation_frame(move || {
            if let Some(input) = input_ref.get_untracked() {
                let _ = input.focus();
                let _ = input.set_selection_range(next_caret as u32, next_caret as u32);
            }
        });
    };

    // Antwort-Leiste und Bedienung von außen (Antworten, „@Name“ per Rechtsklick), je Kanal.
    let reply = RwSignal::new(None::<ReplyTarget>);
    Effect::new(move |_| {
        let login = channel_login.get();
        let read = {
            let login = login.clone();
            move || multiyap_web::chat_tools::reply_target(&login).and_then(|t| crate::bridge::convert::<ReplyTarget>(&t))
        };
        reply.set(read());
        let reply_listener = multiyap_web::chat_tools::subscribe_reply_targets(Rc::new(move || {
            reply.try_set(read());
        }));
        let handlers = multiyap_web::chat_tools::InputHandlers {
            focus: Rc::new(move || {
                if let Some(input) = input_ref.get_untracked() {
                    let _ = input.focus();
                }
            }),
            insert: Rc::new(move |text: &str| {
                let insert = insert_word;
                insert(text.to_string());
            }),
        };
        let input_id = (!login.is_empty()).then(|| multiyap_web::chat_tools::register_input(&login, handlers));
        on_cleanup(move || {
            multiyap_web::chat_tools::unsubscribe_reply_targets(reply_listener);
            if let Some(id) = input_id {
                multiyap_web::chat_tools::unregister_input(&login, id);
            }
        });
    });

    // `resend`: per Strg+Enter erneut gesendet; das Eingabefeld und die Antwort-Leiste bleiben dann unverändert.
    let clear_after_send = move |message: &str, resend: bool| {
        let login = channel_login.get_untracked();
        if !login.is_empty() {
            SENT.with(|h| h.borrow_mut().remember(&login, message));
        }
        if !resend {
            history_index.set(None);
            text.set(String::new());
            caret.set(0);
        }
    };

    let send_text = move |message: String, resend: bool| {
        if message.is_empty() || sending.get_untracked() || !enabled.get_untracked() {
            return;
        }
        sending.set(true);
        let (id, login) = (channel_id.get_untracked(), channel_login.get_untracked());
        spawn_local(async move {
            // „/“-Befehle werden in Aktionen übersetzt und nie als Text gesendet.
            if message.starts_with('/') {
                let ran = multiyap_web::mod_actions::execute_command(&message, id, (!login.is_empty()).then_some(login)).await;
                if ran {
                    clear_after_send(&message, resend);
                }
            } else {
                let reply_id = if resend { None } else { reply.get_untracked().map(|r| r.message_id) };
                let result = match multiyap_web::sessions::session(&login) {
                    None => Some(SendResult { sent: false, reason: Some(tr("Kanal noch nicht verbunden", "Channel not connected yet").into()) }),
                    Some(session) => match session.send(&message, reply_id.as_deref()).await {
                        Ok(r) => Some(SendResult { sent: r.sent, reason: r.reason }),
                        Err(error) => Some(SendResult {
                            sent: false,
                            reason: Some(
                                js_sys::Reflect::get(&error, &"message".into()).ok().and_then(|m| m.as_string()).unwrap_or_else(|| tr("unbekannter Fehler", "unknown error").into()),
                            ),
                        }),
                    },
                };
                match result {
                    Some(r) if r.sent => {
                        if !resend {
                            multiyap_web::chat_tools::set_reply_target(&channel_login.get_untracked(), None);
                        }
                        clear_after_send(&message, resend);
                    }
                    other => {
                        let reason = other.and_then(|r| r.reason).unwrap_or_else(|| tr("unbekannter Grund", "unknown reason").into());
                        toast("error", &multiyap_core::tf!("Nachricht nicht gesendet: {reason}", "Message not sent: {reason}"));
                    }
                }
            }
            sending.try_set(false);
        });
    };

    let submit = move |event: web_sys::SubmitEvent| {
        event.prevent_default();
        send_text(text.get_untracked().trim().to_string(), false);
    };

    let on_key = move |event: KeyboardEvent| {
        if let Some(list) = suggestions.get_untracked() {
            let len = list.len();
            match event.key().as_str() {
                "ArrowDown" => {
                    event.prevent_default();
                    selected.update(|s| *s = (*s + 1) % len);
                    return;
                }
                "ArrowUp" => {
                    event.prevent_default();
                    selected.update(|s| *s = (*s + len - 1) % len);
                    return;
                }
                // Tab: nächster Vorschlag, Umschalt+Tab: vorheriger; Enter übernimmt, nochmal Enter sendet.
                "Tab" => {
                    event.prevent_default();
                    if event.shift_key() {
                        selected.update(|s| *s = (*s + len - 1) % len);
                    } else {
                        selected.update(|s| *s = (*s + 1) % len);
                    }
                    return;
                }
                "Enter" if !event.ctrl_key() => {
                    event.prevent_default();
                    pick(selected.get_untracked());
                    return;
                }
                "Escape" => {
                    event.prevent_default();
                    dismissed.set(true);
                    tab_triggered.set(false);
                    return;
                }
                _ => {}
            }
        }
        let key = event.key();
        let login = channel_login.get_untracked();
        // Strg+Enter: die letzte eigene Nachricht in diesem Kanal noch einmal senden, das Eingabefeld bleibt, wie es
        // ist. „/“-Befehle zählen nicht, damit keine Mod-Aktion wiederholt wird.
        if key == "Enter" && event.ctrl_key() && !event.shift_key() && !event.alt_key() {
            event.prevent_default();
            let last = SENT.with(|h| h.borrow().list(&login).iter().rev().find(|m| !m.starts_with('/')).cloned());
            if let Some(message) = last {
                send_text(message, true);
            }
            return;
        }
        // Pfeil hoch und runter: zuletzt gesendete Nachrichten dieses Kanals (nur ohne offene Vorschlagsliste).
        if (key == "ArrowUp" || key == "ArrowDown") && !login.is_empty() && !event.shift_key() && !event.ctrl_key() && !event.alt_key() && !event.meta_key() {
            let history: Vec<String> = SENT.with(|h| h.borrow().list(&login).to_vec());
            let Some(next) = sent_history::step(history.len(), history_index.get_untracked(), key == "ArrowUp") else { return };
            event.prevent_default();
            if history_index.get_untracked().is_none() {
                draft.set_value(text.get_untracked());
            }
            let value = match next {
                Some(i) => history[i].clone(),
                None => draft.get_value(),
            };
            let end = utf16_len(&value);
            history_index.set(next);
            text.set(value);
            caret.set(end);
            // Keine Vorschlagsliste für das Wort am Ende der geholten Nachricht.
            dismissed.set(true);
            tab_triggered.set(false);
            set_cursor(end);
            return;
        }
        // Ohne „@“ oder „:“: Tab öffnet die gemeinsame Liste aus Namen und Emotes.
        let current_word = word.get_untracked().2;
        if key == "Tab" && !event.shift_key() && !current_word.is_empty() && !current_word.starts_with('@') && !current_word.starts_with(':') {
            event.prevent_default();
            tab_triggered.set(true);
            dismissed.set(false);
        }
    };

    let read_caret = move |event: &web_sys::Event| {
        if let Some(input) = event.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()) {
            caret.set(input.selection_start().ok().flatten().unwrap_or(0) as usize);
        }
    };
    let hold = Callback::new(move |_: ()| pause.hold());

    view! {
        <form class=move || if input_size.get() == "normal" { "relative shrink-0 border-t p-2" } else { "relative shrink-0 border-t p-1" } on:submit=submit>
            {move || enabled.get().then(|| suggestions.get().map(|list| suggestion_list(list, selected, Callback::new(pick))))}
            {move || (enabled.get() && suggestions.with(Option::is_none) && needs_emotes.get() && emotes_loading.get()).then(|| view! {
                <div class="absolute right-0 bottom-full left-0 mb-1 rounded-lg border bg-popover px-3 py-2 text-sm text-muted-foreground shadow-md">
                    {tr("Emotes werden geladen…", "Loading emotes…")}
                </div>
            })}
            {move || reply.get().map(|r| view! {
                <div class="mb-1 flex items-center gap-2 rounded-md bg-muted px-2 py-1 text-xs text-muted-foreground">
                    <span class="min-w-0 flex-1 truncate">
                        {tr("Antwort an ", "Reply to ")}<span class="font-medium text-foreground">{format!("@{}", r.display_name)}</span>{format!(": {}", r.text)}
                    </span>
                    <button type="button" data-slot="button" class=button_class("ghost", "icon-xs", "") aria-label=tr("Antwort abbrechen", "Cancel reply")
                        on:click=move |_| { multiyap_web::chat_tools::set_reply_target(&channel_login.get_untracked(), None); }>
                        {icons::x("")}
                    </button>
                </div>
            })}
            <div class="flex items-center gap-1">
                <input node_ref=input_ref data-slot="input"
                    class=move || match input_size.get().as_str() {
                        "small" => INPUT_CLASS.replacen("h-8 ", "h-7 ", 1),
                        "xsmall" => INPUT_CLASS.replacen("h-8 ", "h-6 ", 1).replacen(" py-1 ", " py-0 ", 1),
                        _ => INPUT_CLASS.to_string(),
                    }
                    style:font-size=move || format!("{}px", input_font_size.get())
                    prop:value=move || text.get()
                    maxlength=MAX_MESSAGE_LENGTH.to_string()
                    disabled=move || !enabled.get()
                    placeholder=move || if enabled.get() { multiyap_core::tf!("Nachricht an {}", "Message to {}", channel_name.get()) } else { tr("Nicht verbunden", "Not connected").to_string() }
                    aria-label=move || multiyap_core::tf!("Nachricht an {}", "Message to {}", channel_name.get())
                    aria-autocomplete="list" autocomplete="off" spellcheck="true"
                    on:input=move |event: web_sys::Event| {
                        let Some(input) = event.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()) else { return };
                        let value = input.value().replace("\r\n", " ").replace('\n', " ");
                        let position = input.selection_start().ok().flatten().map_or_else(|| utf16_len(&value), |p| p as usize);
                        update_text(value, position);
                    }
                    on:keydown=on_key
                    on:keyup=move |event: KeyboardEvent| read_caret(&event)
                    on:click=move |event: web_sys::MouseEvent| read_caret(&event)
                    on:select=move |event: web_sys::Event| read_caret(&event)
                    on:blur=move |_| dismissed.set(true)
                    on:focus=move |_| dismissed.set(false) />
                <EmotePicker channel_id=channel_id disabled=Signal::derive(move || !enabled.get())
                    size=Signal::derive(move || match input_size.get().as_str() { "xsmall" => "icon-xs", "small" => "icon-sm", _ => "icon" })
                    hold=hold third_party_version=third_party_version
                    on_pick=Callback::new(move |emote: Emote| {
                        multiyap_web::emotes::remember(&emote);
                        insert_word(emote.name);
                    }) />
            </div>
        </form>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emote(name: &str, owner: Option<&str>, global: bool) -> Emote {
        Emote {
            source: "twitch".into(),
            id: name.into(),
            name: name.into(),
            image_url: format!("https://static-cdn.jtvnw.net/emoticons/v2/{name}/default/dark/1.0"),
            image_url2x: None,
            animated: None,
            overlay: None,
            owner_id: owner.map(str::to_string),
            global: global.then_some(true),
        }
    }

    #[test]
    fn twitch_groups_channel_own_global_and_search() {
        let groups = group_twitch(&[emote("kanalHi", Some("1"), false), emote("andererHi", Some("2"), false), emote("Kappa", None, true)], "1");
        let labels: Vec<(String, Vec<String>)> = groups.iter().map(|g| (g.label.clone(), g.emotes.iter().map(|e| e.name.clone()).collect())).collect();
        assert_eq!(
            labels,
            vec![
                ("Twitch: dieser Kanal".to_string(), vec!["kanalHi".to_string()]),
                ("Twitch: Abo-Emotes".to_string(), vec!["andererHi".to_string()]),
                ("Twitch: global".to_string(), vec!["Kappa".to_string()]),
            ]
        );
        let found = filter_groups(groups, " HI ");
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|g| g.emotes.iter().all(|e| e.name.to_lowercase().contains("hi"))));
    }
}
