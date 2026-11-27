//! Tab-Einträge und Inhalte der Tabs in Leptos (Nachbau von react-hosts.tsx, channel-view.tsx, stream-view.tsx und
//! highlights-view.tsx). Das Layout (crates/ui/src/layout.rs) legt für jeden Tab einen Eintrag in der Tab-Leiste und
//! einen Inhalt an; welche Art gezeichnet wird, bestimmt der Tab.

use multiyap_core::i18n::tr;
use std::cell::RefCell;
use std::collections::HashSet;

use multiyap_core::chat_modes::{active_mode_badges, ChatModes};
use multiyap_core::model::ChannelRef;
use multiyap_core::modules::platform_label;
use multiyap_web::workspace as web;
use multiyap_web::{chat_tools, highlights, moderation, sessions, youtube};
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::JsValue;

use crate::chat::input::ChatInput;
use crate::chat::list::ChatList;
use crate::chat::pause::ChatPause;
use crate::chat::top::ChannelTop;
use crate::chatters::ChatterList;
use crate::components::button::button_class;
use crate::components::icons;
use crate::components::ComingSoonBadge;
use leptos::portal::Portal;
use crate::components::overlay::{window, Alert};
use crate::store::{encode, session_version, watch_version};
use crate::tabs::{ChannelTabItem, HighlightsTabItem, StreamTabItem, YouTubeTabItem};
use crate::workspace::{workspace_ctx, TabInfo, WorkspaceCtx};

thread_local! {
    /// Chatter-Liste aufgeklappt, je Tab-ID. Nur für die Sitzung, nichts wird gespeichert; bleibt beim Verschieben
    /// des Tabs in einen anderen Bereich erhalten.
    static CHATTERS_OPEN: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// Tab aus dem Stand der Arbeitsfläche und seine Art.
fn tab_signals(ctx: WorkspaceCtx, tab_id: String) -> (Signal<TabInfo>, Memo<Option<String>>) {
    let tab = Memo::new(move |_| ctx.tab(&tab_id));
    // YouTube-Kanäle sind Kanal-Tabs mit eigener Darstellung.
    let kind = Memo::new(move |_| tab.with(|t| t.as_ref().map(|t| if t.kind == "channel" && t.platform == multiyap_core::youtube::PLATFORM { "youtube".to_string() } else { t.kind.clone() })));
    (Signal::derive(move || tab.get().unwrap_or_default()), kind)
}

/// Eintrag eines Tabs in der Tab-Leiste.
#[component]
pub fn TabItem(tab_id: String, area_id: String, #[prop(into)] active: Signal<bool>, #[prop(into)] draggable: Signal<bool>) -> impl IntoView {
    let (tab, kind) = tab_signals(workspace_ctx(), tab_id);
    move || match kind.get().as_deref() {
        Some("channel") => view! { <ChannelTabItem tab=tab area_id=area_id.clone() active=active draggable=draggable /> }.into_any(),
        Some("youtube") => view! { <YouTubeTabItem tab=tab active=active draggable=draggable /> }.into_any(),
        Some("stream") => view! { <StreamTabItem tab=tab active=active draggable=draggable /> }.into_any(),
        Some("highlights") => view! { <HighlightsTabItem tab=tab active=active draggable=draggable /> }.into_any(),
        _ => ().into_any(),
    }
}

/// Inhalt eines Tabs.
#[component]
pub fn TabContent(tab_id: String, #[prop(into)] active: Signal<bool>) -> impl IntoView {
    let (tab, kind) = tab_signals(workspace_ctx(), tab_id);
    move || match kind.get().as_deref() {
        Some("channel") => view! { <ChannelView tab=tab active=active /> }.into_any(),
        Some("youtube") => view! { <YouTubeView tab=tab /> }.into_any(),
        Some("stream") => view! { <StreamView tab=tab /> }.into_any(),
        Some("highlights") => view! { <HighlightsView tab=tab active=active /> }.into_any(),
        _ => ().into_any(),
    }
}

/// Inhalt eines Kanal-Tabs: Chat-Liste und Eingabefeld, in Mod-Kanälen auf Wunsch die Chatter-Liste rechts. Eine Pause
/// für den ganzen Tab, damit auch die Nutzerkarte aus der Chatter-Liste den Chat anhält.
#[component]
fn ChannelView(tab: Signal<TabInfo>, #[prop(into)] active: Signal<bool>) -> impl IntoView {
    let pause = ChatPause::new();
    // Platz für Knöpfe in der Tab-Leiste des eigenen Bereichs (crates/ui/src/layout.rs, `data-tab-actions`).
    let root = NodeRef::<leptos::html::Div>::new();
    let (actions, set_actions) = signal_local(None::<web_sys::Element>);
    Effect::new(move |_| {
        if let Some(element) = root.get() {
            let slot = element.closest("section").ok().flatten().and_then(|s| s.query_selector("[data-tab-actions]").ok().flatten());
            set_actions.set(slot);
        }
    });
    let tab_id = tab.with_untracked(|t| t.id.clone());
    let channel = Memo::new(move |_| tab.with(|t| t.channel.clone()));
    let version = session_version(channel.into());
    let session = move || sessions::session(&channel.get());
    let channel_ref = Memo::new(move |_| {
        version.track();
        session().and_then(|s| s.channel())
    });
    let channel_id = Memo::new(move |_| channel_ref.with(|c| c.as_ref().map(|c| c.id.clone())));
    let channel_name = Memo::new(move |_| channel_ref.with(|c| c.as_ref().map(|c| c.display_name.clone())).unwrap_or_else(|| channel.get()));
    let error = Memo::new(move |_| {
        version.track();
        session().and_then(|s| s.error())
    });
    let joined = Memo::new(move |_| {
        version.track();
        session().is_some_and(|s| s.joined())
    });
    let has_older = Memo::new(move |_| {
        version.track();
        session().is_some_and(|s| s.has_older())
    });
    // Letzter Shoutout oben rechts in der Tab-Leiste, mit Link zum Kanal.
    let shoutout = Memo::new(move |_| {
        version.track();
        session().and_then(|s| s.shoutout())
    });
    let badges = Memo::new(move |_| {
        version.track();
        session().and_then(|s| s.chat_modes()).and_then(|m| serde_json::from_value::<ChatModes>(m).ok()).map(|m| active_mode_badges(&m)).unwrap_or_default()
    });
    // Zeilen als JavaScript-Objekte (je Zeile zwischengespeichert), neu gelesen bei jeder Änderung der Sitzung.
    let items = Signal::derive_local(move || {
        version.track();
        session().map_or_else(|| JsValue::from(js_sys::Array::new()), |s| JsValue::from(s.items_js()))
    });
    let mod_version = watch_version(moderation::subscribe_mod, moderation::unsubscribe_mod);
    let can_moderate = Memo::new(move |_| {
        mod_version.track();
        channel_id.with(|id| id.as_deref().is_some_and(moderation::can_moderate))
    });
    // Chatter-Liste rechts neben dem Chat, nur in Mod-Kanälen.
    let open_id = tab_id.clone();
    let chatters_wanted = RwSignal::new(CHATTERS_OPEN.with(|o| o.borrow().contains(&open_id)));
    let set_chatters = move |open: bool| {
        let id = open_id.clone();
        CHATTERS_OPEN.with(|o| {
            let mut o = o.borrow_mut();
            if open {
                o.insert(id);
            } else {
                o.remove(&id);
            }
        });
        chatters_wanted.set(open);
    };
    let chatters_open = Memo::new(move |_| chatters_wanted.get() && can_moderate.get() && channel_ref.with(Option::is_some));
    let toggle_chatters = {
        let set_chatters = set_chatters.clone();
        Callback::new(move |_| set_chatters(!chatters_open.get_untracked()))
    };
    let close_chatters = Callback::new(move |_| set_chatters(false));
    let load_older = Callback::new(move |_| {
        if let Some(session) = sessions::session(&channel.get_untracked()) {
            session.load_older();
        }
    });

    view! {
        <div node_ref=root class="flex min-h-0 flex-1">
            <div class="flex min-h-0 min-w-0 flex-1 flex-col">
                {move || error.get().map(|message| view! {
                    <div class="p-3">
                        <Alert destructive=true title=tr("Kanal nicht verfügbar", "Channel not available") icon=icons::triangle_alert("").into_any()>
                            {message}
                        </Alert>
                    </div>
                })}
                // Chat-Modi, Chatter-Liste und angepinnte Nachricht (crates/ui/src/chat/top.rs).
                <ChannelTop channel_id=channel_id moderator=can_moderate chatters_open=chatters_open badges=badges active=active actions=actions on_toggle_chatters=toggle_chatters />
                {move || {
                    let mount = active.get().then(|| actions.get()).flatten()?;
                    let (login, name) = shoutout.get()?;
                    Some(view! {
                        <Portal mount=mount>
                            <span class="flex items-center">
                                <a href=format!("https://www.twitch.tv/{}", encode(&login)) target="_blank" rel="noopener noreferrer" data-slot="button"
                                    class=button_class("secondary", "xs", "") title=multiyap_core::tf!("Kanal von {name} im Browser öffnen", "Open {name}’s channel in the browser")>
                                    {format!("Shoutout: {name}")}
                                    {icons::external_link("")}
                                </a>
                                <button type="button" data-slot="button" class=button_class("ghost", "icon-xs", "text-muted-foreground") aria-label=tr("Shoutout ausblenden", "Hide shoutout")
                                    on:click=move |_| if let Some(s) = sessions::session(&channel.get_untracked()) { s.clear_shoutout() }>
                                    {icons::x("")}
                                </button>
                            </span>
                        </Portal>
                    })
                }}
                <ChatList items=items has_older=has_older load_older=load_older search_tab_id=tab_id pause=pause />
                <ChatInput channel_id=channel_id channel_name=channel_name channel_login=channel enabled=joined pause=pause />
            </div>
            {move || chatters_open.get().then(|| view! {
                <ChatterList channel=channel_ref items_version=version on_close=close_chatters pause=pause />
            })}
        </div>
    }
}

/// Inhalt eines YouTube-Tabs: Chat-Liste ohne Eingabefeld, weil YouTube zuerst nur gelesen wird. Oben ein Hinweis,
/// solange der Kanal offline ist, der Chat fehlt oder der Server etwas meldet.
#[component]
fn YouTubeView(tab: Signal<TabInfo>) -> impl IntoView {
    let pause = ChatPause::new();
    let tab_id = tab.with_untracked(|t| t.id.clone());
    let key = Memo::new(move |_| tab.with(|t| t.channel.clone()));
    let version = watch_version(youtube::subscribe, youtube::unsubscribe);
    let status = Memo::new(move |_| {
        version.track();
        youtube::view(&key.get()).map(|v| v.status)
    });
    let items = Signal::derive_local(move || {
        version.track();
        JsValue::from(youtube::items_js(&key.get()))
    });
    let notice = move || -> Option<(bool, String)> {
        if multiyap_core::youtube::SERVER.is_none() {
            return Some((false, tr("YouTube-Chats sind noch nicht verfügbar (Coming Soon).", "YouTube chats are not available yet (Coming Soon).").to_string()));
        }
        Some(match status.get()? {
            youtube::Status::Checking | youtube::Status::Live => return None,
            youtube::Status::Offline => (false, tr("Der Kanal ist gerade nicht live. MultiYap prüft alle 2 Minuten, ob er live geht.", "The channel is not live right now. MultiYap checks every 2 minutes whether it goes live.").to_string()),
            youtube::Status::NoChat => (false, tr("Der Livestream läuft, aber sein Chat ist abgeschaltet.", "The livestream is running, but its chat is turned off.").to_string()),
            youtube::Status::Problem(message) => (true, message),
        })
    };
    view! {
        <div class="flex min-h-0 min-w-0 flex-1 flex-col">
            {move || notice().map(|(problem, message)| if problem {
                view! {
                    <div class="p-3">
                        <Alert destructive=true title=tr("YouTube nicht verfügbar", "YouTube not available") icon=icons::triangle_alert("").into_any()>
                            {message}
                        </Alert>
                    </div>
                }.into_any()
            } else {
                view! { <p class="border-b px-3 py-2 text-sm text-muted-foreground">{message}</p> }.into_any()
            })}
            <ChatList items=items has_older=false search_tab_id=tab_id pause=pause />
            // Statt des Eingabefelds: YouTube ist vorerst nur zum Lesen (ohne Server gar nicht, dann reicht der Hinweis oben).
            {multiyap_core::youtube::SERVER.is_some().then(|| view! {
                <div class="flex shrink-0 items-center gap-2 border-t px-3 py-2 text-sm text-muted-foreground">
                    {icons::eye("size-4 shrink-0")}
                    <span class="min-w-0 flex-1">{tr("YouTube ist vorerst nur zum Lesen. Schreiben und Moderieren folgen später.", "YouTube is read-only for now. Writing and moderating will follow later.")}</span>
                    <ComingSoonBadge />
                </div>
            })}
        </div>
    }
}

/// Stream-Tab: Twitchs offizieller Player als iframe (Doku geprüft am 15.11.2026: player.twitch.tv mit channel und
/// parent; parent ist der eigene Host). Kein Skript wird nachgeladen. Twitch verlangt für automatisches Abspielen
/// mindestens 400 x 300 Pixel.
#[component]
fn StreamView(tab: Signal<TabInfo>) -> impl IntoView {
    let channel = Memo::new(move |_| tab.with(|t| t.channel.clone()));
    let host = window().location().hostname().unwrap_or_default();
    // Beim Ziehen eines Tabs bekäme sonst der Player die Maus, und ein Chat-Tab ließe sich nicht auf den Stream-Tab
    // ziehen.
    let dragging = workspace_ctx().dragged;
    let tab_id = tab.with_untracked(|t| t.id.clone());
    move || {
        let login = channel.get();
        let src = format!("https://player.twitch.tv/?channel={}&parent={}", encode(&login), encode(&host));
        view! {
            <iframe src=src data-stream-tab=tab_id.clone() title=format!("Twitch-Player: {login}") allowfullscreen=true
                class=move || if dragging.with(Option::is_some) { "pointer-events-none min-h-0 w-full flex-1 border-0 bg-black" } else { "min-h-0 w-full flex-1 border-0 bg-black" }></iframe>
        }
    }
}

/// Kanalname und Plattform vor der Uhrzeit jeder Zeile.
fn channel_label(channel: &ChannelRef) -> String {
    format!("{} · {}", channel.display_name, platform_label(&channel.platform))
}

/// Springt zur Stelle im Kanal: einen Tab des Kanals zeigen (bevorzugt einen schon sichtbaren), dann dort zur Zeile
/// scrollen. Ist sie nicht mehr geladen, ein kurzer Hinweis.
fn jump_to(ctx: WorkspaceCtx, login: String, item_id: String) {
    let target = ctx.state.with_untracked(|s| {
        let visible = s.active_tabs();
        let candidates: Vec<&TabInfo> = s.tabs.iter().filter(|t| t.kind == "channel" && t.channel == login).collect();
        candidates.iter().find(|t| visible.contains(&t.id)).or(candidates.first()).map(|t| t.id.clone())
    });
    let Some(target) = target else {
        chat_tools::toast("info", tr("Der Kanal ist nicht mehr offen.", "The channel is no longer open."));
        return;
    };
    spawn_local(async move {
        if let Err(message) = web::activate_tab(&target).await {
            chat_tools::toast("error", &message);
            return;
        }
        // Zwei Bilder warten, bis der Tab sichtbar und gemessen ist.
        request_animation_frame(move || {
            request_animation_frame(move || {
                if !chat_tools::request_jump(&item_id) {
                    chat_tools::toast("info", tr("Die Stelle ist im Kanal nicht mehr geladen.", "That point is no longer loaded in the channel."));
                }
            })
        });
    });
}

/// Inhalt eines Hervorhebungen-Tabs: Treffer aus allen offenen Kanälen (crates/web/src/highlights.rs); Klick auf eine
/// Zeile springt zur Stelle im Kanal.
#[component]
fn HighlightsView(tab: Signal<TabInfo>, active: Signal<bool>) -> impl IntoView {
    let ctx = workspace_ctx();
    let pause = ChatPause::new();
    let tab_id = tab.with_untracked(|t| t.id.clone());
    let version = watch_version(highlights::subscribe, highlights::unsubscribe);
    let show = Memo::new(move |_| tab.with(|t| t.show.clone()));
    let items = Signal::derive_local(move || {
        version.track();
        show.with(|s| JsValue::from(highlights::entries().iter().filter(|e| highlights::shown(e, s)).map(|e| e.js.clone()).collect::<js_sys::Array>()))
    });
    let empty = Memo::new(move |_| {
        version.track();
        show.with(|s| !highlights::entries().iter().any(|e| highlights::shown(e, s)))
    });
    // Solange der Tab sichtbar ist, gilt alles als gesehen.
    let seen_id = tab_id.clone();
    Effect::new(move |_| {
        version.track();
        if active.get() {
            highlights::mark_seen(&seen_id);
        }
    });
    let on_click = Callback::new(move |(login, id): (String, String)| jump_to(ctx, login, id));
    view! {
        <div class="relative flex min-h-0 flex-1 flex-col">
            {move || empty.get().then(|| view! {
                <p class="absolute inset-x-0 top-6 z-10 text-center text-sm text-muted-foreground">
                    {tr("Noch keine Treffer. Hier erscheinen Filter-Treffer, Erwähnungen, Chat-Ereignisse, Mod-Aktionen und AutoMod aus allen offenen Kanälen.", "No matches yet. Filter matches, mentions, chat events, mod actions and AutoMod from all open channels appear here.")}
                </p>
            })}
            <ChatList items=items has_older=false channel_label=channel_label on_item_click=on_click search_tab_id=tab_id pause=pause />
        </div>
    }
}
