//! Chat-Liste in Leptos (Nachbau von chat-list.tsx, chat-scrollbar.tsx, chat-search-bar.tsx, use-hover-pause.ts).
//! Virtualisiert: nur sichtbare Zeilen liegen im DOM, ihre Höhe wird gemessen (ResizeObserver). Scrollt mit, solange
//! man unten ist; beim Hochscrollen erscheint „Neue Nachrichten“. Während der Chat pausiert ist, bleibt die Anzeige
//! stehen; neue Nachrichten laufen im Hintergrund auf. Beim Hochscrollen erscheinen ältere Verlaufszeilen, ohne dass
//! die Ansicht springt.
//!
//! Zeilen (JavaScript-Objekte der Sitzung, je Zeile zwischengespeichert) und Pause gibt der Tab mit
//! (crates/ui/src/tab_views.rs). Einstellungen, Mod-Rechte (zentrale Prüfung in crates/web/src/moderation.rs), Markierungen (Erwähnung, Filter; dieselbe
//! Erkennung wie die Benachrichtigungen) und Spam-Zähler (crates/core/src/spam.rs) bestimmt die Liste selbst.

use multiyap_core::i18n::tr;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use multiyap_core::model::{ChannelRef, Emote};
use leptos::prelude::*;
use send_wrapper::SendWrapper;
use serde::Deserialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{HtmlElement, KeyboardEvent};

use super::item::{ChatItem, ItemStore, Row};
use super::layer::{Layer, LayerView};
use super::pause::ChatPause;
use super::line::{chat_line, term_marker_class, ChatActions, LineEnv, LineInfo, LineOptions, RemovedDisplay, RowEnv, TermInfo};
use crate::bridge::{read_value, use_setting};
use crate::components::button::button_class;
use crate::components::icons;
use crate::components::input::INPUT_CLASS;
use crate::components::overlay::{window, WindowListeners};
use crate::settings::chat_behavior::{SpamSettings, SPAM_DEFAULT};
use crate::settings::model::{QuickButton, ReasonDialog};

/// Abstand zum unteren Rand, ab dem die Liste als „unten“ gilt und mitscrollt.
const BOTTOM_THRESHOLD_PX: f64 = 24.0;
/// Abstand zum oberen Rand, ab dem ältere Verlaufszeilen nachgeladen werden.
const TOP_THRESHOLD_PX: f64 = 80.0;
/// Geschätzte Höhe einer noch nicht gemessenen Zeile.
const ESTIMATE_PX: f64 = 28.0;
/// Zeilen über und unter dem sichtbaren Bereich, die schon im DOM liegen.
const OVERSCAN: usize = 12;
/// Mindesthöhe des Daumens der Scroll-Leiste, damit er greifbar bleibt.
const MIN_THUMB_PX: f64 = 24.0;
/// Striche auf der Scroll-Leiste, die näher beieinander liegen, werden zusammengefasst.
const MARKER_GAP_PX: f64 = 4.0;

/// Lage der Zeilen: Anfang jeder Zeile und Gesamthöhe.
#[derive(Default)]
struct Layout {
    starts: Vec<f64>,
    sizes: Vec<f64>,
    total: f64,
    index: HashMap<String, usize>,
}

impl Layout {
    fn start_of(&self, id: &str) -> f64 {
        self.index.get(id).map_or(0.0, |i| self.starts[*i])
    }

    /// Erste Zeile, die bei `y` (oder danach) endet.
    fn row_at(&self, y: f64) -> usize {
        let i = self.starts.partition_point(|start| *start <= y);
        i.saturating_sub(1)
    }

    /// Bildlaufposition, damit Zeile `index` mittig steht.
    fn scroll_to_center(&self, index: usize, viewport: f64) -> f64 {
        let (Some(start), Some(size)) = (self.starts.get(index), self.sizes.get(index)) else { return 0.0 };
        (start + size / 2.0 - viewport / 2.0).max(0.0)
    }
}

/// Strich auf der Scroll-Leiste: Erwähnung, Filter-Kategorie (abschaltbar) oder „Live ab hier“.
#[derive(Clone, PartialEq)]
struct Marker {
    index: usize,
    class: &'static str,
    preview: String,
}

fn short(text: &str) -> String {
    text.chars().take(80).collect()
}

/// Zwischenspeicher, die nur bei Bedarf neu gelesen werden.
#[derive(Default)]
struct Caches {
    store: ItemStore,
    heights: HashMap<String, f64>,
    info: HashMap<u32, LineInfo>,
    moderate: HashMap<String, bool>,
    labels: HashMap<String, Option<String>>,
    emotes: HashMap<String, Rc<HashMap<String, Emote>>>,
}

/// Markierung einer Nachricht: Erwähnung, sonst die Filter-Kategorie, nur wo Filter an sind und ohne die Kategorien,
/// von denen die Person ausgenommen ist.
fn line_info(js: &JsValue, is_moderator: bool) -> LineInfo {
    let Some(message) = read_value::<serde_json::Value>(js) else { return LineInfo::default() };
    let matcher = multiyap_web::notifications::matcher();
    let mention = matcher.is_mention(&message);
    let term = if mention { None } else { matcher.term_category(&message, is_moderator) }.map(|category| {
        let text = |name: &str| category.get(name).and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
        TermInfo { name: text("name"), color: text("color"), scroll_marker: category.get("scrollMarker").and_then(serde_json::Value::as_bool).unwrap_or(true) }
    });
    LineInfo { mention, term }
}

impl Caches {
    fn info(&mut self, row: &Row) -> LineInfo {
        if let Some(info) = self.info.get(&row.key) {
            return info.clone();
        }
        let info = match &*row.item {
            ChatItem::Message(m) => {
                let is_moderator = self.can_moderate(&m.channel.id);
                line_info(&row.js, is_moderator)
            }
            _ => LineInfo::default(),
        };
        self.info.insert(row.key, info.clone());
        info
    }

    /// Mod-Rechte über die zentrale Prüfung (zwischengespeichert bis zur nächsten Änderung).
    fn can_moderate(&mut self, channel_id: &str) -> bool {
        if let Some(known) = self.moderate.get(channel_id) {
            return *known;
        }
        let value = multiyap_web::moderation::can_moderate(channel_id);
        self.moderate.insert(channel_id.to_string(), value);
        value
    }

    /// Erkennung oder Mod-Rechte haben sich geändert.
    fn forget_rules(&mut self) {
        self.info.clear();
        self.moderate.clear();
    }

    fn channel_label(&mut self, label_of: Option<fn(&ChannelRef) -> String>, row: &Row) -> Option<String> {
        let id = row.item.channel().id.clone();
        if let Some(known) = self.labels.get(&id) {
            return known.clone();
        }
        let label = label_of.map(|f| f(row.item.channel()));
        self.labels.insert(id, label.clone());
        label
    }

    /// Emotes von 7TV, BTTV und FFZ für den Kanal.
    fn emotes(&mut self, channel_id: &str) -> Rc<HashMap<String, Emote>> {
        if let Some(known) = self.emotes.get(channel_id) {
            return known.clone();
        }
        let list = multiyap_web::emotes::third_party_emotes(channel_id);
        let map = Rc::new(list.iter().map(|e| (e.name.clone(), e.clone())).collect::<HashMap<_, _>>());
        self.emotes.insert(channel_id.to_string(), map.clone());
        map
    }

}

/// Zeile unter der Maus für die Mod-Tastenkürzel: Ziel wie bei den Schnellbuttons (keine eigenen Nachrichten, nicht
/// Broadcaster, Mods und Lead Mods; AutoMod-Zeilen ohne Nachricht im Chat). Die Mod-Rechte prüft crates/web beim Drücken.
fn hovered_of(item: &ChatItem) -> multiyap_web::mod_actions::Hovered {
    use multiyap_core::chat_text::is_protected_sender;
    let target = match item {
        ChatItem::Message(m) if m.flags.own != Some(true) && !is_protected_sender(&m.badges) => Some(multiyap_web::mod_actions::ModTarget {
            channel_id: m.channel.id.clone(),
            user_id: m.sender.id.clone(),
            display_name: m.sender.display_name.clone(),
            message_id: Some(m.id.clone()),
        }),
        ChatItem::AutoMod(a) => Some(multiyap_web::mod_actions::ModTarget {
            channel_id: a.channel.id.clone(),
            user_id: a.sender.id.clone(),
            display_name: a.sender.display_name.clone(),
            message_id: None,
        }),
        _ => None,
    };
    multiyap_web::mod_actions::Hovered { item_id: item.id().to_string(), target }
}

/// Zustand der Suche eines Tabs (crates/web/src/chat_tools.rs).
#[derive(Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchState {
    open: bool,
    query: String,
    focus_token: f64,
}

fn normalize_query(query: &str) -> String {
    query.trim().trim_start_matches('@').to_lowercase()
}

/// Passt die Zeile? Nachrichten und AutoMod-Zeilen nach Text, Anzeigename oder Login.
fn matches_search(item: &ChatItem, query: &str) -> bool {
    let (text, name, login) = match item {
        ChatItem::Message(m) => (&m.text, &m.sender.display_name, &m.sender.login),
        ChatItem::AutoMod(a) => (&a.text, &a.sender.display_name, &a.sender.login),
        _ => return false,
    };
    text.to_lowercase().contains(query) || name.to_lowercase().contains(query) || login.to_lowercase().contains(query)
}

/// Hält den Chat an, solange gesetzt; mehrere Auslöser zählt die Pause des Tabs.
#[derive(Clone, Copy)]
struct HoldSlot(StoredValue<Option<js_sys::Function>, LocalStorage>);

impl HoldSlot {
    fn new() -> Self {
        HoldSlot(StoredValue::new_local(None))
    }

    fn set(&self, actions: ChatActions, on: bool) {
        if on {
            if self.0.with_value(Option::is_none) {
                let release = actions.hold().dyn_into::<js_sys::Function>().ok();
                self.0.set_value(release);
            }
        } else if let Some(release) = self.0.try_update_value(Option::take).flatten() {
            let _ = release.call0(&JsValue::NULL);
        }
    }
}

const SCROLL_CLASS: &str = "scrollbar-none min-h-0 flex-1 overflow-y-auto pr-3";
/// Badge „Chat pausiert“: secondary, größer als ein normales Badge.
const PAUSE_BADGE: &str = "group/badge inline-flex h-7 w-fit shrink-0 items-center justify-center gap-1.5 overflow-hidden rounded-4xl border border-transparent px-3 py-0.5 text-sm font-medium whitespace-nowrap transition-all focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 aria-invalid:border-destructive aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 [&>svg]:pointer-events-none [&>svg]:size-4! bg-secondary text-secondary-foreground [a]:hover:bg-secondary/80 shadow-md";

/// Chat-Liste eines Tabs. `items`: Zeilen als JavaScript-Array, älteste zuerst. `channel_label`: Kanal und Plattform
/// vor der Uhrzeit jeder Zeile (Hervorhebungen-Tab). `on_item_click`: Klick auf eine Zeile außerhalb von Knöpfen, Namen
/// und Links, mit Login des Kanals und ID der Zeile. `search_tab_id`: Tab, zu dem die Suche (Strg+F) gehört.
#[component]
pub fn ChatList(
    items: Signal<JsValue, LocalStorage>,
    #[prop(into)] has_older: Signal<bool>,
    #[prop(optional)] load_older: Option<Callback<()>>,
    #[prop(optional)] channel_label: Option<fn(&ChannelRef) -> String>,
    #[prop(optional)] on_item_click: Option<Callback<(String, String)>>,
    search_tab_id: String,
    pause: ChatPause,
) -> impl IntoView {
    let layer = Layer::new();
    let actions = ChatActions { layer, pause: Some(pause) };
    let caches = StoredValue::new_local(RefCell::new(Caches::default()));
    let shown = RwSignal::new_local(Rc::new(Vec::<Row>::new()));
    let frozen = StoredValue::new_local(Rc::new(Vec::<Row>::new()));
    let all_rows = RwSignal::new_local(Rc::new(Vec::<Row>::new()));
    let spam_counts = RwSignal::new_local(Rc::new(HashMap::<String, u32>::new()));
    let epoch = RwSignal::new(0u32);
    // Erkennung (Einstellungen, Anmeldung) oder Mod-Rechte geändert: Markierungen und Rechte neu bestimmen.
    let rules = RwSignal::new(0u32);
    let on_rules: Rc<dyn Fn()> = Rc::new(move || {
        let _ = caches.try_with_value(|c| c.borrow_mut().forget_rules());
        rules.try_update(|r| *r = r.wrapping_add(1));
        epoch.try_update(|e| *e = e.wrapping_add(1));
    });
    let matcher_listener = multiyap_web::notifications::subscribe_matcher(on_rules.clone());
    let mod_listener = multiyap_web::moderation::subscribe_mod(on_rules);
    on_cleanup(move || {
        multiyap_web::notifications::unsubscribe_matcher(matcher_listener);
        multiyap_web::moderation::unsubscribe_mod(mod_listener);
    });
    // Neu zeichnen, sobald Drittanbieter-Emotes, Bits-Bilder oder 7TV-Kosmetik eintreffen.
    let on_images: Rc<dyn Fn()> = Rc::new(move || {
        let _ = caches.try_with_value(|c| c.borrow_mut().emotes.clear());
        epoch.try_update(|e| *e = e.wrapping_add(1));
    });
    let image_listeners = (
        multiyap_web::emotes::subscribe_third_party(on_images.clone()),
        multiyap_web::emotes::subscribe_cheermotes(on_images.clone()),
        multiyap_web::seventv::subscribe(on_images),
    );
    on_cleanup(move || {
        multiyap_web::emotes::unsubscribe_third_party(image_listeners.0);
        multiyap_web::emotes::unsubscribe_cheermotes(image_listeners.1);
        multiyap_web::seventv::unsubscribe(image_listeners.2);
    });
    let layout = RwSignal::new_local(Rc::new(Layout::default()));
    let measured = RwSignal::new(0u32);
    let markers = RwSignal::new(Vec::<Marker>::new());
    let scroll_top = RwSignal::new(0.0f64);
    let viewport = RwSignal::new(0.0f64);
    let at_bottom = RwSignal::new(true);
    let last_seen = RwSignal::new(None::<String>);
    let flash = RwSignal::new(None::<String>);
    let paused = RwSignal::new(false);
    // Zeile, die beim Nachladen oben stand, und ihr Abstand zum oberen Rand.
    let anchor = StoredValue::new(None::<(String, f64)>);
    let scroll_ref = NodeRef::<leptos::html::Div>::new();
    let inner_ref = NodeRef::<leptos::html::Div>::new();

    // Einstellungen für alle Zeilen.
    let time_format = use_setting("chat.timeFormat", "HH:MM".to_string());
    let removed_display = use_setting("chat.removedDisplay", RemovedDisplay::default());
    let readable_names = use_setting("display.readableNames", true);
    let reply_button = use_setting("chat.replyButton", true);
    let mark_first_message = use_setting("chat.markFirstMessage", true);
    let mark_links = use_setting("chat.markLinks", true);
    let quick_in_chat = use_setting("moderation.quickButtonsInChat", true);
    let quick_buttons = use_setting("moderation.quickButtons", Vec::<QuickButton>::new());
    let reason_dialog = use_setting("moderation.reasonDialog", ReasonDialog { timeout: false, ban: false });
    let pause_trigger = use_setting("chat.pauseTrigger", "hover".to_string());
    let pause_max = use_setting("chat.pauseMaxSeconds", 0.0f64);
    let spam_settings = use_setting::<SpamSettings>("spam.settings", SPAM_DEFAULT);

    let env = StoredValue::new_local(None::<LineEnv>);
    Effect::new(move |_| {
        let highlight_at = f64::from(spam_settings.get().highlight_at);
        env.set_value(Some(LineEnv {
            options: LineOptions { time_format: time_format.get(), removed_display: removed_display.get(), readable_names: readable_names.get(), mark_first_message: mark_first_message.get(), mark_links: mark_links.get() },
            reply_button: reply_button.get(),
            quick_buttons_in_chat: quick_in_chat.get(),
            quick_buttons: Rc::new(quick_buttons.get()),
            reason_dialog: reason_dialog.get(),
            actions: Some(actions),
            spam_counts: Some(spam_counts),
            spam_highlight_at: highlight_at,
        }));
        epoch.update(|e| *e += 1);
    });

    let element = move || scroll_ref.get_untracked().map(HtmlElement::from);

    let scroll_to = move |index: usize| {
        let Some(el) = element() else { return };
        let target = layout.with_untracked(|l| l.scroll_to_center(index, f64::from(el.client_height())));
        el.set_scroll_top(target.round() as i32);
    };

    // Lage berechnen; unten bleiben, wenn man unten ist, und nach dem Nachladen die alte Zeile oben halten.
    Effect::new(move |_| {
        measured.track();
        let rows = shown.get();
        let next = caches.with_value(|c| {
            let c = c.borrow();
            let mut layout = Layout { starts: Vec::with_capacity(rows.len()), sizes: Vec::with_capacity(rows.len()), total: 0.0, index: HashMap::with_capacity(rows.len()) };
            for (i, row) in rows.iter().enumerate() {
                let id = row.item.id();
                let size = c.heights.get(id).copied().unwrap_or(ESTIMATE_PX);
                layout.starts.push(layout.total);
                layout.sizes.push(size);
                layout.index.insert(id.to_string(), i);
                layout.total += size;
            }
            layout
        });
        let total = next.total;
        let anchor_target = anchor.try_update_value(Option::take).flatten().and_then(|(id, delta)| next.index.get(&id).map(|i| next.starts[*i] + delta));
        layout.set(Rc::new(next));
        let Some(el) = element() else { return };
        if let Some(inner) = inner_ref.get_untracked() {
            let _ = HtmlElement::from(inner).style().set_property("height", &format!("{total}px"));
        }
        if let Some(top) = anchor_target {
            el.set_scroll_top(top.round() as i32);
        } else if at_bottom.get_untracked() && !rows.is_empty() {
            el.set_scroll_top(el.scroll_height());
        }
    });

    // Spam-Erkennung: nur Nachrichten anderer aus Mod-Kanälen, rein visuell.
    Effect::new(move |_| {
        rules.track();
        let settings = spam_settings.get();
        let rows = all_rows.get();
        let matcher = multiyap_web::notifications::matcher();
        let counts = caches.with_value(|c| {
            let mut c = c.borrow_mut();
            let messages: Vec<_> = rows
                .iter()
                .filter_map(|row| match &*row.item {
                    ChatItem::Message(m) if m.flags.own != Some(true) && c.can_moderate(&m.channel.id) => Some(m),
                    _ => None,
                })
                // Ausgenommene Personen (Filter-Ausnahmen) zählen nicht.
                .filter(|m| {
                    let badges: Vec<&str> = m.badges.iter().map(|b| b.set_id.as_str()).collect();
                    !matcher.is_spam_exempt(&m.sender.id, &m.channel.id, &badges)
                })
                .collect();
            let keys: Vec<String> = messages.iter().map(|m| format!("{}:{}", m.channel.id, m.sender.id)).collect();
            let entries: Vec<multiyap_core::spam::SpamEntry<'_>> = messages
                .iter()
                .zip(&keys)
                .map(|(m, key)| multiyap_core::spam::SpamEntry { key, timestamp: m.timestamp, text: &m.text })
                .collect();
            let found = multiyap_core::spam::spam_counts(&entries, settings.core());
            messages.iter().zip(found).filter(|(_, count)| *count > 0).map(|(m, count)| (m.id.clone(), count)).collect::<HashMap<_, _>>()
        });
        if spam_counts.with_untracked(|old| **old != counts) {
            spam_counts.set(Rc::new(counts));
        }
    });

    // Zeilen lesen, Pause anwenden, Markierungen sammeln.
    Effect::new(move |_| {
        let array = items.get();
        rules.track();
        let rows = Rc::new(caches.with_value(|c| c.borrow().store.rows(&array)));
        let is_paused = pause.paused();
        paused.set(is_paused);
        all_rows.set(rows.clone());
        // Pause hält nur neue Zeilen am Ende zurück. Nachgeladene ältere Zeilen und Änderungen (zum Beispiel
        // „gelöscht von …“) erscheinen weiter, damit Hochscrollen auch pausiert funktioniert.
        let visible = if is_paused {
            let old = frozen.get_value();
            match old.last().map(|r| r.item.id().to_string()) {
                Some(last) => match rows.iter().position(|r| r.item.id() == last) {
                    Some(i) => Rc::new(rows[..=i].to_vec()),
                    None => old,
                },
                None => old,
            }
        } else {
            rows
        };
        frozen.set_value(visible.clone());
        // Striche für Erwähnungen und Begriffe in der Farbe ihrer Kategorie, diese abschaltbar (Abschnitt 6.2).
        let found = caches.with_value(|c| {
            let mut c = c.borrow_mut();
            visible
                .iter()
                .enumerate()
                .filter_map(|(index, row)| match &*row.item {
                    ChatItem::Divider(d) if d.variant == "live" => Some(Marker { index, class: "bg-live-marker", preview: d.label.clone() }),
                    ChatItem::Message(m) => {
                        let info = c.info(row);
                        if info.mention {
                            Some(Marker { index, class: "bg-mention", preview: multiyap_core::tf!("Erwähnung: {}: {}", "Mention: {}: {}", m.sender.display_name, short(&m.text)) })
                        } else {
                            info.term.filter(|t| t.scroll_marker).map(|t| Marker {
                                index,
                                class: term_marker_class(&t.color),
                                preview: format!("{}: {}: {}", t.name, m.sender.display_name, short(&m.text)),
                            })
                        }
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        });
        if markers.with_untracked(|m| *m != found) {
            markers.set(found);
        }
        shown.set(visible);
    });

    // Höhen messen (eine Beobachtung für alle Zeilen) und die Größe des Bildlaufbereichs verfolgen.
    let observer = StoredValue::new_local(None::<web_sys::ResizeObserver>);
    // Oberste sichtbare Zeile und Abstand beim Ausblenden des Tabs.
    let hidden_at = StoredValue::new(None::<(String, f64)>);
    {
        let callback = Closure::<dyn FnMut(js_sys::Array)>::new(move |entries: js_sys::Array| {
            let mut changed = false;
            let mut shift = 0.0;
            // Echte Bildlaufposition: Nach dem Nachladen ist sie schon gesetzt, bevor das Scroll-Ereignis eintrifft.
            let top = element().map_or_else(|| scroll_top.get_untracked(), |el| f64::from(el.scroll_top()));
            for entry in entries.iter() {
                let Ok(entry) = entry.dyn_into::<web_sys::ResizeObserverEntry>() else { continue };
                let target = entry.target();
                if element().is_some_and(|el| AsRef::<web_sys::Element>::as_ref(&el) == &target) {
                    let (before, height) = (viewport.get_untracked(), f64::from(target.client_height()));
                    viewport.set(height);
                    // Tab wird ausgeblendet: Der Browser setzt die Position zurück, darum die oberste sichtbare Zeile
                    // und ihren Abstand merken (die letzte echte Position steht noch in scroll_top).
                    if before >= 1.0 && height < 1.0 {
                        let top = scroll_top.get_untracked();
                        let kept = layout.with_untracked(|l| {
                            let index = l.row_at(top);
                            shown.with_untracked(|s| s.get(index).map(|r| (r.item.id().to_string(), top - l.starts.get(index).copied().unwrap_or(0.0))))
                        });
                        hidden_at.set_value(kept);
                    }
                    // Tab wird wieder sichtbar: dort weiter, wo man ihn verlassen hat. War man unten, ganz unten bei
                    // den neuesten Nachrichten; ist die gemerkte Zeile nicht mehr da, ebenfalls.
                    if before < 1.0 && height >= 1.0 {
                        let kept = hidden_at.try_update_value(Option::take).flatten();
                        let restored = (!at_bottom.get_untracked())
                            .then(|| kept.and_then(|(id, delta)| layout.with_untracked(|l| l.index.get(&id).map(|i| l.starts[*i] + delta))))
                            .flatten();
                        match restored {
                            Some(top) => {
                                target.set_scroll_top(top.round() as i32);
                                scroll_top.set(top);
                            }
                            None => {
                                at_bottom.set(true);
                                last_seen.set(None);
                                target.set_scroll_top(target.scroll_height());
                            }
                        }
                    }
                    continue;
                }
                // Ausgeblendete Zeilen haben die Höhe 0; gemessen wird erst wieder, wenn der Tab sichtbar ist.
                if !target.is_connected() || element().is_some_and(|el| el.client_height() == 0) {
                    continue;
                }
                let Some(id) = target.get_attribute("data-item-id") else { continue };
                let height = target.get_bounding_client_rect().height();
                let old = caches.with_value(|c| c.borrow_mut().heights.insert(id.clone(), height));
                let before = old.unwrap_or(ESTIMATE_PX);
                if (before - height).abs() > 0.5 {
                    changed = true;
                    // Zeilen über dem sichtbaren Bereich ändern ihre Höhe: Ansicht festhalten.
                    if layout.with_untracked(|l| l.start_of(&id)) < top {
                        shift += height - before;
                    }
                }
            }
            if changed {
                if shift != 0.0 && !at_bottom.get_untracked() {
                    if let Some(el) = element() {
                        el.set_scroll_top((f64::from(el.scroll_top()) + shift).round() as i32);
                    }
                }
                measured.update(|m| *m = m.wrapping_add(1));
            }
        });
        if let Ok(watcher) = web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref()) {
            observer.set_value(Some(watcher));
        }
        callback.forget();
    }
    Effect::new(move |_| {
        if let (Some(el), Some(watcher)) = (scroll_ref.get(), observer.get_value()) {
            watcher.observe(&el);
        }
    });
    on_cleanup(move || {
        if let Some(watcher) = observer.try_get_value().flatten() {
            watcher.disconnect();
        }
    });

    // Bildlauf: unten oder nicht, ältere Zeilen nachladen.
    let on_scroll = move |_| {
        let Some(el) = element() else { return };
        // Ausgeblendeter Tab: Der Browser setzt die Position zurück; das ist kein Hochscrollen.
        if el.client_height() == 0 {
            return;
        }
        let top = f64::from(el.scroll_top());
        scroll_top.set(top);
        let bottom = f64::from(el.scroll_height()) - top - f64::from(el.client_height()) <= BOTTOM_THRESHOLD_PX;
        if bottom != at_bottom.get_untracked() {
            at_bottom.set(bottom);
            last_seen.set(if bottom { None } else { shown.with_untracked(|s| s.last().map(|r| r.item.id().to_string())) });
        }
        if top < TOP_THRESHOLD_PX && has_older.get_untracked() && anchor.with_value(Option::is_none) {
            let first = layout.with_untracked(|l| {
                let index = l.row_at(top);
                shown.with_untracked(|s| s.get(index).map(|r| (r.item.id().to_string(), top - l.starts.get(index).copied().unwrap_or(0.0))))
            });
            anchor.set_value(first);
            if let Some(load) = load_older {
                load.run(());
            }
        }
    };

    // Sichtbarer Ausschnitt mit Puffer.
    let range = Memo::new(move |_| {
        let (top, height) = (scroll_top.get(), viewport.get().max(1.0));
        layout.with(|l| {
            if l.starts.is_empty() {
                return (0, 0);
            }
            let first = l.row_at(top);
            let last = l.row_at(top + height);
            (first.saturating_sub(OVERSCAN), (last + OVERSCAN + 1).min(l.starts.len()))
        })
    });

    // ---- Pause: Menüs und Fenster, Maus über dem Chat ----
    let overlay_hold = HoldSlot::new();
    Effect::new(move |_| overlay_hold.set(actions, layer.holding()));
    let hovered = RwSignal::new(false);
    let expired = RwSignal::new(false);
    let alt = RwSignal::new(false);
    let alt_listeners = WindowListeners::new();
    for kind in ["keydown", "keyup"] {
        alt_listeners.add(kind, Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| alt.set(event.alt_key())).into_js_value());
    }
    // Fenster verlassen (zum Beispiel Alt+Tab): Taste gilt als losgelassen.
    alt_listeners.add(
        "blur",
        Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            if event.target().is_some_and(|t| t.dyn_ref::<web_sys::Window>().is_some()) {
                alt.set(false);
            }
        })
        .into_js_value(),
    );
    on_cleanup({
        let alt_listeners = alt_listeners.clone();
        move || alt_listeners.clear()
    });
    let hover_hold = HoldSlot::new();
    let timer = StoredValue::new_local(None::<TimeoutHandle>);
    Effect::new(move |_| {
        let wanted = !expired.get()
            && match pause_trigger.get().as_str() {
                "hover" => hovered.get(),
                "alt-hover" => hovered.get() && alt.get(),
                _ => false,
            };
        if let Some(handle) = timer.try_update_value(Option::take).flatten() {
            handle.clear();
        }
        hover_hold.set(actions, wanted);
        let max = pause_max.get();
        if wanted && max > 0.0 {
            // Nach der Höchstdauer läuft der Chat weiter, bis die Maus ihn einmal verlassen hat.
            let handle = set_timeout_with_handle(move || expired.set(true), std::time::Duration::from_secs_f64(max)).ok();
            timer.set_value(handle);
        }
    });
    on_cleanup(move || {
        overlay_hold.set(actions, false);
        hover_hold.set(actions, false);
    });

    // ---- Sprung aus dem Hervorhebungen-Tab: Zeile suchen, hinscrollen, kurz hervorheben ----
    let flash_timer = StoredValue::new_local(None::<TimeoutHandle>);
    let jump: std::rc::Rc<dyn Fn(&str) -> bool> = std::rc::Rc::new(move |id: &str| {
        let id = id.to_string();
        let Some(index) = shown.with_untracked(|s| s.iter().position(|r| r.item.id() == id)) else { return false };
        at_bottom.set(false);
        // Erst nach dem Sichtbarwerden des Tabs messen und scrollen.
        request_animation_frame(move || scroll_to(index));
        flash.set(Some(id));
        if let Some(handle) = flash_timer.try_update_value(Option::take).flatten() {
            handle.clear();
        }
        flash_timer.set_value(set_timeout_with_handle(move || flash.set(None), std::time::Duration::from_secs(2)).ok());
        true
    });
    let jump_id = multiyap_web::chat_tools::on_jump_request(jump);
    on_cleanup(move || multiyap_web::chat_tools::remove_jump_handler(jump_id));

    // ---- Suche ----
    let tab_id = Memo::new(move |_| Some(search_tab_id.clone()));
    let search = RwSignal::new(SearchState::default());
    let read_search = move || {
        let state = tab_id.get_untracked().and_then(|id| crate::bridge::convert::<SearchState>(&multiyap_web::chat_tools::search_state(&id))).unwrap_or_default();
        if search.with_untracked(|s| *s != state) {
            search.set(state);
        }
    };
    Effect::new(move |_| {
        tab_id.track();
        read_search();
    });
    let search_id = multiyap_web::chat_tools::subscribe_search(std::rc::Rc::new(read_search));
    on_cleanup(move || multiyap_web::chat_tools::unsubscribe_search(search_id));
    let hits = Memo::new(move |_| {
        let (open, query) = search.with(|s| (s.open, normalize_query(&s.query)));
        if !open || query.is_empty() {
            return Vec::<String>::new();
        }
        shown.with(|rows| rows.iter().filter(|r| matches_search(&r.item, &query)).map(|r| r.item.id().to_string()).collect())
    });
    let search_open = Memo::new(move |_| search.with(|s| s.open));
    let current_hit = RwSignal::new(None::<String>);
    let go_to_hit = move |id: String| {
        let Some(index) = layout.with_untracked(|l| l.index.get(&id).copied()) else { return };
        current_hit.set(Some(id));
        at_bottom.set(false);
        scroll_to(index);
    };
    // Neuer Suchtext: zum neuesten Treffer springen (nicht bei jeder neuen Zeile).
    let search_key = Memo::new(move |_| search.with(|s| (s.open, s.query.clone())));
    Effect::new(move |_| {
        search_key.track();
        match hits.get_untracked().last().cloned() {
            Some(id) => go_to_hit(id),
            None => current_hit.set(None),
        }
    });
    let hit_index = Memo::new(move |_| current_hit.get().and_then(|id| hits.with(|h| h.iter().position(|x| *x == id))));
    let step_hit = move |delta: isize| {
        let list = hits.get_untracked();
        if list.is_empty() {
            return;
        }
        // Ohne gültigen aktuellen Treffer beim neuesten anfangen.
        let from = hit_index.get_untracked().map_or(list.len() as isize, |i| i as isize);
        let next = (from + delta).clamp(0, list.len() as isize - 1) as usize;
        go_to_hit(list[next].clone());
    };

    // ---- Zähler ----
    // Neue Nachrichten seit dem Hochscrollen.
    let new_count = Memo::new(move |_| {
        if at_bottom.get() {
            return 0;
        }
        let Some(last) = last_seen.get() else { return 0 };
        shown.with(|rows| {
            let start = rows.iter().position(|r| r.item.id() == last).map_or(0, |i| i + 1);
            rows[start..].iter().filter(|r| matches!(&*r.item, ChatItem::Message(_))).count()
        })
    });
    // Während der Pause aufgelaufene Nachrichten (nichts geht verloren, sie erscheinen danach).
    let paused_count = Memo::new(move |_| {
        if !paused.get() {
            return 0;
        }
        let Some(last) = shown.with(|s| s.last().map(|r| r.item.id().to_string())) else { return 0 };
        all_rows.with(|rows| {
            let start = rows.iter().position(|r| r.item.id() == last).map_or(0, |i| i + 1);
            rows[start..].iter().filter(|r| matches!(&*r.item, ChatItem::Message(_))).count()
        })
    });
    let jump_to_end = move |_| {
        at_bottom.set(true);
        last_seen.set(None);
        if let Some(el) = element() {
            el.set_scroll_top(el.scroll_height());
        }
    };

    // ---- Zeilen ----
    let clickable = on_item_click.is_some();
    let visible_rows = move || {
        let (start, end) = range.get();
        let ep = epoch.get();
        // Zeilen enthalten JavaScript-Werte; im Browser gibt es nur einen Faden.
        shown.with(|rows| rows.get(start..end).map(|slice| slice.iter().map(|r| SendWrapper::new((r.clone(), ep))).collect::<Vec<_>>()).unwrap_or_default())
    };
    let row_key = |entry: &SendWrapper<(Row, u32)>| (entry.0.key, entry.1);
    let render_row = move |entry: SendWrapper<(Row, u32)>| {
        let (row, _) = entry.take();
        let id = row.item.id().to_string();
        let node = NodeRef::<leptos::html::Div>::new();
        let observed = StoredValue::new_local(None::<web_sys::Element>);
        Effect::new(move |_| {
            if let (Some(el), Some(watcher)) = (node.get(), observer.get_value()) {
                let el: web_sys::Element = el.into();
                watcher.observe(&el);
                observed.set_value(Some(el));
            }
        });
        on_cleanup(move || {
            if let (Some(el), Some(watcher)) = (observed.try_update_value(Option::take).flatten(), observer.try_get_value().flatten()) {
                watcher.unobserve(&el);
            }
        });
        let line = env.with_value(|env| {
            let env = env.clone()?;
            let row_env = caches.with_value(|c| {
                let mut c = c.borrow_mut();
                let channel = row.item.channel();
                RowEnv {
                    can_moderate: c.can_moderate(&channel.id),
                    channel_label: c.channel_label(channel_label, &row),
                    info: c.info(&row),
                    emotes: Some(c.emotes(&channel.id)),
                }
            });
            chat_line(&row, &env, &row_env)
        });
        let index_id = id.clone();
        let index = Memo::new(move |_| layout.with(|l| l.index.get(&index_id).copied()));
        let top_id = id.clone();
        let flash_id = id.clone();
        let hit_id = id.clone();
        let click_target = (row.item.channel().login.clone(), id.clone());
        let hovered = hovered_of(&row.item);
        let leave_id = id.clone();
        view! {
            <div node_ref=node data-chat-row="" data-item-id=id.clone() data-index=move || index.get().map(|i| i.to_string())
                // Für Trennstriche und abwechselnde Hintergründe (Darstellung, theme.css).
                data-odd=move || index.get().is_some_and(|i| i % 2 == 1).then_some("")
                class=move || {
                    let mut class = String::from("absolute top-0 left-0 flex w-full items-start transition-colors duration-500");
                    if clickable {
                        class.push_str(" cursor-pointer hover:bg-muted/40");
                    }
                    if flash.with(|f| f.as_deref() == Some(flash_id.as_str())) {
                        class.push_str(" bg-primary/20");
                    }
                    if hits.with(|h| h.contains(&hit_id)) {
                        class.push_str(if current_hit.with(|c| c.as_deref() == Some(hit_id.as_str())) { " bg-primary/25" } else { " bg-primary/10" });
                    }
                    class
                }
                style:transform=move || format!("translateY({}px)", layout.with(|l| l.start_of(&top_id)))
                // Zeile unter der Maus für die Tastenkürzel.
                on:pointerenter=move |_| multiyap_web::mod_actions::set_hovered(hovered.clone())
                on:pointerleave=move |_| multiyap_web::mod_actions::clear_hovered(&leave_id)
                on:click=move |event: web_sys::MouseEvent| {
                    let Some(handler) = on_item_click else { return };
                    // Knöpfe, Namen, Links und Eingaben behalten ihre eigene Wirkung; markierter Text wird kein Klick.
                    let on_control = event.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                        .is_some_and(|t| t.closest("button, a, input, textarea, [role=\"button\"], [role=\"menuitem\"]").ok().flatten().is_some());
                    let selecting = window().get_selection().ok().flatten().and_then(|s| s.to_string().as_string()).is_some_and(|s| !s.is_empty());
                    if !on_control && !selecting {
                        handler.run(click_target.clone());
                    }
                }>
                <div class="min-w-0 flex-1">{line}</div>
            </div>
        }
    };

    view! {
        <div class="flex min-h-0 flex-1 flex-col">
            // Nur Öffnen und Schließen zeichnen die Leiste neu, nicht jeder Tastendruck.
            {move || search_open.get().then(|| tab_id.get().map(|tab| view! {
                <SearchBar tab=tab search=search position=Signal::derive(move || hit_index.get().map_or(0, |i| i + 1)) total=Signal::derive(move || hits.with(Vec::len))
                    on_older=Callback::new(move |_| step_hit(-1)) on_newer=Callback::new(move |_| step_hit(1)) />
            }))}
            <div class="relative flex min-h-0 flex-1 flex-col"
                on:mouseenter=move |_| {
                    expired.set(false);
                    hovered.set(true);
                }
                on:mouseleave=move |_| {
                    hovered.set(false);
                    expired.set(false);
                }>
                <div node_ref=scroll_ref class=SCROLL_CLASS on:scroll=on_scroll>
                    <div node_ref=inner_ref class="relative w-full">
                        <For each=visible_rows key=row_key children=render_row />
                    </div>
                </div>
                <Scrollbar scroll_ref=scroll_ref layout=layout markers=markers scroll_top=scroll_top viewport=viewport layer=layer
                    on_marker=Callback::new(move |index: usize| scroll_to(index)) />
                {move || if paused.get() {
                    // Oben, damit die neueste Nachricht unten sichtbar bleibt.
                    let count = paused_count.get();
                    view! {
                        <div class="pointer-events-none absolute inset-x-0 top-2 z-20 flex justify-center">
                            <span data-slot="badge" data-variant="secondary" class=PAUSE_BADGE>
                                {icons::pause("")}
                                {if count > 0 { multiyap_core::tf!("Chat pausiert ({count} neue)", "Chat paused ({count} new)") } else { tr("Chat pausiert", "Chat paused").to_string() }}
                            </span>
                        </div>
                    }.into_any()
                } else if !at_bottom.get() {
                    let count = new_count.get();
                    view! {
                        <div class="pointer-events-none absolute inset-x-0 bottom-2 z-20 flex justify-center">
                            <button type="button" data-slot="button" class=button_class("default", "sm", "pointer-events-auto shadow-md") on:click=jump_to_end>
                                {if count > 0 { multiyap_core::tf!("{count} neue Nachrichten", "{count} new messages") } else { tr("Neue Nachrichten", "New messages").to_string() }}
                                {icons::arrow_down("")}
                            </button>
                        </div>
                    }.into_any()
                } else {
                    ().into_any()
                }}
            </div>
            <LayerView layer=layer />
        </div>
    }
}

/// Suchleiste über dem Chat. Enter springt zum vorigen (älteren) Treffer, Umschalt+Enter zum nächsten (neueren), Esc
/// schließt.
#[component]
fn SearchBar(tab: String, search: RwSignal<SearchState>, position: Signal<usize>, total: Signal<usize>, on_older: Callback<()>, on_newer: Callback<()>) -> impl IntoView {
    let input_ref = NodeRef::<leptos::html::Input>::new();
    // Bei jedem Strg+F das Suchfeld fokussieren und den Text markieren.
    let focus_token = Memo::new(move |_| search.with(|s| s.focus_token));
    Effect::new(move |_| {
        focus_token.track();
        if let Some(input) = input_ref.get() {
            let _ = input.focus();
            input.select();
        }
    });
    let (set_tab, key_tab, close_tab) = (tab.clone(), tab.clone(), tab);
    let close = move || {
        multiyap_web::chat_tools::close_search(&close_tab);
    };
    let close_button = close.clone();
    view! {
        <div class="flex items-center gap-2 border-b px-3 py-2">
            {icons::search("size-4 shrink-0 text-muted-foreground")}
            <input node_ref=input_ref data-slot="input" class=format!("{INPUT_CLASS} h-8") placeholder=tr("Im Chat suchen (Text oder Name)", "Search in chat (text or name)") aria-label=tr("Im Chat suchen", "Search in chat")
                prop:value=move || search.with(|s| s.query.clone())
                on:input=move |event| { multiyap_web::chat_tools::set_search_query(&set_tab, &event_target_value(&event)); }
                on:keydown=move |event: KeyboardEvent| {
                    let _ = &key_tab;
                    match event.key().as_str() {
                        "Enter" => {
                            event.prevent_default();
                            if event.shift_key() { on_newer.run(()) } else { on_older.run(()) }
                        }
                        "Escape" => {
                            event.prevent_default();
                            close();
                        }
                        "ArrowUp" => {
                            event.prevent_default();
                            on_older.run(());
                        }
                        "ArrowDown" => {
                            event.prevent_default();
                            on_newer.run(());
                        }
                        _ => {}
                    }
                } />
            <span class="shrink-0 text-xs text-muted-foreground tabular-nums" aria-live="polite">
                {move || {
                    if search.with(|s| s.query.trim().is_empty()) {
                        String::new()
                    } else if total.get() == 0 {
                        tr("Keine Treffer", "No matches").to_string()
                    } else {
                        multiyap_core::tf!("{} von {}", "{} of {}", position.get(), total.get())
                    }
                }}
            </span>
            <button type="button" data-slot="button" class=button_class("ghost", "icon-sm", "") aria-label=tr("Älterer Treffer", "Older match") disabled=move || total.get() == 0 on:click=move |_| on_older.run(())>
                {icons::chevron_up("")}
            </button>
            <button type="button" data-slot="button" class=button_class("ghost", "icon-sm", "") aria-label=tr("Neuerer Treffer", "Newer match") disabled=move || total.get() == 0 on:click=move |_| on_newer.run(())>
                {icons::chevron_down("")}
            </button>
            <button type="button" data-slot="button" class=button_class("ghost", "icon-sm", "") aria-label=tr("Suche schließen", "Close search") on:click=move |_| close_button()>
                {icons::x("")}
            </button>
        </div>
    }
}

/// Schmale eigene Scroll-Leiste am Rand des Chats mit Markierungen. Klick auf eine Markierung springt zur Nachricht,
/// Drüberfahren zeigt eine Vorschau, dichte Treffer werden zusammengefasst.
#[component]
fn Scrollbar(
    scroll_ref: NodeRef<leptos::html::Div>,
    layout: RwSignal<Rc<Layout>, LocalStorage>,
    markers: RwSignal<Vec<Marker>>,
    scroll_top: RwSignal<f64>,
    viewport: RwSignal<f64>,
    layer: Layer,
    on_marker: Callback<usize>,
) -> impl IntoView {
    let element = move || scroll_ref.get_untracked().map(HtmlElement::from);
    // (Höhe der Leiste, Gesamthöhe, bildlauffähig)
    let metrics = move || {
        let track = viewport.get();
        let total = layout.with(|l| l.total).max(element().map_or(0.0, |el| f64::from(el.scroll_height())));
        (track, total, track > 0.0 && total > track + 1.0)
    };
    let thumb = move || {
        let (track, total, scrollable) = metrics();
        if !scrollable {
            return (0.0, 0.0);
        }
        let height = (track / total * track).max(MIN_THUMB_PX);
        let max_scroll = (total - track).max(1.0);
        (scroll_top.get() / max_scroll * (track - height), height)
    };
    let drag = StoredValue::new(None::<(f64, f64, f64)>);
    let groups = move || {
        let (track, total, scrollable) = metrics();
        if !scrollable {
            return Vec::new();
        }
        let mut positioned: Vec<(f64, Marker)> =
            markers.with(|list| layout.with(|l| list.iter().map(|m| (l.starts.get(m.index).copied().unwrap_or(0.0) / total * track, m.clone())).collect()));
        positioned.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut groups: Vec<(f64, Vec<Marker>)> = Vec::new();
        for (top, marker) in positioned {
            match groups.last_mut() {
                Some(last) if top - last.0 < MARKER_GAP_PX => last.1.push(marker),
                _ => groups.push((top, vec![marker])),
            }
        }
        groups
    };
    view! {
        <div aria-hidden="true" class="group/scrollbar absolute inset-y-0 right-0 z-10 w-3"
            style:display=move || if metrics().2 { "block" } else { "none" }
            on:pointerdown=move |event: web_sys::PointerEvent| {
                let (Some(el), Some(track)) = (element(), event.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok())) else { return };
                if event.button() != 0 {
                    return;
                }
                let (track_height, total, _) = metrics();
                let (_, thumb_height) = thumb();
                let rect = track.get_bounding_client_rect();
                let position = (f64::from(event.client_y()) - rect.top() - thumb_height / 2.0) / (track_height - thumb_height).max(1.0);
                el.set_scroll_top((position.clamp(0.0, 1.0) * (total - track_height).max(1.0)).round() as i32);
            }>
            <div class="absolute right-0.5 w-1.5 rounded-full bg-muted-foreground/30 transition-colors group-hover/scrollbar:bg-muted-foreground/50"
                style:top=move || format!("{}px", thumb().0) style:height=move || format!("{}px", thumb().1)
                on:pointerdown=move |event: web_sys::PointerEvent| {
                    let Some(el) = element() else { return };
                    if event.button() != 0 {
                        return;
                    }
                    event.prevent_default();
                    event.stop_propagation();
                    if let Some(target) = event.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                        let _ = target.set_pointer_capture(event.pointer_id());
                    }
                    let (track, total, _) = metrics();
                    let ratio = (total - track).max(1.0) / (track - thumb().1).max(1.0);
                    drag.set_value(Some((f64::from(event.client_y()), f64::from(el.scroll_top()), ratio)));
                }
                on:pointermove=move |event: web_sys::PointerEvent| {
                    if let (Some((start_y, start_scroll, ratio)), Some(el)) = (drag.get_value(), element()) {
                        el.set_scroll_top((start_scroll + (f64::from(event.client_y()) - start_y) * ratio).round() as i32);
                    }
                }
                on:pointerup=move |_| drag.set_value(None)
                on:pointercancel=move |_| drag.set_value(None)>
            </div>
            {move || groups().into_iter().map(|(top, group)| {
                let first = group[0].clone();
                let text = if group.len() > 1 { multiyap_core::tf!("{} Treffer: {}", "{} match: {}", group.len(), first.preview) } else { first.preview.clone() };
                let index = first.index;
                view! {
                    <button type="button" tabindex="-1" class=format!("absolute inset-x-0 h-0.5 cursor-pointer {}", first.class) style:top=format!("{top}px")
                        on:pointerdown=|event: web_sys::PointerEvent| event.stop_propagation()
                        on:pointerenter=move |event: web_sys::PointerEvent| {
                            if let Some(target) = event.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                                layer.show_tip(&target, text.clone(), true);
                            }
                        }
                        on:pointerleave=move |_| layer.hide_tip()
                        on:click=move |_| on_marker.run(index)>
                    </button>
                }
            }).collect_view()}
        </div>
    }
}
