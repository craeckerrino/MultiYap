//! Helix-Anfragen (crates/web/src/net.rs) mit nachgebildeten Antworten von Twitch: Adresse und Kopfzeilen, Inhalt,
//! Fehler ohne Token, Verlängern bei 401, Warten bei 429. Vorher in TypeScript (src/platforms/twitch/helix.test.ts)
//! hier.

#![cfg(target_arch = "wasm32")]

use multiyap_web::net::{helix_call, set_helix_override};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

#[wasm_bindgen(inline_js = "
let responses = [];
let calls = [];
let token = 'alt';
let renewTo = null;
let renewCalls = 0;
export function setup(list, renew) {
  responses = list;
  calls = [];
  token = 'alt';
  renewTo = renew ?? null;
  renewCalls = 0;
  globalThis.fetch = async (request) => {
    calls.push(request);
    return responses.shift() ?? new Response('{}', { status: 500 });
  };
}
export function response(status, body, resetHeader) {
  const headers = resetHeader ? { 'Ratelimit-Reset': resetHeader } : {};
  return new Response(body ?? null, { status, headers });
}
export function credentials() { return { userId: 'u1', clientId: 'client', accessToken: token }; }
export function renew() {
  renewCalls += 1;
  if (renewTo) { token = renewTo; return Promise.resolve(true); }
  return Promise.resolve(false);
}
export function hooks() { return [credentials, renew]; }
export function renewCount() { return renewCalls; }
export function callCount() { return calls.length; }
export function sent(index) { return calls[index]; }
export function header(index, name) { return calls[index].headers.get(name); }
export function bodyText(index) { return calls[index].text(); }
")]
extern "C" {
    fn setup(list: js_sys::Array, renew: Option<String>);
    fn response(status: u16, body: Option<String>, reset_header: Option<String>) -> JsValue;
    fn hooks() -> js_sys::Array;
    #[wasm_bindgen(js_name = renewCount)]
    fn renew_count() -> u32;
    #[wasm_bindgen(js_name = callCount)]
    fn call_count() -> u32;
    fn sent(index: u32) -> web_sys::Request;
    fn header(index: u32, name: &str) -> Option<String>;
    #[wasm_bindgen(js_name = bodyText)]
    fn body_text(index: u32) -> js_sys::Promise;
}

fn prepare(list: Vec<JsValue>, renew: Option<&str>) {
    setup(list.into_iter().collect(), renew.map(str::to_string));
    let hooks = hooks();
    set_helix_override(hooks.get(0).into(), hooks.get(1).into());
}

fn field(value: &JsValue, name: &str) -> JsValue {
    js_sys::Reflect::get(value, &name.into()).unwrap_or(JsValue::UNDEFINED)
}

fn json(value: &JsValue) -> String {
    js_sys::JSON::stringify(value).ok().and_then(|s| s.as_string()).unwrap_or_default()
}

#[wasm_bindgen_test]
async fn baut_adresse_und_kopfzeilen_und_liefert_json() {
    prepare(vec![response(200, Some(r#"{"data":[{"id":"1"}]}"#.into()), None)], None);
    let query = vec![("login".to_string(), vec!["a".to_string(), "b".to_string()]), ("first".to_string(), vec!["5".to_string()])];
    let result = helix_call("/users", "GET", &query, None).await.unwrap();
    assert_eq!(json(&result), r#"{"data":[{"id":"1"}]}"#);
    let request = sent(0);
    assert_eq!(request.url(), "https://api.twitch.tv/helix/users?login=a&login=b&first=5");
    assert_eq!(request.method(), "GET");
    assert_eq!(header(0, "Authorization").as_deref(), Some("Bearer alt"));
    assert_eq!(header(0, "Client-Id").as_deref(), Some("client"));
    assert_eq!(header(0, "Content-Type"), None);
}

#[wasm_bindgen_test]
async fn schickt_inhalt_als_json_und_kennt_204() {
    prepare(vec![response(204, None, None)], None);
    let body = JsValue::from_str(r#"{"data":{"user_id":"5"}}"#);
    let result = helix_call("/moderation/bans", "POST", &[], Some(body)).await.unwrap();
    assert!(result.is_undefined());
    assert_eq!(sent(0).method(), "POST");
    assert_eq!(header(0, "Content-Type").as_deref(), Some("application/json"));
    let text = JsFuture::from(body_text(0)).await.unwrap();
    assert_eq!(text.as_string().as_deref(), Some(r#"{"data":{"user_id":"5"}}"#));
}

#[wasm_bindgen_test]
async fn fehler_mit_meldung_von_twitch_ohne_token() {
    prepare(vec![response(400, Some(r#"{"message":"invalid broadcaster"}"#.into()), None)], None);
    let error = helix_call("/x", "GET", &[], None).await.expect_err("Fehler erwartet");
    assert_eq!(field(&error, "helixStatus").as_f64(), Some(400.0));
    let message = field(&error, "message").as_string().unwrap_or_default();
    assert_eq!(message, "invalid broadcaster");
    assert!(!message.contains("alt"));
}

#[wasm_bindgen_test]
async fn verlaengert_bei_401_einmal_und_wiederholt_mit_neuem_token() {
    prepare(vec![response(401, Some("{}".into()), None), response(200, Some(r#"{"ok":1}"#.into()), None)], Some("neu"));
    let result = helix_call("/x", "GET", &[], None).await.unwrap();
    assert_eq!(json(&result), r#"{"ok":1}"#);
    assert_eq!(renew_count(), 1);
    assert_eq!(header(1, "Authorization").as_deref(), Some("Bearer neu"));
}

#[wasm_bindgen_test]
async fn ohne_verlaengerung_bleibt_401_ein_fehler() {
    prepare(vec![response(401, Some("{}".into()), None)], None);
    let error = helix_call("/x", "GET", &[], None).await.expect_err("Fehler erwartet");
    assert_eq!(field(&error, "helixStatus").as_f64(), Some(401.0));
    assert_eq!(field(&error, "message").as_string().as_deref(), Some("Twitch-Fehler 401"));
}

#[wasm_bindgen_test]
async fn wartet_bei_429_bis_zum_reset_und_versucht_einmal_neu() {
    let reset = ((js_sys::Date::now() / 1000.0).floor() as u64).to_string();
    prepare(vec![response(429, Some("{}".into()), Some(reset)), response(200, Some(r#"{"ok":2}"#.into()), None)], None);
    let result = helix_call("/x", "GET", &[], None).await.unwrap();
    assert_eq!(json(&result), r#"{"ok":2}"#);
    assert_eq!(call_count(), 2);
}
