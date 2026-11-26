//! Zeilen eines laufenden Chats (bisher BufferedChannelSession in src/core/chat/channel-session.ts) und das Ergänzen
//! von Mod und Grund zu Bans aus dem Verlauf (bisher src/platforms/twitch/ban-history.ts). Nachrichten liegen nur im
//! Arbeitsspeicher, begrenzt auf `MAX_ITEMS` (Ringpuffer).

use std::collections::HashSet;

use serde_json::{json, Value};

use crate::time::parse_rfc3339_ms;

/// Höchstzahl an Zeilen pro Kanal im Speicher.
pub const MAX_ITEMS: usize = 1500;
/// Wie viele ältere Verlaufszeilen pro Hochscrollen nachgeladen werden.
pub const OLDER_PAGE_SIZE: usize = 100;

/// Eine Zeile mit eindeutiger ID.
pub trait BufferItem {
    fn id(&self) -> &str;
}

impl BufferItem for Value {
    fn id(&self) -> &str {
        self.get("id").and_then(Value::as_str).unwrap_or_default()
    }
}

/// Sichtbare Zeilen, neue Zeilen, die auf die nächste Anzeige warten, und ältere Verlaufszeilen, die erst beim
/// Hochscrollen erscheinen.
#[derive(Debug, Clone)]
pub struct ChatBuffer<T> {
    items: Vec<T>,
    pending: Vec<T>,
    older: Vec<T>,
    max: usize,
}

impl<T> Default for ChatBuffer<T> {
    fn default() -> Self {
        ChatBuffer { items: Vec::new(), pending: Vec::new(), older: Vec::new(), max: MAX_ITEMS }
    }
}

impl<T: BufferItem> ChatBuffer<T> {
    pub fn with_max(max: usize) -> Self {
        ChatBuffer { items: Vec::new(), pending: Vec::new(), older: Vec::new(), max }
    }

    pub fn items(&self) -> &[T] {
        &self.items
    }

    pub fn has_older(&self) -> bool {
        !self.older.is_empty()
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Neue Zeile; sichtbar mit dem nächsten `flush`.
    pub fn push(&mut self, item: T) {
        self.pending.push(item);
    }

    /// Wartende Zeilen anhängen; ältere fallen über der Höchstzahl vorne weg. true: sichtbar geändert.
    pub fn flush(&mut self) -> bool {
        if self.pending.is_empty() {
            return false;
        }
        self.items.append(&mut self.pending);
        if self.items.len() > self.max {
            let excess = self.items.len() - self.max;
            self.items.drain(..excess);
        }
        true
    }

    /// Nächste Seite älterer Verlaufszeilen vorne anfügen. true: sichtbar geändert.
    pub fn load_older(&mut self, page: usize) -> bool {
        if self.older.is_empty() {
            return false;
        }
        let start = self.older.len().saturating_sub(page);
        let mut shown: Vec<T> = self.older.drain(start..).collect();
        shown.append(&mut self.items);
        self.items = shown;
        true
    }

    /// IDs aller vorhandenen Zeilen (zum Aussortieren doppelter Nachrichten).
    pub fn known_ids(&self) -> HashSet<String> {
        self.older.iter().chain(&self.items).chain(&self.pending).map(|i| i.id().to_string()).collect()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.older.iter().chain(&self.items).chain(&self.pending).any(|i| i.id() == id)
    }

    /// Setzt den Verlauf vor die bisherigen Zeilen. Sichtbar werden die neuesten `visible` Zeilen, der Rest erscheint
    /// beim Hochscrollen. Doppelte Zeilen werden verworfen. true: sichtbar geändert.
    pub fn prepend_history(&mut self, history: Vec<T>, visible: usize) -> bool {
        self.flush();
        let known = self.known_ids();
        let mut fresh: Vec<T> = history.into_iter().filter(|item| !known.contains(item.id())).collect();
        if fresh.is_empty() {
            return false;
        }
        let split = fresh.len().saturating_sub(visible);
        let mut shown = fresh.split_off(split);
        fresh.append(&mut self.older);
        self.older = fresh;
        shown.append(&mut self.items);
        self.items = shown;
        true
    }

    /// Ändert vorhandene Zeilen, auch wartende und ältere. `update` gibt die neue Zeile zurück oder None (unverändert).
    /// true: sichtbare Zeilen geändert.
    pub fn update(&mut self, mut update: impl FnMut(&T) -> Option<T>) -> bool {
        for list in [&mut self.pending, &mut self.older] {
            for item in list.iter_mut() {
                if let Some(next) = update(item) {
                    *item = next;
                }
            }
        }
        let mut changed = false;
        for item in self.items.iter_mut() {
            if let Some(next) = update(item) {
                *item = next;
                changed = true;
            }
        }
        changed
    }
}

// ---- Bans aus dem Verlauf (eigener Kanal) ----

/// Höchster Abstand zwischen Zeitpunkt im Verlauf und created_at bei Twitch, damit es dieselbe Aktion ist.
const MATCH_WINDOW_MS: f64 = 10_000.0;

fn text<'a>(value: &'a Value, name: &str) -> &'a str {
    value.get(name).and_then(Value::as_str).unwrap_or_default()
}

/// Bans und Timeouts aus dem Verlauf, zu denen der Mod noch fehlt.
pub fn is_unresolved_history_ban(item: &Value) -> bool {
    text(item, "kind") == "mod-action"
        && item.get("fromHistory").and_then(Value::as_bool) == Some(true)
        && item.get("moderator").is_none_or(Value::is_null)
        && item.get("target").is_some_and(Value::is_object)
        && matches!(text(item, "action"), "ban" | "timeout")
}

/// Ergänzt Mod und Grund aus Get Banned Users (nur eigener Kanal). Twitch liefert nur, was noch gilt: zu abgelaufenen
/// Timeouts und aufgehobenen Bans bleibt die Zeile ohne Mod. Gibt die geänderten Zeilen zurück (gleiche IDs).
pub fn match_bans(actions: &[Value], bans: &[Value]) -> Vec<Value> {
    let mut updated = Vec::new();
    for action in actions {
        let target = action.get("target").map(|t| text(t, "id")).unwrap_or_default();
        let timestamp = action.get("timestamp").and_then(Value::as_f64).unwrap_or_default();
        let is_ban = text(action, "action") == "ban";
        let found = bans.iter().find(|ban| {
            text(ban, "user_id") == target
                && text(ban, "expires_at").is_empty() == is_ban
                && parse_rfc3339_ms(text(ban, "created_at")).is_some_and(|created| (created - timestamp).abs() <= MATCH_WINDOW_MS)
        });
        let Some(ban) = found else { continue };
        let login = text(ban, "moderator_login");
        let name = Some(text(ban, "moderator_name")).filter(|n| !n.is_empty()).unwrap_or(login);
        let mut next = action.clone();
        next["moderator"] = json!({ "id": text(ban, "moderator_id"), "login": login, "displayName": name });
        match Some(text(ban, "reason")).filter(|r| !r.is_empty()) {
            Some(reason) => next["reason"] = json!(reason),
            None => {
                if let Some(object) = next.as_object_mut() {
                    object.remove("reason");
                }
            }
        }
        updated.push(next);
    }
    updated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: &str) -> Value {
        json!({ "id": id })
    }

    fn ids(items: &[Value]) -> Vec<&str> {
        items.iter().map(|i| i.id()).collect()
    }

    #[test]
    fn flush_keeps_newest_up_to_max() {
        let mut buffer = ChatBuffer::with_max(3);
        for id in ["a", "b", "c", "d"] {
            buffer.push(line(id));
        }
        assert!(buffer.flush());
        assert_eq!(ids(buffer.items()), vec!["b", "c", "d"]);
        assert!(!buffer.flush());
    }

    #[test]
    fn history_goes_before_live_without_duplicates() {
        let mut buffer = ChatBuffer::with_max(100);
        buffer.push(line("live1"));
        buffer.push(line("h3"));
        let history = ["h1", "h2", "h3", "h4"].iter().map(|id| line(id)).collect();
        assert!(buffer.prepend_history(history, 2));
        // h3 kam schon live: verworfen. Sichtbar die neuesten 2 des Verlaufs, h1 erst beim Hochscrollen.
        assert_eq!(ids(buffer.items()), vec!["h2", "h4", "live1", "h3"]);
        assert!(buffer.has_older());
        assert!(buffer.load_older(100));
        assert_eq!(ids(buffer.items()), vec!["h1", "h2", "h4", "live1", "h3"]);
        assert!(!buffer.has_older());
    }

    #[test]
    fn older_pages_from_the_end() {
        let mut buffer = ChatBuffer::with_max(100);
        let history = (0..5).map(|i| line(&format!("h{i}"))).collect();
        buffer.prepend_history(history, 1);
        assert!(buffer.load_older(2));
        assert_eq!(ids(buffer.items()), vec!["h2", "h3", "h4"]);
    }

    #[test]
    fn update_reaches_pending_and_older() {
        let mut buffer = ChatBuffer::with_max(100);
        buffer.prepend_history(vec![line("old"), line("shown")], 1);
        buffer.push(line("waiting"));
        let mark = |item: &Value| (item.id() != "shown").then(|| json!({ "id": item.id(), "marked": true }));
        assert!(!buffer.update(mark));
        buffer.flush();
        buffer.load_older(10);
        assert!(buffer.items().iter().filter(|i| i.id() != "shown").all(|i| i["marked"] == true));
    }

    fn action(id: &str, kind: &str, user: &str, timestamp: f64) -> Value {
        json!({ "kind": "mod-action", "id": id, "timestamp": timestamp, "action": kind,
            "target": { "id": user, "login": user, "displayName": user }, "fromHistory": true })
    }

    fn ban(user: &str, created_at: &str, expires: &str) -> Value {
        json!({ "user_id": user, "created_at": created_at, "expires_at": expires, "reason": "Spam",
            "moderator_id": "9", "moderator_login": "modi", "moderator_name": "Modi" })
    }

    #[test]
    fn only_history_bans_without_moderator() {
        let t = 1_794_744_000_000.0;
        let mut with_mod = action("b", "timeout", "u2", t);
        with_mod["moderator"] = json!({ "id": "9" });
        let mut live = action("d", "ban", "u4", t);
        live["fromHistory"] = json!(false);
        let items = [action("a", "ban", "u1", t), with_mod, action("c", "delete", "u3", t), live];
        let open: Vec<&str> = items.iter().filter(|i| is_unresolved_history_ban(i)).map(|i| i.id()).collect();
        assert_eq!(open, vec!["a"]);
    }

    #[test]
    fn bans_match_same_person_kind_and_time() {
        // 2026-11-15T12:00:00Z
        let t = 1_794_744_000_000.0;
        let actions = [action("a", "ban", "u1", t), action("b", "timeout", "u2", t), action("c", "timeout", "u3", t)];
        let bans = [
            ban("u1", "2026-11-15T12:00:02Z", ""),
            ban("u2", "2026-11-15T12:00:00Z", "2026-11-15T12:10:00Z"),
            // Späterer, neuer Timeout: nicht derselbe wie im Verlauf.
            ban("u3", "2026-11-15T12:01:00Z", "2026-11-15T12:10:00Z"),
        ];
        let result = match_bans(&actions, &bans);
        assert_eq!(ids(&result), vec!["a", "b"]);
        assert_eq!(result[0]["moderator"]["displayName"], "Modi");
        assert_eq!(result[0]["reason"], "Spam");
        // Ban bei Twitch, im Verlauf aber ein Timeout: andere Aktion.
        assert!(match_bans(&[action("x", "timeout", "u1", t)], &[ban("u1", "2026-11-15T12:00:00Z", "")]).is_empty());
    }
}
