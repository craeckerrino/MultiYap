//! Reine Operationen auf dem Layout der Arbeitsfläche. Aufbau: oben eine Tab-Leiste mit Seiten; jede Seite ist ein Baum
//! aus Teilungen, jedes Blatt (Bereich) zeigt genau einen Inhalt (Chat, Stream, Hervorhebungen) mit eigener Kopfzeile.
//! Nur eine leere Seite hat einen leeren Bereich.
//!
//! Jede Operation gibt einen neuen Zustand zurück (oder None, wenn sich nichts ändert) und lässt den alten unverändert.
//! Tabs (die Inhalte) bleiben hier unverändert durchgereichte JSON-Werte; nur ihre `id` zählt. Neue IDs kommen vom
//! Aufrufer (`new_id`, im Browser crypto.randomUUID).

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAIN_AREA_ID: &str = "main";
pub const MAIN_PAGE_ID: &str = "page-main";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Node {
    /// Ein Bereich mit höchstens einem Inhalt (`tab_ids` hat null oder ein Element).
    #[serde(rename_all = "camelCase")]
    Area { id: String, tab_ids: Vec<String>, active_tab_id: Option<String> },
    Split {
        id: String,
        /// "row": nebeneinander, "column": übereinander.
        direction: String,
        ratio: f64,
        first: Box<Node>,
        second: Box<Node>,
    },
}

/// Eine Seite (ein Tab der oberen Leiste) mit eigenem Namen und Layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub id: String,
    /// Eigener Name; ohne ihn zeigt die Leiste die Namen der Inhalte.
    pub name: Option<String>,
    pub root: Node,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceState {
    pub tabs: Vec<Value>,
    pub pages: Vec<Page>,
    pub active_page_id: String,
    /// Bereich mit dem Fokus, immer auf der aktiven Seite.
    pub focused_area_id: String,
}

/// Wohin ein neuer Inhalt kommt.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// Eigene neue Seite; ist die aktive Seite leer, dorthin.
    NewPage,
    /// Neben einen Bereich (`side`: left, right, top, bottom); ist der Bereich leer, hinein.
    Split { area: String, side: String },
}

/// Geschlossener Inhalt oder geschlossene Seite, zum Wiederherstellen (nur für die Sitzung).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosedTab {
    pub tabs: Vec<Value>,
    pub page_id: String,
    pub page_index: usize,
    /// Die ganze Seite, wenn sie mit geschlossen wurde.
    pub page: Option<Page>,
}

fn empty_area(id: &str) -> Node {
    Node::Area { id: id.into(), tab_ids: Vec::new(), active_tab_id: None }
}

fn single_area(id: String, tab: &str) -> Node {
    Node::Area { id, tab_ids: vec![tab.to_string()], active_tab_id: Some(tab.to_string()) }
}

impl Default for WorkspaceState {
    fn default() -> Self {
        WorkspaceState {
            tabs: Vec::new(),
            pages: vec![Page { id: MAIN_PAGE_ID.into(), name: None, root: empty_area(MAIN_AREA_ID) }],
            active_page_id: MAIN_PAGE_ID.into(),
            focused_area_id: MAIN_AREA_ID.into(),
        }
    }
}

fn tab_id(tab: &Value) -> &str {
    tab.get("id").and_then(Value::as_str).unwrap_or_default()
}

/// Alle Bereiche eines Baums in Lesereihenfolge.
pub fn collect_areas(node: &Node) -> Vec<&Node> {
    match node {
        Node::Area { .. } => vec![node],
        Node::Split { first, second, .. } => {
            let mut list = collect_areas(first);
            list.extend(collect_areas(second));
            list
        }
    }
}

fn node_id(node: &Node) -> &str {
    match node {
        Node::Area { id, .. } | Node::Split { id, .. } => id,
    }
}

fn area_tabs(node: &Node) -> &[String] {
    match node {
        Node::Area { tab_ids, .. } => tab_ids,
        Node::Split { .. } => &[],
    }
}

pub fn has_area(root: &Node, id: &str) -> bool {
    collect_areas(root).iter().any(|a| node_id(a) == id)
}

fn is_empty_page(page: &Page) -> bool {
    collect_areas(&page.root).iter().all(|a| area_tabs(a).is_empty())
}

impl WorkspaceState {
    pub fn active_page(&self) -> Option<&Page> {
        self.pages.iter().find(|p| p.id == self.active_page_id)
    }

    /// Seite und Bereich eines Inhalts.
    pub fn locate_tab(&self, tab: &str) -> Option<(String, String)> {
        self.pages.iter().find_map(|page| {
            collect_areas(&page.root).into_iter().find(|a| area_tabs(a).iter().any(|t| t == tab)).map(|a| (page.id.clone(), node_id(a).to_string()))
        })
    }

    /// Seite eines Bereichs.
    pub fn page_of_area(&self, area: &str) -> Option<&Page> {
        self.pages.iter().find(|p| has_area(&p.root, area))
    }

    /// Sichtbare Inhalte: alle auf der aktiven Seite.
    pub fn visible_tab_ids(&self) -> Vec<String> {
        self.active_page().map(|p| collect_areas(&p.root).iter().flat_map(|a| area_tabs(a).to_vec()).collect()).unwrap_or_default()
    }

    /// Inhalte einer Seite in Lesereihenfolge.
    pub fn page_tab_ids(&self, page: &str) -> Vec<String> {
        self.pages.iter().find(|p| p.id == page).map(|p| collect_areas(&p.root).iter().flat_map(|a| area_tabs(a).to_vec()).collect()).unwrap_or_default()
    }
}

/// Ersetzt einen Knoten (nach ID); alles andere bleibt.
fn replace_node(node: &Node, target: &str, make: &mut dyn FnMut(&Node) -> Node) -> Node {
    if node_id(node) == target {
        return make(node);
    }
    match node {
        Node::Area { .. } => node.clone(),
        Node::Split { id, direction, ratio, first, second } => Node::Split {
            id: id.clone(),
            direction: direction.clone(),
            ratio: *ratio,
            first: Box::new(replace_node(first, target, make)),
            second: Box::new(replace_node(second, target, make)),
        },
    }
}

/// Nimmt einen Inhalt aus seinem Bereich; der leere Bereich verschwindet, seine Nachbarseite nimmt den Platz der
/// Teilung ein. Ist es der einzige Bereich, bleibt er leer zurück.
fn remove_from_tree(node: &Node, tab: &str) -> Node {
    match node {
        Node::Area { id, tab_ids, .. } if tab_ids.iter().any(|t| t == tab) => empty_area(id),
        Node::Area { .. } => node.clone(),
        Node::Split { id, direction, ratio, first, second } => {
            let (first, second) = (remove_from_tree(first, tab), remove_from_tree(second, tab));
            let empty = |n: &Node| matches!(n, Node::Area { tab_ids, .. } if tab_ids.is_empty());
            match (empty(&first), empty(&second)) {
                (true, false) => second,
                (false, true) => first,
                _ => Node::Split { id: id.clone(), direction: direction.clone(), ratio: *ratio, first: Box::new(first), second: Box::new(second) },
            }
        }
    }
}

/// Teilt einen Bereich: der neue Bereich mit dem Inhalt kommt an die Seite `side`.
fn split_area(root: &Node, target: &str, side: &str, created: Node, split_id: String) -> Node {
    let new_first = side == "left" || side == "top";
    let direction = if side == "left" || side == "right" { "row" } else { "column" };
    replace_node(root, target, &mut |node| {
        let (first, second) = if new_first { (created.clone(), node.clone()) } else { (node.clone(), created.clone()) };
        Node::Split { id: split_id.clone(), direction: direction.into(), ratio: 0.5, first: Box::new(first), second: Box::new(second) }
    })
}

/// Leere Seiten entfernen (außer `keep` und außer es bleibt keine), aktive Seite und Fokus gültig halten.
fn tidy(mut state: WorkspaceState, keep: Option<&str>) -> WorkspaceState {
    let removable: Vec<String> = state.pages.iter().filter(|p| is_empty_page(p) && Some(p.id.as_str()) != keep).map(|p| p.id.clone()).collect();
    for id in removable {
        if state.pages.len() > 1 {
            let index = state.pages.iter().position(|p| p.id == id).unwrap_or(0);
            state.pages.remove(index);
            if state.active_page_id == id {
                // Wie im Browser: die Seite rechts, sonst links.
                let next = index.min(state.pages.len() - 1);
                state.active_page_id = state.pages[next].id.clone();
            }
        }
    }
    if !state.pages.iter().any(|p| p.id == state.active_page_id) {
        state.active_page_id = state.pages[0].id.clone();
    }
    let page = state.active_page().expect("aktive Seite");
    if !has_area(&page.root, &state.focused_area_id) {
        state.focused_area_id = node_id(collect_areas(&page.root)[0]).to_string();
    }
    state
}

fn with_page(state: &WorkspaceState, page: &str, change: impl FnOnce(&Node) -> Node) -> Vec<Page> {
    let mut change = Some(change);
    state
        .pages
        .iter()
        .map(|p| match change.take_if(|_| p.id == page) {
            Some(change) => Page { root: change(&p.root), ..p.clone() },
            None => p.clone(),
        })
        .collect()
}

/// Neuer Inhalt nach `target`; er wird sichtbar und bekommt den Fokus.
pub fn add_tab(state: &WorkspaceState, tab: Value, target: &Target, new_id: &mut dyn FnMut() -> String) -> WorkspaceState {
    let id = tab_id(&tab).to_string();
    let mut tabs = state.tabs.clone();
    tabs.push(tab);
    // Bereich zum Teilen; fehlt er, eine neue Seite.
    let split_target = match target {
        Target::Split { area, side } => state.page_of_area(area).map(|p| (p.id.clone(), area.clone(), side.clone())),
        Target::NewPage => None,
    };
    if let Some((page, area, side)) = split_target {
        let empty = state.page_of_area(&area).and_then(|p| collect_areas(&p.root).into_iter().find(|a| node_id(a) == area)).is_some_and(|a| area_tabs(a).is_empty());
        if empty {
            let pages = with_page(state, &page, |root| replace_node(root, &area, &mut |_| single_area(area.clone(), &id)));
            return WorkspaceState { tabs, pages, active_page_id: page, focused_area_id: area };
        }
        let created = new_id();
        let split_id = new_id();
        let pages = with_page(state, &page, |root| split_area(root, &area, &side, single_area(created.clone(), &id), split_id));
        return WorkspaceState { tabs, pages, active_page_id: page, focused_area_id: created };
    }
    // Leere aktive Seite: dorthin.
    if let Some(page) = state.active_page().filter(|p| is_empty_page(p)) {
        let area = node_id(collect_areas(&page.root)[0]).to_string();
        let page_id = page.id.clone();
        let pages = with_page(state, &page_id, |_| single_area(area.clone(), &id));
        return WorkspaceState { tabs, pages, active_page_id: page_id, focused_area_id: area };
    }
    let (page_id, area) = (new_id(), new_id());
    let mut pages = state.pages.clone();
    pages.push(Page { id: page_id.clone(), name: None, root: single_area(area.clone(), &id) });
    WorkspaceState { tabs, pages, active_page_id: page_id, focused_area_id: area }
}

/// Zeigt die Seite des Inhalts und setzt den Fokus auf ihn.
pub fn activate_tab(state: &WorkspaceState, tab: &str) -> Option<WorkspaceState> {
    let (page, area) = state.locate_tab(tab)?;
    (state.active_page_id != page || state.focused_area_id != area).then(|| WorkspaceState { active_page_id: page, focused_area_id: area, ..state.clone() })
}

/// Zeigt eine Seite; der Fokus geht auf ihren ersten Bereich.
pub fn activate_page(state: &WorkspaceState, page: &str) -> Option<WorkspaceState> {
    if state.active_page_id == page {
        return None;
    }
    let target = state.pages.iter().find(|p| p.id == page)?;
    let area = node_id(collect_areas(&target.root)[0]).to_string();
    Some(WorkspaceState { active_page_id: page.to_string(), focused_area_id: area, ..state.clone() })
}

pub fn focus_area(state: &WorkspaceState, area: &str) -> Option<WorkspaceState> {
    if state.focused_area_id == area || !state.active_page().is_some_and(|p| has_area(&p.root, area)) {
        return None;
    }
    Some(WorkspaceState { focused_area_id: area.to_string(), ..state.clone() })
}

/// Schließt einen Inhalt; seine Seite verschwindet, wenn sie dadurch leer wird (außer es ist die letzte).
pub fn close_tab(state: &WorkspaceState, tab: &str) -> Option<(WorkspaceState, ClosedTab)> {
    let (page, _) = state.locate_tab(tab)?;
    let value = state.tabs.iter().find(|t| tab_id(t) == tab)?.clone();
    let page_index = state.pages.iter().position(|p| p.id == page).unwrap_or(0);
    let pages = with_page(state, &page, |root| remove_from_tree(root, tab));
    let tabs = state.tabs.iter().filter(|t| tab_id(t) != tab).cloned().collect();
    let next = tidy(WorkspaceState { tabs, pages, ..state.clone() }, None);
    Some((next, ClosedTab { tabs: vec![value], page_id: page, page_index, page: None }))
}

/// Schließt eine Seite mit allen Inhalten. Die letzte Seite bleibt als leere Seite.
pub fn close_page(state: &WorkspaceState, page: &str) -> Option<(WorkspaceState, ClosedTab)> {
    let page_index = state.pages.iter().position(|p| p.id == page)?;
    let closed_page = state.pages[page_index].clone();
    let ids = state.page_tab_ids(page);
    let closed_tabs: Vec<Value> = state.tabs.iter().filter(|t| ids.iter().any(|id| id == tab_id(t))).cloned().collect();
    let tabs = state.tabs.iter().filter(|t| !ids.iter().any(|id| id == tab_id(t))).cloned().collect();
    let mut pages = state.pages.clone();
    if pages.len() > 1 {
        pages.remove(page_index);
    } else {
        pages[0] = Page { id: page.to_string(), name: None, root: empty_area(MAIN_AREA_ID) };
    }
    let next = tidy(WorkspaceState { tabs, pages, ..state.clone() }, None);
    Some((next, ClosedTab { tabs: closed_tabs, page_id: page.to_string(), page_index, page: Some(closed_page) }))
}

/// Ersetzt IDs im Baum (für wiederhergestellte Inhalte, deren ID inzwischen vergeben ist).
fn rename_ids(node: &Node, tab_map: &[(String, String)], new_id: &mut dyn FnMut() -> String) -> Node {
    match node {
        Node::Area { tab_ids, active_tab_id, .. } => {
            let map = |id: &String| tab_map.iter().find(|(old, _)| old == id).map_or_else(|| id.clone(), |(_, new)| new.clone());
            Node::Area { id: new_id(), tab_ids: tab_ids.iter().map(map).collect(), active_tab_id: active_tab_id.as_ref().map(map) }
        }
        Node::Split { direction, ratio, first, second, .. } => Node::Split {
            id: new_id(),
            direction: direction.clone(),
            ratio: *ratio,
            first: Box::new(rename_ids(first, tab_map, new_id)),
            second: Box::new(rename_ids(second, tab_map, new_id)),
        },
    }
}

/// Stellt Geschlossenes wieder her: eine Seite an ihrer alten Stelle, einen Inhalt rechts neben dem Fokus seiner alten
/// Seite (gibt es sie nicht mehr, als neue Seite an der alten Stelle).
pub fn restore_tab(state: &WorkspaceState, closed: &ClosedTab, new_id: &mut dyn FnMut() -> String) -> WorkspaceState {
    let mut tab_map = Vec::new();
    let tabs_back: Vec<Value> = closed
        .tabs
        .iter()
        .map(|tab| {
            let mut tab = tab.clone();
            if state.tabs.iter().any(|t| tab_id(t) == tab_id(&tab)) {
                let fresh = new_id();
                tab_map.push((tab_id(&tab).to_string(), fresh.clone()));
                if let Some(object) = tab.as_object_mut() {
                    object.insert("id".into(), Value::String(fresh));
                }
            }
            tab
        })
        .collect();
    let mut tabs = state.tabs.clone();
    tabs.extend(tabs_back.iter().cloned());
    let page_exists = state.pages.iter().any(|p| p.id == closed.page_id);
    match (&closed.page, page_exists, tabs_back.first()) {
        (Some(page), _, _) => {
            let root = rename_ids(&page.root, &tab_map, new_id);
            let id = if page_exists { new_id() } else { page.id.clone() };
            let area = node_id(collect_areas(&root)[0]).to_string();
            let mut pages = state.pages.clone();
            // Die leere letzte Seite weicht der wiederhergestellten.
            if pages.len() == 1 && is_empty_page(&pages[0]) {
                pages.clear();
            }
            let index = closed.page_index.min(pages.len());
            pages.insert(index, Page { id: id.clone(), name: page.name.clone(), root });
            WorkspaceState { tabs, pages, active_page_id: id, focused_area_id: area }
        }
        (None, true, Some(tab)) => {
            let page = state.pages.iter().find(|p| p.id == closed.page_id).expect("Seite");
            let anchor = if state.active_page_id == page.id { state.focused_area_id.clone() } else { node_id(collect_areas(&page.root).last().expect("Bereich")).to_string() };
            add_tab(&WorkspaceState { tabs: state.tabs.clone(), ..state.clone() }, tab.clone(), &Target::Split { area: anchor, side: "right".into() }, new_id)
        }
        (None, false, Some(tab)) => {
            let (page_id, area) = (new_id(), new_id());
            let mut pages = state.pages.clone();
            if pages.len() == 1 && is_empty_page(&pages[0]) {
                pages.clear();
            }
            let index = closed.page_index.min(pages.len());
            pages.insert(index, Page { id: page_id.clone(), name: None, root: single_area(area.clone(), tab_id(tab)) });
            WorkspaceState { tabs, pages, active_page_id: page_id, focused_area_id: area }
        }
        (None, _, None) => state.clone(),
    }
}

/// Zieht einen Inhalt an den Rand eines Bereichs (auch auf einer anderen Seite): der Bereich wird geteilt, der Inhalt
/// kommt in den neuen Teil. `side`: left, right, top oder bottom.
pub fn split_with_tab(state: &WorkspaceState, tab: &str, target: &str, side: &str, new_id: &mut dyn FnMut() -> String) -> Option<WorkspaceState> {
    let (source_page, source_area) = state.locate_tab(tab)?;
    let target_page = state.page_of_area(target)?.id.clone();
    if source_area == target {
        return None;
    }
    let pages = with_page(state, &source_page, |root| remove_from_tree(root, tab));
    let removed = WorkspaceState { pages, ..state.clone() };
    // Der Zielbereich kann durch das Herausnehmen nicht verschwinden (er enthält einen anderen Inhalt); war er leer,
    // kommt der Inhalt hinein.
    let next = add_tab(&WorkspaceState { tabs: Vec::new(), ..removed }, Value::Null, &Target::Split { area: target.to_string(), side: side.to_string() }, new_id);
    let pages = next
        .pages
        .iter()
        .map(|p| Page { root: fill_null(&p.root, tab), ..p.clone() })
        .collect();
    Some(tidy(WorkspaceState { tabs: state.tabs.clone(), pages, active_page_id: target_page.clone(), focused_area_id: next.focused_area_id }, Some(&target_page)))
}

/// Setzt den Platzhalter (leere ID aus `add_tab` mit `Value::Null`) auf den verschobenen Inhalt.
fn fill_null(node: &Node, tab: &str) -> Node {
    match node {
        Node::Area { id, tab_ids, .. } if tab_ids.iter().any(String::is_empty) => single_area(id.clone(), tab),
        Node::Area { .. } => node.clone(),
        Node::Split { id, direction, ratio, first, second } => Node::Split {
            id: id.clone(),
            direction: direction.clone(),
            ratio: *ratio,
            first: Box::new(fill_null(first, tab)),
            second: Box::new(fill_null(second, tab)),
        },
    }
}

/// Verschiebt einen Inhalt auf eine andere Seite (rechts an das Layout der Seite) oder mit `page` None auf eine neue
/// Seite am Ende.
pub fn move_tab_to_page(state: &WorkspaceState, tab: &str, page: Option<&str>, new_id: &mut dyn FnMut() -> String) -> Option<WorkspaceState> {
    let (source_page, _) = state.locate_tab(tab)?;
    if page == Some(source_page.as_str()) {
        return None;
    }
    // Einziger Inhalt einer Seite auf eine neue Seite: nichts zu tun.
    if page.is_none() && state.page_tab_ids(&source_page).len() == 1 {
        return None;
    }
    let pages = with_page(state, &source_page, |root| remove_from_tree(root, tab));
    let mut removed = WorkspaceState { pages, ..state.clone() };
    let (page_id, area) = match page {
        Some(target) => {
            let target_page = removed.pages.iter().find(|p| p.id == target)?;
            let created = new_id();
            let root = if is_empty_page(target_page) {
                single_area(node_id(collect_areas(&target_page.root)[0]).to_string(), tab)
            } else {
                Node::Split { id: new_id(), direction: "row".into(), ratio: 0.5, first: Box::new(target_page.root.clone()), second: Box::new(single_area(created.clone(), tab)) }
            };
            let area = node_id(collect_areas(&root).into_iter().find(|a| area_tabs(a).iter().any(|t| t == tab))?).to_string();
            removed.pages = with_page(&removed, target, |_| root.clone());
            (target.to_string(), area)
        }
        None => {
            let (page_id, area) = (new_id(), new_id());
            removed.pages.push(Page { id: page_id.clone(), name: None, root: single_area(area.clone(), tab) });
            (page_id, area)
        }
    };
    Some(tidy(WorkspaceState { active_page_id: page_id.clone(), focused_area_id: area, ..removed }, Some(&page_id)))
}

/// Verschiebt eine Seite in der Leiste an eine Position.
pub fn move_page(state: &WorkspaceState, page: &str, index: usize) -> Option<WorkspaceState> {
    let from = state.pages.iter().position(|p| p.id == page)?;
    let insert_at = if from < index { index - 1 } else { index };
    if from == insert_at {
        return None;
    }
    let mut pages = state.pages.clone();
    let moved = pages.remove(from);
    pages.insert(insert_at.min(pages.len()), moved);
    Some(WorkspaceState { pages, ..state.clone() })
}

/// Eigener Name einer Seite (None: Namen der Inhalte).
pub fn rename_page(state: &WorkspaceState, page: &str, name: Option<String>) -> Option<WorkspaceState> {
    let current = state.pages.iter().find(|p| p.id == page)?;
    if current.name == name {
        return None;
    }
    let pages = state.pages.iter().map(|p| if p.id == page { Page { name: name.clone(), ..p.clone() } } else { p.clone() }).collect();
    Some(WorkspaceState { pages, ..state.clone() })
}

/// Teilungsverhältnis, in sinnvollen Grenzen (5 bis 95 Prozent).
pub fn set_ratio(state: &WorkspaceState, split: &str, ratio: f64) -> Option<WorkspaceState> {
    let clamped = ratio.clamp(0.05, 0.95);
    let mut changed = false;
    fn walk(node: &Node, split: &str, ratio: f64, changed: &mut bool) -> Node {
        match node {
            Node::Area { .. } => node.clone(),
            Node::Split { id, direction, ratio: own, first, second } => {
                let value = if id == split && *own != ratio {
                    *changed = true;
                    ratio
                } else {
                    *own
                };
                Node::Split {
                    id: id.clone(),
                    direction: direction.clone(),
                    ratio: value,
                    first: Box::new(walk(first, split, ratio, changed)),
                    second: Box::new(walk(second, split, ratio, changed)),
                }
            }
        }
    }
    let pages = state.pages.iter().map(|p| Page { root: walk(&p.root, split, clamped, &mut changed), ..p.clone() }).collect();
    changed.then(|| WorkspaceState { pages, ..state.clone() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tab(id: &str) -> Value {
        json!({ "id": id, "kind": "channel", "platform": "twitch", "channel": id, "nickname": null })
    }

    /// Eindeutige IDs über alle Aufrufe (wie crypto.randomUUID).
    fn counter() -> impl FnMut() -> String {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        || format!("neu-{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }

    /// Jede Seite eine neue Seite.
    fn with_pages(ids: &[&str]) -> WorkspaceState {
        let mut next = counter();
        ids.iter().fold(WorkspaceState::default(), |state, id| add_tab(&state, tab(id), &Target::NewPage, &mut next))
    }

    /// Wie das Schema: jeder Inhalt genau einmal, höchstens einer je Bereich, Fokus auf der aktiven Seite.
    fn valid(state: WorkspaceState) -> WorkspaceState {
        let value = serde_json::to_value(&state).unwrap();
        assert!(crate::stored::named("workspace").unwrap().parse(&value).is_ok(), "ungültig: {value}");
        state
    }

    fn layout(state: &WorkspaceState) -> Vec<Vec<String>> {
        state.pages.iter().map(|p| state.page_tab_ids(&p.id)).collect()
    }

    #[test]
    fn neue_seiten_und_teilen() {
        let state = valid(with_pages(&["a", "b"]));
        // Die leere Startseite nimmt den ersten Inhalt auf.
        assert_eq!(layout(&state), [vec!["a"], vec!["b"]]);
        assert_eq!(state.visible_tab_ids(), ["b"]);
        let area_a = state.locate_tab("a").unwrap().1;
        let split = valid(add_tab(&state, tab("c"), &Target::Split { area: area_a.clone(), side: "right".into() }, &mut counter()));
        assert_eq!(layout(&split), [vec!["a", "c"], vec!["b"]]);
        assert_eq!(split.active_page_id, MAIN_PAGE_ID);
        assert_eq!(split.visible_tab_ids(), ["a", "c"]);
        let top = valid(add_tab(&state, tab("c"), &Target::Split { area: area_a, side: "top".into() }, &mut counter()));
        assert!(matches!(&top.pages[0].root, Node::Split { direction, .. } if direction == "column"));
        assert_eq!(layout(&top), [vec!["c", "a"], vec!["b"]]);
    }

    #[test]
    fn schliessen_und_wiederherstellen() {
        let state = with_pages(&["a", "b"]);
        let area_b = state.locate_tab("b").unwrap().1;
        let state = add_tab(&state, tab("c"), &Target::Split { area: area_b, side: "right".into() }, &mut counter());
        let (closed, info) = close_tab(&state, "c").unwrap();
        assert_eq!(layout(&valid(closed.clone())), [vec!["a"], vec!["b"]]);
        let restored = valid(restore_tab(&closed, &info, &mut counter()));
        assert_eq!(layout(&restored), [vec!["a"], vec!["b", "c"]]);
        // Letzter Inhalt einer Seite: die Seite verschwindet.
        let (closed, info) = close_tab(&state, "a").unwrap();
        assert_eq!(layout(&valid(closed.clone())), [vec!["b", "c"]]);
        assert_eq!(layout(&valid(restore_tab(&closed, &info, &mut counter()))), [vec!["a"], vec!["b", "c"]]);
        // Ganze Seite schließen und zurückholen.
        let page = state.pages[1].id.clone();
        let (closed, info) = close_page(&state, &page).unwrap();
        assert_eq!(layout(&valid(closed.clone())), [vec!["a"]]);
        assert_eq!(layout(&valid(restore_tab(&closed, &info, &mut counter()))), [vec!["a"], vec!["b", "c"]]);
        // Letzte Seite schließen: eine leere Seite bleibt.
        let (empty, _) = close_tab(&with_pages(&["a"]), "a").unwrap();
        assert!(valid(empty.clone()).tabs.is_empty());
        assert_eq!(empty.pages.len(), 1);
    }

    #[test]
    fn verschieben() {
        let state = with_pages(&["a", "b", "c"]);
        let area_a = state.locate_tab("a").unwrap().1;
        let moved = valid(split_with_tab(&state, "c", &area_a, "bottom", &mut counter()).unwrap());
        assert_eq!(layout(&moved), [vec!["a", "c"], vec!["b"]]);
        assert_eq!(moved.active_page_id, MAIN_PAGE_ID);
        let page_b = state.pages[1].id.clone();
        let joined = valid(move_tab_to_page(&moved, "a", Some(&page_b), &mut counter()).unwrap());
        assert_eq!(layout(&joined), [vec!["c"], vec!["b", "a"]]);
        let alone = valid(move_tab_to_page(&joined, "a", None, &mut counter()).unwrap());
        assert_eq!(layout(&alone), [vec!["c"], vec!["b"], vec!["a"]]);
        assert!(move_tab_to_page(&alone, "a", None, &mut counter()).is_none());
        let reordered = valid(move_page(&alone, &alone.pages[0].id.clone(), 3).unwrap());
        assert_eq!(layout(&reordered), [vec!["b"], vec!["a"], vec!["c"]]);
    }

    #[test]
    fn verhaeltnis_und_namen() {
        let state = with_pages(&["a"]);
        let area = state.locate_tab("a").unwrap().1;
        let split = add_tab(&state, tab("b"), &Target::Split { area, side: "right".into() }, &mut counter());
        let id = node_id(&split.pages[0].root).to_string();
        assert!(matches!(set_ratio(&split, &id, 2.0).unwrap().pages[0].root, Node::Split { ratio, .. } if ratio == 0.95));
        assert!(set_ratio(&split, &id, 0.5).is_none());
        let named = valid(rename_page(&split, MAIN_PAGE_ID, Some("Zwei".into())).unwrap());
        assert_eq!(named.pages[0].name.as_deref(), Some("Zwei"));
    }
}
