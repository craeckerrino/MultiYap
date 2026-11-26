//! Layout der Arbeitsfläche in Leptos, neu aufgebaut: oben eine Leiste mit Seiten; jede Seite teilt sich in Bereiche,
//! jeder Bereich zeigt genau einen Chat (oder Stream, Hervorhebungen) mit eigener Kopfzeile (Name, Schwert, Chat-Modi,
//! ⋮). Verschiebbare Trennlinien mit Einrasten. Einen Chat an den Rand eines Bereichs ziehen teilt ihn, auf einen
//! Seiten-Tab ziehen legt ihn auf diese Seite, auf die freie Leiste ziehen gibt ihm eine eigene Seite. Seiten lassen
//! sich in der Leiste umordnen.
//!
//! Alle Seiten bleiben eingehängt und verbunden, sichtbar ist die aktive. Der Baum einer Seite wird nur neu
//! gezeichnet, wenn sich seine Form ändert (Teilung, neuer Bereich). Anteile und Inhalte kommen über eigene Signale,
//! damit Chats beim Umschalten nicht neu aufgebaut werden. Tab-Einträge und Inhalte zeichnet crates/ui/src/tab_views.rs;
//! den Stand teilt die Arbeitsfläche (crates/ui/src/workspace.rs).

use multiyap_core::i18n::tr;
use multiyap_core::snapping::{snap_position, Snapped, SNAP_RATIOS};
use leptos::prelude::*;
use serde::Deserialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::KeyboardEvent;

use multiyap_web::workspace as web;

use crate::bridge::use_setting;
use crate::chat::layer::{Layer, LayerView};
use crate::components::button::button_class;
use crate::components::icons;
use crate::components::input::INPUT_CLASS;
use crate::components::overlay::window;
use crate::tab_views::{TabContent, TabItem};
use crate::workspace::{workspace_ctx, WorkspaceCtx, WorkspaceView};

/// Datenart beim Ziehen eines Tabs (Chat in einem Bereich).
pub const TAB_DRAG_TYPE: &str = "application/x-multiyap-tab";
/// Datenart beim Ziehen einer Seite in der oberen Leiste.
const PAGE_DRAG_TYPE: &str = "application/x-multiyap-page";
const DOUBLE_CLICK_MS: f64 = 400.0;
/// So lange einen Chat über einem Seiten-Tab halten, bis die Seite erscheint.
const HOVER_SWITCH_MS: f64 = 600.0;

/// Knoten des Layout-Baums (wie in crates/core/src/workspace.rs).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Node {
    #[serde(rename_all = "camelCase")]
    Area { id: String, tab_ids: Vec<String>, active_tab_id: Option<String> },
    Split { id: String, direction: String, ratio: f64, first: Box<Node>, second: Box<Node> },
}

/// Form des Baums ohne Anteile und Tabs: Nur eine Änderung daran zeichnet den Baum neu.
#[derive(Clone, Debug, PartialEq)]
enum Shape {
    Area(String),
    Split(String, bool, Box<Shape>, Box<Shape>),
}

fn shape_of(node: &Node) -> Shape {
    match node {
        Node::Area { id, .. } => Shape::Area(id.clone()),
        Node::Split { id, direction, first, second, .. } => Shape::Split(id.clone(), direction == "row", Box::new(shape_of(first)), Box::new(shape_of(second))),
    }
}

/// Inhalt eines Bereichs: None, wenn es den Bereich nicht gibt; Some(None) bei einem leeren Bereich.
pub fn area_tab(node: &Node, area_id: &str) -> Option<Option<String>> {
    match node {
        Node::Area { id, tab_ids, .. } => (id == area_id).then(|| tab_ids.first().cloned()),
        Node::Split { first, second, .. } => area_tab(first, area_id).or_else(|| area_tab(second, area_id)),
    }
}

/// Alle Tabs eines Baums in Lesereihenfolge.
pub fn tabs_in(node: &Node) -> Vec<String> {
    match node {
        Node::Area { tab_ids, .. } => tab_ids.clone(),
        Node::Split { first, second, .. } => {
            let mut list = tabs_in(first);
            list.extend(tabs_in(second));
            list
        }
    }
}

fn find_ratio(node: &Node, split_id: &str) -> Option<f64> {
    match node {
        Node::Area { .. } => None,
        Node::Split { id, ratio, first, second, .. } => if id == split_id { Some(*ratio) } else { find_ratio(first, split_id).or_else(|| find_ratio(second, split_id)) },
    }
}

/// Änderung an Tabs oder Layout im Hintergrund; schlägt das Speichern fehl, ein Hinweis (der Stand bleibt dann
/// unverändert).
pub fn run(change: impl std::future::Future<Output = Result<(), String>> + 'static) {
    leptos::task::spawn_local(async move {
        if let Err(message) = change.await {
            multiyap_web::chat_tools::toast("error", &message);
        }
    });
}

/// Gemeinsamer Stand des Layouts für alle Teile.
#[derive(Clone, Copy)]
struct Ctx {
    workspace: RwSignal<WorkspaceView>,
    locked: ReadSignal<bool>,
    dragged: RwSignal<Option<String>>,
    /// Seite, die gerade in der oberen Leiste gezogen wird.
    dragged_page: RwSignal<Option<String>>,
    layer: Layer,
    shared: WorkspaceCtx,
}

fn has_type(event: &web_sys::DragEvent, kind: &str) -> bool {
    event.data_transfer().is_some_and(|d| d.types().iter().any(|t| t.as_string().as_deref() == Some(kind)))
}

fn is_tab_drag(ctx: Ctx, event: &web_sys::DragEvent) -> bool {
    has_type(event, TAB_DRAG_TYPE) && ctx.dragged.with_untracked(Option::is_some)
}

fn is_page_drag(ctx: Ctx, event: &web_sys::DragEvent) -> bool {
    has_type(event, PAGE_DRAG_TYPE) && ctx.dragged_page.with_untracked(Option::is_some)
}

fn tip(layer: Layer, text: &'static str) -> (impl Fn(web_sys::PointerEvent) + Clone, impl Fn(web_sys::PointerEvent) + Clone) {
    let enter = move |event: web_sys::PointerEvent| {
        if let Some(element) = event.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
            layer.show_tip(&element, text.to_string(), false);
        }
    };
    (enter, move |_| layer.hide_tip())
}

/// Verlässt das Ziehen das Element wirklich (nicht nur in ein Kind hinein)?
fn leave_inside(event: &web_sys::DragEvent) -> bool {
    let related = event.related_target().and_then(|t| t.dyn_into::<web_sys::Node>().ok());
    event.current_target().and_then(|t| t.dyn_into::<web_sys::Node>().ok()).is_some_and(|c| related.is_some_and(|r| c.contains(Some(&r))))
}

fn split_view(ctx: Ctx, page: StoredValue<String>, id: String, row: bool, first: Shape, second: Shape) -> AnyView {
    let stored_id = id.clone();
    let ratio = Memo::new(move |_| ctx.workspace.with(|w| w.pages.iter().find_map(|p| find_ratio(&p.root, &stored_id))).unwrap_or(0.5));
    let drag = RwSignal::new(None::<(f64, Option<Snapped>)>);
    let shown = move || drag.with(|d| d.map(|d| d.0)).unwrap_or_else(|| ratio.get());
    let container = NodeRef::<leptos::html::Div>::new();
    let divider = NodeRef::<leptos::html::Div>::new();
    let last_down = StoredValue::new(None::<(f64, f64, f64)>);
    let direction = if row { "row" } else { "column" };
    let on_down = {
        let id = id.clone();
        move |event: web_sys::PointerEvent| {
            if ctx.locked.get_untracked() || event.button() != 0 {
                return;
            }
            let (Some(container), Some(divider)) = (container.get_untracked(), divider.get_untracked()) else { return };
            event.prevent_default();
            // Doppelklick selbst erkennen: das Abfangen des Zeigers unterdrückt das dblclick-Ereignis.
            let now = js_sys::Date::now();
            let (x, y) = (f64::from(event.client_x()), f64::from(event.client_y()));
            let previous = last_down.get_value();
            last_down.set_value(Some((now, x, y)));
            if previous.is_some_and(|(t, px, py)| now - t < DOUBLE_CLICK_MS && (x - px).hypot(y - py) < 5.0) {
                last_down.set_value(None);
                let id = id.clone();
                run(async move { web::set_split_ratio(&id, 0.5).await });
                return;
            }
            let divider_element: web_sys::Element = divider.clone().into();
            let _ = divider_element.set_pointer_capture(event.pointer_id());
            // Linien anderer Teilungen gleicher Richtung, als Position innerhalb dieses Containers.
            let rect = container.get_bounding_client_rect();
            let mut lines = Vec::new();
            if let Ok(list) = window().document().expect("Dokument").query_selector_all(&format!("[data-divider=\"{direction}\"]")) {
                for element in (0..list.length()).filter_map(|i| list.item(i)).filter_map(|n| n.dyn_into::<web_sys::Element>().ok()) {
                    if element == divider_element {
                        continue;
                    }
                    let r = element.get_bounding_client_rect();
                    if row {
                        let line = r.left() + r.width() / 2.0;
                        if line > rect.left() && line < rect.right() {
                            lines.push(line - rect.left());
                        }
                    } else {
                        let line = r.top() + r.height() / 2.0;
                        if line > rect.top() && line < rect.bottom() {
                            lines.push(line - rect.top());
                        }
                    }
                }
            }
            let latest = std::rc::Rc::new(std::cell::Cell::new(ratio.get_untracked()));
            drag.set(Some((latest.get(), None)));
            let target = divider_element.clone();
            let listeners: std::rc::Rc<std::cell::RefCell<Vec<(&str, JsValue)>>> = Default::default();
            let on_move = {
                let (container, latest) = (container.clone(), latest.clone());
                Closure::<dyn FnMut(web_sys::PointerEvent)>::new(move |event: web_sys::PointerEvent| {
                    let rect = container.get_bounding_client_rect();
                    let size = if row { rect.width() } else { rect.height() };
                    let raw = if row { f64::from(event.client_x()) - rect.left() } else { f64::from(event.client_y()) - rect.top() };
                    let result = snap_position(raw, size, &lines, !event.alt_key());
                    latest.set(result.position / size);
                    drag.set(Some((latest.get(), result.snapped_to)));
                })
                .into_js_value()
            };
            let on_up = {
                let (id, latest, listeners, target) = (id.clone(), latest.clone(), listeners.clone(), target.clone());
                Closure::<dyn FnMut()>::new(move || {
                    for (kind, listener) in listeners.borrow_mut().drain(..) {
                        let _ = target.remove_event_listener_with_callback(kind, listener.unchecked_ref());
                    }
                    drag.set(None);
                    let (id, ratio) = (id.clone(), latest.get());
                    run(async move { web::set_split_ratio(&id, ratio).await });
                })
                .into_js_value()
            };
            for (kind, listener) in [("pointermove", on_move), ("pointerup", on_up.clone()), ("pointercancel", on_up)] {
                let _ = target.add_event_listener_with_callback(kind, listener.unchecked_ref());
                listeners.borrow_mut().push((kind, listener));
            }
        }
    };
    view! {
        <div node_ref=container class=if row { "relative flex min-h-0 min-w-0 flex-1 flex-row" } else { "relative flex min-h-0 min-w-0 flex-1 flex-col" }>
            <div class="flex min-h-0 min-w-0 flex-col" style:flex-basis=move || format!("{}%", shown() * 100.0) style:flex-grow="0" style:flex-shrink="0">
                {node_view(ctx, page, first)}
            </div>
            <div node_ref=divider data-divider=direction role="separator" aria-orientation=if row { "vertical" } else { "horizontal" }
                aria-valuenow=move || (shown() * 100.0).round().to_string()
                class=move || {
                    let locked = ctx.locked.get();
                    // Klar erkennbare Linie; gesperrt nur eine schmale, ruhige Linie.
                    let size = match (locked, row) { (true, true) => "w-px", (true, false) => "h-px", (false, true) => "w-0.5", (false, false) => "h-0.5" };
                    let color = if drag.with(Option::is_some) { "bg-primary" } else if locked { "bg-border" } else { "bg-muted hover:bg-primary/60" };
                    format!("relative z-10 shrink-0 transition-colors {size} {color}")
                }>
                <div class=move || {
                    let area = if row { "absolute inset-y-0 -right-1 -left-1" } else { "absolute inset-x-0 -top-1 -bottom-1" };
                    let cursor = if ctx.locked.get() { "" } else if row { " cursor-col-resize" } else { " cursor-row-resize" };
                    format!("{area}{cursor}")
                }
                    title=move || (!ctx.locked.get()).then_some(tr("Ziehen zum Verschieben, Doppelklick setzt auf die Mitte, Alt setzt das Einrasten aus", "Drag to move, double-click centers it, Alt turns off snapping"))
                    on:pointerdown=on_down>
                </div>
            </div>
            <div class="flex min-h-0 min-w-0 flex-1 flex-col">{node_view(ctx, page, second)}</div>
            {move || drag.get().map(|(_, snapped)| SNAP_RATIOS.iter().map(|snap| {
                let active = matches!(snapped, Some(Snapped::Ratio(r)) if (r - snap).abs() < 1e-9);
                let class = format!("pointer-events-none absolute z-20 {} {}", if row { "inset-y-0 w-px" } else { "inset-x-0 h-px" }, if active { "bg-primary" } else { "bg-primary/30" });
                let position = format!("{}%", snap * 100.0);
                view! { <div aria-hidden="true" class=class style:left=row.then(|| position.clone()) style:top=(!row).then_some(position)></div> }
            }).collect_view())}
        </div>
    }
    .into_any()
}

#[derive(Clone, Copy, PartialEq)]
enum Zone {
    Left,
    Right,
    Top,
    Bottom,
    Center,
}

impl Zone {
    fn class(self) -> &'static str {
        match self {
            Zone::Left => "inset-y-0 left-0 w-1/2",
            Zone::Right => "inset-y-0 right-0 w-1/2",
            Zone::Top => "inset-x-0 top-0 h-1/2",
            Zone::Bottom => "inset-x-0 bottom-0 h-1/2",
            Zone::Center => "inset-0",
        }
    }

    fn side(self) -> &'static str {
        match self {
            Zone::Left => "left",
            Zone::Right => "right",
            Zone::Top => "top",
            Zone::Bottom => "bottom",
            Zone::Center => "center",
        }
    }
}

/// Ziel beim Ziehen über einen Bereich: immer die nächste Seite, der Bereich wird geteilt. Keine mittige Ablage, denn
/// ein Bereich zeigt genau einen Inhalt. Nur ein leerer Bereich nimmt den Tab mittig auf.
fn zone_at(event: &web_sys::DragEvent, element: &web_sys::Element, empty: bool) -> Zone {
    if empty {
        return Zone::Center;
    }
    let rect = element.get_bounding_client_rect();
    let x = (f64::from(event.client_x()) - rect.left()) / rect.width();
    let y = (f64::from(event.client_y()) - rect.top()) / rect.height();
    [(Zone::Left, x), (Zone::Right, 1.0 - x), (Zone::Top, y), (Zone::Bottom, 1.0 - y)]
        .into_iter()
        .fold((Zone::Left, f64::INFINITY), |best, entry| if entry.1 < best.1 { entry } else { best })
        .0
}

/// Ein Bereich mit genau einem Inhalt und eigener Kopfzeile (Name, Schwert, Chat-Modi, ⋮, Knöpfe des Inhalts, „+“ für
/// einen Chat daneben). Den Chat an den Rand eines anderen Bereichs ziehen teilt diesen.
fn area_view(ctx: Ctx, page: StoredValue<String>, id: String) -> AnyView {
    let area_id = StoredValue::new(id);
    let tab = Memo::new(move |_| {
        let id = area_id.get_value();
        ctx.workspace.with(|w| w.pages.iter().find_map(|p| area_tab(&p.root, &id)).flatten())
    });
    let page_active = Signal::derive(move || ctx.workspace.with(|w| page.with_value(|p| w.active_page_id == *p)));
    // Hervorgehoben nur, wenn die Seite mehrere Bereiche hat.
    let focused = Memo::new(move |_| ctx.workspace.with(|w| w.focused_area_id == area_id.get_value() && page.with_value(|p| w.page_tabs(p).len() > 1)));
    let zone = RwSignal::new(None::<Zone>);
    let content = NodeRef::<leptos::html::Div>::new();
    let (add_enter, add_leave) = tip(ctx.layer, tr("Chat daneben öffnen", "Open chat next to it"));
    let add = move || ctx.shared.add_target.set(Some(area_id.get_value()));
    let draggable = Signal::derive(move || !ctx.locked.get());
    view! {
        <section class="flex min-h-0 min-w-0 flex-1 flex-col" aria-label=tr("Bereich", "Area")
            on:pointerdown:capture=move |_| {
                let area = area_id.get_value();
                if ctx.workspace.with_untracked(|w| w.focused_area_id != area) {
                    run(async move { web::focus_area(&area).await });
                }
            }>
            {move || tab.get().map(|tab_id| view! {
                // Eigene Kopfzeile je Bereich; der Unterstrich der Tab-Leiste entfällt hier.
                <div class=move || format!("flex h-9 shrink-0 items-stretch gap-1 border-b pr-1 [&_[role=tab]]:border-b-transparent{}", if focused.get() { " bg-muted/40" } else { "" })>
                    <TabItem tab_id=tab_id area_id=area_id.get_value() active=page_active draggable=draggable />
                    // Knöpfe des Inhalts (zum Beispiel die Chatter-Liste, crates/ui/src/chat/top.rs).
                    <div data-tab-actions="" class="ml-auto flex items-center gap-1"></div>
                    <div class="flex items-center">
                        <button type="button" data-slot="button" class=button_class("ghost", "icon-xs", "") aria-label=tr("Chat daneben öffnen", "Open chat next to it")
                            on:pointerenter=add_enter.clone() on:pointerleave=add_leave.clone() on:click=move |_| { ctx.layer.hide_tip(); add(); }>
                            {icons::plus("")}
                        </button>
                    </div>
                </div>
            })}
            <div node_ref=content class="relative flex min-h-0 flex-1 flex-col"
                on:dragover=move |event: web_sys::DragEvent| {
                    if ctx.locked.get_untracked() || !is_tab_drag(ctx, &event) { return; }
                    let Some(element) = content.get_untracked() else { return };
                    event.prevent_default();
                    if let Some(data) = event.data_transfer() { data.set_drop_effect("move"); }
                    let next = zone_at(&event, &element, tab.with_untracked(Option::is_none));
                    if zone.get_untracked() != Some(next) { zone.set(Some(next)); }
                }
                on:dragleave=move |event: web_sys::DragEvent| if !leave_inside(&event) { zone.set(None) }
                on:drop=move |event: web_sys::DragEvent| {
                    let dragged = ctx.dragged.get_untracked();
                    let empty = tab.with_untracked(Option::is_none);
                    let Some(target) = content.get_untracked().map(|element| zone_at(&event, &element, empty)) else { return };
                    zone.set(None);
                    let Some(dragged) = dragged.filter(|_| !ctx.locked.get_untracked()) else { return };
                    event.prevent_default();
                    let (area, side) = (area_id.get_value(), target.side());
                    run(async move { web::split_with_tab(&dragged, &area, side).await });
                    ctx.shared.end_drag();
                }>
                {move || match tab.get() {
                    Some(tab_id) => view! { <TabContent tab_id=tab_id active=page_active /> }.into_any(),
                    None => view! {
                        <div class="flex flex-1 flex-col items-center justify-center gap-3 text-sm text-muted-foreground">
                            <p>{tr("Noch kein Tab geöffnet.", "No tab open yet.")}</p>
                            <button type="button" data-slot="button" class=button_class("secondary", "default", "") on:click=move |_| add()>
                                {icons::plus("")}
                                {tr("Neuer Tab", "New tab")}
                            </button>
                        </div>
                    }.into_any(),
                }}
                {move || (ctx.dragged.with(Option::is_some)).then(|| zone.get()).flatten().map(|z| view! {
                    <div aria-hidden="true" class=format!("pointer-events-none absolute z-30 rounded-md border-2 border-primary bg-primary/10 {}", z.class())></div>
                })}
            </div>
        </section>
    }
    .into_any()
}

fn node_view(ctx: Ctx, page: StoredValue<String>, shape: Shape) -> AnyView {
    match shape {
        Shape::Area(id) => area_view(ctx, page, id),
        Shape::Split(id, row, first, second) => split_view(ctx, page, id, row, *first, *second),
    }
}

/// Ziel beim Ziehen über die Seiten-Leiste.
#[derive(Clone, Debug, PartialEq)]
enum PageTarget {
    /// Seite zwischen die Seiten schieben.
    Index(usize),
    /// Chat auf diese Seite legen.
    Page(String),
    /// Chat auf eine eigene neue Seite.
    NewPage,
}

fn page_elements(bar: &web_sys::Element) -> Vec<web_sys::Element> {
    let Ok(list) = bar.query_selector_all("[data-page-id]") else { return Vec::new() };
    (0..list.length()).filter_map(|i| list.item(i)).filter_map(|n| n.dyn_into().ok()).collect()
}

fn page_target(bar: &web_sys::Element, event: &web_sys::DragEvent, tab_drag: bool) -> PageTarget {
    let elements = page_elements(bar);
    let (x, y) = (f64::from(event.client_x()), f64::from(event.client_y()));
    if tab_drag {
        return elements
            .iter()
            .find(|e| {
                let r = e.get_bounding_client_rect();
                x >= r.left() && x <= r.right() && y >= r.top() && y <= r.bottom()
            })
            .and_then(|e| e.get_attribute("data-page-id"))
            .map_or(PageTarget::NewPage, PageTarget::Page);
    }
    // Die Leiste bricht in mehrere Zeilen um: nur Seiten der Zeile unter dem Zeiger zählen.
    let row: Vec<(usize, web_sys::DomRect)> = elements.iter().map(|e| e.get_bounding_client_rect()).enumerate().filter(|(_, r)| y >= r.top() && y <= r.bottom()).collect();
    let Some(last) = row.last() else { return PageTarget::Index(elements.len()) };
    PageTarget::Index(row.iter().find(|(_, r)| x < r.left() + r.width() / 2.0).map_or(last.0 + 1, |(i, _)| *i))
}

/// Ein Seiten-Tab: Name (eigener Name oder die Namen der Chats), Live-Punkt, „ungesehen“ und Blinken, wenn einer
/// seiner Chats es meldet. Klick zeigt die Seite, Doppelklick benennt um, Mittelklick oder X schließt sie.
fn page_tab(ctx: Ctx, id: String, drop_index: RwSignal<Option<PageTarget>>) -> impl IntoView {
    let page_id = StoredValue::new(id);
    let tabs = Memo::new(move |_| ctx.workspace.with(|w| w.page_tabs(&page_id.get_value())));
    let own_name = Memo::new(move |_| ctx.workspace.with(|w| w.page(&page_id.get_value()).and_then(|p| p.name.clone())));
    let statuses = Memo::new(move |_| {
        tabs.get()
            .iter()
            .map(|id| {
                ctx.shared.status(id).unwrap_or_else(|| crate::workspace::TabStatus {
                    name: ctx.shared.tab(id).map(|t| t.nickname.unwrap_or(if t.kind == "highlights" { crate::tabs::highlights_default_name().to_string() } else { t.channel })).unwrap_or_default(),
                    ..Default::default()
                })
            })
            .collect::<Vec<_>>()
    });
    let auto_name = Memo::new(move |_| {
        let names: Vec<String> = statuses.with(|s| s.iter().map(|s| s.name.clone()).filter(|n| !n.is_empty()).collect());
        if names.is_empty() { tr("Neue Seite", "New page").to_string() } else { names.join(", ") }
    });
    let name = Memo::new(move |_| own_name.get().unwrap_or_else(|| auto_name.get()));
    let active = Memo::new(move |_| ctx.workspace.with(|w| w.active_page_id == page_id.get_value()));
    let live = Memo::new(move |_| statuses.with(|s| s.iter().any(|s| s.live)));
    let unread = Memo::new(move |_| !active.get() && statuses.with(|s| s.iter().any(|s| s.unread)));
    let blink = Memo::new(move |_| if active.get() { String::new() } else { statuses.with(|s| s.iter().map(|s| s.blink.clone()).find(|b| !b.is_empty())).unwrap_or_default() });
    let renaming = RwSignal::new(false);
    let draft = RwSignal::new(String::new());
    let rename_ref = NodeRef::<leptos::html::Input>::new();
    Effect::new(move |_| {
        if let Some(input) = rename_ref.get() {
            let _ = input.focus();
            input.select();
        }
    });
    let close = move || ctx.shared.request_close_page(page_id.get_value(), name.get_untracked());
    let index = Memo::new(move |_| ctx.workspace.with(|w| w.pages.iter().position(|p| p.id == page_id.get_value())));
    let class = move || {
        let mut class = String::from("group/tab flex h-full shrink-0 items-center gap-0.5 border-b-2 pl-3 pr-1 text-sm");
        class.push_str(if active.get() {
            " border-primary text-foreground"
        } else if unread.get() {
            " border-foreground/60 text-foreground font-semibold"
        } else {
            " border-transparent text-muted-foreground hover:text-foreground"
        });
        let pulse = blink.get();
        if !pulse.is_empty() {
            class.push_str(&format!(" animate-pulse {pulse}"));
        } else if live.get() {
            class.push_str(" bg-live/10");
        }
        if drop_index.with(|d| matches!(d, Some(PageTarget::Page(p)) if *p == page_id.get_value())) {
            class.push_str(" rounded-md ring-2 ring-primary ring-inset");
        }
        class
    };
    view! {
        <div class="flex h-10 items-stretch">
            {move || (drop_index.get().is_some() && drop_index.get() == index.get().map(PageTarget::Index)).then(|| view! { <span aria-hidden="true" class="my-2 w-0.5 rounded-full bg-primary"></span> })}
            <div role="tab" data-page-id=move || page_id.get_value() aria-selected=move || active.get().to_string()
                title=tr("Doppelklick benennt die Seite um", "Double-click renames the page")
                draggable=move || (!ctx.locked.get() && !renaming.get()).to_string()
                class=class
                on:dragstart=move |event: web_sys::DragEvent| {
                    if let Some(data) = event.data_transfer() {
                        let _ = data.set_data(PAGE_DRAG_TYPE, &page_id.get_value());
                        data.set_effect_allowed("move");
                    }
                    ctx.dragged_page.set(Some(page_id.get_value()));
                }
                on:dragend=move |_| ctx.dragged_page.set(None)
                on:mousedown=|event: web_sys::MouseEvent| if event.button() == 1 { event.prevent_default() }
                on:auxclick=move |event: web_sys::MouseEvent| if event.button() == 1 { close() }>
                {move || live.get().then(|| view! { <span class="mr-1 size-2 shrink-0 rounded-full bg-live" aria-label="Live"></span> })}
                {move || if renaming.get() {
                    view! {
                        <input node_ref=rename_ref data-slot="input" class=INPUT_CLASS.replacen("h-8 w-full", "h-7 w-40", 1) maxlength="50" aria-label=tr("Name der Seite", "Name of the page")
                            prop:value=move || draft.get() on:input=move |event| draft.set(event_target_value(&event))
                            on:keydown=move |event: KeyboardEvent| match event.key().as_str() {
                                "Enter" => {
                                    event.prevent_default();
                                    // Leer oder wie die Namen der Chats: eigener Name entfällt.
                                    let value = draft.get_untracked().trim().to_string();
                                    let own = (!value.is_empty() && value != auto_name.get_untracked()).then_some(value);
                                    let id = page_id.get_value();
                                    run(async move { web::rename_page(&id, own).await });
                                    renaming.set(false);
                                }
                                "Escape" => {
                                    event.prevent_default();
                                    renaming.set(false);
                                }
                                _ => {}
                            }
                            on:blur=move |_| renaming.set(false) />
                    }.into_any()
                } else {
                    view! {
                        <button type="button" class="max-w-64 truncate py-1 outline-none focus-visible:underline"
                            on:click=move |_| { let id = page_id.get_value(); run(async move { web::activate_page(&id).await }); }
                            on:dblclick=move |_| { draft.set(name.get_untracked()); renaming.set(true); }>
                            {move || name.get()}
                        </button>
                    }.into_any()
                }}
                {move || (!tabs.with(Vec::is_empty)).then(|| view! {
                    <button type="button" data-slot="button" class=button_class("ghost", "icon-xs", "ml-0.5") aria-label=tr("Seite schließen", "Close page") on:click=move |_| close()>
                        {icons::x("")}
                    </button>
                })}
            </div>
        </div>
    }
}

/// Obere Leiste mit den Seiten, „+“ für eine neue Seite und „Zuletzt geschlossenen Tab wiederherstellen“.
fn page_bar(ctx: Ctx) -> impl IntoView {
    let bar = NodeRef::<leptos::html::Div>::new();
    let target = RwSignal::new(None::<PageTarget>);
    // Seite, über der ein Chat gerade gehalten wird, und seit wann.
    let hover = StoredValue::new(None::<(String, f64)>);
    // Endet das Ziehen woanders (Ablegen in einem Bereich, Esc), verschwindet auch die Anzeige hier.
    Effect::new(move |_| {
        if ctx.dragged.with(Option::is_none) && ctx.dragged_page.with(Option::is_none) && target.with_untracked(Option::is_some) {
            target.set(None);
            hover.set_value(None);
        }
    });
    let page_ids = Memo::new(move |_| ctx.workspace.with(|w| w.pages.iter().map(|p| p.id.clone()).collect::<Vec<_>>()));
    let (add_enter, add_leave) = tip(ctx.layer, tr("Neue Seite", "New page"));
    let (restore_enter, restore_leave) = tip(ctx.layer, tr("Zuletzt geschlossenen Tab wiederherstellen (Strg+Umschalt+T)", "Restore last closed tab (Ctrl+Shift+T)"));
    view! {
        <div node_ref=bar role="tablist" aria-label=tr("Seiten", "Pages")
            // Viele Seiten brechen in weitere Zeilen um, statt seitlich zu scrollen.
            class="flex min-h-10 shrink-0 flex-wrap items-stretch gap-x-1 border-b px-2"
            on:dragover=move |event: web_sys::DragEvent| {
                let (tab_drag, page_drag) = (is_tab_drag(ctx, &event), is_page_drag(ctx, &event));
                if ctx.locked.get_untracked() || !(tab_drag || page_drag) { return; }
                let Some(element) = bar.get_untracked() else { return };
                event.prevent_default();
                if let Some(data) = event.data_transfer() { data.set_drop_effect("move"); }
                let next = Some(page_target(&element, &event, tab_drag));
                // Chat kurz über einer Seite halten zeigt sie, um ihn dort an den Rand eines Bereichs zu legen.
                match &next {
                    Some(PageTarget::Page(page)) => {
                        let now = js_sys::Date::now();
                        match hover.get_value() {
                            Some((held, since)) if held == *page => {
                                if now - since > HOVER_SWITCH_MS {
                                    hover.set_value(Some((held, f64::INFINITY)));
                                    let page = page.clone();
                                    run(async move { web::activate_page(&page).await });
                                }
                            }
                            _ => hover.set_value(Some((page.clone(), now))),
                        }
                    }
                    _ => hover.set_value(None),
                }
                if target.with_untracked(|t| *t != next) { target.set(next); }
            }
            on:dragleave=move |event: web_sys::DragEvent| if !leave_inside(&event) { target.set(None); hover.set_value(None); }
            on:drop=move |event: web_sys::DragEvent| {
                let found = target.get_untracked();
                target.set(None);
                hover.set_value(None);
                if ctx.locked.get_untracked() { return; }
                let (tab, page) = (ctx.dragged.get_untracked(), ctx.dragged_page.get_untracked());
                match (found, tab, page) {
                    (Some(PageTarget::Index(index)), _, Some(page)) => run(async move { web::move_page(&page, index).await }),
                    (Some(PageTarget::Page(page)), Some(tab), _) => run(async move { web::move_tab_to_page(&tab, Some(page)).await }),
                    (Some(PageTarget::NewPage), Some(tab), _) => run(async move { web::move_tab_to_page(&tab, None).await }),
                    _ => return,
                }
                event.prevent_default();
                ctx.shared.end_drag();
                ctx.dragged_page.set(None);
            }>
            <For each=move || page_ids.get() key=|id| id.clone() children=move |id| page_tab(ctx, id, target) />
            {move || (target.get().is_some() && target.get() == Some(PageTarget::Index(page_ids.with(Vec::len)))).then(|| view! { <span aria-hidden="true" class="my-2 w-0.5 rounded-full bg-primary"></span> })}
            {move || (target.get() == Some(PageTarget::NewPage)).then(|| view! {
                <div aria-hidden="true" class="my-1.5 flex items-center rounded-md border-2 border-dashed border-primary px-3 text-sm text-primary">{tr("Neue Seite", "New page")}</div>
            })}
            <div class="flex h-10 items-center gap-1">
                <button type="button" data-slot="button" class=button_class("ghost", "icon-sm", "") aria-label=tr("Neue Seite", "New page")
                    on:pointerenter=add_enter on:pointerleave=add_leave on:click=move |_| { ctx.layer.hide_tip(); ctx.shared.add_target.set(Some(String::new())); }>
                    {icons::plus("")}
                </button>
                {move || {
                    ctx.workspace.track();
                    web::has_closed().then(|| {
                        let (enter, leave) = (restore_enter.clone(), restore_leave.clone());
                        view! {
                            <button type="button" data-slot="button" class=button_class("ghost", "icon-sm", "") aria-label=tr("Zuletzt geschlossenen Tab wiederherstellen", "Restore last closed tab")
                                on:pointerenter=enter on:pointerleave=leave
                                on:click=move |_| { ctx.layer.hide_tip(); run(web::restore_last_closed_tab()); }>
                                {icons::rotate_ccw("")}
                            </button>
                        }
                    })
                }}
            </div>
        </div>
    }
}

/// Eine Seite mit ihrem Baum aus Teilungen und Bereichen; nur die aktive ist sichtbar.
fn page_view(ctx: Ctx, id: String) -> impl IntoView {
    let page = StoredValue::new(id);
    let shape = Memo::new(move |_| ctx.workspace.with(|w| w.page(&page.get_value()).map(|p| shape_of(&p.root))));
    let active = Memo::new(move |_| ctx.workspace.with(|w| w.active_page_id == page.get_value()));
    view! {
        <div class=move || if active.get() { "flex min-h-0 flex-1 flex-col" } else { "hidden" }>
            {move || shape.get().map(|s| node_view(ctx, page, s))}
        </div>
    }
}

/// Seiten-Leiste und Seiten (Stand aus der Arbeitsfläche, crates/ui/src/workspace.rs).
#[component]
pub fn WorkspaceLayout() -> impl IntoView {
    let shared = workspace_ctx();
    let workspace = shared.state;
    let locked = use_setting("layout.locked", false);
    let dragged_page = RwSignal::new(None);
    let ctx = Ctx { workspace, locked, dragged: shared.dragged, dragged_page, layer: Layer::new(), shared };
    let page_ids = Memo::new(move |_| workspace.with(|w| w.pages.iter().map(|p| p.id.clone()).collect::<Vec<_>>()));
    view! {
        <div class="flex min-h-0 flex-1 flex-col">
            {page_bar(ctx)}
            <For each=move || page_ids.get() key=|id| id.clone() children=move |id| page_view(ctx, id) />
        </div>
        <LayerView layer=ctx.layer />
    }
}
