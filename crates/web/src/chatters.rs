//! Chatter laden (bisher src/features/chatters/load.ts) für die Liste rechts neben dem Chat (crates/ui/src/chatters.rs).
//! Nur Mod-Kanäle (zentrale Prüfung in crates/web/src/moderation.rs). Nichts wird gespeichert.

use multiyap_core::i18n::tr;
use crate::api::{self, ChannelRoles};
use crate::moderation;
use crate::net::helix_user_id;
use serde_json::Value;
use wasm_bindgen::JsValue;

/// Geladene Chatter.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedChatters {
    pub chatters: Vec<Value>,
    pub total: f64,
    pub truncated: bool,
    /// Nur im eigenen Kanal: vollständige Mod- und VIP-Listen von Twitch.
    pub own_roles: Option<ChannelRoles>,
}

fn message(error: &JsValue) -> String {
    js_sys::Reflect::get(error, &"message".into()).ok().and_then(|m| m.as_string()).filter(|m| !m.is_empty()).unwrap_or_else(|| tr("Unbekannter Fehler", "Unknown error").into())
}

/// Lädt die Chatter. None ohne Mod-Rechte; Fehler als lesbare Meldung.
pub async fn load(channel_id: &str) -> Result<Option<LoadedChatters>, String> {
    if !moderation::can_moderate(channel_id) {
        return Ok(None);
    }
    let own_id = helix_user_id().ok_or_else(|| tr("Nicht angemeldet.", "Not signed in.").to_string())?;
    let own = own_id == channel_id;
    let (result, roles) = futures::join!(api::get_chatters(channel_id, &own_id), async {
        if own { api::get_own_channel_roles(channel_id).await.map(Some) } else { Ok(None) }
    });
    let result = result.map_err(|e| message(&e))?;
    let own_roles = roles.map_err(|e| message(&e))?;
    Ok(Some(LoadedChatters { chatters: result.chatters, total: result.total, truncated: result.truncated, own_roles }))
}

/// Profilbild einer Person (für die Sitzung zwischengespeichert in api.rs), None bei Fehler oder ohne Bild.
pub async fn avatar(user_id: &str) -> Option<String> {
    let user = api::get_user_by_id(user_id).await.ok().flatten()?;
    user.get("profile_image_url").and_then(Value::as_str).filter(|u| !u.is_empty()).map(str::to_string)
}
