//! Rollen aus Abzeichen und ein Speicher in Einfügereihenfolge mit Obergrenze (bisher
//! src/core/moderation/channel-roles.ts, mod-status.ts und suspicious.ts). Die Speicher selbst liegen in
//! crates/web/src/moderation.rs.

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;

/// Abzeichen mit Mod-Rechten: Broadcaster, Mod, Lead Mod.
pub const MOD_BADGES: [&str; 3] = ["broadcaster", "moderator", "lead_moderator"];

/// Bekannte Rolle einer anderen Person in einem Kanal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnownRole {
    Moderator,
    Vip,
}

/// Rolle aus den Abzeichen einer Nachricht (Lead Mods zählen als Mods).
pub fn role_from_badges(badge_set_ids: &[String]) -> Option<KnownRole> {
    if badge_set_ids.iter().any(|b| b == "moderator" || b == "lead_moderator") {
        Some(KnownRole::Moderator)
    } else if badge_set_ids.iter().any(|b| b == "vip") {
        Some(KnownRole::Vip)
    } else {
        None
    }
}

/// Hat jemand mit diesen Abzeichen Mod-Rechte (Broadcaster, Mod, Lead Mod)?
pub fn has_mod_badge(badge_set_ids: &[String]) -> bool {
    badge_set_ids.iter().any(|b| MOD_BADGES.contains(&b.as_str()))
}

/// Abzeichen, bei denen keine Schnellbuttons erscheinen. VIPs behalten sie.
pub fn is_protected_sender(badge_set_ids: &[String]) -> bool {
    has_mod_badge(badge_set_ids)
}

/// Schlüssel und Werte in Einfügereihenfolge (wie Map in JavaScript): Neu setzen rückt nach hinten, bei mehr als
/// `limit` Einträgen fällt der älteste weg.
#[derive(Debug, Clone)]
pub struct OrderedMap<K, V> {
    entries: HashMap<K, (u64, V)>,
    order: BTreeMap<u64, K>,
    next: u64,
    limit: usize,
}

impl<K: Eq + Hash + Clone, V> OrderedMap<K, V> {
    pub fn new(limit: usize) -> Self {
        OrderedMap { entries: HashMap::new(), order: BTreeMap::new(), next: 0, limit }
    }

    /// Setzt einen Wert ans Ende. Gibt einen wegen der Obergrenze entfernten ältesten Eintrag zurück.
    pub fn insert(&mut self, key: K, value: V) -> Option<(K, V)> {
        self.remove(&key);
        self.order.insert(self.next, key.clone());
        self.entries.insert(key, (self.next, value));
        self.next += 1;
        if self.entries.len() > self.limit {
            let (_, oldest) = self.order.pop_first()?;
            return self.entries.remove(&oldest).map(|(_, v)| (oldest, v));
        }
        None
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let (seq, value) = self.entries.remove(key)?;
        self.order.remove(&seq);
        Some(value)
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.entries.get(key).map(|(_, v)| v)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    /// Einträge in Einfügereihenfolge.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.order.values().filter_map(|k| self.entries.get(k).map(|(_, v)| (k, v)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn roles_from_badges() {
        assert_eq!(role_from_badges(&ids(&["lead_moderator"])), Some(KnownRole::Moderator));
        assert_eq!(role_from_badges(&ids(&["vip", "moderator"])), Some(KnownRole::Moderator));
        assert_eq!(role_from_badges(&ids(&["vip"])), Some(KnownRole::Vip));
        assert_eq!(role_from_badges(&ids(&["subscriber"])), None);
        assert!(is_protected_sender(&ids(&["broadcaster"])));
        assert!(!is_protected_sender(&ids(&["vip"])));
    }

    #[test]
    fn ordered_map_keeps_insertion_order_and_limit() {
        let mut map = OrderedMap::new(3);
        map.insert("a", 1);
        map.insert("b", 2);
        map.insert("c", 3);
        // Neu setzen rückt nach hinten.
        map.insert("a", 4);
        assert_eq!(map.iter().map(|(k, _)| *k).collect::<Vec<_>>(), vec!["b", "c", "a"]);
        // Obergrenze: der älteste fällt weg.
        assert_eq!(map.insert("d", 5), Some(("b", 2)));
        assert_eq!(map.len(), 3);
        assert_eq!(map.get(&"a"), Some(&4));
        assert_eq!(map.remove(&"c"), Some(3));
        assert_eq!(map.iter().map(|(k, _)| *k).collect::<Vec<_>>(), vec!["a", "d"]);
    }
}
