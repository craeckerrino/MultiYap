//! Darstellung (display-section.tsx, chat-font-picker.tsx, chat-preview.tsx): Schrift, Zeilen, Eingabefeld, Uhrzeit,
//! entfernte Nachrichten und Stream-Info, mit Live-Vorschau aus echten Chatzeilen.

use multiyap_core::i18n::tr;
use std::cell::RefCell;

use multiyap_core::model::{ChannelRef, ChatMessage, MessageFlags, MessagePart, Sender};
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};
use wasm_bindgen::{JsCast, JsValue};

use crate::bridge::{save_setting, use_setting};
use crate::chat::line::{message_line, LineOptions, RemovedDisplay};
use crate::components::{Button, CommitInput, Select, SelectEntry, SettingGroup, SettingRow, Switch};

const TIME_FORMAT: &str = "chat.timeFormat";
const REMOVED: &str = "chat.removedDisplay";
const STREAM_INFO: &str = "tabs.streamInfo";
const FONT_SIZE: &str = "display.fontSize";
const DENSITY: &str = "display.density";
const SEPARATORS: &str = "display.separators";
const ZEBRA: &str = "display.zebra";
const READABLE: &str = "display.readableNames";
const INPUT_SIZE: &str = "display.inputSize";
const INPUT_FONT_SIZE: &str = "display.inputFontSize";
const LANGUAGE: &str = multiyap_web::language::KEY;
const CHAT_FONT: &str = "display.chatFont";

const CHAT_FONT_SIZES: [u32; 7] = [12, 13, 14, 15, 16, 18, 20];
const INPUT_FONT_SIZES: [u32; 4] = [11, 12, 13, 14];
fn input_sizes() -> [(&'static str, &'static str); 3] {
    [("normal", tr("Normal (Standard)", "Normal (default)")), ("small", tr("Klein", "Small")), ("xsmall", tr("Sehr klein", "Very small"))]
}
fn stream_info_labels() -> [(&'static str, &'static str); 7] {
    [
    ("title", tr("Streamtitel", "Stream title")),
    ("category", tr("Kategorie", "Category")),
    ("status", tr("Live-Status", "Live status")),
    ("viewers", tr("Zuschauerzahl", "Viewer count")),
    ("uptime", tr("Laufzeit", "Uptime")),
    ("channelName", tr("Echter Kanalname", "Real channel name")),
    ("sharedWith", tr("„Zusammen mit“ (Shared Chat)", "“Together with” (Shared Chat)")),
]
}
const BUNDLED_CHAT_FONTS: [(&str, &str); 7] = [
    ("Inter Variable", "Inter"),
    ("Roboto Variable", "Roboto"),
    ("Open Sans Variable", "Open Sans"),
    ("Nunito Variable", "Nunito"),
    ("Lexend Variable", "Lexend"),
    ("Atkinson Hyperlegible Next Variable", "Atkinson Hyperlegible"),
    ("JetBrains Mono Variable", "JetBrains Mono"),
];
/// Platzhalter im Auswahlfeld für „Standard“ (die Auswahl braucht einen nicht leeren Wert).
const DEFAULT_FONT: &str = "__default";

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
struct Separators {
    enabled: bool,
    thickness: u32,
}

/// CSS-Wert für font-family: gewählte Schrift, sonst und als Rückfall die Standardschrift.
fn font_family_value(family: &str) -> String {
    let name = family.trim();
    if name.is_empty() { "var(--font-sans)".into() } else { format!("\"{name}\", var(--font-sans)") }
}

fn is_valid_font_name(name: &str) -> bool {
    multiyap_core::settings::font_family_name().is_valid(&serde_json::Value::from(name))
}

thread_local! {
    /// Installierte Schriftfamilien, einmal pro Sitzung abgefragt (nur im Arbeitsspeicher, nicht gespeichert).
    static LOCAL_FAMILIES: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

/// Local Font Access API (MDN, geprüft 16.11.2026): nur in Chromium-Browsern, mit Erlaubnis und nach einem Klick.
fn query_local_fonts() -> Option<js_sys::Function> {
    let window = web_sys::window()?;
    js_sys::Reflect::get(&window, &"queryLocalFonts".into()).ok()?.dyn_into().ok()
}

/// Schriftart des Chats: mitgelieferte und auf dem PC installierte Schriften; jeder Eintrag zeigt sich in seiner
/// eigenen Schrift. Kann der Browser nicht auflisten, lässt sich der Name eintippen.
#[component]
fn ChatFontPicker() -> impl IntoView {
    let font = use_setting(CHAT_FONT, String::new());
    let families = RwSignal::new(LOCAL_FAMILIES.with(|cell| cell.borrow().clone()));
    let loading = RwSignal::new(false);
    let problem = RwSignal::new(query_local_fonts().is_none().then(|| tr("Dieser Browser kann die installierten Schriften nicht auflisten.", "This browser cannot list the installed fonts.").to_string()));
    let bundled = |name: &str| BUNDLED_CHAT_FONTS.iter().any(|(family, _)| *family == name);

    let load_local = move || {
        let Some(query) = query_local_fonts() else { return };
        loading.set(true);
        spawn_local(async move {
            let window = web_sys::window().map(JsValue::from).unwrap_or(JsValue::UNDEFINED);
            let result = match query.call0(&window).ok().and_then(|p| p.dyn_into::<js_sys::Promise>().ok()) {
                Some(promise) => wasm_bindgen_futures::JsFuture::from(promise).await,
                None => Err(JsValue::NULL),
            };
            match result {
                Ok(list) => {
                    let mut names: Vec<String> = js_sys::Array::from(&list)
                        .iter()
                        .filter_map(|entry| js_sys::Reflect::get(&entry, &"family".into()).ok()?.as_string())
                        .filter(|family| is_valid_font_name(family) && !BUNDLED_CHAT_FONTS.iter().any(|(b, _)| b == family))
                        .collect();
                    names.sort_by_key(|n| n.to_lowercase());
                    names.dedup();
                    LOCAL_FAMILIES.with(|cell| *cell.borrow_mut() = Some(names.clone()));
                    families.set(Some(names));
                    problem.set(None);
                }
                Err(error) => {
                    let denied = js_sys::Reflect::get(&error, &"name".into()).ok().and_then(|n| n.as_string()).as_deref() == Some("NotAllowedError");
                    problem.set(Some(if denied {
                        tr("Der Browser hat den Zugriff auf die Schriften abgelehnt.", "The browser denied access to the fonts.").into()
                    } else {
                        tr("Die installierten Schriften konnten nicht geladen werden.", "The installed fonts could not be loaded.").into()
                    }));
                }
            }
            loading.set(false);
        });
    };

    let current = Memo::new(move |_| font.get().trim().to_string());
    // Gewählte installierte Schrift auch zeigen, solange die Liste noch nicht geladen ist.
    let custom = Memo::new(move |_| {
        let c = current.get();
        (!c.is_empty() && !bundled(&c) && !families.get().unwrap_or_default().contains(&c)).then_some(c)
    });
    let entries = Signal::derive(move || {
        let mut bundled_items = vec![SelectEntry::item(DEFAULT_FONT, tr("Geist (Standard)", "Geist (default)")).style(format!("font-family: {}", font_family_value("")))];
        bundled_items.extend(BUNDLED_CHAT_FONTS.iter().map(|(family, label)| SelectEntry::item(*family, *label).style(format!("font-family: {}", font_family_value(family)))));
        let mut list = vec![SelectEntry::Group { label: tr("Mitgeliefert", "Included").into(), items: bundled_items }];
        let local = families.get().unwrap_or_default();
        if custom.get().is_some() || !local.is_empty() {
            let mut items: Vec<SelectEntry> = Vec::new();
            if let Some(c) = custom.get() {
                items.push(SelectEntry::item(c.clone(), c.clone()).style(format!("font-family: {}", font_family_value(&c))));
            }
            items.extend(local.into_iter().map(|f| SelectEntry::item(f.clone(), f.clone()).style(format!("font-family: {}", font_family_value(&f)))));
            list.push(SelectEntry::Separator);
            list.push(SelectEntry::Group { label: tr("Auf dem PC", "On this PC").into(), items });
        }
        list
    });

    view! {
        <SettingRow label=tr("Schriftart", "Font") description=tr("Nur für den Chat.", "Only for the chat.") html_for="chat-font">
            <Select
                id="chat-font"
                class="w-56"
                trigger_style=Signal::derive(move || format!("font-family: {}", font_family_value(&current.get())))
                value=Signal::derive(move || { let c = current.get(); if c.is_empty() { DEFAULT_FONT.to_string() } else { c } })
                entries=entries
                on_change=Callback::new(|value: String| save_setting(CHAT_FONT, &if value == DEFAULT_FONT { String::new() } else { value }))
            />
        </SettingRow>
        {move || match problem.get() {
            Some(text) => view! {
                <SettingRow label=tr("Installierte Schrift", "Installed font") description=multiyap_core::tf!("{text} Namen wie in Windows eintippen.", "{text} Type the name as shown in Windows.") html_for="chat-font-name">
                    <CommitInput
                        id="chat-font-name"
                        class="w-56"
                        value=Signal::derive(move || custom.get().unwrap_or_default())
                        max_length=100
                        placeholder=tr("zum Beispiel Segoe UI", "for example Segoe UI")
                        on_commit=Callback::new(|value: String| {
                            let name = value.trim().to_string();
                            if !is_valid_font_name(&name) {
                                return false;
                            }
                            save_setting(CHAT_FONT, &name);
                            true
                        })
                    />
                </SettingRow>
            }
            .into_any(),
            None => view! {
                <SettingRow
                    label=tr("Installierte Schriften", "Installed fonts")
                    description=families.get().map_or(tr("Der Browser fragt einmal nach Erlaubnis.", "The browser asks for permission once.").to_string(), |f| multiyap_core::tf!("{} Schriften gefunden, oben in der Auswahl.", "{} fonts found, at the top of the list.", f.len()))
                >
                    <Button variant="secondary" size="sm" disabled=loading on_click=Callback::new(move |_| load_local())>
                        {move || if families.get().is_some() { {tr("Neu laden", "Reload")} } else { {tr("Laden", "Load")} }}
                    </Button>
                </SettingRow>
            }
            .into_any(),
        }}
    }
}

fn sample(id: &str, name: &str, color: &str, text: &str, offset_minutes: f64, now: f64) -> ChatMessage {
    ChatMessage {
        kind: "message".into(),
        channel: ChannelRef { platform: "twitch".into(), id: "vorschau".into(), login: "vorschau".into(), display_name: tr("Vorschau", "Preview").into() },
        id: id.into(),
        sender: Sender { id: format!("u-{id}"), login: name.to_lowercase(), display_name: name.into(), color: Some(color.into()) },
        timestamp: now - offset_minutes * 60_000.0,
        text: text.into(),
        parts: vec![MessagePart::Text { text: text.into() }],
        badges: Vec::new(),
        flags: MessageFlags::default(),
        reply: None,
        source_channel: None,
    }
}

/// Live-Vorschau der Darstellung: echte Chatzeilen mit Beispieltext, die sofort auf Schriftgröße, Zeilenabstand,
/// Trennstriche, Zeilenhintergründe und lesbare Namen reagieren. Nicht bedienbar. „Nachtschwarz“ hat absichtlich eine
/// sehr dunkle Namensfarbe (lesbare Benutzernamen).
#[component]
fn ChatPreview() -> impl IntoView {
    let time_format = use_setting(TIME_FORMAT, "HH:MM".to_string());
    let removed = use_setting(REMOVED, RemovedDisplay::default());
    let readable = use_setting(READABLE, true);
    let now = js_sys::Date::now();
    let items = [
        sample("1", "Nachtschwarz", "#1b1b5e", tr("Guten Abend zusammen!", "Good evening, everyone!"), 4.0, now),
        sample("2", "Waldgruen", "#2e8b57", tr("Hallo, wie läuft der Stream heute?", "Hi, how is the stream going today?"), 3.0, now),
        sample("3", "Kirschrot", "#b22222", tr("Das war ein richtig guter Zug eben.", "That was a really good move just now."), 2.0, now),
        sample("4", "Himmelblau", "#5f9ea0", tr("Kann ich nur zustimmen, sehr stark gespielt.", "I can only agree, very well played."), 1.0, now),
    ];
    view! {
        <div class="pointer-events-none overflow-hidden rounded-lg border bg-background select-none" aria-hidden="true">
            {move || {
                let options = LineOptions { time_format: time_format.get(), removed_display: removed.get(), readable_names: readable.get(), mark_first_message: true, mark_links: true };
                items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| view! { <div data-chat-row="" data-odd=(index % 2 == 1).then_some("")>{message_line(item, &options)}</div> })
                    .collect_view()
            }}
        </div>
    }
}

fn size_entries(sizes: &[u32], current: u32) -> Vec<SelectEntry> {
    let mut all = sizes.to_vec();
    if !all.contains(&current) {
        all.push(current);
        all.sort_unstable();
    }
    all.into_iter().map(|s| SelectEntry::item(s.to_string(), format!("{s} px{}", if s == 14 { tr(" (Standard)", " (default)") } else { "" }))).collect()
}

fn number_change(key: &'static str) -> Callback<String> {
    Callback::new(move |value: String| {
        if let Ok(n) = value.parse::<u32>() {
            save_setting(key, &n);
        }
    })
}

#[component]
pub fn DisplaySection() -> impl IntoView {
    let time_format = use_setting(TIME_FORMAT, "HH:MM".to_string());
    let removed = use_setting(REMOVED, RemovedDisplay::default());
    let stream_info = use_setting(STREAM_INFO, serde_json::Map::<String, serde_json::Value>::new());
    let font_size = use_setting(FONT_SIZE, 14u32);
    let density = use_setting(DENSITY, "normal".to_string());
    let separators = use_setting(SEPARATORS, Separators { enabled: false, thickness: 1 });
    let zebra = use_setting(ZEBRA, false);
    let readable = use_setting(READABLE, true);
    let input_size = use_setting(INPUT_SIZE, "normal".to_string());
    let input_font_size = use_setting(INPUT_FONT_SIZE, 14u32);
    let language = use_setting(LANGUAGE, "auto".to_string());
    let update_removed = move |change: fn(&mut RemovedDisplay, bool)| {
        Callback::new(move |checked: bool| {
            let mut next = removed.get_untracked();
            change(&mut next, checked);
            save_setting(REMOVED, &next);
        })
    };

    view! {
        <div class="flex flex-col gap-8">
            <SettingGroup>
                <SettingRow label=tr("Sprache der Oberfläche", "Interface language") description=tr("Gilt sofort. „Wie Windows“ folgt der Sprache des Systems: Deutsch, sonst Englisch.", "Applies immediately. “Same as Windows” follows the system language: German, otherwise English.") html_for="app-language">
                    <Select id="app-language" class="w-48" value=language
                        entries=Signal::derive(|| vec![SelectEntry::item("auto", tr("Wie Windows", "Same as Windows")), SelectEntry::item("de", "Deutsch"), SelectEntry::item("en", "English")])
                        on_change=Callback::new(|value: String| save_setting(LANGUAGE, &value)) />
                </SettingRow>
            </SettingGroup>
            <SettingGroup title="Chat" description=tr("Wirkt sofort in Vorschau und allen Chats.", "Takes effect immediately in the preview and in all chats.")>
                <div class="py-3"><ChatPreview /></div>
                <SettingRow label=tr("Schriftgröße", "Font size") html_for="chat-font-size">
                    <Select id="chat-font-size" class="w-40" value=Signal::derive(move || font_size.get().to_string())
                        entries=Signal::derive(move || size_entries(&CHAT_FONT_SIZES, font_size.get())) on_change=number_change(FONT_SIZE) />
                </SettingRow>
                <ChatFontPicker />
                <SettingRow label=tr("Zeilenabstand", "Line spacing") html_for="chat-density">
                    <Select id="chat-density" class="w-32" value=density
                        entries=Signal::stored(vec![SelectEntry::item("normal", "Normal"), SelectEntry::item("compact", tr("Kompakt", "Compact"))])
                        on_change=Callback::new(|value: String| save_setting(DENSITY, &value)) />
                </SettingRow>
                <SettingRow label=tr("Trennstriche zwischen Nachrichten", "Separator lines between messages") html_for="chat-separators">
                    <div class="flex items-center gap-3">
                        <Select id="chat-separator-thickness" class="w-24" aria_label=tr("Dicke der Trennstriche", "Thickness of the separator lines")
                            value=Signal::derive(move || separators.get().thickness.to_string())
                            disabled=Signal::derive(move || !separators.get().enabled)
                            entries=Signal::stored(vec![SelectEntry::item("1", tr("Dünn", "Thin")), SelectEntry::item("2", tr("Dick", "Thick"))])
                            on_change=Callback::new(move |value: String| save_setting(SEPARATORS, &Separators { thickness: if value == "2" { 2 } else { 1 }, ..separators.get_untracked() })) />
                        <Switch id="chat-separators" checked=Signal::derive(move || separators.get().enabled)
                            on_change=Callback::new(move |enabled: bool| save_setting(SEPARATORS, &Separators { enabled, ..separators.get_untracked() })) />
                    </div>
                </SettingRow>
                <SettingRow label=tr("Abwechselnde Zeilenhintergründe", "Alternating row backgrounds") html_for="chat-zebra">
                    <Switch id="chat-zebra" checked=zebra on_change=Callback::new(|checked: bool| save_setting(ZEBRA, &checked)) />
                </SettingRow>
                <SettingRow label=tr("Lesbare Benutzernamen", "Readable user names") description=tr("Hellt zu dunkle Namensfarben auf, der Farbton bleibt.", "Brightens name colors that are too dark, the hue stays.") html_for="chat-readable-names">
                    <Switch id="chat-readable-names" checked=readable on_change=Callback::new(|checked: bool| save_setting(READABLE, &checked)) />
                </SettingRow>
                <SettingRow label=tr("Format der Uhrzeit", "Time format") html_for="time-format">
                    <Select id="time-format" class="w-40" value=time_format
                        entries=Signal::stored(vec![SelectEntry::item("HH:MM", "HH:MM"), SelectEntry::item("HH:MM:SS", "HH:MM:SS")])
                        on_change=Callback::new(|value: String| save_setting(TIME_FORMAT, &value)) />
                </SettingRow>
            </SettingGroup>

            <SettingGroup title="Tabs" description=tr("Stream-Info über dem Tab-Namen.", "Stream info above the tab name.")>
                {stream_info_labels().iter().map(|(key, label)| {
                    let key: &'static str = key;
                    view! {
                        <SettingRow label=multiyap_core::tf!("Stream-Info: {label}", "Stream info: {label}") html_for=format!("stream-info-{key}")>
                            <Switch id=format!("stream-info-{key}") checked=Signal::derive(move || stream_info.get().get(key).and_then(|v| v.as_bool()).unwrap_or(true))
                                on_change=Callback::new(move |checked: bool| {
                                    let mut next = stream_info.get_untracked();
                                    next.insert(key.into(), checked.into());
                                    save_setting(STREAM_INFO, &next);
                                }) />
                        </SettingRow>
                    }
                }).collect_view()}
            </SettingGroup>

            <SettingGroup title=tr("Eingabefeld", "Input field") description=tr("Für das Eingabefeld unter jedem Chat.", "For the input field below each chat.")>
                <SettingRow label=tr("Größe", "Size") html_for="input-size">
                    <Select id="input-size" class="w-40" value=input_size
                        entries=Signal::stored(input_sizes().iter().map(|(v, l)| SelectEntry::item(*v, *l)).collect::<Vec<_>>())
                        on_change=Callback::new(|value: String| save_setting(INPUT_SIZE, &value)) />
                </SettingRow>
                <SettingRow label=tr("Schriftgröße", "Font size") html_for="input-font-size">
                    // Ein früher gewählter größerer Wert bleibt wählbar, bis man ihn ändert.
                    <Select id="input-font-size" class="w-40" value=Signal::derive(move || input_font_size.get().to_string())
                        entries=Signal::derive(move || size_entries(&INPUT_FONT_SIZES, input_font_size.get())) on_change=number_change(INPUT_FONT_SIZE) />
                </SettingRow>
            </SettingGroup>

            <SettingGroup title=tr("Entfernte Nachrichten", "Removed messages") description=tr("Gelöschte, getimeoutete und gebannte Nachrichten (nur Mod-Kanäle).", "Deleted, timed out and banned messages (mod channels only).")>
                <SettingRow label=tr("Darstellung", "Appearance") html_for="removed-mode">
                    <Select id="removed-mode" class="w-40" value=Signal::derive(move || removed.get().mode)
                        entries=Signal::stored(vec![
                            SelectEntry::item("styled", tr("Hervorgehoben", "Highlighted")),
                            SelectEntry::item("normal", "Normal"),
                            SelectEntry::item("hidden", tr("Verborgen (Klick zeigt Text)", "Hidden (click shows text)")),
                        ])
                        on_change=Callback::new(move |mode: String| save_setting(REMOVED, &RemovedDisplay { mode, ..removed.get_untracked() })) />
                </SettingRow>
                {move || (removed.get().mode == "styled").then(|| view! {
                    <SettingRow label=tr("Durchgestrichen", "Struck through") html_for="removed-strike">
                        <Switch id="removed-strike" checked=Signal::derive(move || removed.get().strikethrough) on_change=update_removed(|r, v| r.strikethrough = v) />
                    </SettingRow>
                    <SettingRow label=tr("Ausgegraut", "Grayed out") html_for="removed-dim">
                        <Switch id="removed-dim" checked=Signal::derive(move || removed.get().dimmed) on_change=update_removed(|r, v| r.dimmed = v) />
                    </SettingRow>
                    <SettingRow label=tr("Kursiv", "Italic") html_for="removed-italic">
                        <Switch id="removed-italic" checked=Signal::derive(move || removed.get().italic) on_change=update_removed(|r, v| r.italic = v) />
                    </SettingRow>
                })}
            </SettingGroup>
        </div>
    }
}
