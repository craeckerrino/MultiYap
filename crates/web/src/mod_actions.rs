//! Ausführung der Mod-Aktionen, „/“-Befehle und Mod-Tastenkürzel (bisher src/features/moderation/execute.ts,
//! suspicious-actions.ts, shortcuts.ts und src/features/commands/execute-command.ts). Keine Sicherheitsabfrage, keine
//! automatischen Aktionen. Moderatives prüft vorher die zentrale Mod-Prüfung; ohne Rechte passiert nichts. Fehler
//! werden kurz angezeigt, der Zustand bleibt unverändert. Befehle werden nie als Text in den Chat geschickt. Zerlegen,
//! Rollen und Dauern der Befehle in crates/core/src/commands.rs.

use multiyap_core::i18n::tr;
use std::cell::RefCell;

use multiyap_core::commands::{self as cmd, OwnRoles};
use multiyap_core::hotkeys;
use multiyap_core::twitch::helix_api::MAX_TIMEOUT_SECONDS;
use js_sys::Reflect;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

use crate::chat_tools::toast;
use crate::moderation::{self as md, SuspiciousInfo};
use crate::{api, auth, sessions};

fn no_mod_rights() -> &'static str {
    tr("Keine Mod-Rechte in diesem Kanal.", "No mod rights in this channel.")
}
/// Standard-Dauer für /timeout ohne Angabe (wie bei Twitch).
const DEFAULT_TIMEOUT_SECONDS: u64 = 600;
const DEFAULT_SLOW_SECONDS: u64 = 30;
const MAX_FOLLOWER_MINUTES: u64 = 129_600;

/// Ziel einer Mod-Aktion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModTarget {
    pub channel_id: String,
    pub user_id: String,
    pub display_name: String,
    /// Nur für „Löschen“.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
}

fn helix_status(error: &JsValue) -> Option<u16> {
    Reflect::get(error, &"helixStatus".into()).ok().and_then(|s| s.as_f64()).map(|s| s as u16)
}

fn message_of(error: &JsValue) -> Option<String> {
    Reflect::get(error, &"message".into()).ok().and_then(|m| m.as_string())
}

fn describe(action: &str, duration_seconds: Option<f64>, name: &str) -> String {
    match action {
        "delete" => multiyap_core::tf!("Nachricht von {name} gelöscht", "Message from {name} deleted"),
        "warn" => multiyap_core::tf!("{name} verwarnt", "{name} warned"),
        "timeout" => multiyap_core::tf!("Timeout {} für {name}", "Timeout {} for {name}", hotkeys::format_duration(duration_seconds.unwrap_or_default() as i64)),
        "ban" => multiyap_core::tf!("{name} gebannt", "{name} banned"),
        _ => multiyap_core::tf!("Ban/Timeout von {name} aufgehoben", "Ban/timeout of {name} lifted"),
    }
}

fn mod_error_text(error: &JsValue) -> String {
    match helix_status(error) {
        Some(403) => no_mod_rights().into(),
        Some(409) => tr("Twitch bearbeitet gerade eine andere Aktion für diese Person. Bitte erneut versuchen.", "Twitch is currently handling another action for this person. Please try again.").into(),
        Some(429) => tr("Zu viele Aktionen in kurzer Zeit. Bitte kurz warten.", "Too many actions in a short time. Please wait a moment.").into(),
        Some(_) => message_of(error).unwrap_or_default(),
        None => tr("Unbekannter Fehler", "Unknown error").into(),
    }
}

/// Führt eine Mod-Aktion aus ("delete", "warn", "timeout", "ban", "unban"). Prüft vorher zentral die Mod-Rechte.
pub async fn execute_mod_action(target: ModTarget, action: &str, duration_seconds: Option<f64>, reason: Option<String>) -> bool {
    if !md::can_moderate(&target.channel_id) {
        toast("error", no_mod_rights());
        return false;
    }
    let reason = reason.filter(|r| !r.trim().is_empty());
    let result = match action {
        "delete" => match &target.message_id {
            Some(message_id) => api::delete_message(&target.channel_id, message_id).await,
            None => return false,
        },
        "warn" => match &reason {
            Some(reason) => api::warn_user(&target.channel_id, &target.user_id, reason).await,
            None => {
                toast("error", tr("Verwarnen braucht einen Grund.", "Warning needs a reason."));
                return false;
            }
        },
        "timeout" => api::ban_user(&target.channel_id, &target.user_id, duration_seconds, reason.as_deref()).await,
        "ban" => api::ban_user(&target.channel_id, &target.user_id, None, reason.as_deref()).await,
        "unban" => api::unban_user(&target.channel_id, &target.user_id).await,
        _ => return false,
    };
    let text = describe(action, duration_seconds, &target.display_name);
    match result {
        Ok(()) => {
            toast("success", &text);
            true
        }
        Err(error) => {
            toast("error", &multiyap_core::tf!("{text}: fehlgeschlagen. {}", "{text}: failed. {}", mod_error_text(&error)));
            false
        }
    }
}

/// Überwachen ("monitored"), Einschränken ("restricted") oder Status entfernen (None). Der bekannte Status wird aus der
/// Antwort von Twitch übernommen.
pub async fn change_suspicious(target: ModTarget, status: Option<String>) -> bool {
    use multiyap_core::twitch::suspicious as ts;
    if !md::can_moderate(&target.channel_id) {
        toast("error", no_mod_rights());
        return false;
    }
    let error_text = |error: &JsValue| match helix_status(error) {
        Some(400) => tr("Für diese Person nicht möglich (zum Beispiel Mod, Broadcaster oder Status schon gesetzt).", "Not possible for this person (for example mod, broadcaster or status already set).").to_string(),
        Some(401) => tr("Dem Token fehlt die Berechtigung. Bitte einmal neu anmelden.", "The token is missing the permission. Please sign in again once.").to_string(),
        Some(403) => no_mod_rights().to_string(),
        Some(429) => tr("Zu viele Aktionen in kurzer Zeit. Bitte kurz warten.", "Too many actions in a short time. Please wait a moment.").to_string(),
        Some(_) => message_of(error).unwrap_or_default(),
        None => tr("Unbekannter Fehler", "Unknown error").to_string(),
    };
    match status {
        None => match api::remove_suspicious_status(&target.channel_id, &target.user_id).await {
            Ok(()) => {
                md::set_known_suspicious(&target.channel_id, &target.user_id, None);
                toast("success", &multiyap_core::tf!("Status von {} entfernt", "Status of {} removed", target.display_name));
                true
            }
            Err(error) => {
                toast("error", &error_text(&error));
                false
            }
        },
        Some(status) => {
            let helix_value = if status == "restricted" { "RESTRICTED" } else { "ACTIVE_MONITORING" };
            match api::set_suspicious_status(&target.channel_id, &target.user_id, helix_value).await {
                Ok(response) => {
                    let confirmed = response
                        .as_ref()
                        .and_then(|r| ts::status_from_helix(r.get("status").and_then(Value::as_str)))
                        .map_or(status.clone(), str::to_string);
                    let reasons = match &response {
                        Some(r) => {
                            let types: Vec<String> = r.get("types").and_then(Value::as_array).into_iter().flatten().filter_map(|t| t.as_str().map(str::to_string)).collect();
                            ts::reasons_from_helix(&types)
                        }
                        None => md::known_suspicious(&target.channel_id, &target.user_id).map(|i| i.reasons).unwrap_or_default(),
                    };
                    let label = if confirmed == "restricted" { tr("Eingeschränkt", "Restricted") } else { tr("Überwacht", "Monitored") };
                    md::set_known_suspicious(&target.channel_id, &target.user_id, Some(SuspiciousInfo { status: confirmed, reasons }));
                    toast("success", &format!("{}: {label}", target.display_name));
                    true
                }
                Err(error) => {
                    toast("error", &error_text(&error));
                    false
                }
            }
        }
    }
}

// ---- „/“-Befehle ----

fn fail(message: &str) -> bool {
    toast("error", message);
    false
}

fn command_error_text(error: &JsValue) -> String {
    match helix_status(error) {
        Some(403) => no_mod_rights().into(),
        Some(429) => tr("Zu viele Aktionen in kurzer Zeit. Bitte kurz warten.", "Too many actions in a short time. Please wait a moment.").into(),
        _ => message_of(error).filter(|m| !m.is_empty()).unwrap_or_else(|| tr("Unbekannter Fehler", "Unknown error").into()),
    }
}

/// Verständliche Meldungen zu den Fehlercodes laut Twitch-Doku (Mods und VIPs).
fn role_error_text(command: &str, error: &JsValue) -> String {
    let message = message_of(error).unwrap_or_default();
    match (command, helix_status(error)) {
        (_, Some(401)) if message.to_lowercase().contains("missing scope") => tr("Dem Token fehlt die Berechtigung. Bitte einmal neu anmelden.", "The token is missing the permission. Please sign in again once.").into(),
        (_, Some(401)) => multiyap_core::tf!("Twitch hat abgelehnt ({message}).", "Twitch declined ({message})."),
        (_, Some(429)) => tr("Zu viele Änderungen in kurzer Zeit (höchstens 10 in 10 Sekunden).", "Too many changes in a short time (at most 10 in 10 seconds).").into(),
        ("mod", Some(422)) => tr("Geht nicht: Die Person ist VIP (zuerst /unvip) oder die Höchstzahl an Mods ist erreicht.", "Not possible: the person is a VIP (use /unvip first) or the maximum number of mods is reached.").into(),
        ("vip", Some(422)) => tr("Geht nicht: Die Person ist Mod (zuerst /unmod).", "Not possible: the person is a mod (use /unmod first).").into(),
        ("vip", Some(409)) => tr("Keine freien VIP-Plätze.", "No free VIP slots.").into(),
        ("vip", Some(425)) => tr("VIPs gibt es erst nach dem Twitch-Erfolg „Build a Community“.", "VIPs are only available after the Twitch achievement “Build a Community”.").into(),
        ("mod", Some(400)) => tr("Geht nicht: Die Person ist schon Mod oder gebannt.", "Not possible: the person is already a mod or banned.").into(),
        ("unmod", Some(400)) => tr("Die Person ist kein Mod.", "The person is not a mod.").into(),
        ("vip", Some(400)) => tr("Geht nicht: Die Person ist schon VIP oder gebannt.", "Not possible: the person is already a VIP or banned.").into(),
        (_, Some(400)) => tr("Die Person ist kein VIP.", "The person is not a VIP.").into(),
        _ => command_error_text(error),
    }
}

/// Nutzer zu einem Namensargument: (ID, Anzeigename). None: ungültig oder nicht gefunden.
async fn resolve_user(argument: Option<&str>) -> Result<Option<(String, String)>, JsValue> {
    let Some(login) = cmd::login_from_argument(argument) else { return Ok(None) };
    let user = api::get_user_by_login(&login).await?;
    Ok(user.map(|u| {
        let text = |name: &str| u.get(name).and_then(Value::as_str).unwrap_or_default().to_string();
        (text("id"), text("display_name"))
    }))
}

/// Chat-Modus über die Sitzung des Kanals ändern (ohne Sitzung passiert nichts).
async fn update_modes(channel_login: Option<&str>, patch: Value) -> Result<(), String> {
    match channel_login.and_then(sessions::session) {
        Some(session) => session.update_chat_modes(&patch).await,
        None => Ok(()),
    }
}

/// Führt einen „/“-Befehl aus. true: ausgeführt (dann wird das Eingabefeld geleert).
pub async fn execute_command(text: &str, channel_id: Option<String>, channel_login: Option<String>) -> bool {
    let Some(parsed) = cmd::parse_command_line(text) else { return false };
    let Some(command) = cmd::find_command(&parsed.name) else {
        return fail(&multiyap_core::tf!("Unbekannter Befehl „/{}“. Nur echte Twitch-Befehle werden unterstützt.", "Unknown command “/{}”. Only real Twitch commands are supported.", parsed.name));
    };
    if command.coming_soon {
        return fail(&multiyap_core::tf!("„/{}“ kommt später (Coming Soon).", "“/{}” comes later (Coming Soon).", command.name));
    }
    let Some(channel_id) = channel_id else { return fail(tr("Kanal noch nicht verbunden.", "Channel not connected yet.")) };
    if command.needs_mod && !md::can_moderate(&channel_id) {
        return fail(tr("Mod-Befehle gehen nur in Kanälen, in denen du Mod bist.", "Mod commands only work in channels where you are a mod."));
    }
    let roles = OwnRoles {
        moderator: md::can_moderate(&channel_id),
        broadcaster: md::is_broadcaster(&channel_id),
        vip: channel_login.as_deref().is_some_and(md::is_own_vip),
    };
    let by_role = cmd::allowed_role(command, roles);
    // Ohne Broadcaster-Rechte nur für sich selbst (Mods und VIPs dürfen sich laut Twitch selbst entfernen).
    let only_self = !by_role && cmd::allowed_for_self(command, roles);
    if !by_role && !only_self {
        return fail(&multiyap_core::tf!("„/{}“ geht nur im eigenen Kanal.", "“/{}” only works in your own channel.", command.name));
    }
    let arg = |index: usize| parsed.args.get(index).map(String::as_str);
    let result: Result<bool, JsValue> = async {
        Ok(match command.name {
            "mod" | "unmod" | "vip" | "unvip" => {
                let Some((user_id, name)) = resolve_user(arg(0)).await? else {
                    return Ok(fail(&multiyap_core::tf!("Nutzer „{}“ nicht gefunden.", "User “{}” not found.", arg(0).unwrap_or_default())));
                };
                if only_self && auth::own_user_id().as_deref() != Some(user_id.as_str()) {
                    return Ok(fail(&multiyap_core::tf!("Hier geht „/{}“ nur für dich selbst.", "Here “/{}” only works for yourself.", command.name)));
                }
                let (path, method) = match command.name {
                    "mod" => ("/moderation/moderators", "POST"),
                    "unmod" => ("/moderation/moderators", "DELETE"),
                    "vip" => ("/channels/vips", "POST"),
                    _ => ("/channels/vips", "DELETE"),
                };
                if let Err(error) = api::change_role(path, method, &channel_id, &user_id).await {
                    return Ok(fail(&role_error_text(command.name, &error)));
                }
                let done = match command.name {
                    "mod" => multiyap_core::tf!("{name} ist jetzt Mod", "{name} is now a mod"),
                    "unmod" => multiyap_core::tf!("{name} ist kein Mod mehr", "{name} is no longer a mod"),
                    "vip" => multiyap_core::tf!("{name} ist jetzt VIP", "{name} is now a VIP"),
                    _ => multiyap_core::tf!("{name} ist kein VIP mehr", "{name} is no longer a VIP"),
                };
                toast("success", &done);
                true
            }
            "ban" | "unban" | "untimeout" | "timeout" | "warn" => {
                let Some((user_id, name)) = resolve_user(arg(0)).await? else {
                    return Ok(fail(&multiyap_core::tf!("Nutzer „{}“ nicht gefunden.", "User “{}” not found.", arg(0).unwrap_or_default())));
                };
                let target = ModTarget { channel_id: channel_id.clone(), user_id, display_name: name, message_id: None };
                let reason_from = |start: usize| parsed.args.get(start..).map(|r| r.join(" ")).unwrap_or_default();
                match command.name {
                    "ban" => execute_mod_action(target, "ban", None, Some(reason_from(1))).await,
                    "unban" | "untimeout" => execute_mod_action(target, "unban", None, None).await,
                    "warn" => {
                        let reason = reason_from(1);
                        if reason.is_empty() {
                            return Ok(fail(tr("Verwarnen braucht einen Grund: /warn <Name> <Grund>", "Warning needs a reason: /warn <name> <reason>")));
                        }
                        execute_mod_action(target, "warn", None, Some(reason)).await
                    }
                    _ => {
                        // Timeout: Dauer optional; ist das zweite Argument keine Dauer, gehört es zum Grund.
                        let duration = arg(1).and_then(cmd::parse_duration);
                        let seconds = duration.unwrap_or(DEFAULT_TIMEOUT_SECONDS);
                        if seconds < 1 || seconds > u64::from(MAX_TIMEOUT_SECONDS) {
                            return Ok(fail(tr("Timeout geht von 1 Sekunde bis 2 Wochen.", "Timeouts go from 1 second to 2 weeks.")));
                        }
                        let reason = reason_from(if duration.is_none() { 1 } else { 2 });
                        execute_mod_action(target, "timeout", Some(seconds as f64), Some(reason)).await
                    }
                }
            }
            "clear" => {
                api::clear_chat(&channel_id).await?;
                toast("success", tr("Chat geleert", "Chat cleared"));
                true
            }
            "slow" => {
                let seconds = match arg(0) {
                    Some(value) => cmd::parse_duration(value),
                    None => Some(DEFAULT_SLOW_SECONDS),
                };
                let Some(seconds) = seconds.filter(|s| (3..=120).contains(s)) else {
                    return Ok(fail(tr("Slow-Modus geht von 3 bis 120 Sekunden.", "Slow mode goes from 3 to 120 seconds.")));
                };
                mode_result(update_modes(channel_login.as_deref(), json!({ "slow": true, "slowSeconds": seconds })).await)
            }
            "slowoff" => mode_result(update_modes(channel_login.as_deref(), json!({ "slow": false })).await),
            "followers" => {
                // Ohne Einheit zählt bei /followers die Zahl als Minuten (wie bei Twitch).
                let seconds = match arg(0) {
                    None => Some(0),
                    Some(value) if !value.is_empty() && value.chars().all(|c| c.is_ascii_digit()) => value.parse::<u64>().ok().map(|m| m * 60),
                    Some(value) => cmd::parse_duration(value),
                };
                let Some(seconds) = seconds else {
                    return Ok(fail(tr("Ungültige Dauer, zum Beispiel 10m, 1h, 1d, 1w oder 1mo.", "Invalid duration, for example 10m, 1h, 1d, 1w or 1mo.")));
                };
                let minutes = (seconds as f64 / 60.0).round() as u64;
                if minutes > MAX_FOLLOWER_MINUTES {
                    return Ok(fail(tr("Follower-Modus geht bis höchstens 3 Monate.", "Follower mode goes up to 3 months at most.")));
                }
                mode_result(update_modes(channel_login.as_deref(), json!({ "followerOnly": true, "followerMinutes": minutes })).await)
            }
            "followersoff" => mode_result(update_modes(channel_login.as_deref(), json!({ "followerOnly": false })).await),
            "subscribers" | "subscribersoff" => {
                mode_result(update_modes(channel_login.as_deref(), json!({ "subscriberOnly": command.name == "subscribers" })).await)
            }
            "emoteonly" | "emoteonlyoff" => mode_result(update_modes(channel_login.as_deref(), json!({ "emoteOnly": command.name == "emoteonly" })).await),
            "uniquechat" | "uniquechatoff" => {
                mode_result(update_modes(channel_login.as_deref(), json!({ "uniqueChat": command.name == "uniquechat" })).await)
            }
            "user" => {
                let Some(login) = cmd::login_from_argument(arg(0)) else { return Ok(fail(tr("Bitte einen Namen angeben: /user <Name>", "Please give a name: /user <name>"))) };
                let Some(channel_login) = channel_login.as_deref() else { return Ok(fail(tr("Kanal noch nicht verbunden.", "Channel not connected yet."))) };
                crate::chat_tools::open_twitch_usercard(channel_login, &login);
                true
            }
            "shoutout" => {
                let Some((user_id, name)) = resolve_user(arg(0)).await? else {
                    return Ok(fail(&multiyap_core::tf!("Kanal „{}“ nicht gefunden.", "Channel “{}” not found.", arg(0).unwrap_or_default())));
                };
                api::send_shoutout(&channel_id, &user_id).await?;
                toast("success", &multiyap_core::tf!("Shoutout für {name}", "Shoutout for {name}"));
                true
            }
            name => {
                let color = match name {
                    "announce" => "primary",
                    "announceblue" => "blue",
                    "announcegreen" => "green",
                    "announceorange" => "orange",
                    "announcepurple" => "purple",
                    _ => return Ok(fail(&multiyap_core::tf!("„/{name}“ wird noch nicht unterstützt.", "“/{name}” is not supported yet."))),
                };
                if parsed.rest.is_empty() {
                    return Ok(fail(tr("Ankündigung braucht einen Text.", "An announcement needs a text.")));
                }
                api::send_announcement(&channel_id, &parsed.rest, color).await?;
                true
            }
        })
    }
    .await;
    result.unwrap_or_else(|error| fail(&command_error_text(&error)))
}

fn mode_result(result: Result<(), String>) -> bool {
    match result {
        Ok(()) => true,
        Err(message) => fail(&message),
    }
}

// ---- Tastenkürzel für Mod-Aktionen: Maus über einer Nachricht plus Taste ----

/// Zeile unter der Maus: ihre ID, der Kanal und das Ziel wie bei den Schnellbuttons (keine eigenen Nachrichten, nicht
/// Broadcaster, Mods und Lead Mods; AutoMod-Zeilen ohne Nachricht im Chat).
#[derive(Clone, Debug)]
pub struct Hovered {
    pub item_id: String,
    pub target: Option<ModTarget>,
}

thread_local! {
    static HOVERED: RefCell<Option<Hovered>> = const { RefCell::new(None) };
    static STARTED: RefCell<bool> = const { RefCell::new(false) };
}

pub fn set_hovered(hovered: Hovered) {
    HOVERED.with(|h| *h.borrow_mut() = Some(hovered));
}

/// Nur löschen, wenn die Maus noch über genau dieser Zeile stand (Reihenfolge von Verlassen und Betreten).
pub fn clear_hovered(item_id: &str) {
    HOVERED.with(|h| {
        let mut h = h.borrow_mut();
        if h.as_ref().is_some_and(|x| x.item_id == item_id) {
            *h = None;
        }
    });
}

/// Tippen in Feldern und offene Fenster oder Menüs haben Vorrang (dort gelten eigene Tasten, zum Beispiel 1 bis 9).
fn blocked(event: &web_sys::KeyboardEvent) -> bool {
    if event.repeat() || event.ctrl_key() || event.alt_key() || event.meta_key() {
        return true;
    }
    if let Some(target) = event.target().and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok()) {
        if target.is_content_editable() || target.closest("input, textarea, select").ok().flatten().is_some() {
            return true;
        }
    }
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector("[role=\"dialog\"], [role=\"alertdialog\"], [role=\"menu\"], [role=\"listbox\"]").ok().flatten())
        .is_some()
}

/// Steht die Maus noch über der gemerkten Zeile? Rutscht der Chat unter der ruhenden Maus weiter, passt die Zeile nicht
/// mehr, und es passiert lieber nichts als eine Aktion gegen die falsche Person.
fn still_hovered(item_id: &str) -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector("[data-chat-row]:hover").ok().flatten())
        .and_then(|row| row.get_attribute("data-item-id"))
        .is_some_and(|id| id == item_id)
}

fn handle_shortcut(event: web_sys::KeyboardEvent) {
    let list = crate::settings::get("moderation.shortcutList").unwrap_or(Value::Null);
    if list.get("enabled").and_then(Value::as_bool) != Some(true) {
        return;
    }
    let Some(hovered) = HOVERED.with(|h| h.borrow().clone()) else { return };
    if blocked(&event) || !still_hovered(&hovered.item_id) {
        return;
    }
    let entries: Vec<multiyap_core::shortcuts::ShortcutEntry> =
        list.get("entries").cloned().and_then(|e| serde_json::from_value(e).ok()).unwrap_or_default();
    let Some(entry) = multiyap_core::shortcuts::entry_for_key(&entries, &event.key()) else { return };
    let Some(target) = hovered.target.filter(|t| md::can_moderate(&t.channel_id)) else { return };
    let action = entry.action.clone();
    if action == "delete" && target.message_id.is_none() {
        return;
    }
    event.prevent_default();
    let reason = entry.reason.as_deref().map(str::trim).filter(|r| !r.is_empty()).map(str::to_string);
    if action == "warn" && reason.is_none() {
        toast("error", tr("Verwarnen braucht einen Grund. Bitte beim Tastenkürzel einen Grund eintragen.", "Warning needs a reason. Please add a reason to the keyboard shortcut."));
        return;
    }
    let duration = entry.duration_seconds.map(|d| d as f64);
    let reason = if action == "delete" || action == "unban" { None } else { reason };
    spawn_local(async move {
        execute_mod_action(target, &action, duration, reason).await;
    });
}

/// Startet die Tastenkürzel (einmal).
pub fn start_shortcuts() {
    if STARTED.with(|s| std::mem::replace(&mut *s.borrow_mut(), true)) {
        return;
    }
    let Some(window) = web_sys::window() else { return };
    let listener = Closure::<dyn Fn(web_sys::KeyboardEvent)>::new(handle_shortcut).into_js_value();
    let _ = window.add_event_listener_with_callback("keydown", listener.unchecked_ref());
}

