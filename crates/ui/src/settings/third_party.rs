//! Drittanbieter: jeder Dienst einzeln an/aus, bei 7TV einzeln Emotes, Badges und Namensfarben. Ausgeschaltet heißt:
//! keine Abfragen an den Dienst. Änderungen gelten sofort. Übertragen aus
//! src/features/settings/sections/third-party-section.tsx.

use multiyap_core::i18n::tr;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::bridge::{save_setting, use_setting};
use crate::components::{SettingGroup, SettingRow, Switch};

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
struct SevenTv {
    emotes: bool,
    badges: bool,
    paints: bool,
}

const SEVENTV: &str = "thirdParty.seventv";
const PAINT_SHADOWS: &str = "thirdParty.seventvPaintShadows";
const BTTV: &str = "thirdParty.bttv";
const FFZ: &str = "thirdParty.ffz";

#[component]
pub fn ThirdPartySection() -> impl IntoView {
    let seventv = use_setting(SEVENTV, SevenTv { emotes: true, badges: true, paints: true });
    let paint_shadows = use_setting(PAINT_SHADOWS, false);
    let bttv = use_setting(BTTV, true);
    let ffz = use_setting(FFZ, true);

    let save_seventv = move |change: fn(&mut SevenTv, bool)| {
        Callback::new(move |checked: bool| {
            let mut next = seventv.get_untracked();
            change(&mut next, checked);
            save_setting(SEVENTV, &next);
        })
    };

    view! {
        <div class="flex flex-col gap-8">
            <SettingGroup title="7TV" description=tr("Kanal und global, auch animiert und als Overlay.", "Channel and global, also animated and as overlay.")>
                <SettingRow label="Emotes" html_for="seventv-emotes">
                    <Switch
                        id="seventv-emotes"
                        checked=Signal::derive(move || seventv.get().emotes)
                        on_change=save_seventv(|s, v| s.emotes = v)
                    />
                </SettingRow>
                <SettingRow label="Badges" description=tr("Abzeichen von 7TV am Namen.", "7TV badges next to the name.") html_for="seventv-badges">
                    <Switch
                        id="seventv-badges"
                        checked=Signal::derive(move || seventv.get().badges)
                        on_change=save_seventv(|s, v| s.badges = v)
                    />
                </SettingRow>
                <SettingRow label=tr("Namensfarben", "Name colors") description=tr("Farbige Namen (Paints).", "Colored names (paints).") html_for="seventv-paints">
                    <Switch
                        id="seventv-paints"
                        checked=Signal::derive(move || seventv.get().paints)
                        on_change=save_seventv(|s, v| s.paints = v)
                    />
                </SettingRow>
                <SettingRow label=tr("Schatten der Namensfarben", "Shadows of name colors") description=tr("Weicher Schein um farbige Namen wie bei 7TV. Lässt kleine Schrift unscharf wirken.", "Soft glow around colored names as on 7TV. Makes small text look blurry.") html_for="seventv-paint-shadows">
                    <Switch
                        id="seventv-paint-shadows"
                        checked=paint_shadows
                        disabled=Signal::derive(move || !seventv.get().paints)
                        on_change=Callback::new(|checked: bool| save_setting(PAINT_SHADOWS, &checked))
                    />
                </SettingRow>
            </SettingGroup>

            <SettingGroup title=tr("BTTV und FFZ", "BTTV and FFZ")>
                <SettingRow label=tr("BTTV-Emotes", "BTTV emotes") description=tr("Kanal, geteilt und global.", "Channel, shared and global.") html_for="bttv-emotes">
                    <Switch id="bttv-emotes" checked=bttv on_change=Callback::new(|checked: bool| save_setting(BTTV, &checked)) />
                </SettingRow>
                <SettingRow label=tr("FFZ-Emotes", "FFZ emotes") description=tr("Kanal und global.", "Channel and global.") html_for="ffz-emotes">
                    <Switch id="ffz-emotes" checked=ffz on_change=Callback::new(|checked: bool| save_setting(FFZ, &checked)) />
                </SettingRow>
            </SettingGroup>

            <p class="text-sm text-muted-foreground">
                {tr("Ausgeschaltete Dienste werden nicht abgefragt. Bei gleichen Namen: Twitch vor Kanal vor global, dann 7TV vor BTTV vor FFZ.", "Services that are switched off are not queried. For identical names: Twitch before channel before global, then 7TV before BTTV before FFZ.")}
            </p>
        </div>
    }
}
