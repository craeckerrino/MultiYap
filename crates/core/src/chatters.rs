//! Chatter-Liste nach Rolle (bisher chatters/group.ts): Broadcaster, Mods, VIPs, Zuschauer, jeweils alphabetisch, mit
//! Suchfeld. Twitch liefert die Rollen nur dem Broadcaster; in fremden Mod-Kanälen kommen sie aus dem, was MultiYap
//! seit dem Öffnen gesehen hat.

use crate::i18n::tr;
use std::cmp::Ordering;
use std::collections::HashSet;

use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Chatter {
    pub user_id: String,
    pub user_login: String,
    pub user_name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatterGroup {
    pub title: &'static str,
    pub chatters: Vec<Chatter>,
}

pub struct RoleSource<'a> {
    pub broadcaster_id: &'a str,
    pub moderators: &'a HashSet<String>,
    pub vips: &'a HashSet<String>,
}

fn titles() -> [&'static str; 4] {
    ["Broadcaster", "Mods", "VIPs", tr("Zuschauer", "Viewers")]
}

fn role_of(chatter: &Chatter, roles: &RoleSource) -> usize {
    if chatter.user_id == roles.broadcaster_id {
        0
    } else if roles.moderators.contains(&chatter.user_id) {
        1
    } else if roles.vips.contains(&chatter.user_id) {
        2
    } else {
        3
    }
}

/// Gruppen in fester Reihenfolge, leere entfallen. Suche nach Anzeigename oder Login, Groß- und Kleinschreibung egal.
/// `compare` ordnet die Namen (in der Oberfläche Intl.Collator „de“, Grundzeichen).
pub fn group_chatters(chatters: &[Chatter], roles: &RoleSource, query: &str, compare: impl Fn(&str, &str) -> Ordering) -> Vec<ChatterGroup> {
    let q = query.trim().trim_start_matches('@').to_lowercase();
    let mut groups: [Vec<Chatter>; 4] = Default::default();
    for chatter in chatters {
        if !q.is_empty() && !chatter.user_name.to_lowercase().contains(&q) && !chatter.user_login.contains(&q) {
            continue;
        }
        groups[role_of(chatter, roles)].push(chatter.clone());
    }
    groups
        .into_iter()
        .zip(titles())
        .filter(|(list, _)| !list.is_empty())
        .map(|(mut list, title)| {
            list.sort_by(|a, b| compare(&a.user_name, &b.user_name));
            ChatterGroup { title, chatters: list }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chatter(id: &str, name: &str) -> Chatter {
        Chatter { user_id: id.into(), user_login: name.to_lowercase(), user_name: name.into() }
    }

    /// Wie Intl.Collator „de“ mit Grundzeichen für die Testnamen.
    fn base(text: &str) -> String {
        text.to_lowercase().replace('ö', "o").replace('ä', "a").replace('ü', "u")
    }

    fn titles(groups: &[ChatterGroup]) -> Vec<(&str, Vec<&str>)> {
        groups.iter().map(|g| (g.title, g.chatters.iter().map(|c| c.user_name.as_str()).collect())).collect()
    }

    #[test]
    fn gruppen_und_suche() {
        let chatters = [chatter("5", "zora"), chatter("1", "Kanal"), chatter("2", "Max"), chatter("3", "anna"), chatter("4", "Ömer")];
        let (mods, vips) = (HashSet::from(["2".to_string()]), HashSet::from(["4".to_string()]));
        let roles = RoleSource { broadcaster_id: "1", moderators: &mods, vips: &vips };
        let compare = |a: &str, b: &str| base(a).cmp(&base(b));
        assert_eq!(
            titles(&group_chatters(&chatters, &roles, "", compare)),
            [("Broadcaster", vec!["Kanal"]), ("Mods", vec!["Max"]), ("VIPs", vec!["Ömer"]), ("Zuschauer", vec!["anna", "zora"])]
        );
        let none = HashSet::new();
        let roles = RoleSource { broadcaster_id: "1", moderators: &none, vips: &none };
        assert_eq!(titles(&group_chatters(&chatters, &roles, "@AN", compare)), [("Broadcaster", vec!["Kanal"]), ("Zuschauer", vec!["anna"])]);
    }
}
