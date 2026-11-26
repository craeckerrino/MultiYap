//! Tab-Einträge in der Tab-Leiste in Leptos (Nachbau von tab-item.tsx, stream-tab-item.tsx, highlights-tab-item.tsx,
//! chat-modes-menu.tsx, change-channel-dialog.tsx): Name mit Stream-Infos beim Drüberfahren, Doppelklick benennt um,
//! Schwert in Mod-Kanälen, Live-Punkt und Blinken, Chat-Modi, Tab-Menü, Mittelklick schließt, Ziehen verschiebt. Die
//! Daten kommen direkt aus den Diensten in crates/web.

use multiyap_core::i18n::tr;
use multiyap_core::channel_input;
use multiyap_core::chat_modes::{follower_options, ChatModes};
use multiyap_core::streams as streams_core;
use multiyap_web::workspace as web;
use multiyap_core::youtube as youtube_core;
use multiyap_web::{chat_tools, highlights, moderation, notifications, sessions, streams, youtube};
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use serde_json::Value;
use wasm_bindgen::JsCast;
use web_sys::{HtmlElement, KeyboardEvent};

use crate::bridge::use_setting;
use crate::chat::layer::{dismiss_on_outside, place, show_popover, Layer, LayerView, Rect, Side, POPOVER_CONTENT, POPOVER_RESET};
use crate::components::button::button_class;
use crate::components::dialog::{Dialog, DIALOG_FOOTER, LABEL};
use crate::components::icons;
use crate::components::input::INPUT_CLASS;
use crate::components::overlay::{DropdownMenu, MenuEntry};
use crate::components::{Select, SelectEntry, Switch};
use crate::layout::{run, TAB_DRAG_TYPE};
use crate::show::ShowDialog;
use crate::store::{encode, open_url, session_version, watch_version};
use crate::workspace::{workspace_ctx, TabInfo, TabStatus};

const SLOW_OPTIONS: [u32; 7] = [3, 5, 10, 20, 30, 60, 120];
const DELAY_OPTIONS: [u32; 3] = [2, 4, 6];

/// Änderung eines Chat-Modus (wie ChatModesPatch in chat-modes.ts).
#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct Patch {
    #[serde(skip_serializing_if = "Option::is_none")]
    slow: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    slow_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    emote_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    follower_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    follower_minutes: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    subscriber_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unique_chat: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delay: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delay_seconds: Option<u32>,
}

fn mode_row(label: &'static str, control: impl IntoView + 'static) -> impl IntoView {
    view! {
        <div class="flex items-center justify-between gap-3 py-1.5">
            <span class="text-sm">{label}</span>
            <div class="flex items-center gap-2">{control}</div>
        </div>
    }
}

/// Bedienung der Chat-Modi: Slow, Emote-Only, Follower-Only, Sub-Only, Unique-Chat und Chat-Verzögerung. Änderungen
/// gelten sofort. Nur mit Mod-Rechten (der Aufrufer prüft zentral, die Sitzung erneut).
#[component]
fn ChatModesMenu(channel: Memo<String>, modes: Memo<Option<ChatModes>>) -> impl IntoView {
    let busy = RwSignal::new(false);
    let initial = modes.get_untracked().unwrap_or_default();
    let slow_seconds = RwSignal::new(initial.slow_seconds.unwrap_or(30));
    let follower_minutes = RwSignal::new(initial.follower_minutes.unwrap_or(0));
    let delay_seconds = RwSignal::new(initial.delay_seconds.unwrap_or(2));
    let apply = move |patch: Patch| {
        let Some(session) = sessions::session(&channel.get_untracked()) else { return };
        let patch = serde_json::to_value(&patch).unwrap_or(Value::Null);
        busy.set(true);
        spawn_local(async move {
            if let Err(message) = session.update_chat_modes(&patch).await {
                chat_tools::toast("error", &multiyap_core::tf!("Chat-Modus nicht geändert: {message}", "Chat mode not changed: {message}"));
            }
            busy.try_set(false);
        });
    };
    let disabled = Signal::derive(move || busy.get() || modes.with(Option::is_none));
    let m = move || modes.get().unwrap_or_default();
    let switch = move |label: &'static str, checked: fn(&ChatModes) -> bool, on: fn(bool) -> Patch| {
        view! {
            <Switch id=format!("mode-{label}") aria_label=label checked=Signal::derive(move || checked(&m())) disabled=disabled
                on_change=Callback::new(move |value: bool| apply(on(value))) />
        }
    };
    view! {
        <div class="flex flex-col">
            <p class="pb-1 text-sm font-medium">{tr("Chat-Modi", "Chat modes")}</p>
            {mode_row("Slow", view! {
                <Select id="mode-slow" size="sm" class="w-24" aria_label=tr("Wartezeit", "Wait time") disabled=disabled
                    value=Signal::derive(move || { let m = m(); if m.slow { m.slow_seconds.unwrap_or(slow_seconds.get()) } else { slow_seconds.get() }.to_string() })
                    entries=Signal::derive(|| SLOW_OPTIONS.iter().map(|s| SelectEntry::item(s.to_string(), format!("{s}s"))).collect::<Vec<_>>())
                    on_change=Callback::new(move |value: String| {
                        let seconds = value.parse().unwrap_or(30);
                        slow_seconds.set(seconds);
                        if m().slow { apply(Patch { slow: Some(true), slow_seconds: Some(seconds), ..Default::default() }) }
                    }) />
                <Switch id="mode-slow-switch" aria_label="Slow" checked=Signal::derive(move || m().slow) disabled=disabled
                    on_change=Callback::new(move |on: bool| apply(if on {
                        Patch { slow: Some(true), slow_seconds: Some(slow_seconds.get_untracked()), ..Default::default() }
                    } else {
                        Patch { slow: Some(false), ..Default::default() }
                    })) />
            })}
            {mode_row("Emote-Only", switch("Emote-Only", |m| m.emote_only, |on| Patch { emote_only: Some(on), ..Default::default() }))}
            {mode_row("Follower-Only", view! {
                <Select id="mode-follower" size="sm" class="w-28" aria_label=tr("Mindest-Folgedauer", "Minimum follow time") disabled=disabled
                    value=Signal::derive(move || { let m = m(); if m.follower_only { m.follower_minutes.unwrap_or(follower_minutes.get()) } else { follower_minutes.get() }.to_string() })
                    entries=Signal::derive(|| follower_options().into_iter().map(|(minutes, label)| SelectEntry::item(minutes.to_string(), label)).collect::<Vec<_>>())
                    on_change=Callback::new(move |value: String| {
                        let minutes = value.parse().unwrap_or(0);
                        follower_minutes.set(minutes);
                        if m().follower_only { apply(Patch { follower_only: Some(true), follower_minutes: Some(minutes), ..Default::default() }) }
                    }) />
                <Switch id="mode-follower-switch" aria_label="Follower-Only" checked=Signal::derive(move || m().follower_only) disabled=disabled
                    on_change=Callback::new(move |on: bool| apply(if on {
                        Patch { follower_only: Some(true), follower_minutes: Some(follower_minutes.get_untracked()), ..Default::default() }
                    } else {
                        Patch { follower_only: Some(false), ..Default::default() }
                    })) />
            })}
            {mode_row("Sub-Only", switch("Sub-Only", |m| m.subscriber_only, |on| Patch { subscriber_only: Some(on), ..Default::default() }))}
            {mode_row("Unique-Chat", switch("Unique-Chat", |m| m.unique_chat, |on| Patch { unique_chat: Some(on), ..Default::default() }))}
            {mode_row(tr("Chat-Verzögerung", "Chat delay"), view! {
                <Select id="mode-delay" size="sm" class="w-24" aria_label=tr("Verzögerung", "Delay") disabled=disabled
                    value=Signal::derive(move || { let m = m(); if m.delay == Some(true) { m.delay_seconds.unwrap_or(delay_seconds.get()) } else { delay_seconds.get() }.to_string() })
                    entries=Signal::derive(|| DELAY_OPTIONS.iter().map(|s| SelectEntry::item(s.to_string(), format!("{s}s"))).collect::<Vec<_>>())
                    on_change=Callback::new(move |value: String| {
                        let seconds = value.parse().unwrap_or(2);
                        delay_seconds.set(seconds);
                        if m().delay == Some(true) { apply(Patch { delay: Some(true), delay_seconds: Some(seconds), ..Default::default() }) }
                    }) />
                <Switch id="mode-delay-switch" aria_label=tr("Chat-Verzögerung", "Chat delay") checked=Signal::derive(move || m().delay == Some(true)) disabled=disabled
                    on_change=Callback::new(move |on: bool| apply(if on {
                        Patch { delay: Some(true), delay_seconds: Some(delay_seconds.get_untracked()), ..Default::default() }
                    } else {
                        Patch { delay: Some(false), ..Default::default() }
                    })) />
            })}
        </div>
    }
}

/// „Kanal wechseln“: Tab bleibt an seinem Platz, eigener Anzeigename entfällt.
#[component]
fn ChangeChannelDialog(tab_id: String, channel: Memo<String>, open: RwSignal<bool>) -> impl IntoView {
    let tab_id = StoredValue::new(tab_id);
    let input = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    Effect::new(move |_| {
        if !open.get() {
            input.set(String::new());
            error.set(None);
        }
    });
    let current = Signal::derive(move || multiyap_core::tf!("Aktuell: {}", "Current: {}", channel.get()));
    view! {
        <Dialog open=open title=tr("Kanal wechseln", "Switch channel") description=current max_width="sm:max-w-md">
            <form class="flex flex-col gap-4" on:submit=move |event: web_sys::SubmitEvent| {
                event.prevent_default();
                match channel_input::twitch_channel(&input.get_untracked()) {
                    Some(next) => {
                        let id = tab_id.get_value();
                        run(async move { web::change_tab_channel(&id, &next).await });
                        open.set(false);
                    }
                    None => error.set(Some(tr("Bitte einen gültigen Twitch-Kanalnamen oder Link eingeben.", "Please enter a valid Twitch channel name or link.").into())),
                }
            }>
                <div class="flex flex-col gap-2">
                    <label data-slot="label" for="change-channel-input" class=LABEL>{tr("Neuer Kanalname oder Link", "New channel name or link")}</label>
                    <input id="change-channel-input" data-slot="input" class=INPUT_CLASS autofocus autocomplete="off" spellcheck="false"
                        aria-invalid=move || error.with(Option::is_some).then_some("true")
                        prop:value=move || input.get()
                        on:input=move |event| {
                            input.set(event_target_value(&event));
                            error.set(None);
                        } />
                    {move || error.get().map(|e| view! { <p class="text-sm text-destructive">{e}</p> })}
                </div>
                <div data-slot="dialog-footer" class=DIALOG_FOOTER>
                    <button type="submit" data-slot="button" class=button_class("default", "default", "") disabled=move || input.with(|i| i.trim().is_empty())>
                        {tr("Wechseln", "Switch")}
                    </button>
                </div>
            </form>
        </Dialog>
    }
}

fn channel_url(login: &str) -> String {
    format!("https://www.twitch.tv/{}", encode(login))
}

/// Ein Kanal-Tab in der Tab-Leiste. Daten aus den Diensten in Rust: Sitzung (Name, Chat-Modi), Mod-Rechte (zentrale
/// Prüfung), Stream-Status, Blinken und ungesehene Zeilen.
#[component]
pub fn ChannelTabItem(tab: Signal<TabInfo>, area_id: String, #[prop(into)] active: Signal<bool>, #[prop(into)] draggable: Signal<bool>) -> impl IntoView {
    let ctx = workspace_ctx();
    let tab_id = StoredValue::new(tab.with_untracked(|t| t.id.clone()));
    let area_id = StoredValue::new(area_id);
    let channel = Memo::new(move |_| tab.with(|t| t.channel.clone()));
    let session_version = session_version(channel.into());
    let session_channel = Memo::new(move |_| {
        session_version.track();
        sessions::session(&channel.get()).and_then(|s| s.channel())
    });
    let channel_id = Memo::new(move |_| session_channel.with(|c| c.as_ref().map(|c| c.id.clone())));
    let name = Memo::new(move |_| {
        tab.with(|t| t.nickname.clone()).or_else(|| session_channel.with(|c| c.as_ref().map(|c| c.display_name.clone()))).unwrap_or_else(|| channel.get())
    });
    let modes = Memo::new(move |_| {
        session_version.track();
        sessions::session(&channel.get()).and_then(|s| s.chat_modes()).and_then(|v| serde_json::from_value::<ChatModes>(v).ok())
    });
    let mod_version = watch_version(moderation::subscribe_mod, moderation::unsubscribe_mod);
    let is_mod = Memo::new(move |_| {
        mod_version.track();
        channel_id.with(|id| id.as_deref().is_some_and(moderation::can_moderate))
    });
    let streams_version = watch_version(streams::subscribe, streams::unsubscribe);
    let stream = Memo::new(move |_| {
        streams_version.track();
        channel_id.with(|id| id.as_deref().map(streams::info)).unwrap_or_default()
    });
    let live = Memo::new(move |_| stream.with(|s| s.live));
    let points = use_setting("tabs.streamInfo", Value::Null);
    let live_blink = use_setting("tabs.liveBlink", true);
    let muted_list = use_setting("notifications.mutedChannels", Vec::<String>::new());
    let live_off_list = use_setting("notifications.liveOffChannels", Vec::<String>::new());
    let muted = Memo::new(move |_| muted_list.with(|list| list.contains(&channel.get())));
    let live_off = Memo::new(move |_| live_off_list.with(|list| list.contains(&channel.get())));
    let attention_version = watch_version(notifications::subscribe_attention, notifications::unsubscribe_attention);
    let attention = Memo::new(move |_| {
        attention_version.track();
        notifications::attention(&channel.get())
    });
    let unread_version = watch_version(notifications::subscribe_unread, notifications::unsubscribe_unread);
    // Ungesehene neue Zeilen: Name weiß und fett, stumme Kanäle nie.
    let unread = Memo::new(move |_| {
        unread_version.track();
        notifications::is_unread(&channel.get()) && !active.get() && !muted.get()
    });
    // Pulsieren, bis man hineinklickt; ist der Tab schon sichtbar, kein Blinken. Erwähnung und Begriff haben eigene
    // Farben und gehen vor Live. Stumme Kanäle blinken nicht, Kanäle ohne Live-Benachrichtigung pulsieren nicht beim
    // Live-Gehen.
    let blink_class = Memo::new(move |_| {
        if muted.get() || active.get() {
            return String::new();
        }
        attention
            .with(|a| a.as_ref().map(|a| a.class_name.clone()))
            .or_else(|| (stream.with(|s| s.went_live) && live_blink.get() && !live_off.get()).then(|| "bg-live/25".to_string()))
            .unwrap_or_default()
    });

    // Zustand für den Seiten-Tab oben.
    ctx.publish_status(tab_id.get_value(), move || TabStatus { name: name.get(), live: live.get(), unread: unread.get(), blink: blink_class.get() });

    // Sichtbar: Pulsieren, Blinken und „ungesehen“ enden.
    Effect::new(move |_| {
        if active.get() && stream.with(|s| s.went_live) {
            if let Some(id) = channel_id.get() {
                streams::acknowledge_live(&id);
            }
        }
    });
    Effect::new(move |_| {
        if active.get() && attention.with(Option::is_some) {
            notifications::clear_attention(&channel.get_untracked());
        }
    });
    Effect::new(move |_| {
        if active.get() {
            notifications::clear_unread(&channel.get_untracked());
        }
    });

    // Anzeigenamen der Kanäle im Shared Chat (für die Sitzung zwischengespeichert in crates/web/src/api.rs).
    let shared_ids = Memo::new(move |_| {
        let wanted = points.with(|p| p.get("sharedWith").and_then(Value::as_bool).unwrap_or(true));
        stream.with(|s| if wanted && s.live { s.shared_with.clone() } else { Vec::new() })
    });
    let shared_names = RwSignal::new(Vec::<String>::new());
    Effect::new(move |_| {
        let ids = shared_ids.get();
        if ids.is_empty() {
            shared_names.set(Vec::new());
            return;
        }
        spawn_local(async move {
            let mut names = Vec::new();
            for id in &ids {
                if let Ok(Some(user)) = multiyap_web::api::get_user_by_id(id).await {
                    if let Some(name) = user.get("display_name").and_then(Value::as_str) {
                        names.push(name.to_string());
                    }
                }
            }
            if shared_ids.try_get_untracked().as_ref() == Some(&ids) {
                shared_names.try_set(names);
            }
        });
    });
    let info_lines = move || {
        let lines = stream.with_untracked(|s| streams_core::info_lines(s, &points.get_untracked(), &channel.get_untracked(), &shared_names.get_untracked(), js_sys::Date::now()));
        lines.into_iter().map(|l| (l.label.map(str::to_string), l.value)).collect::<Vec<_>>()
    };

    let layer = Layer::new();
    let renaming = RwSignal::new(false);
    let draft = RwSignal::new(String::new());
    let rename_ref = NodeRef::<leptos::html::Input>::new();
    // Wie autoFocus in React: das neu eingefügte Feld bekommt den Fokus.
    Effect::new(move |_| {
        if let Some(input) = rename_ref.get() {
            let _ = input.focus();
        }
    });
    let change_open = RwSignal::new(false);
    let modes_open = RwSignal::new(false);
    let modes_trigger = NodeRef::<leptos::html::Button>::new();
    let modes_content = NodeRef::<leptos::html::Div>::new();
    dismiss_on_outside(move || modes_open.get(), move || modes_open.set(false));
    Effect::new(move |_| {
        if !modes_open.get() {
            return;
        }
        if let (Some(content), Some(trigger)) = (modes_content.get(), modes_trigger.get()) {
            let content: HtmlElement = content.into();
            show_popover(&content);
            place(&content, Rect::of(&trigger), Side::Below, 4.0);
        }
    });

    let class = move || {
        let mut class = String::from("group/tab flex h-full shrink-0 items-center gap-0.5 border-b-2 pl-3 pr-1 text-sm");
        class.push_str(if active.get() {
            " border-primary text-foreground"
        } else if unread.get() {
            // Ungelesen: heller Unterstrich und heller Name.
            " border-foreground/60 text-foreground font-semibold"
        } else {
            " border-transparent text-muted-foreground hover:text-foreground"
        });
        let blink = blink_class.get();
        if !blink.is_empty() {
            // Pulsieren, bis man hineinklickt; die Farbe der Ursache geht dem Live-Schimmer vor.
            class.push_str(&format!(" animate-pulse {blink}"));
        } else if live.get() {
            class.push_str(" bg-live/10");
        }
        class
    };

    let activate = move || {
        let id = tab_id.get_value();
        run(async move { web::activate_tab(&id).await });
        if let Some(id) = channel_id.get_untracked() {
            streams::acknowledge_live(&id);
        }
        notifications::clear_attention(&channel.get_untracked());
    };
    let close = move || ctx.request_close(tab_id.get_value(), name.get_untracked());
    let finish_rename = move |event: KeyboardEvent| match event.key().as_str() {
        "Enter" => {
            event.prevent_default();
            // Leerer Name oder Kanalname: eigener Anzeigename entfällt.
            let value = draft.get_untracked().trim().to_string();
            let reset = value.is_empty() || value.to_lowercase() == channel.get_untracked();
            let id = tab_id.get_value();
            run(async move { web::rename_tab(&id, (!reset).then_some(value)).await });
            renaming.set(false);
        }
        "Escape" => {
            event.prevent_default();
            renaming.set(false);
        }
        _ => {}
    };

    let item = |icon: fn() -> AnyView, label: &str, on_select: Callback<()>| MenuEntry::IconItem { icon, label: label.to_string(), on_select };
    let entries = Signal::derive(move || {
        let mut list = vec![
            item(|| icons::external_link("").into_any(), tr("Stream im Browser öffnen", "Open stream in the browser"), Callback::new(move |_| open_url(&channel_url(&channel.get_untracked()), "noopener,noreferrer"))),
            item(
                || icons::tv("").into_any(),
                tr("Player öffnen", "Open player"),
                Callback::new(move |_| {
                    let (login, area) = (channel.get_untracked(), area_id.get_value());
                    run(async move { web::open_player_above(&login, area).await });
                }),
            ),
        ];
        if is_mod.get() {
            list.push(item(
                || icons::shield("").into_any(),
                tr("Mod-Ansicht öffnen", "Open mod view"),
                Callback::new(move |_| open_url(&format!("https://www.twitch.tv/moderator/{}", encode(&channel.get_untracked())), "noopener,noreferrer")),
            ));
        }
        list.push(item(|| icons::replace("").into_any(), tr("Kanal wechseln", "Switch channel"), Callback::new(move |_| change_open.set(true))));
        list.push(item(|| icons::copy("").into_any(), tr("Kanal-Link kopieren", "Copy channel link"), Callback::new(move |_| spawn_local(chat_tools::copy_text(channel_url(&channel.get_untracked()))))));
        if is_mod.get() {
            // Erst nach dem Schließen des Menüs öffnen.
            list.push(item(|| icons::sliders_horizontal("").into_any(), tr("Chat-Modi", "Chat modes"), Callback::new(move |_| request_animation_frame(move || modes_open.set(true)))));
        }
        let toggle_muted = Callback::new(move |_| spawn_local(async move { notifications::toggle_muted(&channel.get_untracked()).await }));
        list.push(if muted.get() {
            item(|| icons::bell("").into_any(), tr("Stummschaltung aufheben", "Unmute"), toggle_muted)
        } else {
            item(|| icons::bell_off("").into_any(), tr("Kanal stummschalten", "Mute channel"), toggle_muted)
        });
        list.push(item(
            || icons::radio("").into_any(),
            if live_off.get() { tr("Live-Benachrichtigung einschalten", "Turn on live notification") } else { tr("Live-Benachrichtigung ausschalten", "Turn off live notification") },
            Callback::new(move |_| spawn_local(async move { notifications::toggle_live_notify(&channel.get_untracked()).await })),
        ));
        list.push(item(
            || icons::refresh_cw("").into_any(),
            tr("Neu verbinden", "Reconnect"),
            Callback::new(move |_| {
                if let Some(session) = sessions::session(&channel.get_untracked()) {
                    session.reconnect();
                }
            }),
        ));
        list.push(MenuEntry::Separator);
        list.push(item(|| icons::x("").into_any(), tr("Tab schließen", "Close tab"), Callback::new(move |_| close())));
        list
    });

    view! {
        <div role="tab" aria-selected=move || active.get().to_string() data-tab-id=move || tab_id.get_value()
            draggable=move || (draggable.get() && !renaming.get()).to_string()
            class=class
            on:dragstart=move |event: web_sys::DragEvent| {
                if let Some(data) = event.data_transfer() {
                    let _ = data.set_data(TAB_DRAG_TYPE, &tab_id.get_value());
                    data.set_effect_allowed("move");
                }
                ctx.start_drag(tab_id.get_value());
            }
            on:dragend=move |_| ctx.end_drag()
            // Mittelklick: kein automatisches Scrollen des Browsers; schließt den Tab.
            on:mousedown=|event: web_sys::MouseEvent| if event.button() == 1 { event.prevent_default() }
            on:auxclick=move |event: web_sys::MouseEvent| if event.button() == 1 { close() }>
            {icons::twitch_logo("mr-1.5 size-3.5 shrink-0 opacity-70")}
            {move || is_mod.get().then(|| view! { <span class="contents" aria-label=tr("Mod-Rechte in diesem Kanal", "Mod rights in this channel")>{icons::sword("mr-1 size-3.5 shrink-0 fill-primary text-primary")}</span> })}
            {move || live.get().then(|| view! { <span class="mr-1 size-2 shrink-0 rounded-full bg-live" aria-label="Live"></span> })}
            {move || if renaming.get() {
                view! {
                    <input node_ref=rename_ref data-slot="input" class=INPUT_CLASS.replacen("h-8 w-full", "h-7 w-40", 1) maxlength="50" aria-label=tr("Anzeigename des Tabs", "Display name of the tab")
                        prop:value=move || draft.get() on:input=move |event| draft.set(event_target_value(&event))
                        on:keydown=finish_rename on:blur=move |_| renaming.set(false) />
                }.into_any()
            } else {
                view! {
                    <button type="button" class="max-w-48 truncate py-1 outline-none focus-visible:underline"
                        on:pointerenter=move |event: web_sys::PointerEvent| {
                            if let Some(element) = event.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                                layer.show_lines(&element, info_lines());
                            }
                        }
                        on:pointerleave=move |_| layer.hide_tip()
                        on:click=move |_| activate()
                        on:dblclick=move |_| {
                            layer.hide_tip();
                            draft.set(name.get_untracked());
                            renaming.set(true);
                        }>
                        {move || name.get()}
                    </button>
                }.into_any()
            }}
            // Chat-Modi-Symbol im Tab (zusätzlich zum Menüpunkt), nur mit Mod-Rechten.
            {move || is_mod.get().then(|| view! {
                <button node_ref=modes_trigger type="button" data-slot="button" data-chat-overlay="" class=button_class("ghost", "icon-xs", "")
                    aria-label=tr("Chat-Modi", "Chat modes") title=tr("Chat-Modi", "Chat modes") aria-expanded=move || modes_open.get().to_string()
                    on:click=move |_| modes_open.update(|o| *o = !*o)>
                    {icons::sliders_horizontal("")}
                </button>
            })}
            {move || modes_open.get().then(|| view! {
                <div node_ref=modes_content popover="manual" data-chat-overlay="" data-slot="popover-content" data-state="open" data-side="bottom"
                    class=format!("{POPOVER_RESET} {}", POPOVER_CONTENT.replacen("w-72", "w-80", 1)) style="opacity: 0;">
                    <ChatModesMenu channel=channel modes=modes />
                </div>
            })}
            <DropdownMenu width_class="w-72" trigger_variant="ghost" trigger_size="icon-xs" aria_label=tr("Tab-Menü", "Tab menu") entries=entries>
                {icons::ellipsis_vertical("")}
            </DropdownMenu>
            <button type="button" data-slot="button" class=button_class("ghost", "icon-xs", "") aria-label=tr("Tab schließen", "Close tab") on:click=move |_| close()>
                {icons::x("")}
            </button>
            <ChangeChannelDialog tab_id=tab_id.get_value() channel=channel open=change_open />
            <LayerView layer=layer />
        </div>
    }
}

const COUNT_BADGE: &str = "group/badge inline-flex w-fit shrink-0 items-center justify-center gap-1 overflow-hidden rounded-4xl border border-transparent py-0.5 text-xs font-medium whitespace-nowrap transition-all focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 aria-invalid:border-destructive aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 [&>svg]:pointer-events-none [&>svg]:size-3! bg-primary text-primary-foreground [a]:hover:bg-primary/80 ml-1 h-4 min-w-4 px-1 tabular-nums";

/// Einfacher Tab in der Tab-Leiste (Hervorhebungen und Stream): Name, Doppelklick benennt um, Zähler ungesehener
/// Treffer, ⋮-Menü, Mittelklick schließt, Ziehen verschiebt. `menu`: eigene Einträge vor „Umbenennen“ und „Tab
/// schließen“. `on_rename` bekommt den eingegebenen Namen (ohne Leerraum am Rand) und entscheidet, ob der eigene Name
/// entfällt.
#[component]
fn SimpleTabItem(
    tab_id: String,
    #[prop(into)] name: Signal<String>,
    #[prop(into)] active: Signal<bool>,
    #[prop(into)] draggable: Signal<bool>,
    #[prop(optional)] tv_icon: bool,
    /// YouTube-Logo vor dem Namen (YouTube-Tabs), damit sie sich von Twitch-Tabs unterscheiden.
    #[prop(optional)] youtube_icon: bool,
    /// Live-Punkt und leichter Live-Schimmer wie bei Twitch-Kanälen (YouTube-Tabs).
    #[prop(optional, into)] live: Option<Signal<bool>>,
    /// Ungesehene neue Zeilen: Name weiß und fett wie bei Twitch-Kanälen (YouTube-Tabs).
    #[prop(optional, into)] unread: Option<Signal<bool>>,
    /// Klasse zum Pulsieren (Erwähnung, Filter, live gegangen); leer heißt kein Pulsieren (YouTube-Tabs).
    #[prop(optional, into)] blink: Option<Signal<String>>,
    #[prop(optional)] title: Option<String>,
    #[prop(into)] count: Signal<u32>,
    menu: Signal<Vec<MenuEntry>>,
    on_rename: Callback<String>,
) -> impl IntoView {
    let ctx = workspace_ctx();
    let tab_id = StoredValue::new(tab_id);
    let renaming = RwSignal::new(false);
    let draft = RwSignal::new(String::new());
    let rename_ref = NodeRef::<leptos::html::Input>::new();
    // Wie autoFocus in React: das neu eingefügte Feld bekommt den Fokus.
    Effect::new(move |_| {
        if let Some(input) = rename_ref.get() {
            let _ = input.focus();
        }
    });
    let start_rename = move || {
        draft.set(name.get_untracked());
        renaming.set(true);
    };
    let close = move || ctx.request_close(tab_id.get_value(), name.get_untracked());
    let activate = move || {
        let id = tab_id.get_value();
        run(async move { web::activate_tab(&id).await });
    };
    // Zustand für den Seiten-Tab oben; ungesehene Treffer zählen wie ungesehene Zeilen.
    ctx.publish_status(tab_id.get_value(), move || TabStatus {
        name: name.get(),
        live: live.is_some_and(|l| l.get()),
        unread: unread.is_some_and(|u| u.get()) || count.get() > 0,
        blink: blink.map(|b| b.get()).unwrap_or_default(),
    });
    let entries = Signal::derive(move || {
        let mut list = menu.get();
        list.push(MenuEntry::IconItem {
            icon: || icons::pencil("").into_any(),
            label: tr("Umbenennen", "Rename").into(),
            // Erst nach dem Schließen des Menüs (das Feld bekommt sonst keinen Fokus).
            on_select: Callback::new(move |_| request_animation_frame(start_rename)),
        });
        list.push(MenuEntry::Separator);
        list.push(MenuEntry::IconItem { icon: || icons::x("").into_any(), label: tr("Tab schließen", "Close tab").into(), on_select: Callback::new(move |_| close()) });
        list
    });
    view! {
        <div role="tab" aria-selected=move || active.get().to_string() data-tab-id=move || tab_id.get_value()
            draggable=move || (draggable.get() && !renaming.get()).to_string()
            title=title
            class=move || {
                let mut class = String::from("group/tab flex h-full shrink-0 items-center gap-0.5 border-b-2 pl-3 pr-1 text-sm");
                class.push_str(if active.get() {
                    " border-primary text-foreground"
                } else if unread.is_some_and(|u| u.get()) {
                    // Ungelesen: heller Unterstrich und heller Name.
                    " border-foreground/60 text-foreground font-semibold"
                } else {
                    " border-transparent text-muted-foreground hover:text-foreground"
                });
                let pulse = blink.map(|b| b.get()).unwrap_or_default();
                if !pulse.is_empty() {
                    class.push_str(&format!(" animate-pulse {pulse}"));
                } else if live.is_some_and(|l| l.get()) {
                    class.push_str(" bg-live/10");
                }
                class
            }
            on:dragstart=move |event: web_sys::DragEvent| {
                if let Some(data) = event.data_transfer() {
                    let _ = data.set_data(TAB_DRAG_TYPE, &tab_id.get_value());
                    data.set_effect_allowed("move");
                }
                ctx.start_drag(tab_id.get_value());
            }
            on:dragend=move |_| ctx.end_drag()
            on:mousedown=|event: web_sys::MouseEvent| if event.button() == 1 { event.prevent_default() }
            on:auxclick=move |event: web_sys::MouseEvent| if event.button() == 1 { close() }>
            {tv_icon.then(|| icons::tv("mr-1 size-3.5 shrink-0"))}
            {youtube_icon.then(|| icons::youtube_logo("mr-1.5 size-3.5 shrink-0 opacity-70"))}
            {move || live.is_some_and(|l| l.get()).then(|| view! { <span class="mr-1 size-2 shrink-0 rounded-full bg-live" aria-label="Live"></span> })}
            {move || if renaming.get() {
                view! {
                    <input node_ref=rename_ref data-slot="input" class=INPUT_CLASS.replacen("h-8 w-full", "h-7 w-40", 1) maxlength="50" aria-label=tr("Name des Tabs", "Name of the tab")
                        prop:value=move || draft.get() on:input=move |event| draft.set(event_target_value(&event))
                        on:keydown=move |event: KeyboardEvent| match event.key().as_str() {
                            "Enter" => {
                                event.prevent_default();
                                on_rename.run(draft.get_untracked().trim().to_string());
                                renaming.set(false);
                            }
                            "Escape" => {
                                event.prevent_default();
                                renaming.set(false);
                            }
                            _ => {}
                        }
                        on:blur=move |_| renaming.set(false) />
                }.into_any()
            } else {
                view! {
                    <button type="button" class="max-w-48 truncate py-1 outline-none focus-visible:underline"
                        on:click=move |_| activate()
                        on:dblclick=move |_| start_rename()>
                        {move || name.get()}
                    </button>
                }.into_any()
            }}
            {move || (count.get() > 0).then(|| {
                let n = count.get();
                view! {
                    <span data-slot="badge" data-variant="default" class=COUNT_BADGE aria-label=multiyap_core::tf!("{n} ungesehene Treffer", "{n} unseen matches")>
                        {if n > 99 { "99+".to_string() } else { n.to_string() }}
                    </span>
                }
            })}
            <DropdownMenu width_class="w-72" trigger_variant="ghost" trigger_size="icon-xs" aria_label=tr("Tab-Menü", "Tab menu") entries=entries>
                {icons::ellipsis_vertical("")}
            </DropdownMenu>
            <button type="button" data-slot="button" class=button_class("ghost", "icon-xs", "") aria-label=tr("Tab schließen", "Close tab") on:click=move |_| close()>
                {icons::x("")}
            </button>
        </div>
    }
}

/// Lädt den Player eines Stream-Tabs neu: Erneutes Setzen von `src` lädt den Rahmen nach HTML-Standard neu.
fn reload_stream(tab_id: &str) {
    let Ok(frames) = document().query_selector_all("iframe[data-stream-tab]") else {
        return;
    };
    for index in 0..frames.length() {
        let Some(frame) = frames.item(index).and_then(|node| node.dyn_into::<web_sys::Element>().ok()) else {
            continue;
        };
        if frame.get_attribute("data-stream-tab").as_deref() == Some(tab_id) {
            if let Some(src) = frame.get_attribute("src") {
                let _ = frame.set_attribute("src", &src);
            }
        }
    }
}

/// Stream-Tab in der Tab-Leiste: Bildschirm-Symbol vor dem Namen, ⋮-Menü mit „Stream neu laden“, „Stream im Browser
/// öffnen“ und „Player im eigenen Fenster öffnen“.
#[component]
pub fn StreamTabItem(tab: Signal<TabInfo>, #[prop(into)] active: Signal<bool>, #[prop(into)] draggable: Signal<bool>) -> impl IntoView {
    let channel = Memo::new(move |_| tab.with(|t| t.channel.clone()));
    // Anzeigename des Kanals (zum Beispiel „itsNax0“ statt „itsnax0“), für die Sitzung zwischengespeichert.
    let display_name = RwSignal::new(None::<(String, String)>);
    Effect::new(move |_| {
        let login = channel.get();
        spawn_local(async move {
            if let Ok(Some(user)) = multiyap_web::api::get_user_by_login(&login).await {
                if let Some(name) = user.get("display_name").and_then(Value::as_str) {
                    display_name.try_set(Some((login, name.to_string())));
                }
            }
        });
    });
    let name = Signal::derive(move || {
        tab.with(|t| t.nickname.clone()).unwrap_or_else(|| {
            let login = channel.get();
            display_name.with(|d| d.as_ref().filter(|(l, _)| *l == login).map(|(_, n)| n.clone())).unwrap_or(login)
        })
    });
    let reload_id = tab.with_untracked(|t| t.id.clone());
    let menu = Signal::derive(move || {
        let reload_id = reload_id.clone();
        vec![
            MenuEntry::IconItem {
                icon: || icons::refresh_cw("").into_any(),
                label: tr("Stream neu laden", "Reload stream").into(),
                on_select: Callback::new(move |_| reload_stream(&reload_id)),
            },
            MenuEntry::IconItem {
                icon: || icons::external_link("").into_any(),
                label: tr("Stream im Browser öffnen", "Open stream in the browser").into(),
                on_select: Callback::new(move |_| open_url(&channel_url(&channel.get_untracked()), "noopener,noreferrer")),
            },
            MenuEntry::IconItem {
                icon: || icons::app_window("").into_any(),
                label: tr("Player im eigenen Fenster öffnen", "Open player in its own window").into(),
                // Twitchs Player als eigenes Fenster: Dort ist er keine eingebettete fremde Seite, der Browser gibt ihm
                // deshalb Twitchs Anmeldung.
                on_select: Callback::new(move |_| {
                    let url = format!("https://player.twitch.tv/?channel={}&parent=twitch.tv&player=popout", encode(&channel.get_untracked()));
                    open_url(&url, "popup,width=960,height=540,noopener,noreferrer");
                }),
            },
        ]
    });
    let tab_id = tab.with_untracked(|t| t.id.clone());
    let rename_id = tab_id.clone();
    // Leerer Name oder Kanalname: eigener Name entfällt.
    let on_rename = Callback::new(move |value: String| {
        let reset = value.is_empty() || value.to_lowercase() == channel.get_untracked();
        let id = rename_id.clone();
        run(async move { web::rename_tab(&id, (!reset).then_some(value)).await });
    });
    let title = format!("Player: {}", channel.get_untracked());
    view! {
        <SimpleTabItem tab_id=tab_id name=name active=active draggable=draggable tv_icon=true title=title count=Signal::derive(|| 0) menu=menu on_rename=on_rename />
    }
}

/// YouTube-Tab in der Tab-Leiste: Live-Punkt, Kanalname von YouTube, ungesehene Zeilen, Blinken bei Erwähnung oder
/// Filter und Pulsieren beim Live-Gehen wie bei Twitch. ⋮-Menü mit Kanal und Livestream im Browser, Link kopieren,
/// Stummschalten, Live-Benachrichtigung und Neu verbinden. Zuerst nur lesen, daher ohne Mod-Teile.
#[component]
pub fn YouTubeTabItem(tab: Signal<TabInfo>, #[prop(into)] active: Signal<bool>, #[prop(into)] draggable: Signal<bool>) -> impl IntoView {
    let key = Memo::new(move |_| tab.with(|t| t.channel.clone()));
    let version = watch_version(youtube::subscribe, youtube::unsubscribe);
    let state = Memo::new(move |_| {
        version.track();
        youtube::view(&key.get())
    });
    let title = Memo::new(move |_| state.with(|s| s.as_ref().and_then(|s| s.title.clone())));
    let name = Signal::derive(move || tab.with(|t| t.nickname.clone()).or_else(|| title.get()).unwrap_or_else(|| key.get()));
    let live = Signal::derive(move || state.with(|s| s.as_ref().is_some_and(|s| s.live.is_some())));
    let went_live = Memo::new(move |_| state.with(|s| s.as_ref().is_some_and(|s| s.went_live)));
    let live_blink = use_setting("tabs.liveBlink", true);
    let muted_list = use_setting("notifications.mutedChannels", Vec::<String>::new());
    let live_off_list = use_setting("notifications.liveOffChannels", Vec::<String>::new());
    let muted = Memo::new(move |_| muted_list.with(|list| list.contains(&key.get())));
    let live_off = Memo::new(move |_| live_off_list.with(|list| list.contains(&key.get())));
    let attention_version = watch_version(notifications::subscribe_attention, notifications::unsubscribe_attention);
    let attention = Memo::new(move |_| {
        attention_version.track();
        notifications::attention(&key.get())
    });
    let unread_version = watch_version(notifications::subscribe_unread, notifications::unsubscribe_unread);
    let unread = Signal::derive(move || {
        unread_version.track();
        notifications::is_unread(&key.get()) && !active.get() && !muted.get()
    });
    // Wie bei Twitch: Ursache vor Live, stumme Kanäle blinken nicht, sichtbare Tabs auch nicht.
    let blink = Signal::derive(move || {
        if muted.get() || active.get() {
            return String::new();
        }
        attention
            .with(|a| a.as_ref().map(|a| a.class_name.clone()))
            .or_else(|| (went_live.get() && live_blink.get() && !live_off.get()).then(|| "bg-live/25".to_string()))
            .unwrap_or_default()
    });
    // Sichtbar: Pulsieren, Blinken und „ungesehen“ enden.
    Effect::new(move |_| {
        if active.get() {
            let key = key.get_untracked();
            if went_live.get() {
                youtube::acknowledge_live(&key);
            }
            if attention.with(Option::is_some) {
                notifications::clear_attention(&key);
            }
            notifications::clear_unread(&key);
        }
    });
    let menu = Signal::derive(move || {
        let mut list = vec![MenuEntry::IconItem {
            icon: || icons::external_link("").into_any(),
            label: tr("Kanal im Browser öffnen", "Open channel in the browser").into(),
            on_select: Callback::new(move |_| open_url(&youtube_core::channel_page(&key.get_untracked()), "noopener,noreferrer")),
        }];
        if let Some(video) = state.with(|s| s.as_ref().and_then(|s| s.live.as_ref().map(|l| l.video_id.clone()))) {
            list.push(MenuEntry::IconItem {
                icon: || icons::tv("").into_any(),
                label: tr("Stream im Browser öffnen", "Open stream in the browser").into(),
                on_select: Callback::new(move |_| open_url(&youtube_core::video_page(&video), "noopener,noreferrer")),
            });
        }
        list.push(MenuEntry::IconItem {
            icon: || icons::copy("").into_any(),
            label: tr("Kanal-Link kopieren", "Copy channel link").into(),
            on_select: Callback::new(move |_| spawn_local(chat_tools::copy_text(youtube_core::channel_page(&key.get_untracked())))),
        });
        let toggle_muted = Callback::new(move |_| spawn_local(async move { notifications::toggle_muted(&key.get_untracked()).await }));
        list.push(if muted.get() {
            MenuEntry::IconItem { icon: || icons::bell("").into_any(), label: tr("Stummschaltung aufheben", "Unmute").into(), on_select: toggle_muted }
        } else {
            MenuEntry::IconItem { icon: || icons::bell_off("").into_any(), label: tr("Kanal stummschalten", "Mute channel").into(), on_select: toggle_muted }
        });
        list.push(MenuEntry::IconItem {
            icon: || icons::radio("").into_any(),
            label: if live_off.get() { tr("Live-Benachrichtigung einschalten", "Turn on live notification") } else { tr("Live-Benachrichtigung ausschalten", "Turn off live notification") }.into(),
            on_select: Callback::new(move |_| spawn_local(async move { notifications::toggle_live_notify(&key.get_untracked()).await })),
        });
        list.push(MenuEntry::IconItem {
            icon: || icons::refresh_cw("").into_any(),
            label: tr("Neu verbinden", "Reconnect").into(),
            on_select: Callback::new(move |_| youtube::reconnect(&key.get_untracked())),
        });
        list
    });
    let tab_id = tab.with_untracked(|t| t.id.clone());
    let rename_id = tab_id.clone();
    // Leerer Name, Kanalname oder Schlüssel: eigener Name entfällt.
    let on_rename = Callback::new(move |value: String| {
        let reset = value.is_empty() || Some(&value) == title.get_untracked().as_ref() || value.to_lowercase() == key.get_untracked();
        let id = rename_id.clone();
        run(async move { web::rename_tab(&id, (!reset).then_some(value)).await });
    });
    view! {
        <SimpleTabItem tab_id=tab_id name=name active=active draggable=draggable youtube_icon=true live=live unread=unread blink=blink title="YouTube".to_string() count=Signal::derive(|| 0) menu=menu on_rename=on_rename />
    }
}

pub fn highlights_default_name() -> &'static str {
    tr("Hervorhebungen", "Highlights")
}

/// Hervorhebungen-Tab in der Tab-Leiste: umbenennbar, Zähler ungesehener Treffer, ⋮-Menü mit „Anzeigen …“ und „Zähler
/// ungesehener Treffer“.
#[component]
pub fn HighlightsTabItem(tab: Signal<TabInfo>, #[prop(into)] active: Signal<bool>, #[prop(into)] draggable: Signal<bool>) -> impl IntoView {
    let ctx = workspace_ctx();
    let tab_id = tab.with_untracked(|t| t.id.clone());
    let show_open = RwSignal::new(false);
    let feed_version = watch_version(highlights::subscribe, highlights::unsubscribe);
    let show = Signal::derive(move || tab.with(|t| t.show.clone()));
    let counter = Memo::new(move |_| tab.with(|t| t.counter));
    let count_id = tab_id.clone();
    // Neu zählen bei jedem neuen Treffer.
    let count = Signal::derive(move || {
        feed_version.track();
        if counter.get() && !active.get() { show.with(|s| highlights::unseen_count(&count_id, s)) } else { 0 }
    });
    let name = Signal::derive(move || tab.with(|t| t.nickname.clone()).unwrap_or_else(|| highlights_default_name().to_string()));
    let counter_id = tab_id.clone();
    let menu = Signal::derive(move || {
        let id = counter_id.clone();
        vec![
            MenuEntry::IconItem { icon: || icons::eye("").into_any(), label: tr("Anzeigen …", "Show …").into(), on_select: Callback::new(move |_| show_open.set(true)) },
            MenuEntry::Checkbox {
                label: tr("Zähler ungesehener Treffer", "Counter of unseen matches").into(),
                swatch: None,
                checked: counter.get(),
                on_toggle: Callback::new(move |value: bool| {
                    let id = id.clone();
                    run(async move { web::update_highlights_tab(&id, serde_json::json!({ "counter": value })).await });
                }),
            },
        ]
    });
    let rename_id = tab_id.clone();
    // Leerer Name oder Standardname: eigener Name entfällt.
    let on_rename = Callback::new(move |value: String| {
        let reset = value.is_empty() || value == highlights_default_name();
        let id = rename_id.clone();
        run(async move { web::rename_tab(&id, (!reset).then_some(value)).await });
    });
    view! {
        <SimpleTabItem tab_id=tab_id.clone() name=name active=active draggable=draggable count=count menu=menu on_rename=on_rename />
        <ShowDialog tab_id=tab_id open=show_open show=show channels=Signal::derive(move || ctx.open_channels()) />
    }
}
