//! Zentrale Übersicht aller Module (bisher src/core/modules/catalog.ts). Jedes Modul beschreibt sich selbst und meldet
//! seine Fähigkeiten. Die Oberfläche zeigt nur, was ein Modul kann, und graut Module mit Status „Coming Soon“ aus. Neue
//! Module werden nur hier angemeldet; die Reihenfolge ist die in der Oberfläche.

use crate::i18n::tr;
/// Benutzbar oder sichtbar, ausgegraut mit Hinweis „Coming Soon“.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleStatus {
    Available,
    ComingSoon,
}

/// Ein Modul mit Anzeigename (nur Text, keine Logos) und Fähigkeiten als Ja/Nein-Schalter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Module {
    pub id: &'static str,
    pub label: &'static str,
    pub status: ModuleStatus,
    /// Beschreibung im „+“-Dialog (nur Tab-Arten).
    pub description: &'static str,
    /// Fähigkeiten, die das Modul bietet (fehlende bietet es nicht).
    pub capabilities: &'static [&'static str],
    /// Plattformen, für die eine Quelle gilt (nur Quellen).
    pub platforms: &'static [&'static str],
}

impl Module {
    pub fn available(&self) -> bool {
        self.status == ModuleStatus::Available
    }

    /// Bietet das Modul die Fähigkeit tatsächlich? Module mit Status „Coming Soon“ bieten nie etwas an.
    pub fn supports(&self, capability: &str) -> bool {
        self.available() && self.capabilities.contains(&capability)
    }
}

const fn module(id: &'static str, label: &'static str, status: ModuleStatus) -> Module {
    Module { id, label, status, description: "", capabilities: &[], platforms: &[] }
}

/// Plattformen. Fähigkeiten nach.
pub const PLATFORMS: [Module; 2] = [
    Module {
        capabilities: &[
            "readChat",
            "writeChat",
            "liveStatus",
            "sharedChat",
            "player",
            "deleteMessage",
            "warn",
            "timeout",
            "ban",
            "unban",
            "clearChat",
            "chatModes",
            "automod",
            "suspiciousUsers",
            "pins",
            "announcements",
            "shoutouts",
        ],
        ..module("twitch", "Twitch", ModuleStatus::Available)
    },
    // YouTube: zuerst nur lesen, über den MultiYap-Server. Schreiben und Moderation später. Verwarnen gibt es nur bei
    // Twitch.
    Module { capabilities: &["readChat", "liveStatus"], ..module("youtube", "YouTube", ModuleStatus::Available) },
];

/// Emote-Dienste. Badges nur von 7TV, nicht von BTTV und FFZ.
pub const EMOTE_SERVICES: [Module; 4] = [
    Module { capabilities: &["globalEmotes", "channelEmotes", "userEmotes", "animated"], ..module("twitch", "Twitch", ModuleStatus::Available) },
    Module {
        capabilities: &["globalEmotes", "channelEmotes", "animated", "overlays", "badges", "paints"],
        ..module("seventv", "7TV", ModuleStatus::Available)
    },
    Module { capabilities: &["globalEmotes", "channelEmotes", "animated"], ..module("bttv", "BTTV", ModuleStatus::Available) },
    Module { capabilities: &["globalEmotes", "channelEmotes", "animated"], ..module("ffz", "FFZ", ModuleStatus::Available) },
];

/// Verlaufsquellen.
pub const HISTORY_SOURCES: [Module; 1] = [Module { platforms: &["twitch"], ..module("recent-messages", "recent-messages", ModuleStatus::Available) }];

/// Externe Quellen. Lurkology Firehose nur nach erfolgreicher Machbarkeitsprüfung.
pub const EXTERNAL_SOURCES: [Module; 1] = [Module { platforms: &["twitch"], ..module("lurkology", "Lurkology Firehose", ModuleStatus::ComingSoon) }];

/// Tab-Arten in der Reihenfolge des „+“-Dialogs.
pub fn tab_kinds() -> [Module; 4] {
    [
    Module { description: tr("Chat eines Kanals lesen, schreiben und moderieren.", "Read, write and moderate a channel’s chat."), ..module("channel", tr("Kanal", "Channel"), ModuleStatus::Available) },
    Module {
        description: tr("Filter, Erwähnungen, Ereignisse, Mod-Aktionen und AutoMod aus allen offenen Kanälen.", "Filters, mentions, events, mod actions and AutoMod from all open channels."),
        ..module("highlights", tr("Hervorhebungen", "Highlights"), ModuleStatus::Available)
    },
    Module { description: tr("Twitch-Player eines Kanals.", "A channel’s Twitch player."), ..module("stream", "Stream", ModuleStatus::Available) },
    Module { description: tr("Nachrichten bestimmter Nutzer aus allen Kanälen.", "Messages from specific users across all channels."), ..module("firehose", "Firehose", ModuleStatus::ComingSoon) },
]
}

/// Modul nach ID aus einer Liste.
pub fn find<'a>(list: &'a [Module], id: &str) -> Option<&'a Module> {
    list.iter().find(|m| m.id == id)
}

/// Anzeigename einer Plattform (die ID, wenn sie unbekannt ist).
pub fn platform_label(id: &str) -> &str {
    find(&PLATFORMS, id).map_or(id, |m| m.label)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[Module]) -> Vec<&str> {
        list.iter().map(|m| m.id).collect()
    }

    #[test]
    fn ids_sind_je_liste_eindeutig() {
        for list in [&PLATFORMS[..], &EMOTE_SERVICES, &HISTORY_SOURCES, &EXTERNAL_SOURCES, &tab_kinds()] {
            let mut seen = ids(list);
            seen.sort_unstable();
            seen.dedup();
            assert_eq!(seen.len(), list.len());
        }
    }

    #[test]
    fn faehigkeiten_nur_fuer_verfuegbare_module() {
        let twitch = find(&PLATFORMS, "twitch").unwrap();
        assert!(twitch.supports("warn"));
        let youtube = find(&PLATFORMS, "youtube").unwrap();
        assert!(youtube.supports("readChat"));
        assert!(!youtube.supports("writeChat"));
        assert!(!Module { status: ModuleStatus::ComingSoon, ..*youtube }.supports("readChat"));
    }

    #[test]
    fn verwarnen_nur_bei_twitch() {
        let with_warn: Vec<&str> = PLATFORMS.iter().filter(|p| p.capabilities.contains(&"warn")).map(|p| p.id).collect();
        assert_eq!(with_warn, ["twitch"]);
    }

    #[test]
    fn badges_nur_von_7tv() {
        let with_badges: Vec<&str> = EMOTE_SERVICES.iter().filter(|s| s.capabilities.contains(&"badges")).map(|s| s.id).collect();
        assert_eq!(with_badges, ["seventv"]);
    }

    #[test]
    fn katalog_in_richtiger_reihenfolge() {
        assert_eq!(PLATFORMS.iter().map(|m| m.label).collect::<Vec<_>>(), ["Twitch", "YouTube"]);
        assert_eq!(ids(&EMOTE_SERVICES), ["twitch", "seventv", "bttv", "ffz"]);
        assert_eq!(ids(&HISTORY_SOURCES), ["recent-messages"]);
        assert_eq!(ids(&EXTERNAL_SOURCES), ["lurkology"]);
        assert_eq!(ids(&tab_kinds()), ["channel", "highlights", "stream", "firehose"]);
        assert_eq!(find(&PLATFORMS, "youtube").unwrap().status, ModuleStatus::Available);
        assert_eq!(platform_label("twitch"), "Twitch");
        assert_eq!(platform_label("unbekannt"), "unbekannt");
    }
}
