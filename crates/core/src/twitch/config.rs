//! Feste Angaben zu den eigenen Twitch-Apps und die angefragten Berechtigungen (bisher src/platforms/twitch/config.ts
//! und scopes.ts). Die Client-IDs sind nicht geheim. Ein Client-Secret gehört nie in dieses Projekt.

/// App mit Client-Typ „Confidential“ für die Anmeldung mit Rücksprung (Implicit Grant).
pub const CLIENT_ID: &str = "gp28zvw295lsbrz9q2mzpsjpnbu26l";

/// Muss exakt der in der Entwicklerkonsole eingetragenen Rücksprungadresse entsprechen.
pub const REDIRECT_URI: &str = "http://localhost:8475";

/// Zweite App mit Client-Typ „Public“ für die Anmeldung per Code (Device Code Flow). Public, weil Twitch das Verlängern
/// der Anmeldung sonst nur mit Client-Secret erlaubt. Leer: Anmeldung per Code nicht verfügbar, es gilt die Anmeldung
/// mit Rücksprung.
pub const DEVICE_CLIENT_ID: &str = "4at7xm9zoe26z8awl9a2wlmo0dpase";

/// Berechtigungen, die MultiYap bei der Anmeldung anfragt. Namen am 15.11.2026 in der offiziellen Twitch-Doku („Twitch
/// Access Token Scopes“) geprüft. Neue Berechtigungen erfordern eine neue Anmeldung; deshalb werden alle für nötigen
/// Berechtigungen auf einmal angefragt.
pub const SCOPES: [&str; 22] = [
    // Chat lesen (EventSub channel.chat.message) und schreiben (Send Chat Message)
    "user:read:chat",
    "user:write:chat",
    // Eigene Emotes (Get User Emotes)
    "user:read:emotes",
    // Mod-Status-Erkennung (Get Moderated Channels)
    "user:read:moderated_channels",
    // Ban, Timeout, Unban
    "moderator:manage:banned_users",
    // Nachrichten löschen, Chat leeren, angepinnte Nachrichten
    "moderator:manage:chat_messages",
    // Verwarnen
    "moderator:manage:warnings",
    // Chat-Modi
    "moderator:manage:chat_settings",
    // Folgedatum in der Nutzerkarte
    "moderator:read:followers",
    // AutoMod: zurückgehaltene Nachrichten, Zulassen und Ablehnen
    "moderator:manage:automod",
    // Chatter-Liste
    "moderator:read:chatters",
    // Eingeschränkte und überwachte Nutzer: lesen und ändern
    "moderator:read:suspicious_users",
    "moderator:manage:suspicious_users",
    // /announce und /shoutout
    "moderator:manage:announcements",
    "moderator:manage:shoutouts",
    // Mod-Aktionen im Chat (EventSub channel.moderate): restliche Lese-Berechtigungen
    "moderator:read:blocked_terms",
    "moderator:read:moderators",
    "moderator:read:unban_requests",
    "moderator:read:vips",
    // Erste Nachricht: zusätzliche, nur lesende IRC-Verbindung
    "chat:read",
    // /mod, /unmod, /vip, /unvip (Doku geprüft: nur als Broadcaster)
    "channel:manage:moderators",
    "channel:manage:vips",
];

/// Alle Berechtigungen als Liste.
pub fn scopes() -> Vec<String> {
    SCOPES.iter().map(|s| s.to_string()).collect()
}

/// Berechtigungen, die einem Token fehlen.
pub fn missing_scopes(granted: &[String]) -> Vec<String> {
    SCOPES.iter().filter(|scope| !granted.iter().any(|g| g == *scope)).map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_scopes_lists_only_absent_ones() {
        let granted = vec!["user:read:chat".to_string(), "chat:read".to_string()];
        let missing = missing_scopes(&granted);
        assert_eq!(missing.len(), SCOPES.len() - 2);
        assert!(!missing.contains(&"chat:read".to_string()));
        assert!(missing_scopes(&scopes()).is_empty());
    }
}
