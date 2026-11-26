//! Abzeichen eines Kanals und die globalen (Helix „Get Chat Badges“), einmal je Kanal gebaut.

use std::collections::HashMap;

use crate::model::Badge;

/// Eine Version eines Abzeichens mit Bild und Titel.
#[derive(Debug, Clone, Default)]
pub struct BadgeVersion {
    pub title: String,
    pub image_url_1x: String,
    pub image_url_2x: String,
}

/// Kanal-Abzeichen gehen vor globalen.
#[derive(Debug, Default)]
pub struct BadgeStore {
    channel: HashMap<(String, String), BadgeVersion>,
    global: HashMap<(String, String), BadgeVersion>,
}

/// Rohes Abzeichen einer Nachricht: Art, Version, Zusatz (zum Beispiel genaue Abo-Monate).
pub struct RawBadge<'a> {
    pub set_id: &'a str,
    pub id: &'a str,
    pub info: &'a str,
}

impl BadgeStore {
    pub fn new(
        channel: impl IntoIterator<Item = (String, String, BadgeVersion)>,
        global: impl IntoIterator<Item = (String, String, BadgeVersion)>,
    ) -> Self {
        let collect = |items: Box<dyn Iterator<Item = (String, String, BadgeVersion)>>| {
            items.map(|(set, id, version)| ((set, id), version)).collect()
        };
        Self { channel: collect(Box::new(channel.into_iter())), global: collect(Box::new(global.into_iter())) }
    }

    pub fn resolve<'a>(&self, raw: impl IntoIterator<Item = RawBadge<'a>>) -> Vec<Badge> {
        raw.into_iter()
            .map(|badge| {
                let key = (badge.set_id.to_string(), badge.id.to_string());
                let version = self.channel.get(&key).or_else(|| self.global.get(&key));
                Badge {
                    source: "twitch".into(),
                    set_id: badge.set_id.into(),
                    version: badge.id.into(),
                    info: (!badge.info.is_empty()).then(|| badge.info.to_string()),
                    title: version.map(|v| v.title.clone()),
                    image_url: version.map(|v| v.image_url_1x.clone()),
                    image_url2x: version.map(|v| v.image_url_2x.clone()),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(title: &str) -> BadgeVersion {
        BadgeVersion { title: title.into(), image_url_1x: format!("{title}-1"), image_url_2x: format!("{title}-2") }
    }

    #[test]
    fn kanal_vor_global_unbekannt_ohne_bild() {
        let store = BadgeStore::new(
            [("subscriber".into(), "12".into(), version("Kanal-Abo"))],
            [("subscriber".into(), "12".into(), version("Global")), ("moderator".into(), "1".into(), version("Moderator"))],
        );
        let badges = store.resolve([
            RawBadge { set_id: "subscriber", id: "12", info: "14" },
            RawBadge { set_id: "moderator", id: "1", info: "" },
            RawBadge { set_id: "neu", id: "1", info: "" },
        ]);
        assert_eq!(badges[0].title.as_deref(), Some("Kanal-Abo"));
        assert_eq!(badges[0].info.as_deref(), Some("14"));
        assert_eq!(badges[1].image_url2x.as_deref(), Some("Moderator-2"));
        assert_eq!(badges[1].info, None);
        assert_eq!(badges[2].title, None);
        assert_eq!(badges[2].image_url, None);
    }
}
