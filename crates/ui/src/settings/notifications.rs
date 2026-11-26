//! Benachrichtigungen (notifications-section.tsx) mit den Erwähnungen (Abschnitt 10.3). Abspielen, eigene Sounds und
//! Stummschalten laufen über die Brücke.

use multiyap_core::i18n::tr;
use std::collections::BTreeMap;

use multiyap_core::terms::parse_terms;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsCast;

use super::model::{term_swatch, TermCategory};
use crate::bridge::{save_setting, toast, update_setting, use_setting};
use crate::components::input::{merge_width, INPUT_CLASS, TEXTAREA_CLASS};
use crate::components::overlay::Slider;
use crate::components::{icons, Button, CommitInput, Select, SelectEntry, SettingGroup, SettingRow, Switch};

const MASTER: &str = "notifications.volume";
const LIVE: &str = "notifications.live";
const LIVE_BLINK: &str = "tabs.liveBlink";
const MENTION: &str = "notifications.mention";
const TERM_NOTIFY: &str = "notifications.termCategories";
const CATEGORIES: &str = "highlights.categories";
const MUTED: &str = "notifications.mutedChannels";
const LIVE_OFF: &str = "notifications.liveOffChannels";
const SOUND_CHOICE: &str = "notifications.soundChoice";
const OWN_NAME: &str = "mentions.ownName";
const EXTRA_NAMES: &str = "mentions.extraNames";
const DEFAULT_SOUND: &str = "default";
const MAX_CUSTOM_SOUNDS: usize = 10;
const MAX_MUTED_CHANNELS: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct SoundEvent {
    sound: bool,
    volume: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct NotifyEvent {
    sound: bool,
    blink: bool,
    volume: f64,
}

/// Standard für eine Begriffs-Kategorie ohne eigenen Eintrag: Ton aus, Tab leuchtet.
const TERM_DEFAULT: NotifyEvent = NotifyEvent { sound: false, blink: true, volume: 100.0 };

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CustomSoundInfo {
    id: String,
    name: String,
    mime_type: String,
    size: f64,
}

fn mime_label(mime: &str) -> &'static str {
    match mime {
        "audio/wav" | "audio/x-wav" | "audio/wave" | "audio/vnd.wave" => "WAV",
        "audio/ogg" => "OGG",
        _ => "MP3",
    }
}

fn play(sound: Option<String>, volume: f64) {
    spawn_local(multiyap_web::notifications::play_sound(sound, volume));
}

/// Speichert den gewählten Sound eines Ereignisses; None heißt Standard-Sound.
fn save_sound_choice(key: String, sound: Option<String>) {
    update_setting::<BTreeMap<String, String>>(SOUND_CHOICE, move |mut current| {
        match &sound {
            None => {
                current.remove(&key);
            }
            Some(id) => {
                current.insert(key.clone(), id.clone());
            }
        }
        current
    });
}

/// Lautstärke-Regler: speichert beim Loslassen, damit nicht jede Zwischenstellung geschrieben wird.
#[component]
fn VolumeSlider(#[prop(into)] value: Signal<f64>, #[prop(into)] on_commit: Callback<f64>, #[prop(into)] label: String, #[prop(optional, into)] disabled: Signal<bool>) -> impl IntoView {
    let draft = RwSignal::new(value.get_untracked());
    Effect::new(move |_| draft.set(value.get()));
    view! {
        <div class="flex w-40 items-center gap-3">
            <Slider value=draft label=label step=5.0 disabled=disabled on_input=Callback::new(move |v: f64| draft.set(v)) on_commit=on_commit />
            <span class="w-12 shrink-0 text-right text-sm whitespace-nowrap text-muted-foreground tabular-nums">{move || format!("{} %", draft.get())}</span>
        </div>
    }
}

/// Eine Ereignisart: Ton an/aus, Blinken an/aus, Sound, eigene Lautstärke, Probehören.
#[component]
fn EventRow(
    id: String,
    #[prop(into)] label: Signal<String>,
    #[prop(optional)] swatch: Option<Signal<&'static str>>,
    sound_key: String,
    #[prop(into)] sound: Signal<SoundEvent>,
    #[prop(into)] on_sound: Callback<SoundEvent>,
    #[prop(into)] blink: Signal<bool>,
    #[prop(into)] on_blink: Callback<bool>,
    sounds: ReadSignal<Vec<CustomSoundInfo>>,
) -> impl IntoView {
    let master = use_setting(MASTER, 70.0);
    let choices = use_setting(SOUND_CHOICE, BTreeMap::<String, String>::new());
    // Ein gelöschter Sound zählt als Standard.
    let chosen = {
        let key = sound_key.clone();
        Memo::new(move |_| choices.get().get(&key).filter(|id| sounds.get().iter().any(|s| &s.id == *id)).cloned())
    };
    let entries = Signal::derive(move || {
        let mut list = vec![SelectEntry::item(DEFAULT_SOUND, tr("Standard-Sound", "Default sound"))];
        list.extend(sounds.get().into_iter().map(|s| SelectEntry::item(s.id, s.name)));
        list
    });
    let (sound_id, blink_id) = (format!("{id}-sound"), format!("{id}-blink"));
    view! {
        <div class="flex flex-col gap-3 py-3">
            <div class="flex flex-col gap-1">
                <span class="flex items-center gap-2 text-sm font-medium">
                    {swatch.map(|class| view! { <span class=move || format!("size-3 shrink-0 rounded-full {}", class.get()) aria-hidden="true"></span> })}
                    {move || label.get()}
                </span>
            </div>
            <div class="flex flex-wrap items-center gap-4">
                <div class="flex items-center gap-2">
                    <Switch id=sound_id.clone() checked=Signal::derive(move || sound.get().sound)
                        on_change=Callback::new(move |checked: bool| on_sound.run(SoundEvent { sound: checked, ..sound.get_untracked() })) />
                    <label for=sound_id class="text-sm" on:click=|event| event.prevent_default()>{tr("Ton", "Sound")}</label>
                </div>
                <div class="flex items-center gap-2">
                    <Switch id=blink_id.clone() checked=blink on_change=on_blink />
                    <label for=blink_id class="text-sm" on:click=|event| event.prevent_default()>{tr("Tab blinkt", "Tab blinks")}</label>
                </div>
                <Select
                    id=format!("{id}-choice")
                    size="sm"
                    class="w-44"
                    aria_label=multiyap_core::tf!("Sound für {}", "Sound for {}", label.get_untracked())
                    value=Signal::derive(move || chosen.get().unwrap_or_else(|| DEFAULT_SOUND.into()))
                    entries=entries
                    disabled=Signal::derive(move || !sound.get().sound)
                    on_change=Callback::new(move |value: String| save_sound_choice(sound_key.clone(), (value != DEFAULT_SOUND).then_some(value)))
                />
                <VolumeSlider
                    label=multiyap_core::tf!("Lautstärke {}", "Volume {}", label.get_untracked())
                    value=Signal::derive(move || sound.get().volume)
                    disabled=Signal::derive(move || !sound.get().sound)
                    on_commit=Callback::new(move |volume: f64| on_sound.run(SoundEvent { volume, ..sound.get_untracked() }))
                />
                <Button variant="outline" size="sm" aria_label=multiyap_core::tf!("Probehören: {}", "Preview: {}", label.get_untracked())
                    on_click=Callback::new(move |_| play(chosen.get_untracked(), master.get_untracked() / 100.0 * sound.get_untracked().volume / 100.0))>
                    {icons::play("")}
                    {tr("Probehören", "Preview")}
                </Button>
            </div>
        </div>
    }
}

/// Eigene Sounds: hochladen, umbenennen, probehören, entfernen.
#[component]
fn CustomSoundsGroup(sounds: ReadSignal<Vec<CustomSoundInfo>>) -> impl IntoView {
    let master = use_setting(MASTER, 70.0);
    let busy = RwSignal::new(false);
    let input_ref = NodeRef::<leptos::html::Input>::new();
    view! {
        <SettingGroup title=tr("Eigene Sounds", "Custom sounds") description=multiyap_core::tf!("MP3, WAV oder OGG, höchstens 1 MB, bis zu {MAX_CUSTOM_SOUNDS} Sounds.", "MP3, WAV or OGG, at most 1 MB, up to {MAX_CUSTOM_SOUNDS} sounds.")>
            {move || sounds.get().is_empty().then(|| view! { <p class="py-3 text-sm text-muted-foreground">{tr("Noch keine eigenen Sounds.", "No custom sounds yet.")}</p> })}
            <For each=move || sounds.get() key=|s| (s.id.clone(), s.name.clone()) let:sound>
                {
                    let (rename_id, play_id, remove_id) = (sound.id.clone(), sound.id.clone(), sound.id.clone());
                    let name = sound.name.clone();
                    view! {
                        <div class="flex items-center gap-2 py-3">
                            <CommitInput
                                class="w-56"
                                value=sound.name.clone()
                                max_length=60
                                aria_label=tr("Name des Sounds", "Name of the sound")
                                on_commit=Callback::new(move |value: String| {
                                    if value.trim().is_empty() {
                                        return false;
                                    }
                                    let (id, name) = (rename_id.clone(), value.clone());
                                    spawn_local(async move {
                                        if let Err(message) = multiyap_web::notifications::rename_custom_sound(&id, &name).await {
                                            toast("error", &message);
                                        }
                                    });
                                    true
                                })
                            />
                            <span class="text-sm text-muted-foreground">
                                {format!("{}, {} KB", mime_label(&sound.mime_type), ((sound.size / 1024.0).round() as i64).max(1))}
                            </span>
                            <div class="ml-auto flex gap-1">
                                <Button variant="outline" size="sm" aria_label=multiyap_core::tf!("Probehören: {name}", "Preview: {name}")
                                    on_click=Callback::new(move |_| play(Some(play_id.clone()), master.get_untracked() / 100.0))>
                                    {icons::play("")}
                                    {tr("Probehören", "Preview")}
                                </Button>
                                <Button variant="ghost" size="icon-sm" aria_label=multiyap_core::tf!("{} entfernen", "Remove {}", sound.name)
                                    on_click=Callback::new(move |_| {
                                        let id = remove_id.clone();
                                        spawn_local(async move {
                                            if let Err(message) = multiyap_web::notifications::remove_custom_sound(&id).await {
                                                toast("error", &message);
                                            }
                                        });
                                    })>
                                    {icons::trash("")}
                                </Button>
                            </div>
                        </div>
                    }
                }
            </For>
            <div class="py-3">
                <input
                    node_ref=input_ref
                    type="file"
                    accept=".mp3,.wav,.ogg,.oga,audio/mpeg,audio/wav,audio/ogg"
                    class="hidden"
                    on:change=move |event| {
                        let Some(input) = event.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()) else { return };
                        let Some(file) = input.files().and_then(|files| files.get(0)) else { return };
                        busy.set(true);
                        spawn_local(async move {
                            match multiyap_web::notifications::add_custom_sound(file.into()).await {
                                Ok(sound) => toast("success", &multiyap_core::tf!("„{}“ hinzugefügt", "“{}” added", sound.name)),
                                Err(message) => toast("error", &message),
                            }
                            busy.set(false);
                            input.set_value("");
                        });
                    }
                />
                <Button variant="secondary" size="sm" disabled=Signal::derive(move || busy.get() || sounds.get().len() >= MAX_CUSTOM_SOUNDS)
                    on_click=Callback::new(move |_| { if let Some(input) = input_ref.get_untracked() { input.click(); } })>
                    {move || if busy.get() { icons::loader_circle("animate-spin").into_any() } else { icons::upload("").into_any() }}
                    {tr("Sound hochladen", "Upload sound")}
                </Button>
            </div>
        </SettingGroup>
    }
}

/// Kanal direkt in den Einstellungen stummschalten; Name oder Link wie im „+“-Dialog.
#[component]
fn AddMutedChannel(full: Signal<bool>, muted: ReadSignal<Vec<String>>) -> impl IntoView {
    let input = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let add = move || {
        let channel = multiyap_core::channel_input::parse(&input.get_untracked()).channel;
        let Some(channel) = channel else {
            toast("error", tr("Kein gültiger Twitch-Kanalname.", "Not a valid Twitch channel name."));
            return;
        };
        if muted.get_untracked().contains(&channel) {
            toast("info", &multiyap_core::tf!("{channel} ist schon stummgeschaltet.", "{channel} is already muted."));
            input.set(String::new());
            return;
        }
        busy.set(true);
        spawn_local(async move {
            if multiyap_web::notifications::add_muted(&channel).await {
                input.set(String::new());
            }
            busy.set(false);
        });
    };
    view! {
        <form class="flex items-center gap-2 py-3" on:submit=move |event| { event.prevent_default(); add(); }>
            <input data-slot="input" class=merge_width(INPUT_CLASS, "w-56") maxlength="100" placeholder=tr("Kanalname oder Link", "Channel name or link") aria-label=tr("Kanal zum Stummschalten", "Channel to mute")
                prop:value=move || input.get() on:input=move |event| input.set(event_target_value(&event)) />
            <Button submit=true variant="secondary" size="sm" disabled=Signal::derive(move || busy.get() || input.get().trim().is_empty() || full.get())>
                {icons::plus("")}
                {tr("Stummschalten", "Mute")}
            </Button>
        </form>
    }
}

/// Eigene Sounds als Signal, das sich bei Änderungen der Liste mitändert.
pub(crate) fn use_custom_sounds() -> ReadSignal<Vec<CustomSoundInfo>> {
    let read_sounds = || crate::bridge::convert::<Vec<CustomSoundInfo>>(&multiyap_web::notifications::custom_sounds()).unwrap_or_default();
    let (sounds, set_sounds) = signal(read_sounds());
    let listener = multiyap_web::notifications::subscribe_sounds(std::rc::Rc::new(move || {
        set_sounds.try_set(read_sounds());
    }));
    on_cleanup(move || multiyap_web::notifications::unsubscribe_sounds(listener));
    sounds
}

/// Sound-Einträge der Auswahl: zuerst die festen (`first`), dann der Standard-Sound und die eigenen Sounds.
fn sound_entries(first: Vec<(&'static str, &'static str)>, sounds: ReadSignal<Vec<CustomSoundInfo>>) -> Signal<Vec<SelectEntry>> {
    Signal::derive(move || {
        let mut list: Vec<SelectEntry> = first.iter().map(|(v, l)| SelectEntry::item(*v, *l)).collect();
        list.push(SelectEntry::item(DEFAULT_SOUND, tr("Standard-Sound", "Default sound")));
        list.extend(sounds.get().into_iter().map(|s| SelectEntry::item(s.id, s.name)));
        list
    })
}

const INHERIT: &str = "inherit";
const SOUND_OFF: &str = "off";

/// Sound je weiterem Namen: „Wie Erwähnung“ nutzt den Sound der Erwähnung.
#[component]
fn NameSounds() -> impl IntoView {
    let names = use_setting(EXTRA_NAMES, Vec::<String>::new());
    let choices = use_setting(SOUND_CHOICE, BTreeMap::<String, String>::new());
    let master = use_setting(MASTER, 70.0);
    let mention = use_setting(MENTION, NotifyEvent { sound: true, blink: true, volume: 100.0 });
    let sounds = use_custom_sounds();
    let entries = sound_entries(vec![(INHERIT, tr("Wie Erwähnung", "Same as mention"))], sounds);
    view! {
        {move || (!names.get().is_empty()).then(|| view! {
            <p class="text-sm text-muted-foreground">{tr("Sound je Name. Gilt, wenn bei „Erwähnung“ der Ton an ist.", "Sound per name. Applies when the sound for “Mention” is on.")}</p>
        })}
        <For each=move || names.get() key=|name| name.to_lowercase() let:name>
            {
                let key = multiyap_core::notify::mention_sound_key(&name);
                let (value_key, change_key) = (key.clone(), key.clone());
                // Ein gelöschter eigener Sound zählt wie „Wie Erwähnung“.
                let value = Memo::new(move |_| {
                    choices.get().get(&value_key).filter(|id| *id == DEFAULT_SOUND || sounds.get().iter().any(|s| &s.id == *id)).cloned().unwrap_or_else(|| INHERIT.into())
                });
                let label = name.clone();
                view! {
                    <div class="flex items-center justify-between gap-2">
                        <span class="truncate text-sm">{name.clone()}</span>
                        <div class="flex items-center gap-2">
                            <Select
                                id=format!("name-sound-{}", key.replace(':', "-"))
                                size="sm"
                                class="w-44"
                                aria_label=multiyap_core::tf!("Sound für {label}", "Sound for {label}")
                                value=value
                                entries=entries
                                on_change=Callback::new(move |choice: String| save_sound_choice(change_key.clone(), (choice != INHERIT).then_some(choice)))
                            />
                            <Button variant="ghost" size="icon-sm" aria_label=multiyap_core::tf!("Probehören: {name}", "Preview: {name}")
                                on_click=Callback::new(move |_| {
                                    let keys = [key.clone(), "mention".to_string()];
                                    let choice = multiyap_core::notify::sound_choice(&serde_json::to_value(choices.get_untracked()).unwrap_or_default(), &keys);
                                    play(choice, master.get_untracked() / 100.0 * mention.get_untracked().volume / 100.0);
                                })>
                                {icons::play("")}
                            </Button>
                        </div>
                    </div>
                }
            }
        </For>
    }
}

/// Sound einer Filter-Kategorie direkt in den Filtern: „Kein Ton“, Standard-Sound oder ein eigener Sound. Dieselben
/// Einstellungen wie unter Benachrichtigungen → Filter.
#[component]
pub(crate) fn TermSoundPicker(id: String) -> impl IntoView {
    let term_settings = use_setting(TERM_NOTIFY, BTreeMap::<String, NotifyEvent>::new());
    let categories = use_setting(CATEGORIES, Vec::<TermCategory>::new());
    let choices = use_setting(SOUND_CHOICE, BTreeMap::<String, String>::new());
    let master = use_setting(MASTER, 70.0);
    let sounds = use_custom_sounds();
    let entries = sound_entries(vec![(SOUND_OFF, tr("Kein Ton", "No sound"))], sounds);
    let key = format!("term:{id}");
    let current = {
        let id = id.clone();
        Memo::new(move |_| term_settings.get().get(&id).copied().unwrap_or(TERM_DEFAULT))
    };
    let chosen = {
        let key = key.clone();
        Memo::new(move |_| choices.get().get(&key).filter(|id| sounds.get().iter().any(|s| &s.id == *id)).cloned())
    };
    let value = Signal::derive(move || if current.get().sound { chosen.get().unwrap_or_else(|| DEFAULT_SOUND.into()) } else { SOUND_OFF.into() });
    let (change_id, change_key) = (id.clone(), key.clone());
    view! {
        <Select
            id=format!("term-sound-{id}")
            size="sm"
            class="w-40"
            aria_label=tr("Sound der Kategorie", "Sound of the category")
            value=value
            entries=entries
            on_change=Callback::new(move |choice: String| {
                let existing: Vec<String> = categories.get_untracked().into_iter().map(|c| c.id).collect();
                let sound = choice != SOUND_OFF;
                let next = NotifyEvent { sound, ..current.get_untracked() };
                let id = change_id.clone();
                update_setting::<BTreeMap<String, NotifyEvent>>(TERM_NOTIFY, move |current| {
                    let mut map: BTreeMap<String, NotifyEvent> = current.into_iter().filter(|(k, _)| existing.contains(k)).collect();
                    map.insert(id.clone(), next);
                    map
                });
                if sound {
                    save_sound_choice(change_key.clone(), (choice != DEFAULT_SOUND).then_some(choice));
                }
            })
        />
        <Button variant="ghost" size="icon-sm" aria_label=tr("Probehören", "Preview") disabled=Signal::derive(move || !current.get().sound)
            on_click=Callback::new(move |_| play(chosen.get_untracked(), master.get_untracked() / 100.0 * current.get_untracked().volume / 100.0))>
            {icons::play("")}
        </Button>
    }
}

/// Entfernt gewählte Sounds, deren Name oder Kategorie es nicht mehr gibt.
pub(crate) fn prune_sound_choices(prefix: &'static str, keep: Vec<String>) {
    update_setting::<BTreeMap<String, String>>(SOUND_CHOICE, move |current| {
        current.into_iter().filter(|(key, _)| key.strip_prefix(prefix).is_none_or(|rest| keep.iter().any(|k| k == rest))).collect()
    });
}

#[component]
fn ExtraNamesField() -> impl IntoView {
    let names = use_setting(EXTRA_NAMES, Vec::<String>::new());
    let draft = RwSignal::new(names.get_untracked().join("\n"));
    Effect::new(move |_| draft.set(names.get().join("\n")));
    view! {
        <textarea
            id="mention-extra-names"
            data-slot="textarea"
            class=TEXTAREA_CLASS
            rows="3"
            placeholder=tr("Ein Name pro Zeile", "One name per line")
            prop:value=move || draft.get()
            on:input=move |event| draft.set(event_target_value(&event))
            on:blur=move |_| {
                let next: Vec<String> = parse_terms(&draft.get_untracked())
                    .into_iter()
                    .map(|name| name.trim_start_matches('@').chars().take(50).collect::<String>())
                    .filter(|name| !name.is_empty())
                    .take(50)
                    .collect();
                if next != names.get_untracked() {
                    // Sounds entfernter Namen mit entfernen.
                    let keep = next.iter().map(|n| multiyap_core::notify::mention_sound_key(n).trim_start_matches("mention:").to_string()).collect();
                    save_setting(EXTRA_NAMES, &next);
                    prune_sound_choices("mention:", keep);
                } else {
                    draft.set(names.get_untracked().join("\n"))
                }
            }
        ></textarea>
    }
}

/// Knopf je Kanal in den Listen: Stummschaltung aufheben oder Live-Meldung wieder einschalten.
#[derive(Clone, Copy)]
enum ChannelAction {
    Unmute,
    LiveNotify,
}

fn channel_list(
    list: ReadSignal<Vec<String>>,
    empty: &'static str,
    button: &'static str,
    action: ChannelAction,
) -> impl IntoView {
    view! {
        {move || list.get().is_empty().then(|| view! { <p class="py-3 text-sm text-muted-foreground">{empty}</p> })}
        <For each=move || list.get() key=|login| login.clone() let:login>
            {
                let target = login.clone();
                view! {
                    <div class="flex items-center justify-between gap-2 py-3">
                        <span class="text-sm">{login}</span>
                        <Button variant="outline" size="sm" on_click=Callback::new(move |_| {
                            let target = target.clone();
                            leptos::task::spawn_local(async move {
                                match action {
                                    ChannelAction::Unmute => multiyap_web::notifications::toggle_muted(&target).await,
                                    ChannelAction::LiveNotify => multiyap_web::notifications::toggle_live_notify(&target).await,
                                }
                            });
                        })>{button}</Button>
                    </div>
                }
            }
        </For>
    }
}

#[component]
pub fn NotificationsSection() -> impl IntoView {
    let own_name = use_setting(OWN_NAME, true);
    let master = use_setting(MASTER, 70.0);
    let live = use_setting(LIVE, SoundEvent { sound: true, volume: 100.0 });
    let live_blink = use_setting(LIVE_BLINK, true);
    let mention = use_setting(MENTION, NotifyEvent { sound: true, blink: true, volume: 100.0 });
    let categories = use_setting(CATEGORIES, Vec::<TermCategory>::new());
    let term_settings = use_setting(TERM_NOTIFY, BTreeMap::<String, NotifyEvent>::new());
    let muted = use_setting(MUTED, Vec::<String>::new());
    let live_off = use_setting(LIVE_OFF, Vec::<String>::new());
    let sounds = use_custom_sounds();

    // Ton und Blinken einer Kategorie; Einträge gelöschter Kategorien werden dabei mit entfernt.
    let save_term = move |id: String, value: NotifyEvent| {
        let existing: Vec<String> = categories.get_untracked().into_iter().map(|c| c.id).collect();
        update_setting::<BTreeMap<String, NotifyEvent>>(TERM_NOTIFY, move |current| {
            let mut next: BTreeMap<String, NotifyEvent> = current.into_iter().filter(|(k, _)| existing.contains(k)).collect();
            next.insert(id.clone(), value);
            next
        });
    };
    let category_ids = Memo::new(move |_| categories.get().into_iter().map(|c| c.id).collect::<Vec<_>>());

    view! {
        <div class="flex flex-col gap-8">
            <SettingGroup title=tr("Ton und Blinken", "Sound and blinking")>
                <SettingRow label=tr("Gesamtlautstärke", "Overall volume")>
                    <VolumeSlider label=tr("Gesamtlautstärke", "Overall volume") value=master on_commit=Callback::new(|value: f64| save_setting(MASTER, &value)) />
                </SettingRow>
                <EventRow
                    id="notify-live".into()
                    label=tr("Kanal geht live", "Channel goes live").to_string()
                    sound_key="live".into()
                    sound=live
                    on_sound=Callback::new(|next: SoundEvent| save_setting(LIVE, &next))
                    blink=live_blink
                    on_blink=Callback::new(|checked: bool| save_setting(LIVE_BLINK, &checked))
                    sounds=sounds
                />
                <EventRow
                    id="notify-mention".into()
                    label=tr("Erwähnung", "Mention").to_string()
                    sound_key="mention".into()
                    sound=Signal::derive(move || { let m = mention.get(); SoundEvent { sound: m.sound, volume: m.volume } })
                    on_sound=Callback::new(move |next: SoundEvent| save_setting(MENTION, &NotifyEvent { sound: next.sound, volume: next.volume, ..mention.get_untracked() }))
                    blink=Signal::derive(move || mention.get().blink)
                    on_blink=Callback::new(move |blink: bool| save_setting(MENTION, &NotifyEvent { blink, ..mention.get_untracked() }))
                    sounds=sounds
                />
            </SettingGroup>

            <SettingGroup title="Filter" description=tr("Pro Kategorie, standardmäßig aus. Der Tab blinkt in ihrer Farbe.", "Per category, off by default. The tab blinks in its color.")>
                {move || category_ids.get().is_empty().then(|| view! { <p class="py-3 text-sm text-muted-foreground">{tr("Noch keine Kategorien. Anlegen unter „Filter“.", "No categories yet. Create them under “Filters”.")}</p> })}
                <For each=move || category_ids.get() key=|id| id.clone() let:id>
                    {
                        let lookup = id.clone();
                        let category = Memo::new(move |_| categories.get().into_iter().find(|c| c.id == lookup));
                        let value_id = id.clone();
                        let value = Memo::new(move |_| term_settings.get().get(&value_id).copied().unwrap_or(TERM_DEFAULT));
                        let (sound_id, blink_id) = (id.clone(), id.clone());
                        view! {
                            <EventRow
                                id=format!("notify-term-{id}")
                                label=Signal::derive(move || category.get().map(|c| c.name).unwrap_or_default())
                                swatch=Signal::derive(move || term_swatch(&category.get().map(|c| c.color).unwrap_or_default()))
                                sound_key=format!("term:{id}")
                                sound=Signal::derive(move || { let v = value.get(); SoundEvent { sound: v.sound, volume: v.volume } })
                                on_sound=Callback::new(move |next: SoundEvent| save_term(sound_id.clone(), NotifyEvent { sound: next.sound, volume: next.volume, ..value.get_untracked() }))
                                blink=Signal::derive(move || value.get().blink)
                                on_blink=Callback::new(move |blink: bool| save_term(blink_id.clone(), NotifyEvent { blink, ..value.get_untracked() }))
                                sounds=sounds
                            />
                        }
                    }
                </For>
            </SettingGroup>

            <CustomSoundsGroup sounds=sounds />

            <SettingGroup title=tr("Stumme Kanäle", "Muted channels") description=tr("Kein Ton, kein Blinken. Auch über das ⋮-Menü des Tabs.", "No sound, no blinking. Also via the tab’s ⋮ menu.")>
                {channel_list(muted, tr("Kein Kanal stummgeschaltet.", "No channel muted."), tr("Stummschaltung aufheben", "Unmute"), ChannelAction::Unmute)}
                <AddMutedChannel full=Signal::derive(move || muted.get().len() >= MAX_MUTED_CHANNELS) muted=muted />
            </SettingGroup>

            <SettingGroup title=tr("Kanäle ohne Live-Benachrichtigung", "Channels without live notification") description=tr("Kein Ton und kein Pulsieren beim Live-Gehen. Ausschalten über das ⋮-Menü des Tabs.", "No sound and no pulsing when going live. Turn off via the tab’s ⋮ menu.")>
                {channel_list(live_off, tr("Alle Kanäle melden, wenn sie live gehen.", "All channels notify when they go live."), tr("Wieder einschalten", "Turn on again"), ChannelAction::LiveNotify)}
            </SettingGroup>

            <SettingGroup title=tr("Erwähnungen", "Mentions") description=tr("Farbig hinterlegt mit Strich auf der Scroll-Leiste. Ganze Wörter, eigene Nachrichten zählen nicht.", "Colored background with a mark on the scroll bar. Whole words, your own messages do not count.")>
                <SettingRow label=tr("Bei eigenem Namen", "For your own name") description=tr("Mit oder ohne @, auch Antworten auf dich.", "With or without @, also replies to you.") html_for="mention-own-name">
                    <Switch id="mention-own-name" checked=own_name on_change=Callback::new(|checked: bool| save_setting(OWN_NAME, &checked)) />
                </SettingRow>
                <div class="flex flex-col gap-2 py-3">
                    <label for="mention-extra-names" class="text-sm font-medium">{tr("Weitere Namen und Spitznamen", "More names and nicknames")}</label>
                    <ExtraNamesField />
                    <NameSounds />
                </div>
            </SettingGroup>
        </div>
    }
}
