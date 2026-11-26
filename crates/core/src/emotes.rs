//! Emote-Dienste, übertragen aus src/emotes/third-party/{bttv,ffz,seventv}.ts, src/emotes/twitch-emotes.ts und
//! src/emotes/cheermotes.ts. Nur Umwandlung und Prüfung der Antworten; geladen wird in crates/wasm. Bildadressen werden
//! nur von den offiziellen Servern der Dienste übernommen.

use serde_json::Value;

use crate::model::{Emote, MessagePart};
use crate::twitch::emote_url;

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn emote(source: &str, id: String, name: String, url: String, url2x: Option<String>, animated: bool) -> Emote {
    Emote {
        source: source.into(),
        id,
        name,
        image_url: url,
        image_url2x: url2x,
        animated: Some(animated),
        overlay: None,
        owner_id: None,
        global: None,
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.chars().count() <= 100
}

/// BTTV-Liste (global, channelEmotes oder sharedEmotes). Ungültige Einträge entfallen.
pub fn bttv_emotes(raw: Option<&Value>) -> Vec<Emote> {
    let Some(list) = raw.and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|item| {
            let id = text(item, "id")?;
            let code = text(item, "code")?;
            // IDs sind 24 Hex-Zeichen; nur dann wird eine Bildadresse daraus gebaut.
            if id.len() != 24 || !id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) || !valid_name(code) {
                return None;
            }
            let animated = match item.get("animated") {
                None | Some(Value::Null) => false,
                Some(Value::Bool(value)) => *value,
                Some(_) => return None,
            };
            Some(emote(
                "bttv",
                id.into(),
                code.into(),
                format!("https://cdn.betterttv.net/emote/{id}/1x"),
                Some(format!("https://cdn.betterttv.net/emote/{id}/2x")),
                animated,
            ))
        })
        .collect()
}

/// BTTV-Kanal: Kanal-Emotes und geteilte Emotes.
pub fn bttv_channel(json: Option<&Value>) -> Vec<Emote> {
    let mut emotes = bttv_emotes(json.and_then(|j| j.get("channelEmotes")));
    emotes.extend(bttv_emotes(json.and_then(|j| j.get("sharedEmotes"))));
    emotes
}

/// FFZ-Bildadressen nur vom eigenen CDN; Tabelle Größe → Adresse.
fn ffz_urls(value: Option<&Value>) -> Option<Vec<(String, String)>> {
    let map = value?.as_object()?;
    let mut urls = Vec::new();
    for (size, url) in map {
        let url = url.as_str()?;
        if !url.starts_with("https://cdn.frankerfacez.com/") {
            return None;
        }
        urls.push((size.clone(), url.to_string()));
    }
    Some(urls)
}

fn ffz_emote(raw: &Value) -> Option<Emote> {
    let id = raw.get("id").and_then(Value::as_i64)?;
    let name = text(raw, "name").filter(|n| valid_name(n))?;
    let urls = ffz_urls(raw.get("urls"))?;
    let animated = match raw.get("animated") {
        None | Some(Value::Null) => None,
        Some(value) => Some(ffz_urls(Some(value))?),
    };
    // Effekt-Emotes (modifier) sind keine eigenständigen Bilder.
    if raw.get("modifier").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let get = |list: &[(String, String)], size: &str| list.iter().find(|(s, _)| s == size).map(|(_, u)| u.clone());
    let is_animated = animated.as_ref().is_some_and(|a| get(a, "1").is_some());
    let source = if is_animated { animated.as_deref().unwrap_or(&urls) } else { &urls };
    let url = get(source, "1")?;
    Some(emote("ffz", id.to_string(), name.into(), url, get(source, "2").or_else(|| get(source, "4")), is_animated))
}

fn ffz_sets(sets: Option<&Value>, ids: &[String]) -> Vec<Emote> {
    let Some(sets) = sets.and_then(Value::as_object) else {
        return Vec::new();
    };
    ids.iter()
        .filter_map(|id| sets.get(id))
        .flat_map(|set| set.get("emoticons").and_then(Value::as_array).cloned().unwrap_or_default())
        .filter_map(|raw| ffz_emote(&raw))
        .collect()
}

fn id_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// FFZ global: die Standard-Sets.
pub fn ffz_global(json: Option<&Value>) -> Vec<Emote> {
    let ids: Vec<String> = json
        .and_then(|j| j.get("default_sets"))
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(id_text).collect())
        .unwrap_or_default();
    ffz_sets(json.and_then(|j| j.get("sets")), &ids)
}

/// FFZ-Kanal: das Set des Raums.
pub fn ffz_channel(json: Option<&Value>) -> Vec<Emote> {
    let Some(set) = json.and_then(|j| j.get("room")).and_then(|r| r.get("set")).and_then(id_text) else {
        return Vec::new();
    };
    ffz_sets(json.and_then(|j| j.get("sets")), &[set])
}

const ZERO_WIDTH_ACTIVE: i64 = 1;
const ZERO_WIDTH_EMOTE: i64 = 256;

/// Bildserver von 7TV, Pfad nur aus Buchstaben und Ziffern.
fn seventv_host(url: &str) -> bool {
    url.strip_prefix("//cdn.7tv.app/emote/")
        .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// 7TV-Liste (emotes eines Sets). Overlay (zero-width): Bit 1 im Eintrag oder Bit 256 in den Emote-Daten.
pub fn seventv_emotes(raw: Option<&Value>) -> Vec<Emote> {
    let Some(list) = raw.and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|entry| {
            let id = text(entry, "id").filter(|i| !i.is_empty())?;
            let name = text(entry, "name").filter(|n| valid_name(n))?;
            let data = entry.get("data")?;
            let host = data.get("host").and_then(|h| text(h, "url")).filter(|u| seventv_host(u))?;
            let entry_flags = entry.get("flags").and_then(Value::as_i64).unwrap_or(0);
            let data_flags = data.get("flags").and_then(Value::as_i64).unwrap_or(0);
            let base = format!("https:{host}");
            let mut result = emote(
                "seventv",
                id.into(),
                name.into(),
                format!("{base}/1x.webp"),
                Some(format!("{base}/2x.webp")),
                data.get("animated").and_then(Value::as_bool) == Some(true),
            );
            result.overlay = Some(entry_flags & ZERO_WIDTH_ACTIVE != 0 || data_flags & ZERO_WIDTH_EMOTE != 0);
            Some(result)
        })
        .collect()
}

/// 7TV-Kanal: Emotes des aktiven Sets.
pub fn seventv_channel(json: Option<&Value>) -> Vec<Emote> {
    seventv_emotes(json.and_then(|j| j.get("emote_set")).and_then(|s| s.get("emotes")))
}

/// Twitch-Emote aus „Get User Emotes“ oder „Get Global Emotes“.
pub fn twitch_api_emote(raw: &Value, from_global_list: bool, dark: bool) -> Option<Emote> {
    let id = text(raw, "id")?;
    let name = text(raw, "name")?;
    let animated = raw
        .get("format")
        .and_then(Value::as_array)
        .is_some_and(|formats| formats.iter().any(|f| f.as_str() == Some("animated")));
    let kind = text(raw, "emote_type");
    Some(Emote {
        source: "twitch".into(),
        id: id.into(),
        name: name.into(),
        image_url: emote_url(id, animated, dark, "1.0"),
        image_url2x: Some(emote_url(id, animated, dark, "2.0")),
        animated: Some(animated),
        overlay: None,
        owner_id: text(raw, "owner_id").filter(|o| !o.is_empty()).map(str::to_string),
        // Global: aus „Get Global Emotes“ oder laut emote_type (globals, smilies) aus „Get User Emotes“.
        global: Some(from_global_list || kind == Some("globals") || kind == Some("smilies")),
    })
}

/// Bits im Verlauf (IRC, Tag „bits“): Wörter aus Buchstaben und Zahl wie „Cheer100“ werden zu Bits-Teilen.
/// Ob das Präfix wirklich ein Cheermote ist, zeigt sich beim Anzeigen; sonst bleibt es Text.
pub fn split_cheers(parts: Vec<MessagePart>) -> Vec<MessagePart> {
    let mut result = Vec::new();
    for part in parts {
        let MessagePart::Text { text } = part else {
            result.push(part);
            continue;
        };
        let mut buffer = String::new();
        for token in split_keeping_whitespace(&text) {
            let letters = token.bytes().take_while(u8::is_ascii_alphabetic).count();
            let digits = &token[letters..];
            let is_cheer = letters > 0 && !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit());
            match digits.parse::<u64>().ok().filter(|_| is_cheer) {
                Some(bits) => {
                    if !buffer.is_empty() {
                        result.push(MessagePart::Text { text: std::mem::take(&mut buffer) });
                    }
                    result.push(MessagePart::Cheer { text: token.into(), prefix: token[..letters].into(), bits });
                }
                None => buffer.push_str(token),
            }
        }
        if !buffer.is_empty() {
            result.push(MessagePart::Text { text: buffer });
        }
    }
    result
}

/// Teilt wie `text.split(/(\s+)/)`: Wörter und Leerraum abwechselnd, nichts geht verloren.
pub fn split_keeping_whitespace(text: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut in_space: Option<bool> = None;
    for (index, char) in text.char_indices() {
        let space = char.is_whitespace();
        if in_space.is_some_and(|s| s != space) {
            pieces.push(&text[start..index]);
            start = index;
        }
        in_space = Some(space);
    }
    if start < text.len() {
        pieces.push(&text[start..]);
    }
    pieces
}

/// Höchstens so viele zuletzt benutzte Emotes.
pub const MAX_RECENT_EMOTES: usize = 24;

/// Zuletzt benutzte Emotes (Einstellung „emotes.recent“, nur Name und Bild, neueste zuerst): Emote vorne einreihen,
/// Dopplungen (gleiche Quelle und ID) entfernen, auf die Höchstzahl kürzen. Ungültige Einträge (zum Beispiel ein
/// Bild ohne https) ändern nichts. Felder mit null fallen vor der Prüfung weg.
pub fn add_recent(current: &Value, entry: &Value) -> Value {
    let list: Vec<Value> = current.as_array().cloned().unwrap_or_default();
    let cleaned: Value = match entry.as_object() {
        Some(object) => Value::Object(object.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), v.clone())).collect()),
        None => return Value::Array(list),
    };
    let Ok(parsed) = crate::settings::recent_emote().parse(&cleaned) else { return Value::Array(list) };
    let same = |item: &Value| item.get("source") == parsed.get("source") && item.get("id") == parsed.get("id");
    let mut next = vec![parsed.clone()];
    next.extend(list.into_iter().filter(|item| !same(item)));
    next.truncate(MAX_RECENT_EMOTES);
    Value::Array(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bttv() {
        let emotes = bttv_emotes(Some(&json!([
            { "id": "56e9f494fff3cc5c35e5287e", "code": "monkaS", "animated": false },
            { "id": "kurz", "code": "x" },
            { "id": "5733ff12e72c3c2814a2c74c", "code": "", "animated": true },
            { "id": "5733ff12e72c3c2814a2c74d", "code": "catJAM", "animated": true }
        ])));
        assert_eq!(emotes.len(), 2);
        assert_eq!(emotes[0].image_url, "https://cdn.betterttv.net/emote/56e9f494fff3cc5c35e5287e/1x");
        assert_eq!(emotes[1].animated, Some(true));
        let channel = bttv_channel(Some(&json!({ "channelEmotes": [{ "id": "56e9f494fff3cc5c35e5287e", "code": "a" }], "sharedEmotes": [{ "id": "5733ff12e72c3c2814a2c74d", "code": "b" }] })));
        assert_eq!(channel.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    }

    #[test]
    fn ffz() {
        let json = json!({
            "default_sets": [3],
            "sets": { "3": { "emoticons": [
                { "id": 1, "name": "LilZ", "urls": { "1": "https://cdn.frankerfacez.com/emote/1/1", "4": "https://cdn.frankerfacez.com/emote/1/4" } },
                { "id": 2, "name": "Anim", "urls": { "1": "https://cdn.frankerfacez.com/emote/2/1" }, "animated": { "1": "https://cdn.frankerfacez.com/emote/2/animated/1", "2": "https://cdn.frankerfacez.com/emote/2/animated/2" } },
                { "id": 3, "name": "Mod", "urls": { "1": "https://cdn.frankerfacez.com/emote/3/1" }, "modifier": true },
                { "id": 4, "name": "Fremd", "urls": { "1": "https://evil.example/x" } }
            ] } }
        });
        let emotes = ffz_global(Some(&json));
        assert_eq!(emotes.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["LilZ", "Anim"]);
        assert_eq!(emotes[0].image_url2x.as_deref(), Some("https://cdn.frankerfacez.com/emote/1/4"));
        assert_eq!(emotes[1].image_url, "https://cdn.frankerfacez.com/emote/2/animated/1");
        assert_eq!(emotes[1].animated, Some(true));
        let room = json!({ "room": { "set": 3 }, "sets": json["sets"].clone() });
        assert_eq!(ffz_channel(Some(&room)).len(), 2);
        assert!(ffz_channel(None).is_empty());
    }

    #[test]
    fn seventv() {
        let emotes = seventv_emotes(Some(&json!([
            { "id": "a", "name": "peepoHey", "flags": 0, "data": { "flags": 0, "animated": true, "host": { "url": "//cdn.7tv.app/emote/01ABC" } } },
            { "id": "b", "name": "RainTime", "flags": 1, "data": { "host": { "url": "//cdn.7tv.app/emote/01DEF" } } },
            { "id": "c", "name": "Overlay", "data": { "flags": 256, "host": { "url": "//cdn.7tv.app/emote/01GHI" } } },
            { "id": "d", "name": "Boese", "data": { "host": { "url": "//evil.example/emote/1" } } }
        ])));
        assert_eq!(emotes.len(), 3);
        assert_eq!(emotes[0].image_url, "https://cdn.7tv.app/emote/01ABC/1x.webp");
        assert_eq!(emotes.iter().map(|e| e.overlay).collect::<Vec<_>>(), [Some(false), Some(true), Some(true)]);
        assert_eq!(seventv_channel(Some(&json!({ "emote_set": null }))), Vec::new());
    }

    #[test]
    fn twitch_emotes() {
        let emote = twitch_api_emote(&json!({ "id": "25", "name": "Kappa", "format": ["static"], "emote_type": "globals", "owner_id": "" }), false, true).unwrap();
        assert_eq!((emote.global, emote.owner_id.as_deref(), emote.animated), (Some(true), None, Some(false)));
        let sub = twitch_api_emote(&json!({ "id": "e1", "name": "subHype", "format": ["static", "animated"], "emote_type": "subscriptions", "owner_id": "9" }), false, true).unwrap();
        assert_eq!((sub.global, sub.owner_id.as_deref()), (Some(false), Some("9")));
        assert!(sub.image_url.contains("/animated/"));
        assert_eq!(twitch_api_emote(&json!({ "name": "x" }), true, true), None);
    }

    #[test]
    fn bits_abtrennen() {
        let parts = split_cheers(vec![MessagePart::Text { text: "abc 12 x9 Cheer100  hi".into() }, MessagePart::Mention { text: "@a".into(), login: "a".into() }]);
        assert_eq!(
            parts,
            vec![
                MessagePart::Text { text: "abc 12 ".into() },
                MessagePart::Cheer { text: "x9".into(), prefix: "x".into(), bits: 9 },
                MessagePart::Text { text: " ".into() },
                MessagePart::Cheer { text: "Cheer100".into(), prefix: "Cheer".into(), bits: 100 },
                MessagePart::Text { text: "  hi".into() },
                MessagePart::Mention { text: "@a".into(), login: "a".into() },
            ]
        );
        assert_eq!(split_keeping_whitespace(" a  b"), [" ", "a", "  ", "b"]);
    }

    #[test]
    fn recent_emotes_newest_first_without_duplicates_and_limited() {
        let emote = |name: &str| serde_json::json!({ "source": "twitch", "id": name, "name": name,
            "imageUrl": format!("https://static-cdn.jtvnw.net/emoticons/v2/{name}/default/dark/1.0"), "imageUrl2x": null });
        let mut list = add_recent(&serde_json::json!([]), &emote("Kappa"));
        list = add_recent(&list, &emote("PogChamp"));
        list = add_recent(&list, &emote("Kappa"));
        let names: Vec<&str> = list.as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["Kappa", "PogChamp"]);
        for i in 0..40 {
            list = add_recent(&list, &emote(&format!("e{i}")));
        }
        assert_eq!(list.as_array().unwrap().len(), MAX_RECENT_EMOTES);
        let mut unsafe_image = emote("x");
        unsafe_image["imageUrl"] = serde_json::json!("javascript:alert(1)");
        assert_eq!(add_recent(&serde_json::json!([]), &unsafe_image), serde_json::json!([]));
    }
}
