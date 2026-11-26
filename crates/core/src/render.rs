//! Aufbereitung des Nachrichtentextes für die Anzeige (bisher links.ts, third-party/expand.ts): erst Links in
//! Textteilen erkennen, dann ganze Wörter zu Drittanbieter-Emotes machen, dann 7TV-Overlay-Emotes über das
//! vorherige Emote legen. Alles bleibt Text oder Bild, nie HTML.

use crate::emotes::split_keeping_whitespace;
use crate::links::{split_links, LinkPart};
use crate::model::{Emote, MessagePart};

/// Bestandteile mit erkannten Links.
pub fn with_links(parts: &[MessagePart]) -> Vec<MessagePart> {
    parts
        .iter()
        .flat_map(|part| match part {
            MessagePart::Text { text } => split_links(text)
                .into_iter()
                .map(|piece| match piece {
                    LinkPart::Text(text) => MessagePart::Text { text },
                    LinkPart::Link { text, url } => MessagePart::Link { text, url },
                })
                .collect(),
            other => vec![other.clone()],
        })
        .collect()
}

pub fn has_link(parts: &[MessagePart]) -> bool {
    parts.iter().any(|part| match part {
        MessagePart::Link { .. } => true,
        MessagePart::Text { text } => split_links(text).iter().any(|p| matches!(p, LinkPart::Link { .. })),
        _ => false,
    })
}

/// Ersetzt in Textteilen ganze Wörter, die Namen von Drittanbieter-Emotes sind, durch Emote-Teile. Groß- und
/// Kleinschreibung zählt (Emote-Namen unterscheiden sie).
pub fn expand_emotes(parts: Vec<MessagePart>, lookup: impl Fn(&str) -> Option<Emote>) -> Vec<MessagePart> {
    let mut result = Vec::with_capacity(parts.len());
    for part in parts {
        let MessagePart::Text { text } = &part else {
            result.push(part);
            continue;
        };
        let mut buffer = String::new();
        for token in split_keeping_whitespace(text) {
            let emote = if token.trim().is_empty() { None } else { lookup(token) };
            match emote {
                Some(emote) => {
                    if !buffer.is_empty() {
                        result.push(MessagePart::Text { text: std::mem::take(&mut buffer) });
                    }
                    result.push(MessagePart::Emote { emote });
                }
                None => buffer.push_str(token),
            }
        }
        if !buffer.is_empty() {
            result.push(MessagePart::Text { text: buffer });
        }
    }
    result
}

/// Ein Emote mit darübergelegten 7TV-Overlay-Emotes, oder ein einzelner Teil.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderPart {
    Part(MessagePart),
    Stack { base: Emote, overlays: Vec<Emote> },
}

/// Fasst Overlay-Emotes mit dem vorherigen Emote zusammen; Leerraum dazwischen fällt weg. Ein Overlay ohne
/// vorheriges Emote bleibt ein normales Bild.
pub fn stack_overlays(parts: Vec<MessagePart>) -> Vec<RenderPart> {
    let mut result: Vec<RenderPart> = Vec::with_capacity(parts.len());
    for part in parts {
        if let MessagePart::Emote { emote } = &part {
            if emote.overlay == Some(true) {
                let mut index = result.len();
                if let Some(RenderPart::Part(MessagePart::Text { text })) = result.last() {
                    if text.trim().is_empty() {
                        index -= 1;
                    }
                }
                if index > 0 {
                    match &mut result[index - 1] {
                        RenderPart::Stack { overlays, .. } => {
                            overlays.push(emote.clone());
                            result.truncate(index);
                            continue;
                        }
                        RenderPart::Part(MessagePart::Emote { emote: base }) => {
                            let base = base.clone();
                            result[index - 1] = RenderPart::Stack { base, overlays: vec![emote.clone()] };
                            result.truncate(index);
                            continue;
                        }
                        _ => {}
                    }
                }
            }
        }
        result.push(RenderPart::Part(part));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emote(name: &str, overlay: bool) -> Emote {
        Emote {
            source: "seventv".into(),
            id: name.into(),
            name: name.into(),
            image_url: format!("u/{name}"),
            image_url2x: None,
            animated: None,
            overlay: Some(overlay),
            owner_id: None,
            global: None,
        }
    }

    fn lookup(list: &[Emote]) -> impl Fn(&str) -> Option<Emote> + '_ {
        move |name| list.iter().find(|e| e.name == name).cloned()
    }

    fn text(t: &str) -> MessagePart {
        MessagePart::Text { text: t.into() }
    }

    #[test]
    fn ganze_woerter_werden_emotes() {
        let list = [emote("catJAM", false)];
        assert_eq!(
            expand_emotes(vec![text("hi catJAM  catjam catJAMs")], lookup(&list)),
            vec![text("hi "), MessagePart::Emote { emote: emote("catJAM", false) }, text("  catjam catJAMs")]
        );
    }

    #[test]
    fn overlays_liegen_ueber_dem_vorherigen_emote() {
        let list = [emote("catJAM", false), emote("RainTime", true), emote("PETPET", true)];
        let rendered = stack_overlays(expand_emotes(vec![text("catJAM RainTime PETPET ok")], lookup(&list)));
        assert_eq!(rendered[0], RenderPart::Stack { base: emote("catJAM", false), overlays: vec![emote("RainTime", true), emote("PETPET", true)] });
        assert_eq!(rendered[1], RenderPart::Part(text(" ok")));
        let alone = [emote("RainTime", true)];
        let rendered = stack_overlays(expand_emotes(vec![text("hallo RainTime")], lookup(&alone)));
        assert!(rendered.iter().all(|r| matches!(r, RenderPart::Part(_))));
        assert_eq!(rendered.len(), 2);
    }

    #[test]
    fn links_in_textteilen() {
        let parts = with_links(&[text("schau www.example.com an")]);
        assert!(matches!(parts[1], MessagePart::Link { .. }));
        assert!(has_link(&[text("x https://a.de")]));
        assert!(!has_link(&[text("datei.txt")]));
    }
}
