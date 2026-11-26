//! Community-Geschenke als **eine** Zeile: einzelne Geschenk-Abos, die zu einem bekannten Community-Geschenk gehören,
//! werden als Empfänger an dessen Zeile gehängt statt eigene Zeilen zu erzeugen. Ist das Community-Geschenk (noch)
//! nicht bekannt, erscheint das Geschenk einzeln, damit nichts verloren geht. Mit der Einstellung chat.giftSubLines
//! bekommt jede beschenkte Person zusätzlich zur Zeile des Community-Geschenks eine eigene Zeile.

use std::collections::HashMap;

use crate::model::ChatEventItem;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GiftPlacement {
    /// Eigene Zeile.
    New,
    /// Empfänger an die Zeile mit dieser ID hängen (`attach_recipient`).
    Attach { target_id: String },
}

#[derive(Debug, Default)]
pub struct CommunityGifts {
    /// Community-Geschenk-ID → ID der Zeile.
    lines: HashMap<String, String>,
}

impl CommunityGifts {
    /// `separate`: jede beschenkte Person in eigener Zeile (Einstellung chat.giftSubLines).
    pub fn place(&mut self, item: &ChatEventItem, recipient: Option<&str>, separate: bool) -> GiftPlacement {
        let Some(gift_id) = item.community_gift_id.as_deref().filter(|id| !id.is_empty()) else {
            return GiftPlacement::New;
        };
        if item.event_type == "community_sub_gift" {
            self.lines.insert(gift_id.to_string(), item.id.clone());
            return GiftPlacement::New;
        }
        if item.event_type == "sub_gift" && !separate && recipient.is_some_and(|r| !r.is_empty()) {
            if let Some(target_id) = self.lines.get(gift_id) {
                return GiftPlacement::Attach { target_id: target_id.clone() };
            }
        }
        GiftPlacement::New
    }
}

/// Hängt einen Empfänger an, falls er noch nicht dabei ist.
pub fn attach_recipient(target: &mut ChatEventItem, recipient: &str) {
    let recipients = target.recipients.get_or_insert_with(Vec::new);
    if !recipients.iter().any(|r| r == recipient) {
        recipients.push(recipient.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ChannelRef;

    fn item(id: &str, event_type: &str, gift_id: &str) -> ChatEventItem {
        ChatEventItem {
            kind: "event".into(),
            channel: ChannelRef { platform: "twitch".into(), id: "1".into(), login: "kanal".into(), display_name: "Kanal".into() },
            id: id.into(),
            timestamp: 0.0,
            event_type: event_type.into(),
            text: String::new(),
            message: None,
            recipients: None,
            community_gift_id: Some(gift_id.into()),
            users: None,
            from_history: None,
            link: None,
        }
    }

    #[test]
    fn single_gifts_become_recipients_of_one_line() {
        let mut gifts = CommunityGifts::default();
        let mut line = item("c-line", "community_sub_gift", "c1");
        assert_eq!(gifts.place(&line, None, false), GiftPlacement::New);
        for name in ["Max", "Lisa", "Max"] {
            let single = item(&format!("g-{name}"), "sub_gift", "c1");
            assert_eq!(gifts.place(&single, Some(name), false), GiftPlacement::Attach { target_id: "c-line".into() });
            attach_recipient(&mut line, name);
        }
        assert_eq!(line.recipients, Some(vec!["Max".to_string(), "Lisa".to_string()]));
    }

    #[test]
    fn unknown_community_gift_shows_single_gift() {
        // Nichts geht verloren: ohne bekannte Zeile erscheint das Einzelgeschenk selbst.
        assert_eq!(CommunityGifts::default().place(&item("g", "sub_gift", "x"), Some("Max"), false), GiftPlacement::New);
    }

    #[test]
    fn each_recipient_own_line_when_separate() {
        let mut gifts = CommunityGifts::default();
        assert_eq!(gifts.place(&item("c-line", "community_sub_gift", "c1"), None, true), GiftPlacement::New);
        assert_eq!(gifts.place(&item("g-max", "sub_gift", "c1"), Some("Max"), true), GiftPlacement::New);
    }
}
