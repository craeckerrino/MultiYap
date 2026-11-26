//! Reine Hilfen für Anfragen an Twitch und andere Dienste, ohne selbst etwas zu senden. Das Senden übernimmt
//! crates/wasm mit dem fetch des Browsers.

/// Kodierung wie `URLSearchParams.toString()` (application/x-www-form-urlencoded): Buchstaben, Ziffern und `*-._`
/// bleiben, Leerzeichen wird `+`, alles andere `%XX`.
pub fn form_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => out.push(byte as char),
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Adresse mit Abfrageteil; mehrere Werte für denselben Schlüssel werden einzeln angehängt.
pub fn build_url(base: &str, path: &str, query: &[(String, Vec<String>)]) -> String {
    let search = query
        .iter()
        .flat_map(|(key, values)| values.iter().map(move |value| format!("{}={}", form_encode(key), form_encode(value))))
        .collect::<Vec<_>>()
        .join("&");
    if search.is_empty() {
        format!("{base}{path}")
    } else {
        format!("{base}{path}?{search}")
    }
}

/// Wartezeit nach Antwort 429 aus dem Kopf `Ratelimit-Reset` (Sekunden seit 1970): bis zum Reset, mindestens eine
/// halbe Sekunde, höchstens eine Minute; ohne gültige Angabe eine Sekunde.
pub fn rate_limit_delay_ms(reset_header: Option<&str>, now_ms: f64) -> f64 {
    let reset = reset_header.and_then(|value| value.trim().parse::<f64>().ok()).filter(|r| r.is_finite() && *r > 0.0);
    let delay = reset.map_or(1000.0, |reset| (reset * 1000.0 - now_ms).max(500.0));
    delay.min(60_000.0)
}

/// Fehlermeldung einer Antwort: das Feld `message` von Twitch, sonst „Twitch-Fehler <Status>“. Enthält nie Token.
pub fn error_message(status: u16, body: Option<&serde_json::Value>) -> String {
    body.and_then(|b| b.get("message"))
        .and_then(serde_json::Value::as_str)
        .filter(|m| !m.is_empty())
        .map_or_else(|| crate::tf!("Twitch-Fehler {status}", "Twitch error {status}"), str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn adressen_wie_urlsearchparams() {
        let query = vec![
            ("broadcaster_id".to_string(), vec!["1".to_string()]),
            ("user_id".to_string(), vec!["2".to_string(), "3".to_string()]),
            ("q".to_string(), vec!["a b&c=ä/".to_string()]),
        ];
        assert_eq!(
            build_url("https://api.twitch.tv/helix", "/users", &query),
            "https://api.twitch.tv/helix/users?broadcaster_id=1&user_id=2&user_id=3&q=a+b%26c%3D%C3%A4%2F"
        );
        assert_eq!(build_url("https://x", "/y", &[]), "https://x/y");
        assert_eq!(form_encode("*-._~"), "*-._%7E");
    }

    #[test]
    fn wartezeit_bei_rate_limit() {
        assert_eq!(rate_limit_delay_ms(Some("100"), 90_000.0), 10_000.0);
        assert_eq!(rate_limit_delay_ms(Some("100"), 99_900.0), 500.0);
        assert_eq!(rate_limit_delay_ms(Some("1000"), 0.0), 60_000.0);
        assert_eq!(rate_limit_delay_ms(None, 0.0), 1000.0);
        assert_eq!(rate_limit_delay_ms(Some("kaputt"), 0.0), 1000.0);
        assert_eq!(rate_limit_delay_ms(Some("0"), 0.0), 1000.0);
    }

    #[test]
    fn fehlermeldungen() {
        assert_eq!(error_message(400, Some(&json!({ "message": "invalid broadcaster" }))), "invalid broadcaster");
        assert_eq!(error_message(500, Some(&json!({ "message": "" }))), "Twitch-Fehler 500");
        assert_eq!(error_message(502, None), "Twitch-Fehler 502");
    }
}
