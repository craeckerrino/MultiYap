//! Live-Status und Stream-Info eines Kanals (bisher src/core/streams/stream-status.ts): Umwandeln der Helix-Antworten
//! und das Zusammenführen mit dem bisherigen Stand. Der Speicher selbst liegt in crates/web/src/streams.rs.

use crate::i18n::tr;
use serde::Serialize;
use serde_json::Value;

use crate::time::parse_rfc3339_ms;

/// Stand eines Kanals. Felder wie `StreamInfo` in TypeScript (fehlende Angaben fehlen auch im JSON).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamInfo {
    pub live: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewers: Option<f64>,
    /// Startzeit in Millisekunden.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<f64>,
    /// Andere Kanäle im Shared Chat (IDs).
    pub shared_with: Vec<String>,
    /// Kanal ist live gegangen, während er offen war (Tab pulsiert bis zum Klick).
    pub went_live: bool,
}

/// Ergebnis des Zusammenführens: neuer Stand und was zu melden ist.
#[derive(Debug, Clone, PartialEq)]
pub struct Merged {
    pub info: StreamInfo,
    /// Der Stand hat sich geändert (sonst nichts melden).
    pub changed: bool,
    /// Wechsel zwischen live und offline (nur, wenn der vorige Stand schon bekannt war).
    pub flipped: bool,
    /// Gerade live gegangen (für den Live-Ton); der erste bekannte Stand meldet nichts.
    pub went_live: bool,
}

/// Führt einen neuen Stand (ohne `went_live`) mit dem bisherigen zusammen. `known`: Der Status wurde schon einmal
/// ermittelt; beim ersten Stand pulsiert nichts und es wird kein Wechsel gemeldet.
pub fn merge(previous: &StreamInfo, known: bool, next: StreamInfo) -> Merged {
    let flipped = known && previous.live != next.live;
    let went_live = next.live && (previous.went_live || (known && !previous.live));
    let info = StreamInfo { went_live, ..next };
    Merged { changed: info != *previous, flipped, went_live: went_live && !previous.went_live, info }
}

/// Stand eines Kanals aus einem Eintrag von „Get Streams“; leere Texte fehlen.
pub fn info_from_stream(stream: &Value, shared_with: Vec<String>) -> StreamInfo {
    let text = |name: &str| stream.get(name).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    StreamInfo {
        live: true,
        title: text("title"),
        category: text("game_name"),
        viewers: stream.get("viewer_count").and_then(Value::as_f64),
        started_at: stream.get("started_at").and_then(Value::as_str).and_then(parse_rfc3339_ms),
        shared_with,
        went_live: false,
    }
}

/// Andere Kanäle einer Shared-Chat-Sitzung (Get Shared Chat Session), ohne den Kanal selbst.
pub fn shared_with(response: &Value, channel_id: &str) -> Vec<String> {
    response
        .get("data")
        .and_then(|d| d.get(0))
        .and_then(|s| s.get("participants"))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter().filter_map(|p| p.get("broadcaster_id").and_then(Value::as_str)).filter(|id| *id != channel_id).map(str::to_string).collect()
        })
        .unwrap_or_default()
}

/// Laufzeit seit Streamstart, zum Beispiel „2 Std. 15 Min.“ (Zeiten in Millisekunden).
pub fn format_uptime(started_at: f64, now: f64) -> String {
    let minutes = ((now - started_at) / 60_000.0).floor().max(0.0) as u64;
    let (hours, rest) = (minutes / 60, minutes % 60);
    if hours > 0 { crate::tf!("{hours} Std. {rest} Min.", "{hours} hr {rest} min") } else { crate::tf!("{rest} Min.", "{rest} min") }
}

/// Ganze Zahl mit Tausendertrennern der Sprache, zum Beispiel „12.345“ (Englisch „12,345“).
pub fn format_count(value: f64) -> String {
    let digits = (value.round().abs() as u64).to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(if crate::i18n::is_en() { ',' } else { '.' });
        }
        out.push(digit);
    }
    if value < 0.0 { format!("-{out}") } else { out }
}

/// Eine Zeile des Stream-Info-Tooltips: Beschriftung (fehlt beim Titel) und Wert.
#[derive(Debug, Clone, PartialEq)]
pub struct InfoLine {
    pub label: Option<&'static str>,
    pub value: String,
}

/// Zeilen des Stream-Info-Tooltips über dem Tab-Namen. `points`: Einstellung `tabs.streamInfo`, jeder Punkt einzeln
/// abschaltbar (fehlt einer, gilt er als an). „Zusammen mit“ nur bei aktivem Shared Chat; `shared_names` sind die
/// Anzeigenamen der anderen Kanäle.
pub fn info_lines(info: &StreamInfo, points: &Value, login: &str, shared_names: &[String], now: f64) -> Vec<InfoLine> {
    let on = |name: &str| points.get(name).and_then(Value::as_bool).unwrap_or(true);
    let mut lines = Vec::new();
    let mut push = |label: Option<&'static str>, value: String| lines.push(InfoLine { label, value });
    if on("title") && info.live {
        if let Some(title) = info.title.as_ref().filter(|t| !t.is_empty()) {
            push(None, title.clone());
        }
    }
    if on("category") && info.live {
        if let Some(category) = info.category.as_ref().filter(|c| !c.is_empty()) {
            push(Some(tr("Kategorie", "Category")), category.clone());
        }
    }
    if on("status") {
        push(Some("Status"), if info.live { "Live" } else { "Offline" }.to_string());
    }
    if on("viewers") && info.live {
        if let Some(viewers) = info.viewers {
            push(Some(tr("Zuschauer", "Viewers")), format_count(viewers));
        }
    }
    if on("uptime") && info.live {
        if let Some(started_at) = info.started_at.filter(|s| *s > 0.0) {
            push(Some(tr("Laufzeit", "Uptime")), format_uptime(started_at, now));
        }
    }
    if on("channelName") {
        push(Some(tr("Kanal", "Channel")), login.to_string());
    }
    if on("sharedWith") && info.live && !shared_names.is_empty() {
        push(Some(tr("Zusammen mit", "Together with")), shared_names.join(", "));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn laufzeit_in_stunden_und_minuten() {
        let start = 1_000_000_000.0;
        assert_eq!(format_uptime(start, start + 15.0 * 60_000.0), "15 Min.");
        assert_eq!(format_uptime(start, start + 135.0 * 60_000.0), "2 Std. 15 Min.");
        assert_eq!(format_uptime(start, start - 1000.0), "0 Min.");
    }

    #[test]
    fn zahlen_mit_tausenderpunkten() {
        assert_eq!(format_count(0.0), "0");
        assert_eq!(format_count(999.0), "999");
        assert_eq!(format_count(12_345.0), "12.345");
        assert_eq!(format_count(1_234_567.0), "1.234.567");
    }

    #[test]
    fn tooltip_zeilen_nach_einstellung() {
        let info = StreamInfo { live: true, title: Some("Titel".into()), category: Some("Spiel".into()), viewers: Some(1200.0), started_at: Some(0.0), ..Default::default() };
        let all = json!({});
        let lines = info_lines(&info, &all, "kanal", &["Anderer".into()], 60_000.0);
        let labels: Vec<_> = lines.iter().map(|l| l.label).collect();
        assert_eq!(labels, [None, Some("Kategorie"), Some("Status"), Some("Zuschauer"), Some("Kanal"), Some("Zusammen mit")]);
        assert_eq!(lines[3].value, "1.200");
        let offline = info_lines(&StreamInfo::default(), &json!({ "channelName": false }), "kanal", &[], 0.0);
        assert_eq!(offline, [InfoLine { label: Some("Status"), value: "Offline".into() }]);
    }

    fn live() -> StreamInfo {
        StreamInfo { live: true, ..Default::default() }
    }

    #[test]
    fn first_state_neither_flips_nor_pulses() {
        let merged = merge(&StreamInfo::default(), false, live());
        assert!(merged.changed && !merged.flipped && !merged.went_live && !merged.info.went_live);
    }

    #[test]
    fn going_live_while_open_pulses_until_offline() {
        let offline = StreamInfo::default();
        let merged = merge(&offline, true, live());
        assert!(merged.flipped && merged.went_live && merged.info.went_live);
        // Weiter live: pulsiert weiter, meldet aber nicht erneut.
        let again = merge(&merged.info, true, live());
        assert!(!again.flipped && !again.went_live && again.info.went_live && !again.changed);
        // Offline: Wechsel, Pulsieren endet.
        let off = merge(&again.info, true, StreamInfo::default());
        assert!(off.flipped && !off.info.went_live);
    }

    #[test]
    fn stream_entry_becomes_info() {
        let stream = json!({ "user_id": "1", "game_name": "", "title": "Hallo", "viewer_count": 12, "started_at": "2026-11-17T10:00:00Z" });
        let info = info_from_stream(&stream, vec!["2".into()]);
        assert_eq!(info.title.as_deref(), Some("Hallo"));
        assert_eq!(info.category, None);
        assert_eq!(info.viewers, Some(12.0));
        assert!(info.started_at.is_some());
        assert_eq!(info.shared_with, vec!["2".to_string()]);
    }

    #[test]
    fn shared_chat_without_own_channel() {
        let response = json!({ "data": [{ "participants": [{ "broadcaster_id": "1" }, { "broadcaster_id": "2" }] }] });
        assert_eq!(shared_with(&response, "1"), vec!["2".to_string()]);
        assert!(shared_with(&json!({ "data": [] }), "1").is_empty());
    }
}
