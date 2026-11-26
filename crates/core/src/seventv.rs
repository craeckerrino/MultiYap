//! 7TV-Abzeichen und Namensfarben (Paints) über die EventAPI v3 (wss://events.7tv.io/v3, am 15.11.2026 live geprüft,
//! da keine Doku abrufbar war): pro offenem Kanal ein Abo auf cosmetic.create, entitlement.create und
//! entitlement.delete mit der Bedingung { ctx: "channel", platform: "TWITCH", id }. 7TV meldet darauf die Abzeichen
//! und Paints der Personen im Kanal. Es wird nur empfangen; MultiYap meldet selbst nichts an 7TV (keine „presence“).
//!
//! Farben sind 32-Bit-Zahlen im Format RGBA (auch negativ, dann vorzeichenbehaftet). Hier Prüfen, Umwandeln und der
//! Speicher; die Verbindung selbst steckt in `crates/wasm/src/seventv.rs`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::color::readable_name_color;

pub const EVENTS_URL: &str = "wss://events.7tv.io/v3";
pub const MAX_BACKOFF_MS: f64 = 60_000.0;

/// Operationen der EventAPI.
pub mod op {
    pub const DISPATCH: i64 = 0;
    pub const HELLO: i64 = 1;
    pub const RECONNECT: i64 = 4;
    pub const END_OF_STREAM: i64 = 7;
    pub const SUBSCRIBE: i64 = 35;
}

const SUBSCRIPTION_TYPES: [&str; 3] = ["cosmetic.create", "entitlement.create", "entitlement.delete"];

/// Abo-Nachrichten für einen Kanal (als JSON-Text zum Senden).
pub fn subscribe_messages(channel_id: &str) -> Vec<String> {
    SUBSCRIPTION_TYPES
        .iter()
        .map(|kind| {
            json!({ "op": op::SUBSCRIBE, "d": { "type": kind, "condition": { "ctx": "channel", "platform": "TWITCH", "id": channel_id } } })
                .to_string()
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SevenTvBadge {
    pub id: String,
    pub tooltip: String,
    pub image_url: String,
    pub image_url_2x: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaintStyle {
    pub background_image: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SevenTvPaint {
    pub id: String,
    pub name: String,
    /// CSS für den Namen: Verlauf oder Bild als Hintergrund, im Text ausgeschnitten, dazu Schatten.
    pub style: PaintStyle,
}

fn is_alnum_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())
}

#[derive(Deserialize)]
struct BadgeHost {
    url: String,
}

#[derive(Deserialize)]
struct RawBadge {
    id: String,
    tooltip: Option<String>,
    name: Option<String>,
    host: BadgeHost,
}

pub fn to_badge(raw: &Value) -> Option<SevenTvBadge> {
    let badge: RawBadge = serde_json::from_value(raw.clone()).ok()?;
    let too_long = |text: &Option<String>| text.as_ref().is_some_and(|t| t.chars().count() > 200);
    if !is_alnum_id(&badge.id) || too_long(&badge.tooltip) || too_long(&badge.name) {
        return None;
    }
    let key = badge.host.url.strip_prefix("//cdn.7tv.app/badge/")?;
    if !is_alnum_id(key) {
        return None;
    }
    let base = format!("https:{}", badge.host.url);
    let tooltip = [badge.tooltip, badge.name].into_iter().flatten().find(|t| !t.is_empty()).unwrap_or_else(|| "7TV".into());
    Some(SevenTvBadge { id: badge.id, tooltip, image_url: format!("{base}/1x.webp"), image_url_2x: format!("{base}/2x.webp") })
}

#[derive(Deserialize)]
struct RawStop {
    at: f64,
    color: i64,
}

#[derive(Deserialize)]
struct RawShadow {
    x_offset: f64,
    y_offset: f64,
    radius: f64,
    color: i64,
}

#[derive(Deserialize)]
struct RawPaint {
    id: String,
    name: String,
    function: String,
    color: Option<i64>,
    repeat: Option<bool>,
    angle: Option<f64>,
    shape: Option<String>,
    image_url: Option<String>,
    stops: Option<Vec<RawStop>>,
    shadows: Option<Vec<RawShadow>>,
}

/// RGBA-Zahl nach #rrggbb und Deckkraft.
pub fn decode_color(value: i64) -> (String, f64) {
    let unsigned = (value & 0xFFFF_FFFF) as u32;
    let [r, g, b, a] = unsigned.to_be_bytes();
    (format!("#{r:02x}{g:02x}{b:02x}"), f64::from(a) / 255.0)
}

/// Rundet wie Math.round in JavaScript.
fn js_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

/// Zahl als Text wie in JavaScript (ganze Zahlen ohne Nachkommastellen).
fn js_number(value: f64) -> String {
    if value == 0.0 { "0".into() } else { value.to_string() }
}

fn css_color(value: i64, adjust: Option<&dyn Fn(&str) -> String>) -> String {
    let (hex, alpha) = decode_color(value);
    let hex = adjust.map_or(hex.clone(), |f| f(&hex));
    let [r, g, b] = crate::color::parse_hex(&hex).unwrap_or([0, 0, 0]);
    format!("rgb({r} {g} {b} / {})", js_number(js_round(alpha * 1000.0) / 1000.0))
}

fn clamp_number(value: f64, min: f64, max: f64) -> f64 {
    (if value.is_finite() { value } else { 0.0 }).clamp(min, max)
}

fn valid_image_url(url: &str) -> bool {
    url.strip_prefix("https://cdn.7tv.app/")
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c)))
}

/// Baut das CSS eines Paints nur aus geprüften Zahlen und Farben; ein Bild nur vom CDN von 7TV. `adjust` erlaubt,
/// Farben aufzuhellen (lesbare Benutzernamen gelten auch für 7TV-Farben).
pub fn to_paint(raw: &Value, adjust: Option<&dyn Fn(&str) -> String>) -> Option<SevenTvPaint> {
    let paint: RawPaint = serde_json::from_value(raw.clone()).ok()?;
    let stops = paint.stops.unwrap_or_default();
    let shadows = paint.shadows.unwrap_or_default();
    if !is_alnum_id(&paint.id)
        || paint.name.chars().count() > 200
        || stops.len() > 20
        || shadows.len() > 5
        || stops.iter().any(|s| !(0.0..=1.0).contains(&s.at))
    {
        return None;
    }
    let background_image = match paint.function.as_str() {
        "URL" => {
            let url = paint.image_url.unwrap_or_default();
            if !valid_image_url(&url) {
                return None;
            }
            format!("url(\"{url}\")")
        }
        "LINEAR_GRADIENT" | "RADIAL_GRADIENT" => {
            let mut parts: Vec<String> =
                stops.iter().map(|s| format!("{} {}%", css_color(s.color, adjust), js_number(js_round(s.at * 1000.0) / 10.0))).collect();
            if parts.is_empty() {
                return None;
            }
            if parts.len() == 1 {
                parts.push(parts[0].clone());
            }
            let repeat = if paint.repeat == Some(true) { "repeating-" } else { "" };
            if paint.function == "LINEAR_GRADIENT" {
                let angle = clamp_number(paint.angle.unwrap_or(90.0), -360.0, 360.0);
                format!("{repeat}linear-gradient({}deg, {})", js_number(angle), parts.join(", "))
            } else {
                let shape = if paint.shape.as_deref() == Some("ellipse") { "ellipse" } else { "circle" };
                format!("{repeat}radial-gradient({shape}, {})", parts.join(", "))
            }
        }
        _ => return None,
    };
    let filters: Vec<String> = shadows
        .iter()
        .map(|s| {
            format!(
                "drop-shadow({}px {}px {}px {})",
                js_number(clamp_number(s.x_offset, -10.0, 10.0)),
                js_number(clamp_number(s.y_offset, -10.0, 10.0)),
                js_number(clamp_number(s.radius, 0.0, 10.0)),
                css_color(s.color, None)
            )
        })
        .collect();
    Some(SevenTvPaint {
        id: paint.id,
        name: paint.name,
        style: PaintStyle {
            background_image,
            filter: (!filters.is_empty()).then(|| filters.join(" ")),
            fallback_color: paint.color.map(|c| css_color(c, adjust)),
        },
    })
}

#[derive(Deserialize)]
struct Connection {
    platform: String,
    id: String,
}

#[derive(Deserialize)]
struct EntitlementUser {
    connections: Vec<Connection>,
}

#[derive(Deserialize)]
struct Entitlement {
    kind: String,
    ref_id: String,
    user: EntitlementUser,
}

/// Abzeichen und Paints, die 7TV für Personen in offenen Kanälen gemeldet hat (nur für die Sitzung).
#[derive(Default)]
pub struct Cosmetics {
    badges: HashMap<String, SevenTvBadge>,
    /// Rohdaten der Paints; das CSS entsteht beim Lesen (mit oder ohne Aufhellung).
    paint_data: HashMap<String, Value>,
    paint_cache: HashMap<(bool, bool, String), Option<SevenTvPaint>>,
    /// Twitch-Nutzer-ID → Abzeichen-ID bzw. Paint-ID.
    user_badge: HashMap<String, String>,
    user_paint: HashMap<String, String>,
}

impl Cosmetics {
    /// Verarbeitet eine Meldung; true, wenn sich etwas geändert haben kann.
    pub fn handle_dispatch(&mut self, kind: &str, object: &Value) -> bool {
        if kind == "cosmetic.create" {
            match object.get("kind").and_then(Value::as_str) {
                Some("BADGE") => {
                    if let Some(badge) = object.get("data").and_then(to_badge) {
                        self.badges.insert(badge.id.clone(), badge);
                    }
                }
                Some("PAINT") => {
                    let data = object.get("data");
                    if let Some(id) = data.and_then(|d| d.get("id")).and_then(Value::as_str) {
                        self.paint_data.insert(id.to_string(), data.cloned().unwrap_or(Value::Null));
                        self.paint_cache.clear();
                    }
                }
                _ => {}
            }
            return true;
        }
        let Ok(entitlement) = serde_json::from_value::<Entitlement>(object.clone()) else { return false };
        if entitlement.user.connections.len() > 20 {
            return false;
        }
        let Some(twitch_id) = entitlement.user.connections.iter().find(|c| c.platform == "TWITCH").map(|c| c.id.clone()) else {
            return false;
        };
        let target = match entitlement.kind.as_str() {
            "BADGE" => &mut self.user_badge,
            "PAINT" => &mut self.user_paint,
            _ => return false,
        };
        if kind == "entitlement.create" {
            target.insert(twitch_id, entitlement.ref_id);
        } else if target.get(&twitch_id) == Some(&entitlement.ref_id) {
            target.remove(&twitch_id);
        }
        true
    }

    pub fn badge(&self, twitch_user_id: &str) -> Option<&SevenTvBadge> {
        self.user_badge.get(twitch_user_id).and_then(|id| self.badges.get(id))
    }

    /// Paint einer Person, auf Wunsch aufgehellt (`dark`: dunkles Design).
    pub fn paint(&mut self, twitch_user_id: &str, readable: bool, dark: bool) -> Option<SevenTvPaint> {
        let id = self.user_paint.get(twitch_user_id)?.clone();
        let key = (readable, dark, id.clone());
        if let Some(cached) = self.paint_cache.get(&key) {
            return cached.clone();
        }
        let adjust = move |hex: &str| readable_name_color(hex, dark);
        let paint = self.paint_data.get(&id).and_then(|raw| to_paint(raw, if readable { Some(&adjust) } else { None }));
        self.paint_cache.insert(key, paint.clone());
        paint
    }
}

// ---- Änderungen am Emote-Set eines Kanals ----
//
// Doku geprüft am 23.11.2026 (github.com/SevenTV/EventAPI): Abo `emote_set.update` mit der Bedingung `object_id`
// (ID des Emote-Sets, aus `emote_set.id` von `7tv.io/v3/users/twitch/{id}`). Die Meldung ist eine ChangeMap: `id`
// (Emote-Set), `actor` (wer geändert hat), Listen `pushed`, `pulled` und `updated` mit Feldern `key`, `value` und
// `old_value`. Wie 7TVs eigene Erweiterung (github.com/SevenTV/Extension, emote-set.handler.ts): hinzugefügt steht in
// `pushed[].value`, entfernt in `pulled[].old_value`, umbenannt in `updated[]` mit altem und neuem Wert.

/// Abo-Nachricht für Änderungen eines Emote-Sets (als JSON-Text zum Senden).
pub fn emote_set_subscribe_message(set_id: &str) -> String {
    json!({ "op": op::SUBSCRIBE, "d": { "type": "emote_set.update", "condition": { "object_id": set_id } } }).to_string()
}

/// Eine gemeldete Änderung am Emote-Set: Rohdaten der Emotes wie in der REST-Antwort (`id`, `name`, `data`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EmoteSetChange {
    pub set_id: String,
    pub actor: Option<String>,
    pub added: Vec<Value>,
    pub removed: Vec<Value>,
    /// (alt, neu)
    pub renamed: Vec<(Value, Value)>,
}

impl EmoteSetChange {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.renamed.is_empty()
    }
}

fn emote_fields<'a>(body: &'a Value, list: &str) -> impl Iterator<Item = &'a Value> {
    body.get(list)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|field| field.get("key").and_then(Value::as_str).is_none_or(|key| key == "emotes"))
}

fn has_id(value: &Value) -> bool {
    value.get("id").and_then(Value::as_str).is_some_and(|id| !id.is_empty())
}

/// Liest die ChangeMap einer `emote_set.update`-Meldung.
pub fn parse_emote_set_update(body: &Value) -> Option<EmoteSetChange> {
    let set_id = body.get("id").and_then(Value::as_str).filter(|id| !id.is_empty())?.to_string();
    let actor = body.get("actor").and_then(|actor| {
        ["display_name", "username"].iter().find_map(|name| actor.get(*name).and_then(Value::as_str).filter(|n| !n.is_empty())).map(str::to_string)
    });
    let added = emote_fields(body, "pushed").filter_map(|f| f.get("value")).filter(|v| has_id(v)).cloned().collect();
    let removed = emote_fields(body, "pulled").filter_map(|f| f.get("old_value")).filter(|v| has_id(v)).cloned().collect();
    let renamed = emote_fields(body, "updated")
        .filter_map(|f| Some((f.get("old_value").filter(|v| has_id(v))?.clone(), f.get("value").filter(|v| has_id(v))?.clone())))
        .collect();
    Some(EmoteSetChange { set_id, actor, added, removed, renamed })
}

fn emote_id(value: &Value) -> &str {
    value.get("id").and_then(Value::as_str).unwrap_or_default()
}

/// Adresse des 7TV-Profils aus der Antwort von `7tv.io/v3/users/twitch/{twitch_id}` (Feld `user.id`, geprüft am
/// 23.11.2026); Profile liegen unter `https://7tv.app/users/{id}`. Nur IDs aus Buchstaben und Ziffern.
pub fn profile_url(connection: Option<&Value>) -> Option<String> {
    let id = connection?.get("user")?.get("id")?.as_str()?;
    (!id.is_empty() && id.len() <= 40 && id.chars().all(|c| c.is_ascii_alphanumeric())).then(|| format!("https://7tv.app/users/{id}"))
}

pub fn emote_name(value: &Value) -> &str {
    value.get("name").and_then(Value::as_str).unwrap_or_default()
}

/// Wendet eine Änderung auf die geladene Liste der Kanal-Emotes an.
pub fn apply_emote_set_change(list: &mut Vec<crate::model::Emote>, change: &EmoteSetChange) {
    let gone: Vec<&str> = change.removed.iter().chain(change.renamed.iter().map(|(old, _)| old)).map(emote_id).collect();
    list.retain(|emote| !gone.contains(&emote.id.as_str()));
    let new: Vec<Value> = change.renamed.iter().map(|(_, new)| new.clone()).chain(change.added.iter().cloned()).collect();
    for emote in crate::emotes::seventv_emotes(Some(&Value::Array(new))) {
        if !list.iter().any(|e| e.id == emote.id) {
            list.push(emote);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_profile_url() {
        let answer = json!({ "id": "71092938", "platform": "TWITCH", "user": { "id": "01FE9DRF000009TR6M9N941CYW" } });
        assert_eq!(profile_url(Some(&answer)).as_deref(), Some("https://7tv.app/users/01FE9DRF000009TR6M9N941CYW"));
        assert_eq!(profile_url(None), None);
        assert_eq!(profile_url(Some(&json!({ "user": { "id": "../x" } }))), None);
        assert_eq!(profile_url(Some(&json!({ "user": {} }))), None);
    }

    fn paint_data() -> Value {
        json!({
            "id": "01GTK7N8B000009S59ZP7E4Z93", "name": "Nightclub", "color": null, "function": "LINEAR_GRADIENT",
            "repeat": false, "angle": 90, "shape": "circle", "image_url": "",
            "stops": [{ "at": 0.25, "color": -1670971393 }, { "at": 0.5, "color": -1796800513 }],
            "shadows": [{ "x_offset": 0, "y_offset": 0, "radius": 4, "color": 1680202495 }],
        })
    }

    fn badge_data() -> Value {
        json!({ "id": "01M3Q4RTMNFHTT58VNTFH8152G", "name": "Foxy", "tooltip": "For those who migrated",
            "host": { "url": "//cdn.7tv.app/badge/01M3Q4RTMNFHTT58VNTFH8152G" } })
    }

    #[test]
    fn farben() {
        assert_eq!(decode_color(-1), ("#ffffff".into(), 1.0));
        assert_eq!(decode_color(0xff00_00ff_u32 as i32 as i64).0, "#ff0000");
        assert!((decode_color(0x00ff_0080).1 - 0.502).abs() < 0.001);
    }

    #[test]
    fn verlauf_und_schatten() {
        let paint = to_paint(&paint_data(), None).unwrap();
        let background = &paint.style.background_image;
        assert!(background.starts_with("linear-gradient(90deg, rgb("), "{background}");
        assert!(background.contains(" / 1) 25%, rgb(") && background.ends_with(" / 1) 50%)"), "{background}");
        assert!(paint.style.filter.unwrap().starts_with("drop-shadow(0px 0px 4px rgb("));
    }

    #[test]
    fn bilder_nur_vom_cdn() {
        let with_url = |url: &str| {
            let mut data = paint_data();
            data["function"] = json!("URL");
            data["image_url"] = json!(url);
            to_paint(&data, None)
        };
        assert_eq!(
            with_url("https://cdn.7tv.app/paint/x/layer/1x.webp").unwrap().style.background_image,
            "url(\"https://cdn.7tv.app/paint/x/layer/1x.webp\")"
        );
        assert!(with_url("https://boese.example/x.png").is_none());
        assert!(with_url("https://cdn.7tv.app/a\") ; x(\"").is_none());
    }

    #[test]
    fn abzeichen_nur_mit_cdn() {
        assert_eq!(to_badge(&badge_data()).unwrap().image_url, "https://cdn.7tv.app/badge/01M3Q4RTMNFHTT58VNTFH8152G/1x.webp");
        let mut bad = badge_data();
        bad["host"]["url"] = json!("//boese.example/badge/x");
        assert!(to_badge(&bad).is_none());
    }

    #[test]
    fn zuordnung_per_twitch_id() {
        let user = json!({ "connections": [{ "platform": "TWITCH", "id": "42" }] });
        let mut store = Cosmetics::default();
        store.handle_dispatch("cosmetic.create", &json!({ "kind": "BADGE", "data": badge_data() }));
        store.handle_dispatch("cosmetic.create", &json!({ "kind": "PAINT", "data": paint_data() }));
        store.handle_dispatch("entitlement.create", &json!({ "kind": "BADGE", "ref_id": "01M3Q4RTMNFHTT58VNTFH8152G", "user": user }));
        store.handle_dispatch("entitlement.create", &json!({ "kind": "PAINT", "ref_id": "01GTK7N8B000009S59ZP7E4Z93", "user": user }));
        assert_eq!(store.badge("42").unwrap().tooltip, "For those who migrated");
        assert_eq!(store.paint("42", true, true).unwrap().name, "Nightclub");
        store.handle_dispatch("entitlement.delete", &json!({ "kind": "BADGE", "ref_id": "anderes", "user": user }));
        assert!(store.badge("42").is_some());
        store.handle_dispatch("entitlement.delete", &json!({ "kind": "BADGE", "ref_id": "01M3Q4RTMNFHTT58VNTFH8152G", "user": user }));
        assert!(store.badge("42").is_none());
    }

    #[test]
    fn abo_nachrichten() {
        let messages = subscribe_messages("123");
        assert_eq!(messages.len(), 3);
        let first: Value = serde_json::from_str(&messages[0]).unwrap();
        assert_eq!(first["op"], 35);
        assert_eq!(first["d"]["condition"]["id"], "123");
    }

    fn active(id: &str, name: &str) -> Value {
        json!({ "id": id, "name": name, "flags": 0, "data": { "id": id, "name": name, "animated": false, "host": { "url": format!("//cdn.7tv.app/emote/{id}") } } })
    }

    #[test]
    fn emote_set_abo() {
        let message: Value = serde_json::from_str(&emote_set_subscribe_message("01FE9DRF")).unwrap();
        assert_eq!(message["op"], 35);
        assert_eq!(message["d"]["type"], "emote_set.update");
        assert_eq!(message["d"]["condition"]["object_id"], "01FE9DRF");
    }

    #[test]
    fn emote_set_aenderung() {
        let body = json!({
            "id": "set1",
            "actor": { "username": "mod_a", "display_name": "Mod_A" },
            "pushed": [{ "key": "emotes", "index": 3, "value": active("e3", "Neu") }],
            "pulled": [{ "key": "emotes", "index": 0, "value": null, "old_value": active("e1", "Weg") }],
            "updated": [{ "key": "emotes", "index": 1, "old_value": active("e2", "Alt"), "value": active("e2", "Umbenannt") }]
        });
        let change = parse_emote_set_update(&body).unwrap();
        assert_eq!(change.set_id, "set1");
        assert_eq!(change.actor.as_deref(), Some("Mod_A"));
        assert_eq!(emote_name(&change.added[0]), "Neu");
        assert_eq!(emote_name(&change.removed[0]), "Weg");
        assert_eq!(emote_name(&change.renamed[0].1), "Umbenannt");

        let mut list = crate::emotes::seventv_emotes(Some(&json!([active("e1", "Weg"), active("e2", "Alt")])));
        apply_emote_set_change(&mut list, &change);
        let names: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Umbenannt", "Neu"]);
    }

    #[test]
    fn emote_set_andere_felder_ignoriert() {
        let body = json!({ "id": "set1", "updated": [{ "key": "name", "old_value": "A", "value": "B" }] });
        assert!(parse_emote_set_update(&body).unwrap().is_empty());
        assert!(parse_emote_set_update(&json!({})).is_none());
    }
}
