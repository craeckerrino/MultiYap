//! Kopf des Chats in einem Kanal-Tab (Nachbau von chat-mode-badges.tsx und pinned-bar.tsx): Plaketten der aktiven
//! Chat-Modi, in Mod-Kanälen der Knopf für die Chatter-Liste, darunter die angepinnte Nachricht als einklappbare Leiste
//! (Abschnitt 6.9). Twitch liefert die angepinnte Nachricht nur Broadcaster und Mods und meldet Änderungen nicht über
//! EventSub: Abfrage beim Öffnen und alle 30 Sekunden, nur in Mod-Kanälen.

use multiyap_core::i18n::tr;
use leptos::portal::Portal;
use leptos::prelude::*;
use serde::Deserialize;
use wasm_bindgen::JsCast;

use super::layer::{Layer, LayerView};
use crate::components::button::button_class;
use crate::components::icons;
use crate::components::overlay::{DropdownMenu, MenuEntry};

const POLL_INTERVAL_MS: u64 = 30_000;
/// Anpinnen: Dauern wie im Twitch-Chat; ohne Dauer bis zum Streamende.
fn pin_durations() -> [(&'static str, Option<u32>); 5] {
    [(tr("1 Minute", "1 minute"), Some(60)), (tr("5 Minuten", "5 minutes"), Some(300)), (tr("10 Minuten", "10 minutes"), Some(600)), (tr("30 Minuten", "30 minutes"), Some(1800)), (tr("Bis zum Streamende", "Until the end of the stream"), None)]
}
const SECONDARY_BADGE: &str = "group/badge inline-flex h-5 w-fit shrink-0 items-center justify-center gap-1 overflow-hidden rounded-4xl border border-transparent px-2 py-0.5 text-xs font-medium whitespace-nowrap transition-all focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 aria-invalid:border-destructive aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 [&>svg]:pointer-events-none [&>svg]:size-3! bg-secondary text-secondary-foreground [a]:hover:bg-secondary/80";

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PinnedMessage {
    message_id: String,
    sender_name: String,
    pinned_by: String,
    text: Option<String>,
    ends_at: Option<f64>,
}

/// Restzeit als „noch 4:59“; ohne Ende „bis Streamende“.
fn remaining_label(ends_at: Option<f64>, now: f64) -> String {
    match ends_at {
        None => tr("bis Streamende", "until the end of the stream").into(),
        Some(end) => {
            let seconds = ((end - now) / 1000.0).round().max(0.0) as i64;
            multiyap_core::tf!("noch {}:{:02}", "{}:{:02} left", seconds / 60, seconds % 60)
        }
    }
}

fn users_icon() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
            stroke-linecap="round" stroke-linejoin="round" class="lucide lucide-users" aria-hidden="true">
            <path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"></path>
            <path d="M16 3.128a4 4 0 0 1 0 7.744"></path>
            <path d="M22 21v-2a4 4 0 0 0-3-3.87"></path>
            <circle cx="9" cy="7" r="4"></circle>
        </svg>
    }
}

/// Angepinnte Nachricht als schmale, einklappbare Leiste: Nachricht, wer angepinnt hat, Restzeit; dazu „Dauer
/// ändern“ und „Lösen“.
#[component]
fn PinnedBar(channel_id: Signal<Option<String>>, moderator: Signal<bool>) -> impl IntoView {
    let read = move || {
        let id = channel_id.get_untracked()?;
        multiyap_web::chat_tools::pinned(&id).and_then(|p| crate::bridge::convert::<PinnedMessage>(&p))
    };
    let pin = RwSignal::new(read());
    let pin_listener = multiyap_web::chat_tools::subscribe_pinned(std::rc::Rc::new(move || {
        pin.try_set(read());
    }));
    on_cleanup(move || multiyap_web::chat_tools::unsubscribe_pinned(pin_listener));
    // Abfrage beim Öffnen und alle 30 Sekunden, solange die Ansicht offen ist und man Mod ist.
    Effect::new(move |_| {
        let (Some(id), true) = (channel_id.get(), moderator.get()) else { return };
        pin.set(read());
        leptos::task::spawn_local(multiyap_web::chat_tools::refresh_pinned(id.clone()));
        let handle = set_interval_with_handle(
            move || {
                leptos::task::spawn_local(multiyap_web::chat_tools::refresh_pinned(id.clone()));
            },
            std::time::Duration::from_millis(POLL_INTERVAL_MS),
        )
        .ok();
        on_cleanup(move || {
            if let Some(handle) = handle {
                handle.clear();
            }
        });
    });
    // Sekundentakt für die Restzeit (nur solange eine Endzeit läuft).
    let now = RwSignal::new(js_sys::Date::now());
    Effect::new(move |_| {
        if pin.with(|p| p.as_ref().is_none_or(|p| p.ends_at.is_none())) {
            return;
        }
        let handle = set_interval_with_handle(move || now.set(js_sys::Date::now()), std::time::Duration::from_secs(1)).ok();
        on_cleanup(move || {
            if let Some(handle) = handle {
                handle.clear();
            }
        });
    });
    let expired = Memo::new(move |_| pin.with(|p| p.as_ref().and_then(|p| p.ends_at)).is_some_and(|end| end <= now.get()));
    // Abgelaufen: Leiste ausblenden und den Stand bei Twitch neu holen.
    Effect::new(move |_| {
        if let (true, Some(id)) = (expired.get(), channel_id.get_untracked()) {
            leptos::task::spawn_local(multiyap_web::chat_tools::refresh_pinned(id));
        }
    });
    let collapsed = RwSignal::new(false);

    move || {
        let current = pin.get()?;
        let id = channel_id.get()?;
        if !moderator.get() || expired.get() {
            return None;
        }
        let (menu_id, menu_message) = (id.clone(), current.message_id.clone());
        let entries = pin_durations()
            .iter()
            .map(|(label, seconds)| {
                let (id, message) = (menu_id.clone(), menu_message.clone());
                let seconds = *seconds;
                MenuEntry::Item {
                    label: (*label).to_string(),
                    on_select: Callback::new(move |_| {
                        leptos::task::spawn_local(multiyap_web::chat_tools::change_pin_duration(id.clone(), message.clone(), seconds.map(f64::from)));
                    }),
                }
            })
            .collect::<Vec<_>>();
        let (release_id, release_message) = (id, current.message_id.clone());
        let ends_at = current.ends_at;
        Some(view! {
            <div class="flex flex-col gap-1 border-b bg-muted/40 px-3 py-1.5 text-sm">
                <div class="flex items-center gap-2">
                    {icons::pin("size-3.5 shrink-0 text-muted-foreground")}
                    <span class="min-w-0 flex-1 truncate text-xs text-muted-foreground">
                        {move || multiyap_core::tf!("Angepinnt von {}, {}", "Pinned by {}, {}", current.pinned_by, remaining_label(ends_at, now.get()))}
                    </span>
                    {move || (!collapsed.get()).then(|| {
                        let entries = entries.clone();
                        let (id, message) = (release_id.clone(), release_message.clone());
                        view! {
                            <div class="flex shrink-0 items-center gap-1">
                                <DropdownMenu align_end=true trigger_size="xs" width_class="" entries=Signal::derive(move || entries.clone())>
                                    {tr("Dauer ändern", "Change duration")}
                                </DropdownMenu>
                                <button type="button" data-slot="button" class=button_class("outline", "xs", "")
                                    on:click=move |_| { leptos::task::spawn_local(multiyap_web::chat_tools::release_pin(id.clone(), message.clone())); }>
                                    {icons::pin_off("")}
                                    {tr("Lösen", "Unpin")}
                                </button>
                            </div>
                        }
                    })}
                    <button type="button" data-slot="button" class=button_class("ghost", "icon-xs", "")
                        aria-label=move || if collapsed.get() { {tr("Angepinnte Nachricht aufklappen", "Expand pinned message")} } else { {tr("Angepinnte Nachricht einklappen", "Collapse pinned message")} }
                        on:click=move |_| collapsed.update(|c| *c = !*c)>
                        {move || if collapsed.get() { icons::chevron_down("").into_any() } else { icons::chevron_up("").into_any() }}
                    </button>
                </div>
                {move || (!collapsed.get()).then(|| view! {
                    <p class="text-chat break-words">
                        <span class="font-semibold">{current.sender_name.clone()}</span>
                        <span class="text-muted-foreground">": "</span>
                        {match current.text.clone() {
                            Some(text) => text.into_any(),
                            None => view! { <span class="text-muted-foreground italic">{tr("Text nicht verfügbar", "Text not available")}</span> }.into_any(),
                        }}
                    </p>
                })}
            </div>
        })
    }
}

/// Knopf für die Chatter-Liste.
fn chatters_button(layer: Layer, chatters_open: Signal<bool>, on_toggle_chatters: Callback<()>) -> impl IntoView {
    view! {
        <button type="button" data-slot="button"
            class=move || button_class("ghost", "icon-sm", if chatters_open.get() { "text-foreground" } else { "text-muted-foreground" })
            aria-label=move || if chatters_open.get() { {tr("Chatter-Liste zuklappen", "Collapse chatter list")} } else { {tr("Chatter-Liste aufklappen", "Expand chatter list")} }
            aria-pressed=move || chatters_open.get().to_string()
            on:pointerenter=move |event: web_sys::PointerEvent| {
                if let Some(element) = event.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                    layer.show_tip(&element, tr("Chatter-Liste", "Chatter list").into(), false);
                }
            }
            on:pointerleave=move |_| layer.hide_tip()
            on:click=move |_| {
                layer.hide_tip();
                on_toggle_chatters.run(());
            }>
            {users_icon()}
        </button>
    }
}

/// Kopf des Chats: Plaketten der aktiven Chat-Modi (in jedem Kanal sichtbar, auch ohne Mod-Rechte), Knopf für die
/// Chatter-Liste (nur Mod-Kanäle) und die angepinnte Nachricht. Die Leiste erscheint nur bei aktiven Modi; sonst steht
/// der Knopf für die Chatter-Liste eine Zeile höher in der Tab-Leiste (`actions`, nur für den sichtbaren Tab).
#[component]
pub fn ChannelTop(
    #[prop(into)] channel_id: Signal<Option<String>>,
    #[prop(into)] moderator: Signal<bool>,
    #[prop(into)] chatters_open: Signal<bool>,
    #[prop(into)] badges: Signal<Vec<String>>,
    #[prop(into)] active: Signal<bool>,
    actions: ReadSignal<Option<web_sys::Element>, LocalStorage>,
    on_toggle_chatters: Callback<()>,
) -> impl IntoView {
    let layer = Layer::new();
    view! {
        {move || (!badges.with(Vec::is_empty)).then(|| view! {
            <div class="flex shrink-0 items-center gap-2 border-b px-3 py-1.5">
                <div class="flex min-w-0 flex-1 flex-wrap gap-1" aria-label=tr("Aktive Chat-Modi", "Active chat modes")>
                    {badges.get().into_iter().map(|label| view! { <span data-slot="badge" data-variant="secondary" class=SECONDARY_BADGE>{label}</span> }).collect_view()}
                </div>
                {moderator.get().then(|| chatters_button(layer, chatters_open, on_toggle_chatters))}
            </div>
        })}
        {move || {
            let show = badges.with(Vec::is_empty) && moderator.get() && active.get();
            show.then(|| actions.get()).flatten().map(|mount| view! {
                <Portal mount=mount>{move || chatters_button(layer, chatters_open, on_toggle_chatters)}</Portal>
            })
        }}
        <PinnedBar channel_id=channel_id moderator=moderator />
        <LayerView layer=layer />
    }
}

#[cfg(test)]
mod tests {
    use super::remaining_label;

    #[test]
    fn restzeit() {
        assert_eq!(remaining_label(None, 0.0), "bis Streamende");
        assert_eq!(remaining_label(Some(299_000.0), 0.0), "noch 4:59");
        assert_eq!(remaining_label(Some(1_000.0), 5_000.0), "noch 0:00");
    }
}
