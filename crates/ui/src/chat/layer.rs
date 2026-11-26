//! Gemeinsame Ebene einer Chat-Liste für alles, was über den Zeilen schwebt: Rechtsklick-Menü der Nachrichten
//! (Nachbau von shadcn ContextMenu), Grund-Fenster der Schnellbuttons (Popover mit ReasonPanel) und Tooltips
//! (Abzeichen, Striche der Scroll-Leiste). Jede Art gibt es nur einmal je Liste statt einmal je Zeile.
//!
//! Die Inhalte liegen über die Popover-Schnittstelle des Browsers (`popover="manual"`, `showPopover()`) in der
//! obersten Ebene: So schneidet der Bildlauf der Liste sie nicht ab, und verschobene Zeilen (transform) stören die
//! Lage nicht. Esc, Klick daneben und Scrollen schließen wie bei Radix.

use multiyap_core::i18n::tr;
use std::rc::Rc;

use multiyap_core::chat_text::is_protected_sender;
use multiyap_core::model::ChatMessage;
use multiyap_core::shortcuts::next_free_hotkey;
use leptos::prelude::*;
use serde::Serialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{HtmlElement, KeyboardEvent};

use crate::bridge::{random_id, update_setting, use_setting};
use crate::components::button::button_class;
use crate::components::input::INPUT_CLASS;
use crate::components::overlay::{window, WindowListeners};
use crate::components::checkbox::Checkbox;
use crate::components::{icons, ComingSoonBadge};
use crate::settings::model::{ReasonTemplate, ReasonTemplates, MAX_REASON_LENGTH};

/// Rechteck auf dem Bildschirm (Bezug für Menüs, Fenster und Tooltips).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl Rect {
    pub fn of(element: &web_sys::Element) -> Rect {
        let r = element.get_bounding_client_rect();
        Rect { left: r.left(), top: r.top(), right: r.right(), bottom: r.bottom() }
    }

    pub fn point(x: f64, y: f64) -> Rect {
        Rect { left: x, top: y, right: x, bottom: y }
    }
}

/// Ziel einer Mod-Aktion (wie ModTarget in crates/web/src/mod_actions.rs).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModTarget {
    pub channel_id: String,
    pub user_id: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
}

/// Mod-Aktion ausführen (crates/web prüft die Mod-Rechte zentral und meldet das Ergebnis).
pub fn run_mod_action(target: &ModTarget, action: &str, duration_seconds: Option<i64>, reason: Option<String>) {
    let reason = reason.filter(|r| !r.trim().is_empty());
    let Some(target) = crate::bridge::convert::<multiyap_web::mod_actions::ModTarget>(target) else { return };
    let action = action.to_string();
    leptos::task::spawn_local(async move {
        multiyap_web::mod_actions::execute_mod_action(target, &action, duration_seconds.map(|d| d as f64), reason).await;
    });
}

/// Offenes Rechtsklick-Menü einer Nachricht.
#[derive(Clone)]
pub struct MenuState {
    pub x: f64,
    pub y: f64,
    pub message: Rc<ChatMessage>,
    pub js: JsValue,
    pub can_moderate: bool,
}

/// Offenes Grund-Fenster.
#[derive(Clone)]
pub struct ReasonState {
    pub anchor: Rect,
    /// warn, timeout oder ban.
    pub action: String,
    pub duration_seconds: Option<i64>,
    pub target: ModTarget,
}

/// Tooltip: Text an einem Element, oben oder links davon.
#[derive(Clone)]
pub struct TipState {
    pub anchor: Rect,
    pub text: String,
    pub left: bool,
    /// Mehrere Zeilen (Beschriftung, Wert), zum Beispiel die Stream-Infos am Tab. Ohne Beschriftung fett.
    pub lines: Vec<(Option<String>, String)>,
}

#[derive(Clone, Copy)]
pub struct Layer {
    pub menu: RwSignal<Option<MenuState>, LocalStorage>,
    pub reason: RwSignal<Option<ReasonState>, LocalStorage>,
    pub tip: RwSignal<Option<TipState>, LocalStorage>,
}

impl Layer {
    pub fn new() -> Self {
        Layer { menu: RwSignal::new_local(None), reason: RwSignal::new_local(None), tip: RwSignal::new_local(None) }
    }

    /// Etwas ist offen, das den Chat anhält (Menü oder Grund-Fenster).
    pub fn holding(&self) -> bool {
        self.menu.with(Option::is_some) || self.reason.with(Option::is_some)
    }

    pub fn show_tip(&self, element: &web_sys::Element, text: String, left: bool) {
        self.tip.set(Some(TipState { anchor: Rect::of(element), text, left, lines: Vec::new() }));
    }

    pub fn show_lines(&self, element: &web_sys::Element, lines: Vec<(Option<String>, String)>) {
        if !lines.is_empty() {
            self.tip.set(Some(TipState { anchor: Rect::of(element), text: String::new(), left: false, lines }));
        }
    }

    pub fn hide_tip(&self) {
        self.tip.set(None);
    }
}

impl Default for Layer {
    fn default() -> Self {
        Self::new()
    }
}

// ---- Lage ----

fn viewport() -> (f64, f64) {
    let w = window();
    (
        w.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(1000.0),
        w.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(800.0),
    )
}

/// Seite, an der ein schwebender Inhalt erscheint.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Side {
    /// Unter dem Bezug, linke Kanten bündig.
    Below,
    /// Über dem Bezug, mittig.
    Above,
    /// Links daneben, mittig.
    Left,
    /// Rechts daneben, oben bündig (Untermenü).
    Right,
    /// Am Mauszeiger (Rechtsklick-Menü).
    Cursor,
    /// Über dem Bezug, rechte Kanten bündig (Emote-Fenster).
    AboveEnd,
}

const EDGE: f64 = 10.0;

/// Legt den Inhalt an den Bezug; passt er nicht, auf die Gegenseite (wie Radix), immer im Fenster.
pub(crate) fn place(element: &HtmlElement, anchor: Rect, side: Side, offset: f64) {
    let (vw, vh) = viewport();
    let (w, h) = (f64::from(element.offset_width()), f64::from(element.offset_height()));
    let center_x = (anchor.left + anchor.right) / 2.0 - w / 2.0;
    let center_y = (anchor.top + anchor.bottom) / 2.0 - h / 2.0;
    let (mut left, mut top) = match side {
        Side::Below => {
            let below = anchor.bottom + offset;
            let top = if below + h > vh - EDGE && anchor.top - offset - h > EDGE { anchor.top - offset - h } else { below };
            (anchor.left, top)
        }
        Side::Above => {
            let above = anchor.top - offset - h;
            (center_x, if above < EDGE { anchor.bottom + offset } else { above })
        }
        Side::Left => {
            let left = anchor.left - offset - w;
            (if left < EDGE { anchor.right + offset } else { left }, center_y)
        }
        Side::Right => {
            let right = anchor.right + offset;
            (if right + w > vw - EDGE { anchor.left - offset - w } else { right }, anchor.top - 4.0)
        }
        Side::AboveEnd => {
            let above = anchor.top - offset - h;
            (anchor.right - w, if above < EDGE { anchor.bottom + offset } else { above })
        }
        Side::Cursor => {
            let left = if anchor.left + w > vw - EDGE { anchor.left - w } else { anchor.left };
            let top = if anchor.top + h > vh - EDGE { anchor.top - h } else { anchor.top };
            (left, top)
        }
    };
    left = left.clamp(EDGE, (vw - EDGE - w).max(EDGE));
    top = top.clamp(EDGE, (vh - EDGE - h).max(EDGE));
    let style = element.style();
    let _ = style.set_property("left", &format!("{left}px"));
    let _ = style.set_property("top", &format!("{top}px"));
    let _ = style.set_property("opacity", "1");
}

/// In die oberste Ebene heben (Popover-Schnittstelle des Browsers).
pub(crate) fn show_popover(element: &HtmlElement) {
    if let Ok(show) = js_sys::Reflect::get(element, &"showPopover".into()).and_then(|f| f.dyn_into::<js_sys::Function>()) {
        let _ = show.call0(element);
    }
}

/// Lage eines schwebenden Inhalts in der obersten Ebene. Rand, Innenabstand und Außenabstand setzt Tailwind (preflight)
/// schon zurück; Hintergrund und Schrift kommen aus den shadcn-Klassen des Inhalts.
pub(crate) const POPOVER_RESET: &str = "fixed inset-auto m-0";

/// Hängt Esc, Klick daneben und Scrollen an das Fenster, solange `open` gilt.
pub(crate) fn dismiss_on_outside(open: impl Fn() -> bool + 'static, close: impl Fn() + Clone + 'static) {
    let listeners = WindowListeners::new();
    Effect::new({
        let listeners = listeners.clone();
        move |_| {
            listeners.clear();
            if !open() {
                return;
            }
            let on_escape = close.clone();
            listeners.add(
                "keydown",
                Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| {
                    if event.key() == "Escape" {
                        event.stop_propagation();
                        event.prevent_default();
                        on_escape();
                    }
                })
                .into_js_value(),
            );
            let inside = |event: &web_sys::Event| {
                event
                    .target()
                    .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                    .is_some_and(|t| t.closest("[data-chat-overlay]").ok().flatten().is_some())
            };
            for kind in ["pointerdown", "scroll"] {
                let close = close.clone();
                listeners.add(
                    kind,
                    Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
                        if !inside(&event) {
                            close();
                        }
                    })
                    .into_js_value(),
                );
            }
            // Nur das Fenster selbst verliert den Fokus (zum Beispiel Alt+Tab); Fokuswechsel zwischen Elementen
            // kommen in der Capture-Phase auch hier an und zählen nicht.
            let on_blur = close.clone();
            listeners.add(
                "blur",
                Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
                    if event.target().is_some_and(|t| t.dyn_ref::<web_sys::Window>().is_some()) {
                        on_blur();
                    }
                })
                .into_js_value(),
            );
        }
    });
    on_cleanup(move || listeners.clear());
}

// ---- Rechtsklick-Menü ----

const CONTEXT_CONTENT: &str = "z-50 max-h-(--radix-context-menu-content-available-height) min-w-36 origin-(--radix-context-menu-content-transform-origin) overflow-x-hidden overflow-y-auto rounded-lg bg-popover p-1 text-popover-foreground shadow-md ring-1 ring-foreground/10 duration-100 data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95 w-56 outline-none";
const CONTEXT_SUB_CONTENT: &str = "z-50 min-w-32 origin-(--radix-context-menu-content-transform-origin) overflow-hidden rounded-lg border bg-popover p-1 text-popover-foreground shadow-lg duration-100 data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95 outline-none";
const CONTEXT_ITEM: &str = "group/context-menu-item relative flex cursor-default items-center gap-1.5 rounded-md px-1.5 py-1 text-sm outline-hidden select-none focus:bg-accent focus:text-accent-foreground data-inset:pl-7 data-[variant=destructive]:text-destructive data-[variant=destructive]:focus:bg-destructive/10 data-[variant=destructive]:focus:text-destructive dark:data-[variant=destructive]:focus:bg-destructive/20 data-disabled:pointer-events-none data-disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4 focus:*:[svg]:text-accent-foreground data-[variant=destructive]:*:[svg]:text-destructive";
const CONTEXT_SUB_TRIGGER: &str = "flex cursor-default items-center gap-1.5 rounded-md px-1.5 py-1 text-sm outline-hidden select-none focus:bg-accent focus:text-accent-foreground data-inset:pl-7 data-open:bg-accent data-open:text-accent-foreground [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4";
const CONTEXT_SEPARATOR: &str = "-mx-1 my-1 h-px bg-border";

/// Radix setzt an Menüinhalten outline: none (Fokus zeigt der hervorgehobene Eintrag); daher outline-none oben.
/// Anpinnen: Dauern wie im Twitch-Chat; ohne Dauer bis zum Streamende.
fn pin_durations() -> [(&'static str, Option<u32>); 5] {
    [(tr("1 Minute", "1 minute"), Some(60)), (tr("5 Minuten", "5 minutes"), Some(300)), (tr("10 Minuten", "10 minutes"), Some(600)), (tr("30 Minuten", "30 minutes"), Some(1800)), (tr("Bis zum Streamende", "Until the end of the stream"), None)]
}

fn menu_items(content: &web_sys::Element) -> Vec<HtmlElement> {
    let Ok(list) = content.query_selector_all("[role=menuitem]:not([data-disabled])") else { return Vec::new() };
    (0..list.length()).filter_map(|i| list.item(i)).filter_map(|n| n.dyn_into::<HtmlElement>().ok()).collect()
}

/// Pfeiltasten, Pos1, Ende und Enter in einem Menü (wie Radix).
fn menu_key(event: &KeyboardEvent, content: &web_sys::Element) {
    let items = menu_items(content);
    if items.is_empty() {
        return;
    }
    let active = window().document().and_then(|d| d.active_element());
    let index = active.as_ref().and_then(|a| items.iter().position(|i| AsRef::<web_sys::Element>::as_ref(i) == a));
    let target = match event.key().as_str() {
        "ArrowDown" => index.map_or(0, |i| (i + 1) % items.len()),
        "ArrowUp" => index.map_or(items.len() - 1, |i| if i == 0 { items.len() - 1 } else { i - 1 }),
        "Home" => 0,
        "End" => items.len() - 1,
        "Enter" | " " => {
            event.prevent_default();
            if let Some(i) = index {
                items[i].click();
            }
            return;
        }
        _ => return,
    };
    event.prevent_default();
    let _ = items[target].focus();
}

fn focus_on_hover(event: web_sys::PointerEvent) {
    if let Some(item) = event.current_target().and_then(|t| t.dyn_into::<HtmlElement>().ok()) {
        let _ = item.focus();
    }
}

fn menu_item(label: &'static str, icon: Option<AnyView>, disabled: bool, destructive: bool, on_select: impl Fn() + 'static) -> impl IntoView {
    view! {
        <div role="menuitem" data-slot="context-menu-item" data-variant=if destructive { "destructive" } else { "default" }
            data-disabled=disabled.then_some("") aria-disabled=disabled.then_some("true") tabindex="-1" class=CONTEXT_ITEM
            on:pointermove=focus_on_hover on:click=move |_| if !disabled { on_select() }>
            {icon}
            {label}
        </div>
    }
}

/// Rechtsklick-Menü auf Nachrichten: Antworten, Anpinnen (bei schon angepinnten Nachrichten ändert eine neue Wahl die
/// Dauer), Löschen, Text kopieren, Übersetzen (ausgegraut, „Coming Soon“). Moderatives nur mit Mod-Rechten, über die
/// zentrale Prüfung.
#[component]
fn ContextMenu(layer: Layer) -> impl IntoView {
    let content_ref = NodeRef::<leptos::html::Div>::new();
    let sub_ref = NodeRef::<leptos::html::Div>::new();
    let sub_trigger_ref = NodeRef::<leptos::html::Div>::new();
    let sub_open = RwSignal::new(false);
    let close = move || {
        sub_open.set(false);
        layer.menu.set(None);
    };
    dismiss_on_outside(move || layer.menu.with(Option::is_some), close);

    Effect::new(move |_| {
        let Some(state) = layer.menu.with(|m| m.as_ref().map(|s| (s.x, s.y))) else { return };
        if let Some(content) = content_ref.get() {
            let content: HtmlElement = content.into();
            show_popover(&content);
            place(&content, Rect::point(state.0, state.1), Side::Cursor, 0.0);
            let _ = content.focus();
        }
    });
    Effect::new(move |_| {
        if !sub_open.get() {
            return;
        }
        if let (Some(sub), Some(trigger)) = (sub_ref.get(), sub_trigger_ref.get()) {
            let sub: HtmlElement = sub.into();
            show_popover(&sub);
            place(&sub, Rect::of(&trigger), Side::Right, 0.0);
            if let Some(first) = menu_items(&sub).into_iter().next() {
                let _ = first.focus();
            }
        }
    });

    move || {
        let state = layer.menu.get()?;
        let message = state.message.clone();
        let removed = message.flags.removed.is_some();
        // YouTube: zuerst nur lesen, Antworten gibt es dort noch nicht.
        let no_reply = removed || message.channel.platform != "twitch";
        // Löschen: Twitch erlaubt keine Nachrichten des Broadcasters oder anderer Mods; eigene schon.
        let deletable = state.can_moderate && !removed && (message.flags.own == Some(true) || !is_protected_sender(&message.badges));
        let reply = {
            let message = message.clone();
            move || {
                close();
                multiyap_web::chat_tools::start_reply(&message.channel.login, &message.id, &message.sender.display_name, &message.text);
            }
        };
        let delete = {
            let message = message.clone();
            move || {
                close();
                let target = ModTarget {
                    channel_id: message.channel.id.clone(),
                    user_id: message.sender.id.clone(),
                    display_name: message.sender.display_name.clone(),
                    message_id: Some(message.id.clone()),
                };
                run_mod_action(&target, "delete", None, None);
            }
        };
        let copy = {
            let text = message.text.clone();
            move || {
                close();
                leptos::task::spawn_local(multiyap_web::chat_tools::copy_text(text.clone()));
            }
        };
        let sub = {
            let (pin_channel, pin_message) = (message.channel.id.clone(), message.id.clone());
            move || {
                let (pin_channel, pin_message) = (pin_channel.clone(), pin_message.clone());
                sub_open.get().then(|| {
                    let (unpin_channel, unpin_message) = (pin_channel.clone(), pin_message.clone());
                    view! {
                        <div node_ref=sub_ref popover="manual" data-chat-overlay="" role="menu" aria-orientation="vertical" data-slot="context-menu-sub-content"
                            data-state="open" data-side="right" tabindex="-1" class=format!("{POPOVER_RESET} {CONTEXT_SUB_CONTENT}") style="opacity: 0;"
                            on:keydown=move |event: KeyboardEvent| {
                                if event.key() == "ArrowLeft" {
                                    event.prevent_default();
                                    sub_open.set(false);
                                    if let Some(trigger) = sub_trigger_ref.get_untracked() {
                                        let _ = HtmlElement::from(trigger).focus();
                                    }
                                    return;
                                }
                                if let Some(sub) = sub_ref.get_untracked() {
                                    menu_key(&event, &sub);
                                }
                            }>
                            {pin_durations().iter().map(|(label, seconds)| {
                                let (channel, id) = (pin_channel.clone(), pin_message.clone());
                                let seconds = *seconds;
                                menu_item(label, None, false, false, move || {
                                    close();
                                    leptos::task::spawn_local(multiyap_web::chat_tools::pin_from_menu(channel.clone(), id.clone(), seconds.map(f64::from)));
                                })
                            }).collect_view()}
                            <div role="separator" aria-orientation="horizontal" data-slot="context-menu-separator" class=CONTEXT_SEPARATOR></div>
                            {menu_item(tr("Lösen", "Unpin"), Some(icons::pin_off("").into_any()), false, false, move || {
                                close();
                                leptos::task::spawn_local(multiyap_web::chat_tools::unpin_from_menu(unpin_channel.clone(), unpin_message.clone()));
                            })}
                        </div>
                    }
                })
            }
        };
        Some(view! {
            <div node_ref=content_ref popover="manual" data-chat-overlay="" role="menu" aria-orientation="vertical" data-slot="context-menu-content"
                data-state="open" tabindex="-1" class=format!("{POPOVER_RESET} {CONTEXT_CONTENT}") style="opacity: 0;"
                on:contextmenu=|event: web_sys::MouseEvent| event.prevent_default()
                // Zeiger über einem anderen Eintrag des Hauptmenüs schließt das Untermenü.
                on:pointermove=move |event: web_sys::PointerEvent| {
                    let on_trigger = event.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                        .is_some_and(|t| t.closest("[data-slot=context-menu-sub-trigger]").ok().flatten().is_some());
                    if !on_trigger && sub_open.get_untracked() {
                        sub_open.set(false);
                    }
                }
                on:keydown=move |event: KeyboardEvent| {
                    if let Some(content) = content_ref.get_untracked() {
                        menu_key(&event, &content);
                    }
                }>
                {menu_item(tr("Antworten", "Reply"), Some(icons::message_square_reply("").into_any()), no_reply, false, reply)}
                {state.can_moderate.then(|| view! {
                    <div node_ref=sub_trigger_ref role="menuitem" aria-haspopup="menu" aria-expanded=move || sub_open.get().to_string()
                        data-slot="context-menu-sub-trigger" data-state=move || if sub_open.get() { "open" } else { "closed" }
                        data-open=move || sub_open.get().then_some("") data-disabled=removed.then_some("") aria-disabled=removed.then_some("true")
                        tabindex="-1" class=CONTEXT_SUB_TRIGGER
                        on:pointermove=move |event| {
                            focus_on_hover(event);
                            if !removed {
                                sub_open.set(true);
                            }
                        }
                        on:click=move |_| if !removed { sub_open.set(true) }
                        on:keydown=move |event: KeyboardEvent| {
                            if event.key() == "ArrowRight" && !removed {
                                event.prevent_default();
                                event.stop_propagation();
                                sub_open.set(true);
                            }
                        }>
                        {icons::pin("")}
                        {tr("Anpinnen", "Pin")}
                        {icons::chevron_right("ml-auto")}
                    </div>
                })}
                {deletable.then(|| menu_item(tr("Löschen", "Delete"), Some(icons::trash("").into_any()), false, true, delete))}
                {menu_item(tr("Text kopieren", "Copy text"), Some(icons::copy("").into_any()), false, false, copy)}
                <div role="separator" aria-orientation="horizontal" data-slot="context-menu-separator" class=CONTEXT_SEPARATOR></div>
                <div role="menuitem" data-slot="context-menu-item" data-variant="default" data-disabled="" aria-disabled="true" tabindex="-1" class=CONTEXT_ITEM>
                    {icons::languages("")}
                    {tr("Übersetzen", "Translate")}
                    <ComingSoonBadge />
                </div>
            </div>
            {sub}
        })
    }
}

// ---- Grund-Fenster ----

pub(crate) const POPOVER_CONTENT: &str = "z-50 flex w-72 origin-(--radix-popover-content-transform-origin) flex-col gap-2.5 rounded-lg bg-popover p-2.5 text-sm text-popover-foreground shadow-md ring-1 ring-foreground/10 outline-hidden duration-100 data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95";
const LABEL: &str = "flex items-center gap-2 text-sm leading-none font-normal select-none group-data-[disabled=true]:pointer-events-none group-data-[disabled=true]:opacity-50 peer-disabled:cursor-not-allowed peer-disabled:opacity-50";

fn reason_title(action: &str) -> &'static str {
    match action {
        "warn" => tr("Grund für die Verwarnung", "Reason for the warning"),
        "ban" => tr("Grund für den Ban", "Reason for the ban"),
        _ => tr("Grund für den Timeout", "Reason for the timeout"),
    }
}

/// Speichert einen neuen Text als Vorlage der Aktion, mit nächstem freiem Hotkey.
fn save_template(action: String, text: String, no_reason_hotkey: String) {
    update_setting::<ReasonTemplates>("moderation.reasonTemplates", move |mut templates| {
        let list = templates.list_mut(&action);
        if !list.iter().any(|t| t.text == text) {
            let hotkey = next_free_hotkey(list.iter().map(|t| t.hotkey.as_deref()), &no_reason_hotkey);
            list.push(ReasonTemplate { id: random_id(), text: text.clone(), hotkey });
        }
        templates
    });
}

/// Inhalt des Grund-Fensters: Vorlagen mit Hotkeys, „Kein Grund“ (nur Timeout und Ban), eigener Text mit Enter und
/// Häkchen „Als Vorlage speichern“. Esc schließt.
#[component]
fn ReasonPanel(action: String, heading: Option<String>, on_submit: Callback<Option<String>>) -> impl IntoView {
    let templates = use_setting::<ReasonTemplates>("moderation.reasonTemplates", ReasonTemplates::default());
    let no_reason = use_setting::<String>("moderation.noReasonHotkey", String::new());
    let text = RwSignal::new(String::new());
    let save = RwSignal::new(false);
    let input_ref = NodeRef::<leptos::html::Input>::new();
    let allow_no_reason = action != "warn";
    let checkbox_id = format!("save-template-{action}");

    let submit_text = {
        let action = action.clone();
        move || {
            let reason = text.get_untracked().trim().to_string();
            if reason.is_empty() {
                return;
            }
            if save.get_untracked() {
                save_template(action.clone(), reason.clone(), no_reason.get_untracked());
            }
            on_submit.run(Some(reason));
        }
    };

    // Hotkeys gelten, solange der Fokus nicht im Textfeld liegt. Andere Zeichen springen ins Textfeld.
    let on_panel_key = {
        let action = action.clone();
        move |event: KeyboardEvent| {
            let in_input = input_ref.get_untracked().zip(event.target()).is_some_and(|(input, target)| {
                let input: &web_sys::EventTarget = input.as_ref();
                *input == target
            });
            if in_input || event.ctrl_key() || event.alt_key() || event.meta_key() {
                return;
            }
            let key = event.key().to_lowercase();
            if allow_no_reason && key == no_reason.get_untracked() {
                event.prevent_default();
                on_submit.run(None);
                return;
            }
            if let Some(template) = templates.with_untracked(|t| t.list(&action).iter().find(|t| t.hotkey.as_deref() == Some(key.as_str())).cloned()) {
                event.prevent_default();
                on_submit.run(Some(template.text));
                return;
            }
            if event.key().chars().count() == 1 {
                if let Some(input) = input_ref.get_untracked() {
                    let _ = input.focus();
                }
            }
        }
    };

    let list = {
        let action = action.clone();
        move || {
            let entries = templates.with(|t| t.list(&action).clone());
            (!entries.is_empty() || allow_no_reason).then(|| {
                let between = button_class("ghost", "sm", "").replacen("justify-center", "justify-between", 1);
                let template_class = between.replacen("gap-1 ", "gap-3 ", 1);
                view! {
                    <div class="flex flex-col gap-1">
                        {allow_no_reason.then(|| view! {
                            <button type="button" data-slot="button" class=between.clone() on:click=move |_| on_submit.run(None)>
                                <span class="text-muted-foreground">{tr("Kein Grund", "No reason")}</span>
                                <kbd class="text-xs text-muted-foreground">{move || no_reason.get()}</kbd>
                            </button>
                        })}
                        {entries.into_iter().map(|template| {
                            let chosen = template.text.clone();
                            view! {
                                <button type="button" data-slot="button" class=template_class.clone() on:click=move |_| on_submit.run(Some(chosen.clone()))>
                                    <span class="truncate">{template.text}</span>
                                    {template.hotkey.map(|key| view! { <kbd class="text-xs text-muted-foreground">{key}</kbd> })}
                                </button>
                            }
                        }).collect_view()}
                    </div>
                }
            })
        }
    };

    view! {
        <div class="flex flex-col gap-3 outline-none" tabindex="-1" on:keydown=on_panel_key>
            <p class="text-sm font-medium">{reason_title(&action)}{heading.map(|h| format!(" ({h})"))}</p>
            {list}
            <form class="flex flex-col gap-2" on:submit=move |event: web_sys::SubmitEvent| {
                event.prevent_default();
                submit_text();
            }>
                <input node_ref=input_ref data-slot="input" class=INPUT_CLASS maxlength=MAX_REASON_LENGTH.to_string()
                    placeholder=tr("Eigener Grund, Enter schickt ab", "Custom reason, Enter sends") aria-label=tr("Eigener Grund", "Custom reason")
                    prop:value=move || text.get() on:input=move |event| text.set(event_target_value(&event)) />
                <div class="flex items-center gap-2">
                    <Checkbox id=checkbox_id.clone() checked=save on_change=Callback::new(move |value: bool| save.set(value)) />
                    <label data-slot="label" for=checkbox_id class=LABEL>{tr("Als Vorlage speichern", "Save as template")}</label>
                </div>
            </form>
        </div>
    }
}

#[component]
fn ReasonPopover(layer: Layer) -> impl IntoView {
    let content_ref = NodeRef::<leptos::html::Div>::new();
    let close = move || layer.reason.set(None);
    dismiss_on_outside(move || layer.reason.with(Option::is_some), close);
    Effect::new(move |_| {
        let Some(anchor) = layer.reason.with(|r| r.as_ref().map(|s| s.anchor)) else { return };
        if let Some(content) = content_ref.get() {
            let content: HtmlElement = content.into();
            show_popover(&content);
            place(&content, anchor, Side::Below, 4.0);
            // Wie Radix: Fokus auf das erste bedienbare Element, damit die Hotkeys sofort gelten.
            if let Ok(Some(first)) = content.query_selector("button, input") {
                if let Ok(first) = first.dyn_into::<HtmlElement>() {
                    let _ = first.focus();
                }
            }
        }
    });
    move || {
        let state = layer.reason.get()?;
        let heading = (state.action == "timeout").then(|| multiyap_core::hotkeys::format_duration(state.duration_seconds.unwrap_or(0)));
        let action = state.action.clone();
        let submit = Callback::new(move |reason: Option<String>| {
            layer.reason.set(None);
            run_mod_action(&state.target, &state.action, state.duration_seconds, reason);
        });
        Some(view! {
            <div node_ref=content_ref popover="manual" data-chat-overlay="" data-slot="popover-content" data-state="open" data-side="bottom"
                class=format!("{POPOVER_RESET} {POPOVER_CONTENT}") style="opacity: 0;">
                <ReasonPanel action=action heading=heading on_submit=submit />
            </div>
        })
    }
}

// ---- Tooltip ----

const TOOLTIP_CONTENT: &str = "z-50 inline-flex w-fit max-w-xs origin-(--radix-tooltip-content-transform-origin) items-center gap-1.5 rounded-md border bg-popover px-3 py-1.5 text-xs text-popover-foreground shadow-md has-data-[slot=kbd]:pr-1.5 data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 **:data-[slot=kbd]:relative **:data-[slot=kbd]:isolate **:data-[slot=kbd]:z-50 **:data-[slot=kbd]:rounded-sm data-[state=delayed-open]:animate-in data-[state=delayed-open]:fade-in-0 data-[state=delayed-open]:zoom-in-95 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95";

#[component]
fn Tooltip(layer: Layer) -> impl IntoView {
    let content_ref = NodeRef::<leptos::html::Div>::new();
    Effect::new(move |_| {
        let Some((anchor, left)) = layer.tip.with(|t| t.as_ref().map(|t| (t.anchor, t.left))) else { return };
        if let Some(content) = content_ref.get() {
            let content: HtmlElement = content.into();
            show_popover(&content);
            place(&content, anchor, if left { Side::Left } else { Side::Above }, 0.0);
        }
    });
    move || {
        let state = layer.tip.get()?;
        Some(view! {
            <div node_ref=content_ref popover="manual" role="tooltip" data-slot="tooltip-content" data-state="delayed-open"
                data-side=if state.left { "left" } else { "top" }
                class=format!("{POPOVER_RESET} pointer-events-none {TOOLTIP_CONTENT}") style="opacity: 0;">
                {state.text}
                {(!state.lines.is_empty()).then(|| view! {
                    <div class="flex max-w-72 flex-col gap-0.5">
                        {state.lines.into_iter().map(|(label, value)| match label {
                            Some(label) => view! { <p><span class="opacity-70">{format!("{label}: ")}</span>{value}</p> }.into_any(),
                            None => view! { <p class="font-medium">{value}</p> }.into_any(),
                        }).collect_view()}
                    </div>
                })}
            </div>
        })
    }
}

/// Alle schwebenden Inhalte einer Chat-Liste.
#[component]
pub fn LayerView(layer: Layer) -> impl IntoView {
    view! {
        <ContextMenu layer=layer />
        <ReasonPopover layer=layer />
        <Tooltip layer=layer />
    }
}
