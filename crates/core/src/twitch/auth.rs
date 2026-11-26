//! Anmeldung bei Twitch, übertragen aus src/platforms/twitch/auth.ts: Anmelde-Adresse, Rücksprung lesen, Antworten von
//! id.twitch.tv auswerten. Ohne Netzwerk; das Senden macht crates/wasm. Token stehen nie in Fehlermeldungen.

use crate::i18n::tr;
use serde::Serialize;
use serde_json::Value;
use url::Url;

use crate::net::form_encode;

pub const AUTHORIZE_URL: &str = "https://id.twitch.tv/oauth2/authorize";
const ACTIVATE_FALLBACK_URL: &str = "https://www.twitch.tv/activate";

fn query_string(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v))).collect::<Vec<_>>().join("&")
}

/// Anmelde-Adresse für den Implicit Grant (Rücksprung mit Token).
pub fn build_authorize_url(client_id: &str, redirect_uri: &str, scopes: &[String], state: &str, force_verify: bool) -> String {
    let scope = scopes.join(" ");
    let mut pairs = vec![
        ("response_type", "token"),
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("scope", scope.as_str()),
        ("state", state),
    ];
    if force_verify {
        pairs.push(("force_verify", "true"));
    }
    format!("{AUTHORIZE_URL}?{}", query_string(&pairs))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RedirectResult {
    #[serde(rename_all = "camelCase")]
    Token { access_token: String, scopes: Vec<String> },
    Error { message: String },
}

/// Parameter aus dem Abfrageteil oder dem Fragment (ohne `#`), wie URLSearchParams.
fn params(text: &str) -> Vec<(String, String)> {
    url::form_urlencoded::parse(text.as_bytes()).into_owned().collect()
}

fn get<'a>(params: &'a [(String, String)], key: &str) -> Option<&'a str> {
    params.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// Enthält die Adresse einen Rücksprung von Twitch (Token im Fragment oder Fehler)?
pub fn is_auth_redirect(href: &str) -> bool {
    let Ok(url) = Url::parse(href) else {
        return false;
    };
    let hash = params(url.fragment().unwrap_or(""));
    let query = params(url.query().unwrap_or(""));
    get(&hash, "access_token").is_some() || get(&query, "error").is_some() || get(&hash, "error").is_some()
}

/// Wertet einen Rücksprung aus. `expected_state` ist der vorher gemerkte Zufallswert (schützt vor untergeschobenen
/// Rücksprüngen). None: die Adresse enthält keinen Rücksprung.
pub fn interpret_redirect(href: &str, expected_state: Option<&str>) -> Option<RedirectResult> {
    let url = Url::parse(href).ok()?;
    let hash = params(url.fragment().unwrap_or(""));
    let query = params(url.query().unwrap_or(""));
    let has_token = get(&hash, "access_token").is_some();
    let has_error = get(&query, "error").is_some() || get(&hash, "error").is_some();
    if !has_token && !has_error {
        return None;
    }
    let error = |message: &str| Some(RedirectResult::Error { message: message.into() });
    let returned_state = if has_token { get(&hash, "state") } else { get(&query, "state").or_else(|| get(&hash, "state")) };
    let expected = expected_state.filter(|s| !s.is_empty());
    if expected.is_none() || returned_state != expected {
        return error(tr("Die Anmeldung konnte nicht bestätigt werden. Bitte erneut anmelden.", "The sign-in could not be confirmed. Please sign in again."));
    }
    if has_error {
        let code = get(&query, "error").or_else(|| get(&hash, "error")).unwrap_or("");
        if code == "access_denied" {
            return error(tr("Die Anmeldung wurde bei Twitch abgebrochen.", "The sign-in was canceled on Twitch."));
        }
        let description = get(&query, "error_description").or_else(|| get(&hash, "error_description")).unwrap_or(code);
        return error(&crate::tf!("Twitch hat die Anmeldung abgelehnt: {description}", "Twitch declined the sign-in: {description}"));
    }
    let access_token = get(&hash, "access_token").unwrap_or("");
    if access_token.is_empty() {
        return error(tr("Twitch hat kein Token geliefert.", "Twitch did not deliver a token."));
    }
    let scopes = get(&hash, "scope").unwrap_or("").split(' ').filter(|s| !s.is_empty()).map(str::to_string).collect();
    Some(RedirectResult::Token { access_token: access_token.into(), scopes })
}

/// Bereinigt ein eingefügtes Token (zum Beispiel mit Vorsilbe „oauth:“ oder Leerzeichen).
pub fn normalize_pasted_token(input: &str) -> String {
    let trimmed = input.trim();
    let without = if trimmed.len() >= 6 && trimmed[..6].eq_ignore_ascii_case("oauth:") { &trimmed[6..] } else { trimmed };
    without.trim().to_string()
}

/// Der Bestätigungslink kommt von Twitch und wird nur geöffnet, wenn er per https auf twitch.tv zeigt.
pub fn safe_verification_uri(uri: &str) -> String {
    match Url::parse(uri) {
        Ok(url) if url.scheme() == "https" && url.host_str().is_some_and(|h| h == "twitch.tv" || h.ends_with(".twitch.tv")) => {
            url.to_string()
        }
        _ => ACTIVATE_FALLBACK_URL.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidatedToken {
    pub client_id: String,
    pub user_id: String,
    pub login: String,
    pub scopes: Vec<String>,
    /// Restlaufzeit in Sekunden; 0 heißt ohne Ablauf.
    pub expires_in: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ValidateResult {
    Valid { token: ValidatedToken },
    Invalid,
    /// Twitch nicht erreichbar oder unerwartete Antwort; das Token gilt weiter.
    Unreachable,
}

/// Antwort von /oauth2/validate auswerten. `body` None: keine JSON-Antwort.
pub fn interpret_validate(status: u16, ok: bool, body: Option<&Value>) -> ValidateResult {
    if status == 401 {
        return ValidateResult::Invalid;
    }
    if !ok {
        return ValidateResult::Unreachable;
    }
    let Some(body) = body.filter(|b| b.is_object()) else {
        return ValidateResult::Unreachable;
    };
    let text = |key: &str| body.get(key).and_then(Value::as_str).map(str::to_string);
    let (Some(user_id), Some(login), Some(client_id)) = (text("user_id"), text("login"), text("client_id")) else {
        // App-Token ohne Nutzer: für MultiYap nicht brauchbar.
        return ValidateResult::Invalid;
    };
    let scopes = body
        .get("scopes")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let expires_in = body.get("expires_in").and_then(Value::as_f64).unwrap_or(0.0);
    ValidateResult::Valid { token: ValidatedToken { client_id, user_id, login, scopes, expires_in } }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: f64,
    pub interval: f64,
}

/// Antwort von /oauth2/device auswerten; Fehler als verständliche Meldung.
pub fn interpret_device_code(ok: bool, body: Option<&Value>) -> Result<DeviceCode, String> {
    let fail = || tr("Twitch hat keinen Code geliefert. Bitte später erneut versuchen.", "Twitch did not deliver a code. Please try again later.").to_string();
    let body = body.ok_or_else(fail)?;
    let text = |key: &str| body.get(key).and_then(Value::as_str).map(str::to_string);
    let (true, Some(device_code), Some(user_code), Some(verification_uri)) =
        (ok, text("device_code"), text("user_code"), text("verification_uri"))
    else {
        return Err(fail());
    };
    let positive = |key: &str, fallback: f64| body.get(key).and_then(Value::as_f64).filter(|n| *n > 0.0).unwrap_or(fallback);
    Ok(DeviceCode {
        device_code,
        user_code,
        verification_uri: safe_verification_uri(&verification_uri),
        expires_in: positive("expires_in", 1800.0),
        interval: positive("interval", 5.0),
    })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssuedToken {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: f64,
}

fn parse_issued_token(body: Option<&Value>) -> Option<IssuedToken> {
    let body = body?;
    let text = |key: &str| body.get(key).and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_string);
    Some(IssuedToken {
        access_token: text("access_token")?,
        refresh_token: text("refresh_token")?,
        expires_in: body.get("expires_in").and_then(Value::as_f64).unwrap_or(0.0),
    })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum DevicePollResult {
    Token { token: IssuedToken },
    Pending,
    SlowDown,
    Denied,
    Expired,
    Unreachable,
    Error { message: String },
}

/// Antwort beim Abfragen des Tokens zu einem Code auswerten.
pub fn interpret_device_poll(status: u16, ok: bool, body: Option<&Value>) -> DevicePollResult {
    if ok {
        return match parse_issued_token(body) {
            Some(token) => DevicePollResult::Token { token },
            None => DevicePollResult::Error { message: tr("Twitch hat kein Token geliefert.", "Twitch did not deliver a token.").into() },
        };
    }
    if status >= 500 {
        return DevicePollResult::Unreachable;
    }
    let message = body.and_then(|b| b.get("message")).and_then(Value::as_str).unwrap_or("").to_lowercase();
    if message.contains("authorization_pending") {
        DevicePollResult::Pending
    } else if message.contains("slow_down") {
        DevicePollResult::SlowDown
    } else if message.contains("denied") || message.contains("declined") {
        DevicePollResult::Denied
    } else if message.contains("invalid device code") || message.contains("expired") {
        DevicePollResult::Expired
    } else {
        DevicePollResult::Error { message: tr("Twitch hat die Anmeldung abgelehnt.", "Twitch declined the sign-in.").into() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RefreshResult {
    Token { token: IssuedToken },
    /// Refresh-Token ungültig oder abgelaufen: neu anmelden.
    Invalid,
    /// Twitch nicht erreichbar: später erneut versuchen.
    Unreachable,
}

/// Antwort beim Verlängern auswerten.
pub fn interpret_refresh(status: u16, ok: bool, body: Option<&Value>) -> RefreshResult {
    if status == 400 || status == 401 {
        return RefreshResult::Invalid;
    }
    if !ok {
        return RefreshResult::Unreachable;
    }
    parse_issued_token(body).map_or(RefreshResult::Unreachable, |token| RefreshResult::Token { token })
}

/// Inhalt eines Formulars (application/x-www-form-urlencoded) wie URLSearchParams.
pub fn form_body(pairs: &[(&str, &str)]) -> String {
    query_string(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn anmelde_adresse() {
        let url = build_authorize_url("cid", "http://localhost:8475", &["user:read:chat".into(), "user:write:chat".into()], "abc", false);
        assert_eq!(
            url,
            "https://id.twitch.tv/oauth2/authorize?response_type=token&client_id=cid&redirect_uri=http%3A%2F%2Flocalhost%3A8475&scope=user%3Aread%3Achat+user%3Awrite%3Achat&state=abc"
        );
        assert!(build_authorize_url("c", "r", &[], "s", true).ends_with("&force_verify=true"));
    }

    #[test]
    fn ruecksprung() {
        assert_eq!(interpret_redirect("http://localhost:8475/", Some("s1")), None);
        assert!(!is_auth_redirect("http://localhost:8475/"));
        let token = "http://localhost:8475/#access_token=geheim&scope=user%3Aread%3Achat+user%3Awrite%3Achat&state=s1&token_type=bearer";
        assert!(is_auth_redirect(token));
        assert_eq!(
            interpret_redirect(token, Some("s1")),
            Some(RedirectResult::Token { access_token: "geheim".into(), scopes: vec!["user:read:chat".into(), "user:write:chat".into()] })
        );
        let wrong = interpret_redirect("http://localhost:8475/#access_token=geheim&state=falsch", Some("s1"));
        assert!(matches!(wrong, Some(RedirectResult::Error { ref message }) if !message.contains("geheim")));
        assert!(matches!(interpret_redirect(token, None), Some(RedirectResult::Error { .. })));
        assert_eq!(
            interpret_redirect("http://localhost:8475/?error=access_denied&error_description=The+user+denied&state=s2", Some("s2")),
            Some(RedirectResult::Error { message: "Die Anmeldung wurde bei Twitch abgebrochen.".into() })
        );
        assert_eq!(
            interpret_redirect("http://localhost:8475/?error=x&error_description=Kaputt+hier&state=s2", Some("s2")),
            Some(RedirectResult::Error { message: "Twitch hat die Anmeldung abgelehnt: Kaputt hier".into() })
        );
    }

    #[test]
    fn eingefuegtes_token() {
        assert_eq!(normalize_pasted_token("  oauth:abc123  "), "abc123");
        assert_eq!(normalize_pasted_token("OAuth:abc"), "abc");
        assert_eq!(normalize_pasted_token("abc"), "abc");
        assert_eq!(normalize_pasted_token("äöü"), "äöü");
    }

    #[test]
    fn bestaetigungslink() {
        assert_eq!(safe_verification_uri("https://www.twitch.tv/activate?x=1"), "https://www.twitch.tv/activate?x=1");
        for bad in ["http://www.twitch.tv/activate", "https://twitch.tv.evil.example/", "https://eviltwitch.tv/", "javascript:alert(1)", "kein link"] {
            assert_eq!(safe_verification_uri(bad), "https://www.twitch.tv/activate", "{bad}");
        }
    }

    #[test]
    fn token_pruefen() {
        let body = json!({ "client_id": "c", "user_id": "1", "login": "a", "scopes": ["x", 3], "expires_in": 100 });
        assert_eq!(
            interpret_validate(200, true, Some(&body)),
            ValidateResult::Valid { token: ValidatedToken { client_id: "c".into(), user_id: "1".into(), login: "a".into(), scopes: vec!["x".into()], expires_in: 100.0 } }
        );
        assert_eq!(interpret_validate(401, false, None), ValidateResult::Invalid);
        assert_eq!(interpret_validate(500, false, None), ValidateResult::Unreachable);
        assert_eq!(interpret_validate(200, true, None), ValidateResult::Unreachable);
        assert_eq!(interpret_validate(200, true, Some(&json!({ "client_id": "c" }))), ValidateResult::Invalid);
    }

    #[test]
    fn code_abfragen_verlaengern() {
        let code = interpret_device_code(true, Some(&json!({ "device_code": "d", "user_code": "U", "verification_uri": "https://evil.example/", "expires_in": 0 }))).unwrap();
        assert_eq!((code.verification_uri.as_str(), code.expires_in, code.interval), ("https://www.twitch.tv/activate", 1800.0, 5.0));
        assert!(interpret_device_code(false, Some(&json!({}))).unwrap_err().contains("keinen Code"));

        let pending = json!({ "message": "authorization_pending" });
        assert_eq!(interpret_device_poll(400, false, Some(&pending)), DevicePollResult::Pending);
        assert_eq!(interpret_device_poll(400, false, Some(&json!({ "message": "slow_down" }))), DevicePollResult::SlowDown);
        assert_eq!(interpret_device_poll(400, false, Some(&json!({ "message": "authorization_declined" }))), DevicePollResult::Denied);
        assert_eq!(interpret_device_poll(400, false, Some(&json!({ "message": "invalid device code" }))), DevicePollResult::Expired);
        assert_eq!(interpret_device_poll(503, false, None), DevicePollResult::Unreachable);
        let token = json!({ "access_token": "a", "refresh_token": "r", "expires_in": 14400 });
        assert_eq!(
            interpret_device_poll(200, true, Some(&token)),
            DevicePollResult::Token { token: IssuedToken { access_token: "a".into(), refresh_token: "r".into(), expires_in: 14400.0 } }
        );
        assert_eq!(interpret_refresh(400, false, None), RefreshResult::Invalid);
        assert_eq!(interpret_refresh(500, false, None), RefreshResult::Unreachable);
        assert!(matches!(interpret_refresh(200, true, Some(&token)), RefreshResult::Token { .. }));
        assert_eq!(form_body(&[("grant_type", "refresh_token"), ("refresh_token", "a/b")]), "grant_type=refresh_token&refresh_token=a%2Fb");
    }
}
