//! Chatter-Liste rechts neben dem Chat in Leptos (Nachbau von chatter-list.tsx). Nur Mod-Kanäle. Lädt beim Öffnen und
//! auf Klick auf „Aktualisieren“, keine Abfragen im Hintergrund. Nichts wird gespeichert. Virtualisiert mit festen
//! Höhen (Überschrift 32 px, Name 24 px).

use multiyap_core::i18n::tr;
use std::collections::HashSet;

use multiyap_core::chatters::{group_chatters, Chatter, RoleSource};
use multiyap_core::model::{ChannelRef, UserRef};
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::chat::layer::Rect;
use crate::chat::pause::ChatPause;
use crate::components::button::button_class;
use crate::components::icons;
use crate::components::input::INPUT_CLASS;

const HEADER_PX: f64 = 32.0;
const ROW_PX: f64 = 24.0;
const OVERSCAN: usize = 20;

#[derive(Clone, Debug, Default, PartialEq)]
struct Roles {
    moderators: Vec<String>,
    vips: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct Loaded {
    chatters: Vec<Chatter>,
    total: f64,
    truncated: bool,
    own_roles: Option<Roles>,
}

#[derive(Clone, PartialEq)]
enum Row {
    Header(&'static str, usize),
    Chatter(Chatter),
}

fn format_count(value: f64) -> String {
    let format = js_sys::Intl::NumberFormat::new(&js_sys::Array::of1(&tr("de-DE", "en-US").into()), &js_sys::Object::new()).format();
    format.call1(&JsValue::NULL, &JsValue::from_f64(value)).ok().and_then(|v| v.as_string()).unwrap_or_else(|| value.to_string())
}

fn refresh_icon(spinning: bool) -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
            stroke-linecap="round" stroke-linejoin="round" class=if spinning { "lucide lucide-refresh-cw animate-spin" } else { "lucide lucide-refresh-cw" } aria-hidden="true">
            <path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"></path>
            <path d="M21 3v5h-5"></path>
            <path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"></path>
            <path d="M8 16H3v5"></path>
        </svg>
    }
}

/// `items_version` steigt mit jeder Änderung im Chat (erkannte Rollen in fremden Kanälen neu lesen).
#[component]
pub fn ChatterList(
    #[prop(into)] channel: Signal<Option<ChannelRef>>,
    #[prop(into)] items_version: Signal<u32>,
    on_close: Callback<()>,
    pause: ChatPause,
) -> impl IntoView {
    let loaded = RwSignal::new(None::<Loaded>);
    let loading = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let query = RwSignal::new(String::new());
    let scroll_top = RwSignal::new(0.0f64);
    let viewport = RwSignal::new(400.0f64);
    let scroll_ref = NodeRef::<leptos::html::Div>::new();

    let load = move || {
        let Some(id) = channel.get_untracked().map(|c| c.id) else { return };
        loading.set(true);
        error.set(None);
        spawn_local(async move {
            match multiyap_web::chatters::load(&id).await {
                Ok(Some(result)) => {
                    loaded.try_set(Some(Loaded {
                        chatters: result.chatters.into_iter().filter_map(|c| serde_json::from_value(c).ok()).collect(),
                        total: result.total,
                        truncated: result.truncated,
                        own_roles: result.own_roles.map(|r| Roles { moderators: r.moderators, vips: r.vips }),
                    }));
                }
                Ok(None) => {}
                Err(text) => {
                    error.try_set(Some(text));
                }
            }
            loading.try_set(false);
        });
    };
    // Beim Öffnen und bei einem anderen Kanal laden.
    Effect::new(move |_| {
        channel.with(|c| c.as_ref().map(|c| c.id.clone()));
        load();
    });

    // Fremde Kanäle: erkannte Rollen; neu gelesen, wenn neue Zeilen kommen.
    let roles = Memo::new(move |_| {
        if let Some(own) = loaded.with(|l| l.as_ref().and_then(|l| l.own_roles.clone())) {
            return own;
        }
        items_version.track();
        let Some(c) = channel.get() else { return Roles::default() };
        let known = multiyap_web::moderation::known_chatter_roles(&c.id, &c.login);
        Roles { moderators: known.moderators, vips: known.vips }
    });

    let rows = Memo::new(move |_| {
        let Some(data) = loaded.get() else { return Vec::new() };
        let Some(c) = channel.get() else { return Vec::new() };
        let roles = roles.get();
        let (moderators, vips): (HashSet<String>, HashSet<String>) = (roles.moderators.into_iter().collect(), roles.vips.into_iter().collect());
        let source = RoleSource { broadcaster_id: &c.id, moderators: &moderators, vips: &vips };
        // Alphabetisch wie Intl.Collator „de“ (Grundzeichen, Groß- und Kleinschreibung egal).
        let options = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&options, &"sensitivity".into(), &"base".into());
        let collator = js_sys::Intl::Collator::new(&js_sys::Array::of1(&"de".into()), &options).compare();
        let compare = |a: &str, b: &str| {
            let n = collator.call2(&JsValue::NULL, &JsValue::from_str(a), &JsValue::from_str(b)).ok().and_then(|v| v.as_f64()).unwrap_or(0.0);
            n.partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal)
        };
        group_chatters(&data.chatters, &source, &query.get(), compare)
            .into_iter()
            .flat_map(|g| std::iter::once(Row::Header(g.title, g.chatters.len())).chain(g.chatters.into_iter().map(Row::Chatter)))
            .collect::<Vec<_>>()
    });
    let starts = Memo::new(move |_| {
        rows.with(|list| {
            let mut total = 0.0;
            let mut starts = Vec::with_capacity(list.len() + 1);
            for row in list {
                starts.push(total);
                total += if matches!(row, Row::Header(..)) { HEADER_PX } else { ROW_PX };
            }
            starts.push(total);
            starts
        })
    });
    let visible = Memo::new(move |_| {
        let (top, height) = (scroll_top.get(), viewport.get());
        starts.with(|s| {
            let count = s.len().saturating_sub(1);
            let first = s.partition_point(|start| *start <= top).saturating_sub(1);
            let last = s.partition_point(|start| *start < top + height);
            (first.saturating_sub(OVERSCAN), (last + OVERSCAN).min(count))
        })
    });

    view! {
        <aside class="flex w-56 shrink-0 flex-col border-l" aria-label=tr("Chatter-Liste", "Chatter list")>
            <div class="flex items-center gap-1 border-b px-3 py-1.5">
                <span class="flex-1 truncate text-sm font-medium">
                    {move || loaded.with(|l| l.as_ref().map(|l| multiyap_core::tf!("{} im Chat", "{} in chat", format_count(l.total)))).unwrap_or_else(|| "Chatter".into())}
                </span>
                <button type="button" data-slot="button" class=button_class("ghost", "icon-sm", "") aria-label=tr("Aktualisieren", "Refresh") title=tr("Aktualisieren", "Refresh")
                    disabled=move || loading.get() on:click=move |_| load()>
                    {move || refresh_icon(loading.get())}
                </button>
                <button type="button" data-slot="button" class=button_class("ghost", "icon-sm", "") aria-label=tr("Chatter-Liste schließen", "Close chatter list")
                    on:click=move |_| {
                        on_close.run(());
                    }>
                    {icons::x("")}
                </button>
            </div>
            <div class="border-b px-3 py-2">
                <input data-slot="input" class=INPUT_CLASS placeholder=tr("Name suchen", "Search name") aria-label=tr("Chatter suchen", "Search chatters")
                    prop:value=move || query.get() on:input=move |event| query.set(event_target_value(&event)) />
            </div>
            {move || error.get().map(|e| view! { <p class="px-3 py-2 text-xs text-destructive">{multiyap_core::tf!("Konnte nicht laden: {e}", "Could not load: {e}")}</p> })}
            {move || loaded.get().filter(|l| l.truncated).map(|l| view! {
                <p class="px-3 py-2 text-xs text-muted-foreground">{multiyap_core::tf!("Gekürzt: {} von {} geladen.", "Shortened: {} of {} loaded.", format_count(l.chatters.len() as f64), format_count(l.total))}</p>
            })}
            {move || loaded.with(|l| l.as_ref().is_some_and(|l| l.own_roles.is_none())).then(|| view! {
                <p class="px-3 py-2 text-xs text-muted-foreground">
                    {tr("Twitch gibt die Mod- und VIP-Listen nur dem Broadcaster. Erkannt ist, wer seit dem Öffnen im Chat geschrieben oder moderiert hat; alle anderen stehen unter Zuschauer.", "Twitch only gives the mod and VIP lists to the broadcaster. Recognized is whoever has written or moderated in the chat since it was opened; everyone else is listed under viewers.")}
                </p>
            })}
            <div node_ref=scroll_ref class="min-h-0 flex-1 overflow-y-auto"
                on:scroll=move |_| {
                    if let Some(el) = scroll_ref.get_untracked() {
                        scroll_top.set(f64::from(el.scroll_top()));
                        viewport.set(f64::from(el.client_height()));
                    }
                }>
                {move || (loaded.with(Option::is_some) && rows.with(Vec::is_empty)).then(|| view! {
                    <p class="px-3 py-2 text-sm text-muted-foreground">{move || if query.with(String::is_empty) { {tr("Niemand im Chat.", "Nobody in chat.")} } else { {tr("Niemand gefunden.", "Nobody found.")} }}</p>
                })}
                <div class="relative w-full" style:height=move || format!("{}px", starts.with(|s| s.last().copied().unwrap_or(0.0)))>
                    {move || {
                        let (first, last) = visible.get();
                        let tops = starts.get();
                        rows.with(|list| list.get(first..last).map(|slice| slice.iter().cloned().enumerate().map(|(offset, row)| {
                            let index = first + offset;
                            let (top, height) = (tops[index], tops[index + 1] - tops[index]);
                            let style = format!("height: {height}px; transform: translateY({top}px);");
                            match row {
                                Row::Header(title, count) => view! {
                                    <div class="absolute top-0 left-0 w-full px-3" style=style>
                                        <div class="flex h-full items-end pb-1 text-xs font-medium text-muted-foreground">{format!("{title} ({})", format_count(count as f64))}</div>
                                    </div>
                                }.into_any(),
                                Row::Chatter(chatter) => {
                                    let name = chatter.user_name.clone();
                                    view! {
                                        <div class="absolute top-0 left-0 w-full px-3" style=style>
                                            <button type="button" class="block h-full w-full truncate text-left text-sm hover:underline"
                                                on:click=move |event: web_sys::MouseEvent| {
                                                    let (Some(c), Some(element)) = (channel.get_untracked(), event.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok())) else { return };
                                                    let rect = Rect::of(&element);
                                                    let user = UserRef { id: chatter.user_id.clone(), login: chatter.user_login.clone(), display_name: chatter.user_name.clone() };
                                                    let release = pause.hold().dyn_into::<js_sys::Function>().ok();
                                                    multiyap_web::chat_tools::open_card(c, user, None, rect.left, rect.bottom + 4.0, release);
                                                }>
                                                {name}
                                            </button>
                                        </div>
                                    }.into_any()
                                }
                            }
                        }).collect_view())).unwrap_or_default()
                    }}
                </div>
            </div>
        </aside>
    }
}
