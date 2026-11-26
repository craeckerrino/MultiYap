//! Nachbau von src/components/ui/select.tsx (shadcn/ui, Radix Select mit position "item-aligned"): gleiche Klassen,
//! data-Attribute und Rollen. Verhalten wie Radix: Die Liste legt den gewählten Eintrag genau über den Knopf und
//! bleibt im Fenster; Pfeiltasten, Pos1/Ende, Enter/Leertaste und Tippen zum Springen; Escape schließt nur die
//! Liste; Klick außerhalb, Scrollen oder Größenänderung schließen sie.
//!
//! Anders als Radix bleibt die Liste im Dokument neben dem Knopf (kein Portal ans Ende von body): So bleibt sie im
//! Einstellungsfenster (Fokus-Falle und „Klick außerhalb“ des Fensters). Sie ist `position: fixed` und rechnet den
//! umgebenden Bezugsrahmen heraus (das Fenster ist per transform zentriert).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use leptos::prelude::*;
use send_wrapper::SendWrapper;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, KeyboardEvent};

use super::icons;

const TRIGGER_CLASS: &str = "flex w-fit items-center justify-between gap-1.5 rounded-lg border border-input bg-transparent py-2 pr-2 pl-2.5 text-sm whitespace-nowrap transition-colors outline-none select-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 data-placeholder:text-muted-foreground data-[size=default]:h-8 data-[size=sm]:h-7 data-[size=sm]:rounded-[min(var(--radius-md),10px)] *:data-[slot=select-value]:line-clamp-1 *:data-[slot=select-value]:flex *:data-[slot=select-value]:items-center *:data-[slot=select-value]:gap-1.5 dark:bg-input/30 dark:hover:bg-input/50 dark:aria-invalid:border-destructive/50 dark:aria-invalid:ring-destructive/40 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4";

const CONTENT_CLASS: &str = "relative z-50 max-h-(--radix-select-content-available-height) min-w-36 origin-(--radix-select-content-transform-origin) overflow-x-hidden overflow-y-auto rounded-lg bg-popover text-popover-foreground shadow-md ring-1 ring-foreground/10 duration-100 data-[align-trigger=true]:animate-none data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95";

const ITEM_CLASS: &str = "relative flex w-full cursor-default items-center gap-1.5 rounded-md py-1 pr-8 pl-1.5 text-sm outline-hidden select-none focus:bg-accent focus:text-accent-foreground not-data-[variant=destructive]:focus:**:text-accent-foreground data-disabled:pointer-events-none data-disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4 *:[span]:last:flex *:[span]:last:items-center *:[span]:last:gap-2";

const GROUP_CLASS: &str = "scroll-my-1 p-1";
const LABEL_CLASS: &str = "px-1.5 py-1 text-xs text-muted-foreground";
const SEPARATOR_CLASS: &str = "pointer-events-none -mx-1 my-1 h-px bg-border";

/// Abstand zum Fensterrand wie bei Radix.
const VIEWPORT_PADDING: f64 = 10.0;

/// Eintrag der Liste.
#[derive(Clone, PartialEq)]
pub enum SelectEntry {
    Item {
        value: String,
        label: String,
        /// Ausgegraut, nicht wählbar.
        disabled: bool,
        /// Farbpunkt vor dem Text (Klassen, zum Beispiel "bg-term-1").
        swatch: Option<String>,
        /// Eigener Stil des Eintrags (zum Beispiel die Schriftart, in der er sich zeigt).
        style: Option<String>,
    },
    /// Gruppe mit Überschrift (wie SelectGroup und SelectLabel).
    Group { label: String, items: Vec<SelectEntry> },
    Separator,
}

impl SelectEntry {
    pub fn item(value: impl Into<String>, label: impl Into<String>) -> Self {
        SelectEntry::Item { value: value.into(), label: label.into(), disabled: false, swatch: None, style: None }
    }

    pub fn disabled(mut self, is_disabled: bool) -> Self {
        if let SelectEntry::Item { disabled, .. } = &mut self {
            *disabled = is_disabled;
        }
        self
    }

    pub fn swatch(mut self, class: impl Into<String>) -> Self {
        if let SelectEntry::Item { swatch, .. } = &mut self {
            *swatch = Some(class.into());
        }
        self
    }

    pub fn style(mut self, css: impl Into<String>) -> Self {
        if let SelectEntry::Item { style, .. } = &mut self {
            *style = Some(css.into());
        }
        self
    }

    fn swatch_of(&self, current: &str) -> Option<String> {
        match self {
            SelectEntry::Item { value, swatch, .. } if value == current => swatch.clone(),
            _ => None,
        }
    }

    fn labels(&self) -> Vec<(&str, &str)> {
        match self {
            SelectEntry::Item { value, label, .. } => vec![(value.as_str(), label.as_str())],
            SelectEntry::Group { items, .. } => items.iter().flat_map(SelectEntry::labels).collect(),
            SelectEntry::Separator => Vec::new(),
        }
    }
}

/// Klassen zusammenführen wie `cn` für den Fall, der hier vorkommt: eine eigene Breite ersetzt `w-fit`.
fn trigger_class(extra: &str) -> String {
    if extra.split_whitespace().any(|c| c.starts_with("w-")) {
        format!("{} {extra}", TRIGGER_CLASS.replacen("w-fit ", "", 1))
    } else if extra.is_empty() {
        TRIGGER_CLASS.to_string()
    } else {
        format!("{TRIGGER_CLASS} {extra}")
    }
}

fn window() -> web_sys::Window {
    web_sys::window().expect("Fenster")
}

fn options(content: &HtmlElement) -> Vec<HtmlElement> {
    let Ok(list) = content.query_selector_all("[role=option]:not([data-disabled])") else { return Vec::new() };
    (0..list.length()).filter_map(|i| list.item(i)).filter_map(|n| n.dyn_into::<HtmlElement>().ok()).collect()
}

/// Zustand, der zwischen den Ereignissen geteilt wird (nur im Hauptthread, daher SendWrapper).
struct Shared {
    listeners: RefCell<Vec<(&'static str, JsValue)>>,
    typed: RefCell<String>,
    typed_at: Cell<f64>,
}

#[component]
pub fn Select(
    #[prop(into)] id: String,
    #[prop(into)] value: Signal<String>,
    #[prop(into)] entries: Signal<Vec<SelectEntry>>,
    #[prop(into)] on_change: Callback<String>,
    #[prop(optional, into)] disabled: Signal<bool>,
    /// Zusätzliche Klassen für den Knopf, zum Beispiel eine Breite.
    #[prop(optional, into)]
    class: String,
    /// "default" oder "sm".
    #[prop(optional)]
    size: Option<&'static str>,
    /// Name für Screenreader, wenn keine Beschriftung über `for` verknüpft ist.
    #[prop(optional, into)]
    aria_label: Option<String>,
    /// Eigener Stil des Knopfs (zum Beispiel die gewählte Schriftart).
    #[prop(optional, into)]
    trigger_style: Signal<String>,
) -> impl IntoView {
    let open = RwSignal::new(false);
    let trigger_ref = NodeRef::<leptos::html::Button>::new();
    let wrapper_ref = NodeRef::<leptos::html::Div>::new();
    let content_ref = NodeRef::<leptos::html::Div>::new();
    let content_id = format!("{id}-content");
    let shared = SendWrapper::new(Rc::new(Shared { listeners: RefCell::new(Vec::new()), typed: RefCell::new(String::new()), typed_at: Cell::new(0.0) }));

    let label_of = move |current: &str| {
        entries.with(|list| list.iter().flat_map(SelectEntry::labels).find(|(v, _)| *v == current).map(|(_, l)| l.to_string())).unwrap_or_default()
    };

    let remove_listeners = {
        let shared = shared.clone();
        move || {
            for (kind, listener) in shared.listeners.borrow_mut().drain(..) {
                let _ = window().remove_event_listener_with_callback_and_bool(kind, listener.unchecked_ref(), true);
            }
        }
    };

    let close = {
        let remove_listeners = remove_listeners.clone();
        move |refocus: bool| {
            remove_listeners();
            open.set(false);
            if refocus {
                if let Some(trigger) = trigger_ref.get_untracked() {
                    let _ = trigger.focus();
                }
            }
        }
    };

    let choose = {
        let close = close.clone();
        move |next: String| {
            close(true);
            if next != value.get_untracked() {
                on_change.run(next);
            }
        }
    };

    // Lage berechnen, sobald die Liste im Dokument ist: gewählter Eintrag über dem Knopf, innerhalb des Fensters.
    let position = move || {
        let (Some(trigger), Some(wrapper), Some(content)) = (trigger_ref.get_untracked(), wrapper_ref.get_untracked(), content_ref.get_untracked())
        else {
            return;
        };
        let (wrapper, content): (HtmlElement, HtmlElement) = (wrapper.into(), content.into());
        let win = window();
        let view_width = win.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(1000.0);
        let view_height = win.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(800.0);
        let style = wrapper.style();
        let _ = style.set_property("left", "0px");
        let _ = style.set_property("top", "0px");
        // Bezugsrahmen von position: fixed (ein Vorfahr mit transform verschiebt ihn).
        let origin = wrapper.get_bounding_client_rect();
        let trigger_rect = trigger.get_bounding_client_rect();
        let content_style = content.style();
        let _ = content_style.set_property("min-width", &format!("{}px", trigger_rect.width()));
        let max_height = view_height - 2.0 * VIEWPORT_PADDING;
        let _ = content_style.set_property("max-height", &format!("{max_height}px"));
        let selected = content
            .query_selector("[role=option][data-state=checked]")
            .ok()
            .flatten()
            .or_else(|| content.query_selector("[role=option]").ok().flatten())
            .and_then(|e| e.dyn_into::<HtmlElement>().ok());
        let content_height = (content.scroll_height() as f64).min(max_height);
        let content_width = content.get_bounding_client_rect().width();
        let trigger_center = trigger_rect.top() + trigger_rect.height() / 2.0;
        let (item_offset, item_height) = selected.as_ref().map_or((0.0, trigger_rect.height()), |item| (item.offset_top() as f64, item.offset_height() as f64));
        let wanted_top = trigger_center - (item_offset + item_height / 2.0);
        let top = wanted_top.clamp(VIEWPORT_PADDING, (view_height - VIEWPORT_PADDING - content_height).max(VIEWPORT_PADDING));
        // Liegt der Eintrag zu weit unten, rollt die Liste, damit er trotzdem über dem Knopf steht.
        let scroll = (top + item_offset + item_height / 2.0 - trigger_center).max(0.0);
        content.set_scroll_top(scroll as i32);
        let left = trigger_rect.left().clamp(VIEWPORT_PADDING, (view_width - VIEWPORT_PADDING - content_width).max(VIEWPORT_PADDING));
        let _ = style.set_property("left", &format!("{}px", left - origin.left()));
        let _ = style.set_property("top", &format!("{}px", top - origin.top()));
        // Bis zur fertigen Lage unsichtbar über opacity (nicht visibility, sonst ließe sich der Eintrag nicht fokussieren).
        let _ = style.set_property("opacity", "1");
        if let Some(item) = selected {
            let _ = item.focus();
        }
    };

    let open_list = {
        let shared = shared.clone();
        let close = close.clone();
        move || {
            if disabled.get_untracked() || open.get_untracked() {
                return;
            }
            open.set(true);
            let win = window();
            let mut listeners = shared.listeners.borrow_mut();
            // Escape schließt nur die Liste: Zuhörer am Fenster in der Capture-Phase, vor denen am Dokument.
            let close_escape = close.clone();
            let on_key = Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| {
                if event.key() == "Escape" {
                    event.stop_propagation();
                    event.prevent_default();
                    close_escape(true);
                }
            })
            .into_js_value();
            let close_outside = close.clone();
            let on_pointer = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
                let target = event.target().and_then(|t| t.dyn_into::<web_sys::Node>().ok());
                let inside = |node: Option<HtmlElement>| node.zip(target.clone()).is_some_and(|(n, t)| n.contains(Some(&t)));
                let in_content = inside(content_ref.get_untracked().map(Into::into));
                let in_trigger = inside(trigger_ref.get_untracked().map(Into::into));
                if !in_content && !in_trigger {
                    close_outside(false);
                }
            })
            .into_js_value();
            let close_scroll = close.clone();
            let on_scroll = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
                let target = event.target().and_then(|t| t.dyn_into::<web_sys::Node>().ok());
                let in_content = content_ref.get_untracked().zip(target).is_some_and(|(c, t)| HtmlElement::from(c).contains(Some(&t)));
                if !in_content {
                    close_scroll(false);
                }
            })
            .into_js_value();
            let close_resize = close.clone();
            let on_resize = Closure::<dyn FnMut()>::new(move || close_resize(false)).into_js_value();
            for (kind, listener) in [("keydown", on_key), ("pointerdown", on_pointer), ("scroll", on_scroll), ("resize", on_resize)] {
                let _ = win.add_event_listener_with_callback_and_bool(kind, listener.unchecked_ref(), true);
                listeners.push((kind, listener));
            }
            drop(listeners);
            request_animation_frame(position);
        }
    };

    on_cleanup({
        let remove_listeners = SendWrapper::new(remove_listeners);
        move || (remove_listeners)()
    });

    let on_trigger_key = {
        let open_list = open_list.clone();
        move |event: KeyboardEvent| {
            if matches!(event.key().as_str(), "ArrowDown" | "ArrowUp" | "Enter" | " ") {
                event.prevent_default();
                open_list();
            }
        }
    };

    let on_content_key = {
        let shared = shared.clone();
        let close = close.clone();
        let choose = choose.clone();
        move |event: KeyboardEvent| {
            let Some(content) = content_ref.get_untracked() else { return };
            let content: HtmlElement = content.into();
            let items = options(&content);
            let active = web_sys::window().and_then(|w| w.document()).and_then(|d| d.active_element());
            let index = active.as_ref().and_then(|a| items.iter().position(|i| AsRef::<Element>::as_ref(i) == a));
            let focus = |i: usize| {
                if let Some(item) = items.get(i) {
                    let _ = item.focus();
                    item.scroll_into_view_with_bool(false);
                }
            };
            match event.key().as_str() {
                "ArrowDown" => {
                    event.prevent_default();
                    focus(index.map_or(0, |i| (i + 1).min(items.len().saturating_sub(1))));
                }
                "ArrowUp" => {
                    event.prevent_default();
                    focus(index.map_or(0, |i| i.saturating_sub(1)));
                }
                "Home" => {
                    event.prevent_default();
                    focus(0);
                }
                "End" => {
                    event.prevent_default();
                    focus(items.len().saturating_sub(1));
                }
                "Enter" | " " => {
                    event.prevent_default();
                    if let Some(value) = index.and_then(|i| items[i].get_attribute("data-value")) {
                        choose(value);
                    }
                }
                "Tab" => {
                    event.prevent_default();
                    close(true);
                }
                key if key.chars().count() == 1 && !event.ctrl_key() && !event.meta_key() && !event.alt_key() => {
                    // Tippen springt zum ersten passenden Eintrag (Puffer für eine Sekunde wie bei Radix).
                    let now = js_sys::Date::now();
                    if now - shared.typed_at.get() > 1000.0 {
                        shared.typed.borrow_mut().clear();
                    }
                    shared.typed_at.set(now);
                    shared.typed.borrow_mut().push_str(&key.to_lowercase());
                    let typed = shared.typed.borrow().clone();
                    let start = index.map_or(0, |i| i + 1);
                    let order = (start..items.len()).chain(0..start);
                    let hit = order.into_iter().find(|&i| items[i].text_content().unwrap_or_default().to_lowercase().starts_with(&typed));
                    if let Some(i) = hit {
                        focus(i);
                    }
                }
                _ => {}
            }
        }
    };

    let item_view = {
        let choose = choose.clone();
        move |item_value: String, label: String, disabled: bool, swatch: Option<String>, style: Option<String>| {
            let checked = {
                let item_value = item_value.clone();
                Memo::new(move |_| value.get() == item_value)
            };
            let state = move || if checked.get() { "checked" } else { "unchecked" };
            let choose = choose.clone();
            let chosen = item_value.clone();
            view! {
                <div
                    role="option"
                    data-slot="select-item"
                    tabindex="-1"
                    data-value=item_value.clone()
                    data-disabled=disabled.then_some("")
                    aria-disabled=disabled.then_some("true")
                    aria-selected=move || checked.get().to_string()
                    data-state=state
                    class=ITEM_CLASS
                    style=style
                    on:pointermove=|event| {
                        if let Some(item) = event.current_target().and_then(|t| t.dyn_into::<HtmlElement>().ok()) {
                            let _ = item.focus();
                        }
                    }
                    on:click=move |_| choose(chosen.clone())
                >
                    <span class="pointer-events-none absolute right-2 flex size-4 items-center justify-center">
                        {move || checked.get().then(|| icons::check("pointer-events-none"))}
                    </span>
                    <span>
                        {swatch.map(|class| view! { <span class=format!("size-3 rounded-full {class}") aria-hidden="true"></span> })}
                        {label}
                    </span>
                </div>
            }
        }
    };

    let list = move || {
        let item_view = item_view.clone();
        entries
            .get()
            .into_iter()
            .map(|entry| match entry {
                SelectEntry::Item { value, label, disabled, swatch, style } => item_view(value, label, disabled, swatch, style).into_any(),
                SelectEntry::Group { label, items } => {
                    let item_view = item_view.clone();
                    view! {
                        <div role="group" data-slot="select-group" class=GROUP_CLASS>
                            <div data-slot="select-label" class=LABEL_CLASS>{label}</div>
                            {items
                                .into_iter()
                                .filter_map(|entry| match entry {
                                    SelectEntry::Item { value, label, disabled, swatch, style } => Some(item_view(value, label, disabled, swatch, style)),
                                    _ => None,
                                })
                                .collect_view()}
                        </div>
                    }
                    .into_any()
                }
                SelectEntry::Separator => view! { <div data-slot="select-separator" aria-hidden="true" class=SEPARATOR_CLASS></div> }.into_any(),
            })
            .collect_view()
    };

    let state = move || if open.get() { "open" } else { "closed" };
    let toggle = {
        let open_list = open_list.clone();
        let close = close.clone();
        move |_| if open.get_untracked() { close(true) } else { open_list() }
    };

    view! {
        <button
            type="button"
            role="combobox"
            id=id
            node_ref=trigger_ref
            data-slot="select-trigger"
            data-size=size.unwrap_or("default")
            aria-controls=content_id.clone()
            aria-label=aria_label
            aria-expanded=move || open.get().to_string()
            aria-autocomplete="none"
            dir="ltr"
            data-state=state
            data-disabled=move || disabled.get().then_some("")
            disabled=move || disabled.get()
            class=trigger_class(&class)
            style=move || trigger_style.get()
            on:click=toggle
            on:keydown=on_trigger_key
        >
            <span data-slot="select-value" style="pointer-events: none;">
                {move || {
                    let current = value.get();
                    entries.with(|list| list.iter().find_map(|e| e.swatch_of(&current)))
                        .map(|class| view! { <span class=format!("size-3 rounded-full {class}") aria-hidden="true"></span> })
                }}
                {move || label_of(&value.get())}
            </span>
            {icons::chevron_down("pointer-events-none size-4 text-muted-foreground")}
        </button>
        {move || {
            let list = list.clone();
            let on_content_key = on_content_key.clone();
            open.get().then(|| view! {
                <div
                    node_ref=wrapper_ref
                    style="position: fixed; left: 0px; top: 0px; z-index: 50; display: flex; flex-direction: column; box-sizing: border-box; pointer-events: auto; opacity: 0;"
                >
                    <div
                        node_ref=content_ref
                        id=content_id.clone()
                        role="listbox"
                        data-slot="select-content"
                        data-align-trigger="true"
                        data-state="open"
                        dir="ltr"
                        class=CONTENT_CLASS
                        style="box-sizing: border-box; display: flex; flex-direction: column; outline: none;"
                        on:keydown=on_content_key
                    >
                        <div data-radix-select-viewport="" role="presentation" style="position: relative; flex: 1 1 0%; overflow: hidden auto;">
                            {list()}
                        </div>
                    </div>
                </div>
            })
        }}
    }
}
