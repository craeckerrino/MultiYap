//! Schemas der übrigen gespeicherten und empfangenen Werte, übertragen aus den zod-Schemas mit derselben Wirkung:
//! Konto, Arbeitsfläche mit Tabs und Layout, eigene Sounds, automatische Sicherungen und die angepinnte Nachricht von
//! Twitch.

use crate::i18n::tr;
use std::collections::HashSet;

use serde_json::Value;

use crate::schema::*;

const MAX_SAFE: i64 = 9_007_199_254_740_991;
pub const MAX_SOUND_BYTES: i64 = 1024 * 1024;
const SOUND_MIME_TYPES: [&str; 7] = ["audio/mpeg", "audio/mp3", "audio/wav", "audio/x-wav", "audio/wave", "audio/vnd.wave", "audio/ogg"];

/// Ein angemeldetes Konto (Token und Refresh-Token sind geheim: nie loggen, nie exportieren).
fn account() -> Schema {
    object(vec![
        field("platform", literal("twitch")),
        field("userId", non_empty()),
        field("login", non_empty()),
        field("clientId", non_empty()),
        field("accessToken", non_empty()),
        field("scopes", list(text())),
        field("obtainedAt", int(0, MAX_SAFE)),
        field("expiresAt", nullable(int(0, MAX_SAFE))),
        field("source", one_of_texts(&["login", "manual", "device"])),
        optional("refreshToken", non_empty()),
    ])
}

/// Twitch-Login: Buchstaben (klein), Ziffern, Unterstrich, 1 bis 25 Zeichen.
fn login() -> Schema {
    text_check(
        None,
        None,
        |t| (1..=25).contains(&t.len()) && t.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
        ("Ungültiger Kanalname", "Invalid channel name"),
    )
}

fn nickname() -> Schema {
    nullable(trimmed(1, 50))
}

fn channel_like_tab(kind: &'static str) -> Schema {
    object(vec![
        field("id", non_empty()),
        field("kind", literal(kind)),
        field("platform", literal("twitch")),
        field("channel", login()),
        field("nickname", nickname()),
    ])
}

/// YouTube-Kanal-Tab: Kanal als „@name“ oder „UC…“ (crates/core/src/youtube.rs). Kein Stream-Tab für YouTube.
fn youtube_channel_tab() -> Schema {
    object(vec![
        field("id", non_empty()),
        field("kind", literal("channel")),
        field("platform", literal("youtube")),
        field("channel", text_check(None, None, crate::youtube::is_channel_key, ("Ungültiger YouTube-Kanal", "Invalid YouTube channel"))),
        field("nickname", nickname()),
    ])
}

fn all_or(items: Schema, max: usize) -> Schema {
    union(vec![literal("all"), array(items, max)])
}

fn highlights_tab() -> Schema {
    object(vec![
        field("id", non_empty()),
        field("kind", literal("highlights")),
        field("nickname", nickname()),
        field(
            "show",
            object(vec![
                field("terms", all_or(non_empty(), 30)),
                field("mentions", boolean()),
                field("events", boolean()),
                field("modActions", boolean()),
                field("automod", boolean()),
                field("channels", all_or(login(), 100)),
            ]),
        ),
        field("counter", boolean()),
    ])
}

fn tab() -> Schema {
    union(vec![channel_like_tab("channel"), youtube_channel_tab(), highlights_tab(), channel_like_tab("stream")])
}

fn layout_node() -> Schema {
    union(vec![
        object(vec![
            field("type", literal("area")),
            field("id", non_empty()),
            field("tabIds", list(non_empty())),
            field("activeTabId", nullable(text())),
        ]),
        object(vec![
            field("type", literal("split")),
            field("id", non_empty()),
            field("direction", one_of_texts(&["row", "column"])),
            field("ratio", Schema::Number { int: false, min: None, max: None, gt: Some(0.0), lt: Some(1.0) }),
            field("first", lazy(layout_node)),
            field("second", lazy(layout_node)),
        ]),
    ])
}

fn collect_nodes<'a>(node: &'a Value, ids: &mut Vec<&'a str>, areas: &mut Vec<&'a Value>) {
    if let Some(id) = node["id"].as_str() {
        ids.push(id);
    }
    if node["type"] == "area" {
        areas.push(node);
    } else {
        collect_nodes(&node["first"], ids, areas);
        collect_nodes(&node["second"], ids, areas);
    }
}

/// Stimmigkeit: jeder Tab genau einmal in genau einem Bereich, höchstens ein Tab je Bereich, keine doppelten IDs, die
/// aktive Seite existiert und der fokussierte Bereich liegt auf ihr.
fn workspace_consistency(state: &Value) -> Option<&'static str> {
    let tab_ids: Vec<&str> = state["tabs"].as_array().into_iter().flatten().filter_map(|t| t["id"].as_str()).collect();
    let unique = |items: &[&str]| items.iter().collect::<HashSet<_>>().len() == items.len();
    if !unique(&tab_ids) {
        return Some(tr("Doppelte Tab-IDs", "Duplicate tab IDs"));
    }
    let pages: Vec<&Value> = state["pages"].as_array().into_iter().flatten().collect();
    let page_ids: Vec<&str> = pages.iter().filter_map(|p| p["id"].as_str()).collect();
    if pages.is_empty() || !unique(&page_ids) {
        return Some(tr("Seiten fehlen oder sind doppelt", "Pages missing or duplicated"));
    }
    let Some(active_page) = pages.iter().find(|p| p["id"] == state["activePageId"]) else {
        return Some(tr("Aktive Seite existiert nicht", "Active page does not exist"));
    };
    let (mut node_ids, mut areas) = (Vec::new(), Vec::new());
    for page in &pages {
        collect_nodes(&page["root"], &mut node_ids, &mut areas);
    }
    if !unique(&node_ids) {
        return Some(tr("Doppelte Bereichs-IDs", "Duplicate area IDs"));
    }
    let placed: Vec<&str> = areas.iter().flat_map(|a| a["tabIds"].as_array().into_iter().flatten().filter_map(Value::as_str)).collect();
    if placed.len() != tab_ids.len() || !unique(&placed) || placed.iter().any(|id| !tab_ids.contains(id)) {
        return Some(tr("Tabs und Bereiche passen nicht zusammen", "Tabs and areas do not match"));
    }
    if areas.iter().any(|a| a["tabIds"].as_array().is_some_and(|ids| ids.len() > 1)) {
        return Some(tr("Mehr als ein Tab in einem Bereich", "More than one tab in an area"));
    }
    for area in &areas {
        if let Some(active) = area["activeTabId"].as_str() {
            if !area["tabIds"].as_array().is_some_and(|ids| ids.iter().any(|id| id == active)) {
                return Some(tr("Aktiver Tab liegt nicht im Bereich", "Active tab is not in the area"));
            }
        }
    }
    let (mut active_ids, mut active_areas) = (Vec::new(), Vec::new());
    collect_nodes(&active_page["root"], &mut active_ids, &mut active_areas);
    if !active_areas.iter().any(|a| a["id"] == state["focusedAreaId"]) {
        return Some(tr("Fokussierter Bereich existiert nicht", "Focused area does not exist"));
    }
    None
}

/// Eine Seite der oberen Tab-Leiste.
fn page() -> Schema {
    object(vec![field("id", non_empty()), field("name", nickname()), field("root", layout_node())])
}

fn workspace() -> Schema {
    verify(
        object(vec![
            field("tabs", list(tab())),
            field("pages", list(page())),
            field("activePageId", non_empty()),
            field("focusedAreaId", non_empty()),
        ]),
        workspace_consistency,
    )
}

/// Ein eigener Sound; die Daten liegen als Base64-Text vor.
fn custom_sound() -> Schema {
    let max_data = ((MAX_SOUND_BYTES as usize).div_ceil(3)) * 4;
    object(vec![
        field("id", text_len(1, 64)),
        field("name", trimmed(1, 60)),
        field("mimeType", one_of_texts(&SOUND_MIME_TYPES)),
        field("size", int(1, MAX_SOUND_BYTES)),
        field(
            "data",
            text_check(
                None,
                Some(max_data),
                |t| {
                    let body = t.trim_end_matches('=');
                    t.len() - body.len() <= 2 && body.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/')
                },
                ("Ungültige Base64-Daten", "Invalid Base64 data"),
            ),
        ),
        field("createdAt", text()),
    ])
}

/// Automatische Sicherung im Browser (ohne Prüfsumme; wird beim Wiederherstellen wie eine Datei geprüft).
fn auto_backup() -> Schema {
    object(vec![
        field("createdAt", text()),
        field("appVersion", text()),
        field("schemaVersion", int(1, MAX_SAFE)),
        field("data", record(text(), record(text(), any()))),
    ])
}

/// Angepinnte Nachricht laut Twitch (GET /chat/pins).
fn pinned_message() -> Schema {
    object(vec![
        field("message_id", non_empty()),
        field("sender_user_login", text()),
        field("sender_user_name", text()),
        field("pinned_by_user_name", text()),
        field("message", nullable(object(vec![field("text", text())]))),
        field("starts_at", text()),
        field("ends_at", nullable(text())),
    ])
}

/// Schema nach Name.
pub fn named(name: &str) -> Option<Schema> {
    Some(match name {
        "account" => account(),
        "activeAccountId" => non_empty(),
        "tab" => tab(),
        "highlightsTab" => highlights_tab(),
        "workspace" => workspace(),
        "customSound" => custom_sound(),
        "autoBackup" => auto_backup(),
        "pinnedMessage" => pinned_message(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(name: &str, value: Value) -> Result<Value, Vec<Issue>> {
        named(name).unwrap().parse(&value)
    }

    fn state() -> Value {
        json!({
            "tabs": [
                { "id": "a", "kind": "channel", "platform": "twitch", "channel": "kanal", "nickname": " Name " },
                { "id": "b", "kind": "stream", "platform": "twitch", "channel": "kanal", "nickname": null },
            ],
            "pages": [{ "id": "p", "name": null, "root": { "type": "split", "id": "s", "direction": "row", "ratio": 0.5,
                "first": { "type": "area", "id": "main", "tabIds": ["a"], "activeTabId": "a" },
                "second": { "type": "area", "id": "zwei", "tabIds": ["b"], "activeTabId": "b" } } }],
            "activePageId": "p",
            "focusedAreaId": "main",
        })
    }

    #[test]
    fn arbeitsflaeche() {
        let parsed = parse("workspace", state()).unwrap();
        assert_eq!(parsed["tabs"][0]["nickname"], "Name");
        let mut doubled = state();
        doubled["pages"][0]["root"]["second"]["tabIds"] = json!(["b", "a"]);
        assert_eq!(parse("workspace", doubled).unwrap_err()[0].message, "Tabs und Bereiche passen nicht zusammen");
        let mut two = state();
        two["pages"][0]["root"]["first"]["tabIds"] = json!(["a", "b"]);
        two["pages"][0]["root"]["second"]["tabIds"] = json!([]);
        two["pages"][0]["root"]["second"]["activeTabId"] = json!(null);
        assert_eq!(parse("workspace", two).unwrap_err()[0].message, "Mehr als ein Tab in einem Bereich");
        let mut page = state();
        page["activePageId"] = json!("weg");
        assert_eq!(parse("workspace", page).unwrap_err()[0].message, "Aktive Seite existiert nicht");
        let mut lost = state();
        lost["focusedAreaId"] = json!("weg");
        assert_eq!(parse("workspace", lost).unwrap_err()[0].message, "Fokussierter Bereich existiert nicht");
        let mut ratio = state();
        ratio["pages"][0]["root"]["ratio"] = json!(1);
        assert!(parse("workspace", ratio).is_err());
        let mut name = state();
        name["tabs"][0]["channel"] = json!("Groß");
        assert!(parse("workspace", name).is_err());
    }

    #[test]
    fn youtube_tab() {
        let tab = json!({ "id": "y", "kind": "channel", "platform": "youtube", "channel": "@lofigirl", "nickname": null });
        assert!(parse("tab", tab.clone()).is_ok());
        let mut by_id = tab.clone();
        by_id["channel"] = json!("UCSJ4gkVC6NrvII8umztf0Ow");
        assert!(parse("tab", by_id).is_ok());
        for bad in ["@LofiGirl", "lofigirl", "@a"] {
            let mut wrong = tab.clone();
            wrong["channel"] = json!(bad);
            assert!(parse("tab", wrong).is_err(), "{bad}");
        }
        // Kein Stream-Tab für YouTube.
        let mut stream = tab;
        stream["kind"] = json!("stream");
        assert!(parse("tab", stream).is_err());
    }

    #[test]
    fn hervorhebungen_tab() {
        let tab = json!({ "id": "h", "kind": "highlights", "nickname": null, "counter": true,
            "show": { "terms": "all", "mentions": true, "events": true, "modActions": true, "automod": true, "channels": ["a_b"] } });
        assert!(parse("tab", tab.clone()).is_ok());
        let mut bad = tab;
        bad["show"]["terms"] = json!("einige");
        assert!(parse("highlightsTab", bad).is_err());
    }

    #[test]
    fn konto_sound_sicherung_pin() {
        let account = json!({ "platform": "twitch", "userId": "1", "login": "a", "clientId": "c", "accessToken": "t", "scopes": [],
            "obtainedAt": 1, "expiresAt": null, "source": "device" });
        assert!(parse("account", account.clone()).is_ok());
        let mut bad = account;
        bad["refreshToken"] = json!("");
        assert!(parse("account", bad).is_err());
        let sound = json!({ "id": "s", "name": " Pling ", "mimeType": "audio/wav", "size": 3, "data": "AQID", "createdAt": "x" });
        assert_eq!(parse("customSound", sound.clone()).unwrap()["name"], "Pling");
        let mut bad = sound;
        bad["data"] = json!("A=B=");
        assert!(parse("customSound", bad).is_err());
        assert!(parse("autoBackup", json!({ "createdAt": "x", "appVersion": "1", "schemaVersion": 17, "data": { "settings": { "a": [1] } } })).is_ok());
        let pin = json!({ "message_id": "m", "sender_user_login": "a", "sender_user_name": "A", "pinned_by_user_name": "B",
            "message": { "text": "x" }, "starts_at": "s", "ends_at": null });
        assert!(parse("pinnedMessage", pin).is_ok());
    }
}
