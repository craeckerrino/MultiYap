//! Nachbauten von alert-dialog.tsx, alert.tsx, slider.tsx und dropdown-menu.tsx (shadcn/ui, Radix).
//!
//! AlertDialog nutzt das eingebaute `<dialog>` mit `showModal()`: Es liegt in der obersten Ebene über allem (auch
//! über dem per transform zentrierten Einstellungsfenster), sperrt den Rest der Seite und bleibt trotzdem im Dokument
//! neben seinem Knopf, also innerhalb der Fokus-Falle des Einstellungsfensters. Escape schließt nur ihn: Ein Zuhörer
//! am Fenster in der Capture-Phase hält die Taste vor dem Fenster ab.
//!
//! Das Menü (DropdownMenu) liegt wie die Auswahlliste neben dem Knopf mit `position: fixed` und rechnet den
//! Bezugsrahmen heraus (siehe select.rs); unter dem Knopf, rechts oder links bündig, 4 px Abstand wie bei Radix.

use multiyap_core::i18n::tr;
use std::cell::RefCell;
use std::rc::Rc;

use leptos::prelude::*;
use send_wrapper::SendWrapper;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{HtmlElement, KeyboardEvent};

use super::button::button_class;
use super::icons;

pub(crate) fn window() -> web_sys::Window {
    web_sys::window().expect("Fenster")
}

/// Zuhörer am Fenster (Capture-Phase), die beim Schließen wieder entfernt werden.
type ListenerList = Rc<RefCell<Vec<(&'static str, JsValue)>>>;

#[derive(Clone)]
pub(crate) struct WindowListeners(SendWrapper<ListenerList>);

impl WindowListeners {
    pub(crate) fn new() -> Self {
        WindowListeners(SendWrapper::new(Rc::new(RefCell::new(Vec::new()))))
    }

    pub(crate) fn add(&self, kind: &'static str, listener: JsValue) {
        let _ = window().add_event_listener_with_callback_and_bool(kind, listener.unchecked_ref(), true);
        self.0.borrow_mut().push((kind, listener));
    }

    pub(crate) fn clear(&self) {
        for (kind, listener) in self.0.borrow_mut().drain(..) {
            let _ = window().remove_event_listener_with_callback_and_bool(kind, listener.unchecked_ref(), true);
        }
    }
}

/// Escape schließt nur die oberste Ebene: Taste abfangen, bevor sie das Einstellungsfenster erreicht.
pub(crate) fn escape_listener(close: impl Fn() + 'static) -> JsValue {
    Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| {
        if event.key() == "Escape" {
            event.stop_propagation();
            event.prevent_default();
            close();
        }
    })
    .into_js_value()
}

// ---- AlertDialog ----

/// `open:grid` statt `grid`: Eine feste Anzeige-Klasse würde das geschlossene `<dialog>` sichtbar machen.
const ALERT_CONTENT: &str = "group/alert-dialog-content fixed top-1/2 left-1/2 z-50 open:grid w-full -translate-x-1/2 -translate-y-1/2 gap-4 rounded-xl bg-popover p-4 text-popover-foreground ring-1 ring-foreground/10 duration-100 outline-none data-[size=default]:max-w-xs data-[size=sm]:max-w-xs data-[size=default]:sm:max-w-sm data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95";
/// Hintergrund wie AlertDialogOverlay, als ::backdrop des eingebauten Dialogs. Rahmen und Ränder des Browsers
/// zurückgesetzt.
const NATIVE_RESET: &str = "m-0 max-h-none max-w-none border-0 backdrop:bg-black/10 backdrop:backdrop-blur-xs";
const ALERT_HEADER: &str = "grid grid-rows-[auto_1fr] place-items-center gap-1.5 text-center has-data-[slot=alert-dialog-media]:grid-rows-[auto_auto_1fr] has-data-[slot=alert-dialog-media]:gap-x-4 sm:group-data-[size=default]/alert-dialog-content:place-items-start sm:group-data-[size=default]/alert-dialog-content:text-left sm:group-data-[size=default]/alert-dialog-content:has-data-[slot=alert-dialog-media]:grid-rows-[auto_1fr]";
const ALERT_FOOTER: &str = "-mx-4 -mb-4 flex flex-col-reverse gap-2 rounded-b-xl border-t bg-muted/50 p-4 group-data-[size=sm]/alert-dialog-content:grid group-data-[size=sm]/alert-dialog-content:grid-cols-2 sm:flex-row sm:justify-end";
const ALERT_TITLE: &str = "text-base font-medium sm:group-data-[size=default]/alert-dialog-content:group-has-data-[slot=alert-dialog-media]/alert-dialog-content:col-start-2";
const ALERT_DESCRIPTION: &str = "text-sm text-balance text-muted-foreground md:text-pretty *:[a]:underline *:[a]:underline-offset-3 *:[a]:hover:text-foreground";

/// Bestätigung vor einer folgenreichen Aktion: Knopf öffnet, „Abbrechen“ oder Escape schließt, die Aktion schließt
/// und ruft `on_confirm`. `trigger` ist der Inhalt des auslösenden Knopfs (Variante secondary, Größe sm).
#[component]
pub fn AlertDialog(
    #[prop(into)] title: String,
    #[prop(into)] description: String,
    #[prop(into)] action: String,
    #[prop(into)] on_confirm: Callback<()>,
    #[prop(optional, into)] trigger_disabled: Signal<bool>,
    children: Children,
) -> impl IntoView {
    let dialog_ref = NodeRef::<leptos::html::Dialog>::new();
    let open = RwSignal::new(false);
    let listeners = WindowListeners::new();
    let close = {
        let listeners = listeners.clone();
        move || {
            listeners.clear();
            if let Some(dialog) = dialog_ref.get_untracked() {
                dialog.close();
            }
            open.set(false);
        }
    };
    let show = {
        let listeners = listeners.clone();
        let close = close.clone();
        move || {
            if let Some(dialog) = dialog_ref.get_untracked() {
                let _ = dialog.show_modal();
                open.set(true);
                listeners.add("keydown", escape_listener(close.clone()));
            }
        }
    };
    on_cleanup({
        let listeners = listeners.clone();
        move || listeners.clear()
    });
    let (cancel, confirm) = (close.clone(), close.clone());
    view! {
        <button
            type="button"
            data-slot="alert-dialog-trigger"
            data-variant="secondary"
            data-size="sm"
            aria-haspopup="dialog"
            aria-expanded=move || open.get().to_string()
            disabled=move || trigger_disabled.get()
            class=button_class("secondary", "sm", "")
            on:click=move |_| show()
        >
            {children()}
        </button>
        <dialog
            node_ref=dialog_ref
            // Rolle nur, solange offen (andere Teile prüfen [role=alertdialog], um bei offenem Fenster zu ruhen).
            role=move || open.get().then_some("alertdialog")
            data-slot="alert-dialog-content"
            data-size="default"
            data-state=move || if open.get() { "open" } else { "closed" }
            class=format!("{ALERT_CONTENT} {NATIVE_RESET}")
            on:cancel=move |event: web_sys::Event| {
                event.prevent_default();
                cancel();
            }
        >
            <div data-slot="alert-dialog-header" class=ALERT_HEADER>
                <h2 data-slot="alert-dialog-title" class=ALERT_TITLE>{title}</h2>
                <p data-slot="alert-dialog-description" class=ALERT_DESCRIPTION>{description}</p>
            </div>
            <div data-slot="alert-dialog-footer" class=ALERT_FOOTER>
                <button type="button" data-slot="alert-dialog-cancel" class=button_class("outline", "default", "") on:click={
                    let close = close.clone();
                    move |_| close()
                }>
                    {tr("Abbrechen", "Cancel")}
                </button>
                <button type="button" data-slot="alert-dialog-action" class=button_class("default", "default", "") on:click=move |_| {
                    confirm();
                    on_confirm.run(());
                }>
                    {action}
                </button>
            </div>
        </dialog>
    }
}

/// Bestätigung ohne eigenen Knopf (geöffnet von außen, zum Beispiel „Tab schließen?“). „Abbrechen“, Esc und die
/// Aktion schließen; die Aktion ruft zusätzlich `on_confirm`.
#[component]
pub fn ConfirmDialog(
    #[prop(into)] open: Signal<bool>,
    #[prop(into)] title: String,
    #[prop(into)] description: Signal<String>,
    #[prop(into)] action: String,
    #[prop(into)] on_confirm: Callback<()>,
    #[prop(into)] on_cancel: Callback<()>,
) -> impl IntoView {
    let dialog_ref = NodeRef::<leptos::html::Dialog>::new();
    Effect::new(move |_| {
        let Some(dialog) = dialog_ref.get() else { return };
        if open.get() {
            if !dialog.open() {
                let _ = dialog.show_modal();
            }
        } else if dialog.open() {
            dialog.close();
        }
    });
    view! {
        <dialog
            node_ref=dialog_ref
            role=move || open.get().then_some("alertdialog")
            data-slot="alert-dialog-content"
            data-size="default"
            data-state=move || if open.get() { "open" } else { "closed" }
            class=format!("{ALERT_CONTENT} {NATIVE_RESET}")
            on:cancel=move |event: web_sys::Event| {
                event.prevent_default();
                on_cancel.run(());
            }
        >
            <div data-slot="alert-dialog-header" class=ALERT_HEADER>
                <h2 data-slot="alert-dialog-title" class=ALERT_TITLE>{title}</h2>
                <p data-slot="alert-dialog-description" class=ALERT_DESCRIPTION>{move || description.get()}</p>
            </div>
            <div data-slot="alert-dialog-footer" class=ALERT_FOOTER>
                <button type="button" data-slot="alert-dialog-cancel" class=button_class("outline", "default", "") on:click=move |_| on_cancel.run(())>
                    {tr("Abbrechen", "Cancel")}
                </button>
                <button type="button" data-slot="alert-dialog-action" class=button_class("default", "default", "") on:click=move |_| on_confirm.run(())>
                    {action}
                </button>
            </div>
        </dialog>
    }
}

// ---- Alert ----

const ALERT_BASE: &str = "group/alert relative grid w-full gap-0.5 rounded-lg border px-2.5 py-2 text-left text-sm has-data-[slot=alert-action]:relative has-data-[slot=alert-action]:pr-18 has-[>svg]:grid-cols-[auto_1fr] has-[>svg]:gap-x-2 *:[svg]:row-span-2 *:[svg]:translate-y-0.5 *:[svg]:text-current *:[svg:not([class*='size-'])]:size-4";

/// Hinweis-Kasten. `destructive`: Fehler (rot).
#[component]
pub fn Alert(
    #[prop(optional)] destructive: bool,
    #[prop(into)] title: String,
    #[prop(optional)] icon: Option<AnyView>,
    /// Knopf oben rechts (AlertAction).
    #[prop(optional)]
    action: Option<AnyView>,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    let variant = if destructive {
        "bg-card text-destructive *:data-[slot=alert-description]:text-destructive/90 *:[svg]:text-current"
    } else {
        "bg-card text-card-foreground"
    };
    view! {
        <div data-slot="alert" role="alert" class=format!("{ALERT_BASE} {variant}")>
            {icon}
            <div data-slot="alert-title" class="font-medium group-has-[>svg]/alert:col-start-2 [&_a]:underline [&_a]:underline-offset-3 [&_a]:hover:text-foreground">{title}</div>
            {children.map(|children| view! {
                <div data-slot="alert-description" class="text-sm text-balance text-muted-foreground md:text-pretty [&_a]:underline [&_a]:underline-offset-3 [&_a]:hover:text-foreground [&_p:not(:last-child)]:mb-4">
                    {children()}
                </div>
            })}
            {action.map(|action| view! { <div data-slot="alert-action" class="absolute top-2 right-2">{action}</div> })}
        </div>
    }
}

// ---- Slider ----

/// Schieberegler (ein Wert). `on_input` bei jeder Bewegung, `on_commit` beim Loslassen oder nach einer Taste.
#[component]
pub fn Slider(
    #[prop(into)] value: Signal<f64>,
    #[prop(into)] on_input: Callback<f64>,
    #[prop(into)] on_commit: Callback<f64>,
    #[prop(into)] label: String,
    #[prop(optional)] min: f64,
    #[prop(default = 100.0)] max: f64,
    #[prop(default = 1.0)] step: f64,
    #[prop(optional, into)] disabled: Signal<bool>,
) -> impl IntoView {
    let track_ref = NodeRef::<leptos::html::Span>::new();
    let dragging = RwSignal::new(false);
    let snap = move |raw: f64| ((raw - min) / step).round().mul_add(step, min).clamp(min, max);
    let from_pointer = move |client_x: i32| -> Option<f64> {
        let track: HtmlElement = track_ref.get_untracked()?.into();
        let rect = track.get_bounding_client_rect();
        let ratio = ((client_x as f64 - rect.left()) / rect.width()).clamp(0.0, 1.0);
        Some(snap(min + ratio * (max - min)))
    };
    let percent = move || (value.get() - min) / (max - min) * 100.0;
    let orientation_attrs = "horizontal";
    view! {
        <span
            data-slot="slider"
            data-orientation=orientation_attrs
            data-horizontal=""
            data-disabled=move || disabled.get().then_some("")
            dir="ltr"
            class="relative flex w-full touch-none items-center select-none data-disabled:opacity-50 data-vertical:h-full data-vertical:min-h-40 data-vertical:w-auto data-vertical:flex-col"
            on:pointerdown=move |event| {
                if disabled.get_untracked() {
                    return;
                }
                if let Some(target) = event.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                    let _ = target.set_pointer_capture(event.pointer_id());
                }
                dragging.set(true);
                if let Some(next) = from_pointer(event.client_x()) {
                    on_input.run(next);
                }
                if let Some(thumb) = event.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()).and_then(|e| e.query_selector("[role=slider]").ok().flatten()).and_then(|e| e.dyn_into::<HtmlElement>().ok()) {
                    let _ = thumb.focus();
                }
            }
            on:pointermove=move |event| {
                if dragging.get_untracked() {
                    if let Some(next) = from_pointer(event.client_x()) {
                        on_input.run(next);
                    }
                }
            }
            on:pointerup=move |event| {
                if dragging.get_untracked() {
                    dragging.set(false);
                    if let Some(next) = from_pointer(event.client_x()) {
                        on_commit.run(next);
                    }
                }
            }
        >
            <span node_ref=track_ref data-slot="slider-track" data-orientation=orientation_attrs data-horizontal=""
                class="relative grow overflow-hidden rounded-full bg-muted data-horizontal:h-1 data-horizontal:w-full data-vertical:h-full data-vertical:w-1">
                <span data-slot="slider-range" data-orientation=orientation_attrs data-horizontal=""
                    class="absolute bg-primary select-none data-horizontal:h-full data-vertical:w-full"
                    style=move || format!("left: 0%; right: {}%;", 100.0 - percent())></span>
            </span>
            <span style=move || format!("transform: translateX(-50%); position: absolute; left: calc({}% + {}px);", percent(), 6.0 - percent() * 0.12)>
                <span
                    role="slider"
                    data-slot="slider-thumb"
                    aria-label=label
                    aria-valuemin=min
                    aria-valuemax=max
                    aria-valuenow=move || value.get()
                    aria-orientation="horizontal"
                    tabindex=move || if disabled.get() { "-1" } else { "0" }
                    data-disabled=move || disabled.get().then_some("")
                    class="relative block size-3 shrink-0 rounded-full border border-ring bg-white ring-ring/50 transition-[color,box-shadow] select-none after:absolute after:-inset-2 hover:ring-3 focus-visible:ring-3 focus-visible:outline-hidden active:ring-3 disabled:pointer-events-none disabled:opacity-50"
                    on:keydown=move |event| {
                        if disabled.get_untracked() {
                            return;
                        }
                        let current = value.get_untracked();
                        let next = match event.key().as_str() {
                            "ArrowRight" | "ArrowUp" => current + step,
                            "ArrowLeft" | "ArrowDown" => current - step,
                            "PageUp" => current + step * 10.0,
                            "PageDown" => current - step * 10.0,
                            "Home" => min,
                            "End" => max,
                            _ => return,
                        };
                        event.prevent_default();
                        let next = snap(next);
                        on_input.run(next);
                        on_commit.run(next);
                    }
                ></span>
            </span>
        </span>
    }
}

// ---- DropdownMenu ----

const MENU_CONTENT: &str = "z-50 max-h-(--radix-dropdown-menu-content-available-height) w-(--radix-dropdown-menu-trigger-width) min-w-32 origin-(--radix-dropdown-menu-content-transform-origin) overflow-x-hidden overflow-y-auto rounded-lg bg-popover p-1 text-popover-foreground shadow-md ring-1 ring-foreground/10 duration-100 data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 data-[state=closed]:overflow-hidden data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95";
pub const MENU_ITEM: &str = "group/dropdown-menu-item relative flex cursor-default items-center gap-1.5 rounded-md px-1.5 py-1 text-sm outline-hidden select-none focus:bg-accent focus:text-accent-foreground not-data-[variant=destructive]:focus:**:text-accent-foreground data-inset:pl-7 data-[variant=destructive]:text-destructive data-[variant=destructive]:focus:bg-destructive/10 data-[variant=destructive]:focus:text-destructive dark:data-[variant=destructive]:focus:bg-destructive/20 data-disabled:pointer-events-none data-disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4 data-[variant=destructive]:*:[svg]:text-destructive";
pub const MENU_CHECKBOX: &str = "relative flex cursor-default items-center gap-1.5 rounded-md py-1 pr-8 pl-1.5 text-sm outline-hidden select-none focus:bg-accent focus:text-accent-foreground focus:**:text-accent-foreground data-inset:pl-7 data-disabled:pointer-events-none data-disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4";
pub const MENU_LABEL: &str = "px-1.5 py-1 text-xs font-medium text-muted-foreground data-inset:pl-7";
pub const MENU_SEPARATOR: &str = "-mx-1 my-1 h-px bg-border";

/// Eintrag eines Menüs.
#[derive(Clone)]
pub enum MenuEntry {
    Label(String),
    Separator,
    /// Kästchen mit Haken; bleibt beim Wählen offen (wie onSelect mit preventDefault).
    Checkbox { label: String, swatch: Option<String>, checked: bool, on_toggle: Callback<bool> },
    /// Normaler Eintrag; schließt das Menü.
    Item { label: String, on_select: Callback<()> },
    /// Eintrag mit Symbol davor; schließt das Menü.
    IconItem { icon: fn() -> AnyView, label: String, on_select: Callback<()> },
}

fn menu_items(content: &HtmlElement) -> Vec<HtmlElement> {
    let Ok(list) = content.query_selector_all("[role=menuitem],[role=menuitemcheckbox]") else { return Vec::new() };
    (0..list.length()).filter_map(|i| list.item(i)).filter_map(|n| n.dyn_into::<HtmlElement>().ok()).collect()
}

/// Menü an einem Knopf (Variante outline, Größe sm). `entries` wird beim Öffnen und bei Änderungen neu gebaut.
/// `align_end`: rechte Kanten bündig (sonst linke). `width_class`: Breite des Menüs, zum Beispiel "w-56".
#[component]
pub fn DropdownMenu(
    #[prop(into)] entries: Signal<Vec<MenuEntry>>,
    #[prop(optional)] align_end: bool,
    #[prop(into)] width_class: String,
    #[prop(optional, into)] trigger_class: String,
    /// Knopf nur mit Symbol (Größe icon-sm statt sm).
    #[prop(optional)]
    icon: bool,
    /// Größe des Knopfs, zum Beispiel "xs" (sonst sm, mit Symbol icon-sm).
    #[prop(optional)]
    trigger_size: Option<&'static str>,
    /// Variante des Knopfs (sonst outline).
    #[prop(optional)]
    trigger_variant: Option<&'static str>,
    #[prop(optional, into)] aria_label: Option<String>,
    children: Children,
) -> impl IntoView {
    let open = RwSignal::new(false);
    let trigger_ref = NodeRef::<leptos::html::Button>::new();
    let size = trigger_size.unwrap_or(if icon { "icon-sm" } else { "sm" });
    let variant = trigger_variant.unwrap_or("outline");
    let wrapper_ref = NodeRef::<leptos::html::Div>::new();
    let content_ref = NodeRef::<leptos::html::Div>::new();
    let listeners = WindowListeners::new();
    let id = format!("menu-{}", crate::bridge::random_id());

    let close = {
        let listeners = listeners.clone();
        move |refocus: bool| {
            listeners.clear();
            open.set(false);
            if refocus {
                if let Some(trigger) = trigger_ref.get_untracked() {
                    let _ = trigger.focus();
                }
            }
        }
    };

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
        let origin = wrapper.get_bounding_client_rect();
        let anchor = trigger.get_bounding_client_rect();
        // Maße ohne die Einblend-Animation (zoom-in-95 verkleinert getBoundingClientRect am Anfang).
        let (width, height) = (content.offset_width() as f64, content.offset_height() as f64);
        let mut left = if align_end { anchor.right() - width } else { anchor.left() };
        left = left.clamp(10.0, (view_width - 10.0 - width).max(10.0));
        // Unter dem Knopf, passt es nicht, darüber (wie Radix: Seite wechseln bei Platzmangel).
        let below = anchor.bottom() + 4.0;
        let top = if below + height > view_height - 10.0 && anchor.top() - 4.0 - height > 10.0 {
            anchor.top() - 4.0 - height
        } else {
            below
        };
        let _ = style.set_property("left", &format!("{}px", left - origin.left()));
        let _ = style.set_property("top", &format!("{}px", top - origin.top()));
        let _ = style.set_property("opacity", "1");
        if let Some(first) = menu_items(&content).into_iter().next() {
            let _ = first.focus();
        }
    };

    let open_menu = {
        let listeners = listeners.clone();
        let close = close.clone();
        move || {
            if open.get_untracked() {
                return;
            }
            open.set(true);
            listeners.add("keydown", escape_listener({
                let close = close.clone();
                move || close(true)
            }));
            let close_outside = close.clone();
            listeners.add(
                "pointerdown",
                Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
                    let target = event.target().and_then(|t| t.dyn_into::<web_sys::Node>().ok());
                    let inside = |node: Option<HtmlElement>| node.zip(target.clone()).is_some_and(|(n, t)| n.contains(Some(&t)));
                    if !inside(content_ref.get_untracked().map(Into::into)) && !inside(trigger_ref.get_untracked().map(Into::into)) {
                        close_outside(false);
                    }
                })
                .into_js_value(),
            );
            let close_scroll = close.clone();
            listeners.add(
                "scroll",
                Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
                    let target = event.target().and_then(|t| t.dyn_into::<web_sys::Node>().ok());
                    let in_content = content_ref.get_untracked().zip(target).is_some_and(|(c, t)| HtmlElement::from(c).contains(Some(&t)));
                    if !in_content {
                        close_scroll(false);
                    }
                })
                .into_js_value(),
            );
            request_animation_frame(position);
        }
    };

    on_cleanup({
        let listeners = listeners.clone();
        move || listeners.clear()
    });

    let on_key = {
        let close = close.clone();
        move |event: KeyboardEvent| {
            let Some(content) = content_ref.get_untracked() else { return };
            let items = menu_items(&content.into());
            let active = window().document().and_then(|d| d.active_element());
            let index = active.as_ref().and_then(|a| items.iter().position(|i| AsRef::<web_sys::Element>::as_ref(i) == a));
            let focus = |i: usize| {
                if let Some(item) = items.get(i) {
                    let _ = item.focus();
                }
            };
            match event.key().as_str() {
                "ArrowDown" => {
                    event.prevent_default();
                    focus(index.map_or(0, |i| (i + 1) % items.len().max(1)));
                }
                "ArrowUp" => {
                    event.prevent_default();
                    focus(index.map_or(items.len().saturating_sub(1), |i| if i == 0 { items.len().saturating_sub(1) } else { i - 1 }));
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
                    if let Some(i) = index {
                        items[i].click();
                    }
                }
                "Tab" => {
                    event.prevent_default();
                    close(true);
                }
                _ => {}
            }
        }
    };

    let render = {
        let close = close.clone();
        move || {
            entries
                .get()
                .into_iter()
                .map(|entry| match entry {
                    MenuEntry::Label(text) => view! { <div data-slot="dropdown-menu-label" class=MENU_LABEL>{text}</div> }.into_any(),
                    MenuEntry::Separator => view! { <div role="separator" aria-orientation="horizontal" data-slot="dropdown-menu-separator" class=MENU_SEPARATOR></div> }.into_any(),
                    MenuEntry::Checkbox { label, swatch, checked, on_toggle } => view! {
                        <div
                            role="menuitemcheckbox"
                            aria-checked=checked.to_string()
                            data-state=if checked { "checked" } else { "unchecked" }
                            data-slot="dropdown-menu-checkbox-item"
                            tabindex="-1"
                            class=MENU_CHECKBOX
                            on:pointermove=|event| {
                                if let Some(item) = event.current_target().and_then(|t| t.dyn_into::<HtmlElement>().ok()) {
                                    let _ = item.focus();
                                }
                            }
                            on:click=move |_| on_toggle.run(!checked)
                        >
                            <span class="pointer-events-none absolute right-2 flex items-center justify-center" data-slot="dropdown-menu-checkbox-item-indicator">
                                {checked.then(|| icons::check(""))}
                            </span>
                            {swatch.map(|class| view! { <span class=format!("size-3 rounded-full {class}") aria-hidden="true"></span> })}
                            {label}
                        </div>
                    }
                    .into_any(),
                    MenuEntry::IconItem { icon, label, on_select } => {
                        let close = close.clone();
                        view! {
                            <div role="menuitem" data-slot="dropdown-menu-item" data-variant="default" tabindex="-1" class=MENU_ITEM
                                on:pointermove=|event| {
                                    if let Some(item) = event.current_target().and_then(|t| t.dyn_into::<HtmlElement>().ok()) {
                                        let _ = item.focus();
                                    }
                                }
                                on:click=move |_| {
                                    close(true);
                                    on_select.run(());
                                }>
                                {icon()}
                                {label}
                            </div>
                        }
                        .into_any()
                    }
                    MenuEntry::Item { label, on_select } => {
                        let close = close.clone();
                        view! {
                            <div
                                role="menuitem"
                                data-slot="dropdown-menu-item"
                                data-variant="default"
                                tabindex="-1"
                                class=MENU_ITEM
                                on:pointermove=|event| {
                                    if let Some(item) = event.current_target().and_then(|t| t.dyn_into::<HtmlElement>().ok()) {
                                        let _ = item.focus();
                                    }
                                }
                                on:click=move |_| {
                                    close(true);
                                    on_select.run(());
                                }
                            >
                                {label}
                            </div>
                        }
                        .into_any()
                    }
                })
                .collect_view()
        }
    };

    let toggle = {
        let close = close.clone();
        let open_menu = open_menu.clone();
        move |_| if open.get_untracked() { close(true) } else { open_menu() }
    };

    view! {
        <button
            type="button"
            node_ref=trigger_ref
            data-slot="dropdown-menu-trigger"
            data-variant=variant
            data-size=size
            aria-label=aria_label
            aria-haspopup="menu"
            aria-expanded=move || open.get().to_string()
            aria-controls=id.clone()
            data-state=move || if open.get() { "open" } else { "closed" }
            class=button_class(variant, size, &trigger_class)
            on:click=toggle
            on:keydown={
                let open_menu = open_menu.clone();
                move |event: KeyboardEvent| {
                    if matches!(event.key().as_str(), "ArrowDown" | "Enter" | " ") {
                        event.prevent_default();
                        open_menu();
                    }
                }
            }
        >
            {children()}
        </button>
        {move || {
            let render = render.clone();
            let on_key = on_key.clone();
            let width_class = width_class.clone();
            let id = id.clone();
            open.get().then(|| view! {
                <div node_ref=wrapper_ref style="position: fixed; left: 0px; top: 0px; z-index: 50; min-width: max-content; opacity: 0;">
                    <div
                        node_ref=content_ref
                        id=id
                        role="menu"
                        aria-orientation="vertical"
                        data-slot="dropdown-menu-content"
                        data-state="open"
                        data-side="bottom"
                        tabindex="-1"
                        class=format!("{MENU_CONTENT} {width_class}")
                        on:keydown=on_key
                    >
                        {render()}
                    </div>
                </div>
            })
        }}
    }
}
