//! Chat-Verhalten: Antwort-Button, Chat pausieren; die Spam-Erkennung ist hier definiert, steht aber im Bereich
//! „Filter“. Übertragen aus chat-behavior-section.tsx und spam-group.tsx. Änderungen gelten sofort.

use multiyap_core::i18n::tr;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::bridge::{save_setting, use_setting};
use crate::components::{Select, SelectEntry, SettingGroup, SettingRow, Switch};

const PAUSE_TRIGGER: &str = "chat.pauseTrigger";
const PAUSE_MAX: &str = "chat.pauseMaxSeconds";
const REPLY_BUTTON: &str = "chat.replyButton";
const PRIVATE_LINKS: &str = "chat.openLinksPrivate";
const GIFT_LINES: &str = "chat.giftSubLines";
const SPAM: &str = "spam.settings";

fn trigger_labels() -> [(&'static str, &'static str); 3] {
    [("hover", tr("Maus über dem Chat", "Mouse over the chat")), ("alt-hover", tr("Alt gedrückt und Maus über dem Chat", "Alt held and mouse over the chat")), ("off", tr("Aus", "Off"))]
}

fn max_options() -> [(u32, &'static str); 5] {
    [(5, tr("5 Sekunden", "5 seconds")), (10, tr("10 Sekunden", "10 seconds")), (30, tr("30 Sekunden", "30 seconds")), (60, tr("1 Minute", "1 minute")), (0, tr("Unbegrenzt", "Unlimited"))]
}

#[component]
pub fn ChatBehaviorSection() -> impl IntoView {
    let trigger = use_setting(PAUSE_TRIGGER, "hover".to_string());
    let max = use_setting(PAUSE_MAX, 10u32);
    let reply_button = use_setting(REPLY_BUTTON, true);
    let private_links = use_setting(PRIVATE_LINKS, false);
    let gift_lines = use_setting(GIFT_LINES, false);

    let trigger_entries = trigger_labels().iter().map(|(v, l)| SelectEntry::item(*v, *l)).collect::<Vec<_>>();
    let max_entries = max_options().iter().map(|(v, l)| SelectEntry::item(v.to_string(), *l)).collect::<Vec<_>>();

    view! {
        <div class="flex flex-col gap-8">
            <SettingGroup>
                <SettingRow label=tr("Antwort-Button", "Reply button") description=tr("Erscheint beim Überfahren am Ende jeder Nachricht. Rechtsklick auf einen Namen fügt „@Name“ ein.", "Appears at the end of each message on hover. Right-clicking a name inserts “@Name”.") html_for="reply-button">
                    <Switch id="reply-button" checked=reply_button on_change=Callback::new(|checked: bool| save_setting(REPLY_BUTTON, &checked)) />
                </SettingRow>
                <SettingRow label=tr("Links im privaten Fenster öffnen", "Open links in a private window") description=tr("Öffnet Links im privaten Fenster des Standardbrowsers (Edge, Chrome, Brave, Vivaldi, Opera, Firefox).", "Opens links in a private window of the default browser (Edge, Chrome, Brave, Vivaldi, Opera, Firefox).") html_for="private-links">
                    <Switch id="private-links" checked=private_links on_change=Callback::new(|checked: bool| save_setting(PRIVATE_LINKS, &checked)) />
                </SettingRow>
                <SettingRow label=tr("Gift Subs: jede beschenkte Person in eigener Zeile", "Gift subs: each recipient on its own line") description=tr("Zusätzlich zur Zeile des Community-Geschenks. Aus: die Beschenkten stehen aufklappbar in dieser einen Zeile. Gilt für neue Geschenke und den Verlauf beim nächsten Laden.", "In addition to the line of the community gift. Off: the recipients can be expanded in that one line. Applies to new gifts and to the history the next time it loads.") html_for="gift-lines">
                    <Switch id="gift-lines" checked=gift_lines on_change=Callback::new(|checked: bool| save_setting(GIFT_LINES, &checked)) />
                </SettingRow>
            </SettingGroup>

            <SettingGroup title=tr("Chat pausieren", "Pause chat") description=tr("Nur der Chat unter der Maus. Neue Nachrichten erscheinen danach.", "Only the chat under the mouse. New messages appear afterwards.")>
                <SettingRow label=tr("Auslöser", "Trigger") html_for="pause-trigger">
                    <Select
                        id="pause-trigger"
                        class="w-72"
                        value=trigger
                        entries=Signal::stored(trigger_entries)
                        on_change=Callback::new(|value: String| save_setting(PAUSE_TRIGGER, &value))
                    />
                </SettingRow>
                <SettingRow label=tr("Höchstens pausieren für", "Pause for at most") description=tr("Danach läuft der Chat von selbst weiter.", "After that the chat continues on its own.") html_for="pause-max">
                    <Select
                        id="pause-max"
                        class="w-40"
                        value=Signal::derive(move || max.get().to_string())
                        entries=Signal::stored(max_entries)
                        disabled=Signal::derive(move || trigger.get() == "off")
                        on_change=Callback::new(|value: String| {
                            if let Ok(seconds) = value.parse::<u32>() {
                                save_setting(PAUSE_MAX, &seconds);
                            }
                        })
                    />
                </SettingRow>
            </SettingGroup>
        </div>
    }
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpamSettings {
    pub enabled: bool,
    pub similarity: u32,
    pub min_count: u32,
    pub window_minutes: u32,
    pub min_length: u32,
    pub highlight_at: u32,
}

impl SpamSettings {
    /// Für die Zählung in crates/core/src/spam.rs.
    pub(crate) fn core(self) -> multiyap_core::spam::SpamSettings {
        multiyap_core::spam::SpamSettings {
            enabled: self.enabled,
            similarity: f64::from(self.similarity),
            min_count: self.min_count,
            window_minutes: f64::from(self.window_minutes),
            min_length: self.min_length as usize,
        }
    }
}

pub(crate) const SPAM_DEFAULT: SpamSettings =
    SpamSettings { enabled: true, similarity: 70, min_count: 2, window_minutes: 5, min_length: 5, highlight_at: 5 };

/// Auswahl einer Zahl; ein gespeicherter Wert, der nicht in der Liste steht (zum Beispiel aus einer Sicherung),
/// bleibt wählbar.
#[component]
fn NumberSelect(
    #[prop(into)] id: String,
    #[prop(into)] value: Signal<u32>,
    options: &'static [u32],
    format: fn(u32) -> String,
    #[prop(into)] disabled: Signal<bool>,
    #[prop(into)] on_change: Callback<u32>,
) -> impl IntoView {
    let entries = Signal::derive(move || {
        let current = value.get();
        let mut all = options.to_vec();
        if !all.contains(&current) {
            all.push(current);
            all.sort_unstable();
        }
        all.into_iter().map(|v| SelectEntry::item(v.to_string(), format(v))).collect::<Vec<_>>()
    });
    view! {
        <Select
            id=id
            class="w-40"
            value=Signal::derive(move || value.get().to_string())
            entries=entries
            disabled=disabled
            on_change=Callback::new(move |next: String| {
                if let Ok(number) = next.parse::<u32>() {
                    on_change.run(number);
                }
            })
        />
    }
}

fn standard(is_default: bool) -> &'static str {
    if is_default { tr(" (Standard)", " (default)") } else { "" }
}

/// Spam-Erkennung: rein visuell, nur in Mod-Kanälen. Steht seit 23.11.2026 im Bereich „Filter“
/// (Crates/ui/src/settings/terms.rs).
#[component]
pub(crate) fn SpamGroup() -> impl IntoView {
    let settings = use_setting(SPAM, SPAM_DEFAULT);
    let save = move |change: fn(&mut SpamSettings, u32)| {
        Callback::new(move |value: u32| {
            let mut next = settings.get_untracked();
            change(&mut next, value);
            save_setting(SPAM, &next);
        })
    };
    let off = Signal::derive(move || !settings.get().enabled);

    view! {
        <SettingGroup
            title=tr("Spam-Erkennung", "Spam detection")
            description=tr("Zeigt „(5x)“ hinter ähnlichen Nachrichten derselben Person. Nur Mod-Kanäle, keine automatische Aktion. Ausgenommene Benutzer zählen nicht.", "Shows “(5x)” after similar messages from the same person. Mod channels only, no automatic action. Excluded users do not count.")
        >
            <SettingRow label=tr("Einschalten", "Turn on") html_for="spam-enabled">
                <Switch
                    id="spam-enabled"
                    checked=Signal::derive(move || settings.get().enabled)
                    on_change=Callback::new(move |enabled: bool| {
                        let mut next = settings.get_untracked();
                        next.enabled = enabled;
                        save_setting(SPAM, &next);
                    })
                />
            </SettingRow>
            <SettingRow label=tr("Übereinstimmung", "Similarity") description=tr("Wie ähnlich zwei Nachrichten sein müssen.", "How similar two messages must be.") html_for="spam-similarity">
                <NumberSelect
                    id="spam-similarity"
                    value=Signal::derive(move || settings.get().similarity)
                    options=&[50, 60, 70, 80, 90, 100]
                    format=|v| format!("{v} %{}", standard(v == 70))
                    disabled=off
                    on_change=save(|s, v| s.similarity = v)
                />
            </SettingRow>
            <SettingRow label=tr("Zähler ab", "Count from") html_for="spam-min-count">
                <NumberSelect
                    id="spam-min-count"
                    value=Signal::derive(move || settings.get().min_count)
                    options=&[2, 3, 4, 5, 10]
                    format=|v| format!("{v}x{}", standard(v == 2))
                    disabled=off
                    on_change=save(|s, v| s.min_count = v)
                />
            </SettingRow>
            <SettingRow label=tr("Zeitfenster", "Time window") html_for="spam-window">
                <NumberSelect
                    id="spam-window"
                    value=Signal::derive(move || settings.get().window_minutes)
                    options=&[1, 2, 5, 10, 15, 30, 60]
                    format=|v| format!("{v} {}{}", if v == 1 { {tr("Minute", "minute")} } else { {tr("Minuten", "minutes")} }, standard(v == 5))
                    disabled=off
                    on_change=save(|s, v| s.window_minutes = v)
                />
            </SettingRow>
            <SettingRow label=tr("Kurze Nachrichten ignorieren", "Ignore short messages") description=tr("Mindestlänge ohne Leerzeichen.", "Minimum length without spaces.") html_for="spam-min-length">
                <NumberSelect
                    id="spam-min-length"
                    value=Signal::derive(move || settings.get().min_length)
                    options=&[1, 3, 5, 10, 20]
                    format=|v| multiyap_core::tf!("{v} Zeichen{}", "{v} characters{}", standard(v == 5))
                    disabled=off
                    on_change=save(|s, v| s.min_length = v)
                />
            </SettingRow>
            <SettingRow label=tr("Rot hervorheben ab", "Highlight red from") html_for="spam-highlight">
                <NumberSelect
                    id="spam-highlight"
                    value=Signal::derive(move || settings.get().highlight_at)
                    options=&[3, 5, 10, 20]
                    format=|v| format!("{v}x{}", standard(v == 5))
                    disabled=off
                    on_change=save(|s, v| s.highlight_at = v)
                />
            </SettingRow>
        </SettingGroup>
    }
}
