//! Reine Teile der Chat-Werkzeuge (bisher src/features/chat/chat-search.ts, reply-target.ts, pin-action.ts,
//! src/features/pins/pinned-store.ts und src/features/usercard/card-windows.ts, recent.ts): Suche im Chat, Eingabefelder
//! je Kanal, Anpinnen mit Ausweichen auf „Dauer ändern“, angepinnte Nachricht, Nutzerkarten als Fenster und die letzten
//! Nachrichten einer Person. Alles nur im Arbeitsspeicher; die Speicher selbst liegen in crates/web/src/chat_tools.rs.

use std::future::Future;

use serde::Serialize;
use serde_json::Value;

use crate::model::{Badge, ChannelRef, UserRef};
use crate::time::parse_rfc3339_ms;

// ---- Suche im Chat ----

/// Zustand der Suche eines Tabs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchState {
    pub open: bool,
    pub query: String,
    /// Steigt bei jedem Strg+F, damit das Suchfeld erneut den Fokus bekommt.
    pub focus_token: u32,
}

impl SearchState {
    /// Öffnen oder, wenn schon offen, das Suchfeld wieder fokussieren.
    pub fn opened(&self) -> SearchState {
        SearchState { open: true, query: self.query.clone(), focus_token: self.focus_token + 1 }
    }

    /// Schließen leert den Text.
    pub fn closed(&self) -> SearchState {
        SearchState { open: false, query: String::new(), focus_token: self.focus_token }
    }
}

/// Suchbegriff vergleichbar machen: ohne Leerraum am Rand, ohne „@“ vorne, klein geschrieben.
fn normalize(query: &str) -> String {
    let trimmed = query.trim();
    trimmed.strip_prefix('@').unwrap_or(trimmed).to_lowercase()
}

fn text<'a>(value: &'a Value, name: &str) -> &'a str {
    value.get(name).and_then(Value::as_str).unwrap_or_default()
}

/// Passt die Zeile? Nachrichten und AutoMod-Zeilen nach Text, Anzeigename oder Login.
pub fn matches_search(item: &Value, query: &str) -> bool {
    let q = normalize(query);
    if q.is_empty() || !matches!(text(item, "kind"), "message" | "automod") {
        return false;
    }
    let sender = item.get("sender").unwrap_or(&Value::Null);
    [text(item, "text"), text(sender, "displayName"), text(sender, "login")].iter().any(|t| t.to_lowercase().contains(&q))
}

/// IDs aller Treffer in Anzeigereihenfolge (älteste zuerst).
pub fn search_hits(items: &[Value], query: &str) -> Vec<String> {
    if normalize(query).is_empty() {
        return Vec::new();
    }
    items.iter().filter(|i| matches_search(i, query)).map(|i| text(i, "id").to_string()).collect()
}

// ---- Eingabefelder je Kanal ----

/// Angemeldete Eingabefelder je Kanal (Login, Groß- und Kleinschreibung egal); ein neues ersetzt das alte. Abmelden
/// wirkt nur, solange noch dasselbe Feld angemeldet ist.
#[derive(Debug, Clone)]
pub struct InputRegistry<H> {
    entries: Vec<(String, u32, H)>,
    next: u32,
}

impl<H> Default for InputRegistry<H> {
    fn default() -> Self {
        InputRegistry { entries: Vec::new(), next: 0 }
    }
}

impl<H> InputRegistry<H> {
    pub fn register(&mut self, channel_login: &str, handler: H) -> u32 {
        let login = channel_login.to_lowercase();
        self.next += 1;
        self.entries.retain(|(l, _, _)| *l != login);
        self.entries.push((login, self.next, handler));
        self.next
    }

    pub fn unregister(&mut self, channel_login: &str, id: u32) {
        let login = channel_login.to_lowercase();
        self.entries.retain(|(l, i, _)| !(*l == login && *i == id));
    }

    pub fn get(&self, channel_login: &str) -> Option<&H> {
        let login = channel_login.to_lowercase();
        self.entries.iter().find(|(l, _, _)| *l == login).map(|(_, _, h)| h)
    }
}

// ---- Anpinnen ----

/// Ergebnis von `pin_or_update`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinOutcome {
    Pinned,
    Updated,
}

/// Anpinnen; ist die Nachricht schon angepinnt (409), wird stattdessen die Dauer geändert (zum Beispiel erst 1 Minute,
/// danach bis zum Streamende). `status` liest den HTTP-Status aus einem Fehler.
pub async fn pin_or_update<E, P, U, F>(pin: P, update: U, status: impl Fn(&E) -> Option<u16>) -> Result<PinOutcome, E>
where
    P: Future<Output = Result<(), E>>,
    U: FnOnce() -> F,
    F: Future<Output = Result<(), E>>,
{
    match pin.await {
        Ok(()) => Ok(PinOutcome::Pinned),
        Err(error) if status(&error) == Some(409) => {
            update().await?;
            Ok(PinOutcome::Updated)
        }
        Err(error) => Err(error),
    }
}

/// Lösen: true, wenn gelöst; false, wenn die Nachricht gar nicht angepinnt war (404).
pub async fn unpin<E>(remove: impl Future<Output = Result<(), E>>, status: impl Fn(&E) -> Option<u16>) -> Result<bool, E> {
    match remove.await {
        Ok(()) => Ok(true),
        Err(error) if status(&error) == Some(404) => Ok(false),
        Err(error) => Err(error),
    }
}

/// Angepinnte Nachricht eines Kanals.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinnedMessage {
    pub message_id: String,
    pub sender_name: String,
    pub pinned_by: String,
    /// None, wenn Twitch den Text nicht liefert.
    pub text: Option<String>,
    /// Ende in Millisekunden; None heißt bis zum Streamende.
    pub ends_at: Option<f64>,
}

/// Antwort von GET /chat/pins (ein Eintrag), geprüft mit dem Schema „pinnedMessage“ aus crates/core/src/stored.rs.
pub fn to_pinned_message(raw: &Value) -> Option<PinnedMessage> {
    let parsed = crate::stored::named("pinnedMessage")?.parse(raw).ok()?;
    let sender = Some(text(&parsed, "sender_user_name")).filter(|n| !n.is_empty()).unwrap_or(text(&parsed, "sender_user_login"));
    Some(PinnedMessage {
        message_id: text(&parsed, "message_id").to_string(),
        sender_name: sender.to_string(),
        pinned_by: text(&parsed, "pinned_by_user_name").to_string(),
        text: parsed.get("message").and_then(|m| m.get("text")).and_then(Value::as_str).map(str::to_string),
        ends_at: parsed.get("ends_at").and_then(Value::as_str).and_then(parse_rfc3339_ms),
    })
}

// ---- Nutzerkarten ----

/// Breite der Karte in Pixeln.
pub const CARD_WIDTH: f64 = 320.0;
const CARD_MARGIN: f64 = 8.0;
/// Höchstens so viele Nachrichten in der Karte (die neuesten).
pub const RECENT_MESSAGES_LIMIT: usize = 50;

/// Position innerhalb des sichtbaren Bereichs halten (die Karte bleibt immer greifbar).
pub fn clamp_position(x: f64, y: f64, width: f64, height: f64) -> (f64, f64) {
    (x.max(CARD_MARGIN).min((width - CARD_WIDTH - CARD_MARGIN).max(CARD_MARGIN)), y.max(CARD_MARGIN).min((height - 80.0).max(CARD_MARGIN)))
}

/// Eine offene Karte. `release` hebt die Chat-Pause auf, die die Karte beim Öffnen gesetzt hat (nur solange nicht
/// angepinnt).
#[derive(Debug, Clone)]
pub struct CardWindow<R> {
    pub id: String,
    pub channel: ChannelRef,
    pub user: UserRef,
    pub badges: Option<Vec<Badge>>,
    pub x: f64,
    pub y: f64,
    pub pinned: bool,
    pub release: Option<R>,
}

/// Offene Nutzerkarten als frei verschiebbare Fenster. Eine nicht angepinnte Karte schließt beim Klick daneben oder mit
/// Esc und wird von der nächsten ersetzt; angepinnte bleiben offen. Methoden geben die Pausen zurück, die freizugeben
/// sind.
#[derive(Debug, Clone)]
pub struct CardWindows<R> {
    pub windows: Vec<CardWindow<R>>,
    counter: u32,
}

impl<R> Default for CardWindows<R> {
    fn default() -> Self {
        CardWindows { windows: Vec::new(), counter: 0 }
    }
}

impl<R> CardWindows<R> {
    /// Neue Karte öffnen; eine offene, nicht angepinnte wird ersetzt. Dieselbe Person im selben Kanal nur einmal
    /// (dann nach vorne). `viewport`: Breite und Höhe des sichtbaren Bereichs.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        &mut self,
        channel: ChannelRef,
        user: UserRef,
        badges: Option<Vec<Badge>>,
        x: f64,
        y: f64,
        release: Option<R>,
        viewport: (f64, f64),
    ) -> Vec<R> {
        if let Some(id) = self.windows.iter().find(|w| w.user.id == user.id && w.channel.id == channel.id).map(|w| w.id.clone()) {
            self.focus(&id);
            return release.into_iter().collect();
        }
        let mut released = Vec::new();
        let mut kept = Vec::new();
        for mut window in std::mem::take(&mut self.windows) {
            if window.pinned {
                kept.push(window);
            } else if let Some(r) = window.release.take() {
                released.push(r);
            }
        }
        self.counter += 1;
        let (x, y) = clamp_position(x, y, viewport.0, viewport.1);
        kept.push(CardWindow { id: format!("card-{}", self.counter), channel, user, badges, x, y, pinned: false, release });
        self.windows = kept;
        released
    }

    pub fn close(&mut self, id: &str) -> Vec<R> {
        let Some(index) = self.windows.iter().position(|w| w.id == id) else { return Vec::new() };
        self.windows.remove(index).release.into_iter().collect()
    }

    /// Alle nicht angepinnten Karten schließen (Klick daneben, Esc). None: es war keine offen.
    pub fn close_unpinned(&mut self) -> Option<Vec<R>> {
        if self.windows.iter().all(|w| w.pinned) {
            return None;
        }
        let mut released = Vec::new();
        self.windows.retain_mut(|w| {
            if w.pinned {
                return true;
            }
            released.extend(w.release.take());
            false
        });
        Some(released)
    }

    /// Anpinnen: bleibt offen und gibt den Chat wieder frei (die Karte hängt nicht mehr an der Zeile).
    pub fn set_pinned(&mut self, id: &str, pinned: bool) -> Vec<R> {
        let Some(window) = self.windows.iter_mut().find(|w| w.id == id) else { return Vec::new() };
        window.pinned = pinned;
        if pinned { window.release.take().into_iter().collect() } else { Vec::new() }
    }

    pub fn move_to(&mut self, id: &str, x: f64, y: f64, viewport: (f64, f64)) {
        if let Some(window) = self.windows.iter_mut().find(|w| w.id == id) {
            (window.x, window.y) = clamp_position(x, y, viewport.0, viewport.1);
        }
    }

    /// Zuletzt angefasste Karte nach vorne holen. false: war schon vorne oder fehlt.
    pub fn focus(&mut self, id: &str) -> bool {
        match self.windows.iter().position(|w| w.id == id) {
            Some(index) if index + 1 < self.windows.len() => {
                let window = self.windows.remove(index);
                self.windows.push(window);
                true
            }
            _ => false,
        }
    }
}

/// Nachrichten der Person im Kanal aus dem geladenen Chat, älteste zuerst, höchstens die neuesten `limit`.
pub fn recent_messages_of<'a>(items: impl IntoIterator<Item = &'a Value>, user_id: &str, limit: usize) -> Vec<Value> {
    let own: Vec<&Value> = items
        .into_iter()
        .filter(|i| text(i, "kind") == "message" && i.get("sender").map(|s| text(s, "id")) == Some(user_id))
        .collect();
    own[own.len().saturating_sub(limit)..].iter().map(|v| (*v).clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(id: &str, login: &str, name: &str, text: &str) -> Value {
        json!({ "kind": "message", "id": id, "text": text, "sender": { "id": login, "login": login, "displayName": name } })
    }

    #[test]
    fn search_finds_text_and_names() {
        let items = vec![
            message("a", "anna", "Anna", "Hallo zusammen"),
            json!({ "kind": "divider", "id": "d", "label": "Live ab hier" }),
            message("b", "bernd_99", "Bernd_99", "hallo Anna"),
            message("c", "carl", "Carl", "tschüss"),
        ];
        assert_eq!(search_hits(&items, "HALLO"), vec!["a", "b"]);
        assert_eq!(search_hits(&items, "@anna"), vec!["a", "b"]);
        assert_eq!(search_hits(&items, "bernd"), vec!["b"]);
        assert!(search_hits(&items, "  ").is_empty());
        assert!(!matches_search(&items[1], "live"));
    }

    #[test]
    fn search_state_per_tab() {
        let state = SearchState::default().opened();
        let state = SearchState { query: "anna".into(), ..state };
        assert_eq!(state, SearchState { open: true, query: "anna".into(), focus_token: 1 });
        assert_eq!(state.opened().focus_token, 2);
        assert_eq!(state.opened().closed(), SearchState { open: false, query: String::new(), focus_token: 2 });
    }

    #[test]
    fn inputs_by_channel_ignore_case() {
        let mut registry = InputRegistry::default();
        let id = registry.register("Kanal", "feld");
        assert_eq!(registry.get("KANAL"), Some(&"feld"));
        registry.unregister("kanal", id + 1);
        assert!(registry.get("kanal").is_some());
        registry.unregister("kanal", id);
        assert!(registry.get("kanal").is_none());
    }

    fn status(error: &u16) -> Option<u16> {
        Some(*error)
    }

    #[test]
    fn pin_falls_back_to_update_only_on_409() {
        use futures::executor::block_on;
        let ok = block_on(pin_or_update(async { Ok::<(), u16>(()) }, || async { Ok(()) }, status));
        assert_eq!(ok, Ok(PinOutcome::Pinned));
        let updated = block_on(pin_or_update(async { Err(409u16) }, || async { Ok(()) }, status));
        assert_eq!(updated, Ok(PinOutcome::Updated));
        let failed = block_on(pin_or_update(async { Err(400u16) }, || async { Ok(()) }, status));
        assert_eq!(failed, Err(400));
        assert_eq!(block_on(unpin(async { Err(404u16) }, status)), Ok(false));
        assert_eq!(block_on(unpin(async { Ok::<(), u16>(()) }, status)), Ok(true));
    }

    #[test]
    fn pinned_message_without_end_lasts_until_stream_end() {
        let pin = to_pinned_message(&json!({
            "message_id": "m1", "broadcaster_id": "1", "sender_user_id": "2", "sender_user_login": "jemand",
            "sender_user_name": "Jemand", "pinned_by_user_id": "3", "pinned_by_user_login": "mod", "pinned_by_user_name": "Mod",
            "message": { "text": "Willkommen!", "fragments": [] }, "starts_at": "2026-11-15T12:00:00Z", "ends_at": null,
            "updated_at": "2026-11-15T12:00:00Z"
        }));
        assert_eq!(
            pin,
            Some(PinnedMessage { message_id: "m1".into(), sender_name: "Jemand".into(), pinned_by: "Mod".into(), text: Some("Willkommen!".into()), ends_at: None })
        );
        assert_eq!(to_pinned_message(&Value::Null), None);
    }

    fn channel() -> ChannelRef {
        ChannelRef { platform: "twitch".into(), id: "k".into(), login: "kanal".into(), display_name: "Kanal".into() }
    }

    fn user(id: &str) -> UserRef {
        UserRef { id: id.into(), login: id.into(), display_name: id.into() }
    }

    #[test]
    fn cards_replace_unpinned_and_release_pause() {
        let view = (1000.0, 800.0);
        let mut cards = CardWindows::default();
        assert!(cards.open(channel(), user("a"), None, 10.0, 10.0, Some("a"), view).is_empty());
        assert_eq!(cards.open(channel(), user("b"), None, 10.0, 10.0, Some("b"), view), vec!["a"]);
        assert_eq!(cards.windows.iter().map(|w| w.user.id.as_str()).collect::<Vec<_>>(), vec!["b"]);
        let b = cards.windows[0].id.clone();
        assert_eq!(cards.set_pinned(&b, true), vec!["b"]);
        cards.open(channel(), user("c"), None, 10.0, 10.0, None, view);
        assert_eq!(cards.close_unpinned(), Some(Vec::new()));
        assert_eq!(cards.windows.iter().map(|w| w.user.id.as_str()).collect::<Vec<_>>(), vec!["b"]);
        assert!(cards.close(&b).is_empty());
        assert!(cards.windows.is_empty());
    }

    #[test]
    fn cards_stay_visible_when_moved() {
        assert_eq!(clamp_position(-50.0, -50.0, 1000.0, 800.0), (8.0, 8.0));
        assert_eq!(clamp_position(5000.0, 5000.0, 1000.0, 800.0), (1000.0 - CARD_WIDTH - 8.0, 720.0));
        let mut cards: CardWindows<()> = CardWindows::default();
        cards.open(channel(), user("d"), None, 10.0, 10.0, None, (1000.0, 800.0));
        let id = cards.windows[0].id.clone();
        cards.move_to(&id, 100.0, 120.0, (1000.0, 800.0));
        assert_eq!((cards.windows[0].x, cards.windows[0].y), (100.0, 120.0));
    }

    #[test]
    fn recent_messages_newest_limited() {
        let items: Vec<Value> = ["a1", "b1", "a2", "a3"].iter().map(|id| message(id, &id[..1], &id[..1], "x")).collect();
        let ids = |list: Vec<Value>| list.iter().map(|m| m["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(ids(recent_messages_of(&items, "a", 50)), vec!["a1", "a2", "a3"]);
        assert_eq!(ids(recent_messages_of(&items, "a", 2)), vec!["a2", "a3"]);
        assert!(recent_messages_of(&items, "x", 50).is_empty());
    }
}
