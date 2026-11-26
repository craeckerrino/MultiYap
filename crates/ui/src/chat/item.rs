//! Zeilen der Chat-Liste. Jede Zeile wird nur einmal gelesen: Ein
//! WeakMap-Zwischenspeicher erkennt dasselbe JavaScript-Objekt wieder (Zeilen sind unveränderlich, eine Änderung wie
//! „gelöscht von …“ kommt als neues Objekt). So bleibt eine neue Nachricht bei 1500 Zeilen billig. Das Original-Objekt
//! bleibt erhalten, damit Aktionen (Antworten, Tastenkürzel) es zurückgeben können.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use multiyap_core::model::{AutoModItem, ChannelRef, ChatEventItem, ChatMessage, ModActionItem};
use serde::Deserialize;
use wasm_bindgen::{JsCast, JsValue};

/// Systemmeldung (Verbinde…, Verbunden, Stream live und ähnliche).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemNotice {
    pub channel: ChannelRef,
    pub id: String,
    pub timestamp: f64,
    #[serde(rename = "type")]
    pub notice_type: String,
    #[serde(default)]
    pub detail: Option<String>,
}

/// Trennlinie: „Live ab hier“ oder Datum.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DividerItem {
    pub channel: ChannelRef,
    pub id: String,
    pub timestamp: f64,
    pub variant: String,
    pub label: String,
}

// Zeilen liegen hinter Rc im Zwischenspeicher; die Größe einzelner Arten spielt keine Rolle.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum ChatItem {
    Message(ChatMessage),
    System(SystemNotice),
    ModAction(ModActionItem),
    AutoMod(AutoModItem),
    Divider(DividerItem),
    Event(ChatEventItem),
}

impl ChatItem {
    pub fn id(&self) -> &str {
        match self {
            ChatItem::Message(m) => &m.id,
            ChatItem::System(s) => &s.id,
            ChatItem::ModAction(m) => &m.id,
            ChatItem::AutoMod(a) => &a.id,
            ChatItem::Divider(d) => &d.id,
            ChatItem::Event(e) => &e.id,
        }
    }

    pub fn channel(&self) -> &ChannelRef {
        match self {
            ChatItem::Message(m) => &m.channel,
            ChatItem::System(s) => &s.channel,
            ChatItem::ModAction(m) => &m.channel,
            ChatItem::AutoMod(a) => &a.channel,
            ChatItem::Divider(d) => &d.channel,
            ChatItem::Event(e) => &e.channel,
        }
    }

    /// Aus einem JavaScript-Objekt; None bei unbekannter Art oder ungültigen Daten.
    fn from_js(value: &JsValue) -> Option<ChatItem> {
        let kind = js_sys::Reflect::get(value, &"kind".into()).ok()?.as_string()?;
        fn read<T: serde::de::DeserializeOwned>(value: &JsValue) -> Option<T> {
            serde_wasm_bindgen::from_value(value.clone()).ok()
        }
        Some(match kind.as_str() {
            "message" => ChatItem::Message(read(value)?),
            "system" => ChatItem::System(read(value)?),
            "mod-action" => ChatItem::ModAction(read(value)?),
            "automod" => ChatItem::AutoMod(read(value)?),
            "divider" => ChatItem::Divider(read(value)?),
            "event" => ChatItem::Event(read(value)?),
            _ => return None,
        })
    }
}

/// Eine Zeile: gelesene Daten, Original-Objekt und eine Nummer, die genau dieses Objekt kennzeichnet.
#[derive(Clone)]
pub struct Row {
    pub item: Rc<ChatItem>,
    pub js: JsValue,
    /// Wechselt, sobald TypeScript ein neues Objekt für die Zeile liefert (dann wird sie neu gezeichnet).
    pub key: u32,
}

/// Zwischenspeicher der gelesenen Zeilen einer Liste.
pub struct ItemStore {
    keys: js_sys::WeakMap,
    parsed: RefCell<HashMap<u32, Rc<ChatItem>>>,
    next: Cell<u32>,
}

impl Default for ItemStore {
    fn default() -> Self {
        ItemStore { keys: js_sys::WeakMap::new(), parsed: RefCell::new(HashMap::new()), next: Cell::new(1) }
    }
}

impl ItemStore {
    /// Liest ein Array von Zeilen; schon bekannte Objekte werden nicht erneut gelesen.
    pub fn rows(&self, array: &JsValue) -> Vec<Row> {
        let Some(array) = array.dyn_ref::<js_sys::Array>() else { return Vec::new() };
        let old = std::mem::take(&mut *self.parsed.borrow_mut());
        let mut fresh = HashMap::with_capacity(array.length() as usize);
        let mut rows = Vec::with_capacity(array.length() as usize);
        for value in array.iter() {
            let Some(object) = value.dyn_ref::<js_sys::Object>() else { continue };
            let known = self.keys.get(object).as_f64().map(|k| k as u32);
            let entry = known.and_then(|key| old.get(&key).cloned().map(|item| (key, item)));
            let (key, item) = match entry {
                Some(found) => found,
                None => {
                    let Some(item) = ChatItem::from_js(&value) else { continue };
                    let key = self.next.get();
                    self.next.set(key.wrapping_add(1).max(1));
                    self.keys.set(object, &JsValue::from_f64(f64::from(key)));
                    (key, Rc::new(item))
                }
            };
            fresh.insert(key, item.clone());
            rows.push(Row { item, js: value, key });
        }
        *self.parsed.borrow_mut() = fresh;
        rows
    }
}
