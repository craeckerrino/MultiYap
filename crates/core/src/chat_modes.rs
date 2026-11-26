//! Chat-Modi eines Kanals (bisher src/core/chat/chat-modes.ts), plattformunabhängig.

use crate::i18n::tr;
use serde::Deserialize;

/// Aktive Chat-Modi (Felder wie in der Sitzung).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatModes {
    pub slow: bool,
    pub slow_seconds: Option<u32>,
    pub emote_only: bool,
    pub follower_only: bool,
    /// Mindest-Folgedauer in Minuten (0 = alle Follower).
    pub follower_minutes: Option<u32>,
    pub subscriber_only: bool,
    pub unique_chat: bool,
    /// Chat-Verzögerung für Nicht-Mods (nur mit Mod-Rechten bekannt).
    pub delay: Option<bool>,
    pub delay_seconds: Option<u32>,
}

/// Auswahl für Follower-Only in Minuten: bis 3 Monate (90 Tage).
pub fn follower_options() -> [(u32, &'static str); 8] {
    [(0, tr("0 Min", "0 min")), (10, tr("10 Min", "10 min")), (30, tr("30 Min", "30 min")), (60, tr("1 Std", "1 hr")), (1_440, tr("1 Tag", "1 day")), (10_080, tr("1 Woche", "1 week")), (43_200, tr("1 Monat", "1 month")), (129_600, tr("3 Monate", "3 months"))]
}

/// Follower-Dauer lesbar, auch außerhalb der Auswahl.
pub fn follower_label(minutes: Option<u32>) -> String {
    let value = minutes.unwrap_or(0);
    if let Some((_, label)) = follower_options().iter().find(|(m, _)| *m == value) {
        return (*label).to_string();
    }
    if value.is_multiple_of(1_440) {
        crate::tf!("{} Tage", "{} days", value / 1_440)
    } else if value.is_multiple_of(60) {
        crate::tf!("{} Std", "{} hr", value / 60)
    } else {
        crate::tf!("{value} Min", "{value} min")
    }
}

fn seconds(value: Option<u32>) -> String {
    value.map_or_else(|| "?".to_string(), |s| s.to_string())
}

/// Plaketten der aktiven Modi, zum Beispiel „Slow 30s“ (in jedem Kanal sichtbar, auch ohne Mod-Rechte).
pub fn active_mode_badges(modes: &ChatModes) -> Vec<String> {
    let mut badges = Vec::new();
    if modes.slow {
        badges.push(format!("Slow {}s", seconds(modes.slow_seconds)));
    }
    if modes.emote_only {
        badges.push("Emote-Only".to_string());
    }
    if modes.follower_only {
        badges.push(match modes.follower_minutes {
            Some(minutes) if minutes > 0 => format!("Follower {}", follower_label(Some(minutes))),
            _ => "Follower-Only".to_string(),
        });
    }
    if modes.subscriber_only {
        badges.push("Sub-Only".to_string());
    }
    if modes.unique_chat {
        badges.push("Unique-Chat".to_string());
    }
    if modes.delay == Some(true) {
        badges.push(crate::tf!("Verzögerung {}s", "Delay {}s", seconds(modes.delay_seconds)));
    }
    badges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plaketten_fuer_aktive_modi() {
        let off = ChatModes::default();
        assert!(active_mode_badges(&off).is_empty());
        let modes = ChatModes { slow: true, slow_seconds: Some(30), follower_only: true, follower_minutes: Some(10), subscriber_only: true, ..off.clone() };
        assert_eq!(active_mode_badges(&modes), ["Slow 30s", "Follower 10 Min", "Sub-Only"]);
        let modes = ChatModes { follower_only: true, follower_minutes: Some(0), emote_only: true, ..off.clone() };
        assert_eq!(active_mode_badges(&modes), ["Emote-Only", "Follower-Only"]);
        let modes = ChatModes { unique_chat: true, delay: Some(true), delay_seconds: Some(2), ..off };
        assert_eq!(active_mode_badges(&modes), ["Unique-Chat", "Verzögerung 2s"]);
    }

    #[test]
    fn follower_dauern_lesbar() {
        assert_eq!(follower_label(Some(129_600)), "3 Monate");
        assert_eq!(follower_label(Some(43_200)), "1 Monat");
        assert_eq!(follower_label(Some(2_880)), "2 Tage");
        assert_eq!(follower_label(Some(120)), "2 Std");
        assert_eq!(follower_label(Some(45)), "45 Min");
    }

    #[test]
    fn liest_den_stand_der_sitzung() {
        let modes: ChatModes = serde_json::from_value(serde_json::json!({ "slow": true, "slowSeconds": 10, "emoteOnly": false, "followerMinutes": null })).unwrap();
        assert!(modes.slow);
        assert_eq!(modes.slow_seconds, Some(10));
        assert_eq!(modes.delay, None);
    }
}
