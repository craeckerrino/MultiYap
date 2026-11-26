//! Nutzerkarte in Leptos (Nachbau von user-card.tsx, recent-messages.tsx, exception-button.tsx, quick-buttons.tsx für
//! die Karte). Geöffnet per Klick auf einen Namen als eigenes Fenster direkt darunter; anpinnbar, am Rand verschiebbar.
//! Klick daneben und Esc schließen nicht angepinnte Karten.

use multiyap_core::i18n::tr;
use std::rc::Rc;

use multiyap_core::chat_text::is_protected_sender;
use multiyap_core::model::{Badge, ChannelRef, ChatMessage, Emote, UserRef};
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Deserialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use multiyap_web::moderation;

use crate::bridge::{save_setting, use_setting};
use crate::chat::layer::{run_mod_action, Layer, LayerView, ModTarget, ReasonState, Rect};
use crate::chat::line::{parts_view, ChatActions};
use crate::components::badge::ComingSoonBadge;
use crate::components::button::button_class;
use crate::components::icons;
use crate::components::overlay::{window, DropdownMenu, MenuEntry, WindowListeners};
use crate::settings::model::{action_label, QuickButton, ReasonDialog, TermCategory, TermException};
use crate::settings::terms::{exception_menu, save_exception, ExceptionUser};

/// Breite der Karte in Pixeln (wie CARD_WIDTH in card-windows.ts).
const CARD_WIDTH: f64 = 320.0;
/// Abstand der Karte zum Fensterrand.
const EDGE: f64 = 8.0;
/// Klicks in diesen Bereichen gehören noch zur Karte (Menüs, Fenster und Hinweise öffnen sich außerhalb).
const INSIDE_SELECTOR: &str = "[data-user-card], [data-radix-popper-content-wrapper], [role=\"menu\"], [role=\"listbox\"], [role=\"dialog\"], [role=\"alertdialog\"], [data-sonner-toaster], [data-chat-overlay]";

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct CardWindow {
    id: String,
    channel: ChannelRef,
    user: UserRef,
    #[serde(default)]
    badges: Option<Vec<Badge>>,
    x: f64,
    y: f64,
    pinned: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CardUser {
    display_name: String,
    profile_image_url: String,
    created_at: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct BanStatus {
    #[serde(rename = "type")]
    kind: String,
    by: Option<String>,
    reason: Option<String>,
    until: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CardData {
    user: Option<CardUser>,
    followers: Option<f64>,
    follow_queried: bool,
    followed_at: Option<String>,
    live: bool,
    ban: Option<BanStatus>,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
struct ModStatus {
    moderator: bool,
    broadcaster: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct SuspiciousInfo {
    status: String,
    reasons: Vec<String>,
}

// ---- Formate (bisher format.ts) ----

fn intl_date(options: &[(&str, &str)], value: f64) -> String {
    let object = js_sys::Object::new();
    for (key, val) in options {
        let _ = js_sys::Reflect::set(&object, &(*key).into(), &(*val).into());
    }
    let format = js_sys::Intl::DateTimeFormat::new(&js_sys::Array::of1(&tr("de-DE", "en-US").into()), &object).format();
    format.call1(&JsValue::NULL, &JsValue::from_f64(value)).ok().and_then(|v| v.as_string()).unwrap_or_default()
}

fn parse_date(text: &str) -> f64 {
    js_sys::Date::new(&JsValue::from_str(text)).value_of()
}

fn format_date(value: f64) -> String {
    intl_date(&[("day", "2-digit"), ("month", "2-digit"), ("year", "numeric")], value)
}

fn format_clock(value: f64) -> String {
    intl_date(&[("hour", "2-digit"), ("minute", "2-digit")], value)
}

fn format_count(value: f64) -> String {
    let format = js_sys::Intl::NumberFormat::new(&js_sys::Array::of1(&tr("de-DE", "en-US").into()), &js_sys::Object::new()).format();
    format.call1(&JsValue::NULL, &JsValue::from_f64(value)).ok().and_then(|v| v.as_string()).unwrap_or_else(|| value.to_string())
}

/// Alter eines Accounts in Ortszeit, zum Beispiel „3 Jahre, 2 Monate“.
fn format_age(created: f64) -> String {
    let (start, end) = (js_sys::Date::new(&JsValue::from_f64(created)), js_sys::Date::new_0());
    let ymd = |d: &js_sys::Date| (i64::from(d.get_full_year()), i64::from(d.get_month()), i64::from(d.get_date()));
    let days = ((end.value_of() - created) / 86_400_000.0).floor() as i64;
    multiyap_core::time::account_age(ymd(&start), ymd(&end), days)
}

/// Sub-Info aus den Chat-Abzeichen.
fn sub_info(badges: Option<&[Badge]>) -> String {
    let Some(badges) = badges else { return tr("Unbekannt", "Unknown").into() };
    let Some(sub) = badges.iter().find(|b| b.set_id == "subscriber" || b.set_id == "founder") else { return tr("Kein Abo", "No subscription").into() };
    let label = if sub.set_id == "founder" { tr("Gründer-Abo", "Founder subscription") } else { tr("Abonniert", "Subscribed") };
    match sub.info.as_deref().and_then(|i| i.parse::<f64>().ok()).filter(|m| m.is_finite() && *m > 0.0) {
        Some(m) => format!("{label}, {m} {}", if m == 1.0 { tr("Monat", "month") } else { tr("Monate", "months") }),
        None => sub.title.clone().unwrap_or_else(|| label.into()),
    }
}

fn ban_text(ban: &BanStatus) -> String {
    let by = ban.by.as_ref().map(|b| multiyap_core::tf!(" von {b}", " by {b}")).unwrap_or_default();
    let reason = ban.reason.as_ref().map(|r| multiyap_core::tf!(", Grund: {r}", ", reason: {r}")).unwrap_or_default();
    if ban.kind == "ban" {
        multiyap_core::tf!("Gebannt{by}{reason}", "Banned{by}{reason}")
    } else {
        multiyap_core::tf!("Timeout bis {}{by}{reason}", "Timeout until {}{by}{reason}", ban.until.map_or_else(|| "?".to_string(), format_clock))
    }
}

/// Wert aus einem Dienst in Rust (crates/web) als Signal, das sich bei jeder Meldung des Dienstes erneuert.
fn watch_web<T: Clone + 'static>(
    read: impl Fn() -> Option<T> + 'static,
    subscribe: fn(std::rc::Rc<dyn Fn()>) -> u32,
    unsubscribe: fn(u32),
) -> RwSignal<Option<T>, LocalStorage> {
    let read = std::rc::Rc::new(read);
    let value = RwSignal::new_local(read());
    let id = subscribe(std::rc::Rc::new(move || {
        value.try_set(read());
    }));
    on_cleanup(move || unsubscribe(id));
    value
}

fn row(label: &'static str, value: impl IntoView + 'static) -> impl IntoView {
    view! {
        <div class="flex justify-between gap-4">
            <dt class="text-muted-foreground">{label}</dt>
            <dd class="text-right">{value}</dd>
        </div>
    }
}

/// Letzte Nachrichten der Person im aktuellen Kanal: aus dem geladenen Chat, nichts wird extra gespeichert. Neueste
/// unten wie im Chat; entfernte Nachrichten durchgestrichen. Ein- und ausklappbar; der Zustand gilt für alle Karten
/// (Einstellung usercard.recentMessagesOpen).
#[component]
fn RecentMessages(channel: ChannelRef, user_id: String, actions: ChatActions) -> impl IntoView {
    // YouTube: Zeilen aus crates/web/src/youtube.rs, Schlüssel unverändert (Kanal-IDs unterscheiden Groß- und
    // Kleinschreibung), ohne Emotes von Drittanbietern.
    let youtube = channel.platform == multiyap_core::youtube::PLATFORM;
    let login = if youtube { channel.login.clone() } else { channel.login.to_lowercase() };
    let read = {
        let (login, user_id) = (login.clone(), user_id.clone());
        move || {
            let lines = if youtube { multiyap_web::youtube::recent_messages(&login, &user_id) } else { multiyap_web::sessions::recent_messages(&login, &user_id) };
            lines.into_iter().filter_map(|m| serde_json::from_value::<ChatMessage>(m).ok()).collect::<Vec<_>>()
        }
    };
    let messages = RwSignal::new_local(read());
    let refresh = Rc::new(move || {
        let next = read();
        if messages.with_untracked(|m| *m != next) {
            messages.try_set(next);
        }
    });
    if youtube {
        let watch = multiyap_web::youtube::subscribe(refresh);
        on_cleanup(move || multiyap_web::youtube::unsubscribe(watch));
    } else {
        let watch = multiyap_web::sessions::watch_login(&login, refresh);
        on_cleanup(move || multiyap_web::sessions::unwatch_login(watch));
    }
    let emotes: Rc<std::collections::HashMap<String, Emote>> = if youtube {
        Rc::default()
    } else {
        Rc::new(multiyap_web::emotes::third_party_emotes(&channel.id).iter().cloned().map(|e| (e.name.clone(), e)).collect())
    };
    let emotes = send_wrapper::SendWrapper::new(emotes);
    let open = use_setting(RECENT_OPEN, true);
    let list_ref = NodeRef::<leptos::html::Div>::new();
    // Beim Öffnen, Aufklappen und bei neuen Nachrichten nach unten (neueste sichtbar).
    Effect::new(move |_| {
        messages.with(Vec::len);
        open.track();
        if let Some(list) = list_ref.get() {
            list.set_scroll_top(list.scroll_height());
        }
    });
    view! {
        <div class="flex flex-col gap-1 text-xs">
            <button type="button" class="flex w-fit items-center gap-1 rounded-md text-muted-foreground outline-none hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50"
                aria-expanded=move || open.get().to_string()
                on:click=move |_| save_setting(RECENT_OPEN, &!open.get_untracked())>
                {move || if open.get() { icons::chevron_down("size-3.5").into_any() } else { icons::chevron_right("size-3.5").into_any() }}
                {move || {
                    let count = messages.with(Vec::len);
                    if count > 0 { multiyap_core::tf!("Letzte Nachrichten ({count})", "Recent messages ({count})") } else { tr("Letzte Nachrichten", "Recent messages").to_string() }
                }}
            </button>
            {move || if !open.get() {
                ().into_any()
            } else if messages.with(Vec::is_empty) {
                view! { <p class="text-xs text-muted-foreground">{tr("Keine Nachrichten im geladenen Chat.", "No messages in the loaded chat.")}</p> }.into_any()
            } else {
                let channel = channel.clone();
                let emotes = emotes.clone();
                view! {
                    <div node_ref=list_ref class="flex max-h-40 flex-col gap-0.5 overflow-y-auto rounded-md bg-muted/40 p-1.5 text-[0.7rem] leading-snug">
                        {messages.get().into_iter().map(|message| {
                            let class = if message.flags.removed.is_some() { "break-words text-muted-foreground line-through" } else { "break-words" };
                            view! {
                                <p class=class>
                                    <span class="mr-1 text-muted-foreground tabular-nums">{format_clock(message.timestamp)}</span>
                                    {parts_view(&message.parts, Some(&emotes), Some(&channel), Some(actions))}
                                </p>
                            }
                        }).collect_view()}
                    </div>
                }.into_any()
            }}
        </div>
    }
}

/// Eingeschränkt/überwacht: Status und Knöpfe, nur in Mod-Kanälen. Twitch liefert den Status nicht auf Abfrage; bekannt
/// ist er aus Ereignissen und eigenen Änderungen dieser Sitzung.
#[component]
fn SuspiciousControls(channel_id: String, user: UserRef) -> impl IntoView {
    let info = {
        let (channel_id, user_id) = (channel_id.clone(), user.id.clone());
        watch_web(
            move || {
                moderation::known_suspicious(&channel_id, &user_id).map(|i| SuspiciousInfo { status: i.status, reasons: i.reasons })
            },
            moderation::subscribe_suspicious,
            moderation::unsubscribe_suspicious,
        )
    };
    let busy = RwSignal::new(false);
    let target = multiyap_web::mod_actions::ModTarget { channel_id, user_id: user.id.clone(), display_name: user.display_name.clone(), message_id: None };
    let change = move |status: Option<&'static str>| {
        busy.set(true);
        let target = target.clone();
        spawn_local(async move {
            multiyap_web::mod_actions::change_suspicious(target, status.map(str::to_string)).await;
            busy.try_set(false);
        });
    };
    let status = move || info.with(|i| i.as_ref().map(|i| i.status.clone()));
    let button = move |label: &'static str, value: Option<&'static str>| {
        let change = change.clone();
        view! {
            <button type="button" data-slot="button" class=button_class("outline", "xs", "")
                disabled=move || busy.get() || (value.is_some() && status().as_deref() == value)
                on:click=move |_| change(value)>
                {label}
            </button>
        }
    };
    view! {
        <div class="flex flex-col gap-1.5">
            <div class="flex justify-between gap-4">
                <span class="text-muted-foreground">{tr("Überwachung", "Monitoring")}</span>
                <span class=move || match status().as_deref() {
                    Some("restricted") => "text-right text-restricted",
                    Some("monitored") => "text-right text-monitored",
                    _ => "text-right",
                }>
                    {move || match info.get() {
                        Some(i) => view! {
                            {multiyap_core::chat_text::suspicious_label(&i.status)}
                            {(!i.reasons.is_empty()).then(|| view! { <span class="block text-xs text-muted-foreground">{i.reasons.join(", ")}</span> })}
                        }.into_any(),
                        None => tr("Kein Status bekannt", "No status known").into_any(),
                    }}
                </span>
            </div>
            <div class="flex flex-wrap gap-1">
                {button(tr("Überwachen", "Monitor"), Some("monitored"))}
                {button(tr("Einschränken", "Restrict"), Some("restricted"))}
                {button(tr("Status entfernen", "Remove status"), None)}
            </div>
        </div>
    }
}

/// Einstellung: letzte Nachrichten in der Nutzerkarte aufgeklappt.
const RECENT_OPEN: &str = "usercard.recentMessagesOpen";

/// Gruppe der Symbolknöpfe Verwarnen, Ban und Unban: wie BUTTON_GROUP, aber nur so breit wie der Inhalt.
const ICON_GROUP: &str = "group/button-group flex w-fit items-stretch *:focus-visible:relative *:focus-visible:z-10 [&>*:not(:first-child)]:rounded-l-none [&>*:not(:first-child)]:border-l-0 [&>*:not(:last-child)]:rounded-r-none";

const BUTTON_GROUP: &str = "group/button-group flex w-fit items-stretch *:focus-visible:relative *:focus-visible:z-10 has-[>[data-slot=button-group]]:gap-2 has-[select[aria-hidden=true]:last-child]:[&>[data-slot=select-trigger]:last-of-type]:rounded-r-lg [&>[data-slot=select-trigger]:not([class*='w-'])]:w-fit [&>input]:flex-1 [&>*:not(:first-child)]:rounded-l-none [&>*:not(:first-child)]:border-l-0 [&>*:not(:last-child)]:rounded-r-none [&>[data-slot]:not(:has(~[data-slot]))]:rounded-r-lg! w-full";

/// „Von Filtern ausnehmen“: ein Klick nimmt von allen Kategorien aus, der Pfeil öffnet die Auswahl. Ist die Person
/// schon ausgenommen: „Ausnahme bearbeiten“.
#[component]
fn ExceptionButton(user: UserRef) -> impl IntoView {
    let categories = use_setting("highlights.categories", Vec::<TermCategory>::new());
    let exceptions = use_setting("highlights.exceptions", Vec::<TermException>::new());
    let user_id = user.id.clone();
    let target = move || ExceptionUser { user_id: user.id.clone(), login: user.login.clone(), display_name: user.display_name.clone() };
    let exception = Memo::new(move |_| exceptions.with(|list| list.iter().find(|e| e.user_id == user_id).cloned()));
    move || match exception.get() {
        Some(existing) => {
            let target = target.clone();
            view! {
                <DropdownMenu width_class="w-56" trigger_class="w-full"
                    entries=Signal::derive(move || exception_menu(target(), Some(existing.clone()), categories.get()))>
                    {tr("Ausnahme bearbeiten", "Edit exception")}
                    {icons::chevron_down("")}
                </DropdownMenu>
            }
            .into_any()
        }
        None => {
            let (add, menu) = (target.clone(), target.clone());
            view! {
                <div role="group" data-slot="button-group" data-orientation="horizontal" class=BUTTON_GROUP>
                    <button type="button" data-slot="button" class=button_class("outline", "sm", "flex-1")
                        on:click=move |_| save_exception(add(), Some((true, Vec::new())))>
                        {tr("Von Filtern ausnehmen", "Exclude from filters")}
                    </button>
                    <DropdownMenu align_end=true icon=true aria_label=tr("Kategorien für die Ausnahme wählen", "Choose categories for the exception") width_class="w-56"
                        entries=Signal::derive(move || exception_menu(menu(), None, categories.get()))>
                        {icons::chevron_down("")}
                    </DropdownMenu>
                </div>
            }
            .into_any()
        }
    }
}

/// Seite im Browser öffnen.
fn open_page(url: &str) {
    let _ = window().open_with_url_and_target_and_features(url, "_blank", "noopener,noreferrer");
}

/// Seiten, auf denen man die Person öffnen kann: Bezeichnung und Adresse.
fn profile_pages(login: &str) -> Vec<(&'static str, String)> {
    let login = String::from(js_sys::encode_uri_component(&login.to_lowercase()));
    vec![
        (tr("Twitch-Kanal", "Twitch channel"), format!("https://www.twitch.tv/{login}")),
        (tr("Lurkology: Rollen", "Lurkology: roles"), format!("https://lurkology.com/roles?u={login}")),
    ]
}

/// „Twitch Usercard“ mit Pfeil wie bei „Von Filtern ausnehmen“: ein Klick öffnet die Usercard von Twitch, der Pfeil
/// listet die Seiten, auf denen man die Person öffnen kann (mit 7TV-Profil, falls die Person eines hat).
#[component]
fn ProfileButton(channel_login: String, user_login: String, seventv: RwSignal<Option<String>>) -> impl IntoView {
    let pages = profile_pages(&user_login);
    let entries = Signal::derive(move || {
        let seventv = seventv.get().map(|url| (tr("7TV-Profil", "7TV profile"), url));
        pages
            .iter()
            .cloned()
            .chain(seventv)
            .map(|(label, url)| {
                MenuEntry::IconItem { icon: || icons::external_link("").into_any(), label: label.to_string(), on_select: Callback::new(move |_| open_page(&url)) }
            })
            .collect::<Vec<_>>()
    });
    view! {
        <div role="group" data-slot="button-group" data-orientation="horizontal" class=BUTTON_GROUP>
            <button type="button" data-slot="button" class=button_class("secondary", "sm", "flex-1")
                on:click=move |_| { multiyap_web::chat_tools::open_twitch_usercard(&channel_login, &user_login); }>
                {icons::external_link("")}
                "Twitch Usercard"
            </button>
            <div data-slot="button-group-separator" data-orientation="vertical" role="separator" aria-orientation="vertical" class="relative m-0! w-px shrink-0 self-stretch bg-input"></div>
            <DropdownMenu align_end=true icon=true trigger_variant="secondary" aria_label=tr("Person auf einer Seite öffnen", "Open person on a site") width_class="w-56" entries=entries>
                {icons::chevron_down("")}
            </DropdownMenu>
        </div>
    }
}

fn card_title(button: &QuickButton) -> String {
    match button.action.as_str() {
        "timeout" => format!("Timeout {}", multiyap_core::hotkeys::format_duration(button.duration_seconds.unwrap_or(0))),
        other => action_label(other).to_string(),
    }
}

/// Inhalt einer Karte.
#[component]
fn CardContent(card_id: String, channel: ChannelRef, user: UserRef, badges: Option<Vec<Badge>>, pinned: Signal<bool>, layer: Layer) -> impl IntoView {
    let actions = ChatActions { layer, pause: None };
    // Zentrale Prüfung der Mod-Rechte in Rust (crates/web/src/moderation.rs).
    let status = {
        let id = channel.id.clone();
        watch_web(
            move || Some(ModStatus { moderator: moderation::can_moderate(&id), broadcaster: moderation::is_broadcaster(&id) }),
            moderation::subscribe_mod,
            moderation::unsubscribe_mod,
        )
    };
    let moderator = Memo::new(move |_| status.with(|s| s.unwrap_or_default().moderator));
    let broadcaster = Memo::new(move |_| status.with(|s| s.unwrap_or_default().broadcaster));
    let own_id = multiyap_web::auth::own_user_id();
    let is_self = own_id.as_deref() == Some(user.id.as_str());
    let protected = badges.as_deref().is_some_and(is_protected_sender);
    let show_actions = Memo::new(move |_| moderator.get() && !is_self && !protected);
    let card_actions = use_setting("usercard.actions", Vec::<QuickButton>::new());
    let reason_dialog = use_setting("moderation.reasonDialog", ReasonDialog { timeout: false, ban: false });

    // Daten beim Öffnen laden; ändern sich die Mod-Rechte, neu laden. Ältere Antworten werden verworfen.
    let data = RwSignal::new(None::<CardData>);
    let request = StoredValue::new(0u32);
    {
        let (channel_id, user_id) = (channel.id.clone(), user.id.clone());
        Effect::new(move |_| {
            let (moderator, broadcaster) = (moderator.get(), broadcaster.get());
            let number = request.with_value(|n| n + 1);
            request.set_value(number);
            data.set(None);
            let (channel_id, user_id) = (channel_id.clone(), user_id.clone());
            spawn_local(async move {
                let value = serde_json::from_value::<CardData>(multiyap_web::chat_tools::load_card_data(&channel_id, &user_id, moderator, broadcaster).await).ok();
                if request.try_with_value(|n| *n == number) == Some(true) {
                    data.try_set(value.or(Some(CardData {
                        user: None,
                        followers: None,
                        follow_queried: false,
                        followed_at: None,
                        live: false,
                        ban: None,
                        error: Some(tr("Nutzer konnte nicht geladen werden.", "User could not be loaded.").into()),
                    })));
                }
            });
        });
    }

    let channel_url = format!("https://www.twitch.tv/{}", js_sys::encode_uri_component(&user.login));
    // 7TV-Profil der Person für das Menü der Seiten: beim Öffnen der Karte nachschlagen, damit der Klick die Seite
    // direkt öffnet. None: noch nicht bekannt oder kein 7TV-Konto.
    let seventv = RwSignal::new(None::<String>);
    {
        let user_id = user.id.clone();
        spawn_local(async move {
            let url = multiyap_web::seventv::profile_url(&user_id).await;
            seventv.try_set(url);
        });
    }
    let (copy_login, usercard_channel, usercard_user) = (user.login.clone(), channel.login.clone(), user.login.clone());
    let fallback_name = user.display_name.clone();
    let open_label = multiyap_core::tf!("Kanal von {} im Browser öffnen", "Open {}’s channel in the browser", user.display_name);
    let target = ModTarget { channel_id: channel.id.clone(), user_id: user.id.clone(), display_name: user.display_name.clone(), message_id: None };
    let (pin_id, close_id) = (card_id.clone(), card_id);
    let sub = sub_info(badges.as_deref());

    // Knöpfe der Aktionsliste: Timeouts über den letzten Nachrichten, Verwarnen, Ban und Unban als Symbole in einer
    // Gruppe. Reihenfolge und Auswahl aus Einstellungen → Nutzerkarte.
    let mod_buttons = move |timeouts: bool| {
        let buttons = card_actions.get().into_iter().filter(|b| (b.action == "timeout") == timeouts).collect::<Vec<_>>();
        (show_actions.get() && !buttons.is_empty()).then(|| {
            let banned = data.with(|d| d.as_ref().is_some_and(|d| d.ban.is_some()));
            let dialog = reason_dialog.get();
            let views = buttons.into_iter().map(|button| {
                let title = card_title(&button);
                let class = match button.action.as_str() {
                    "timeout" => button_class("outline", "xs", ""),
                    "ban" => button_class("outline", "icon-sm", "text-destructive hover:text-destructive"),
                    // Unban grün; hervorgehoben, wenn die Person gebannt ist.
                    "unban" if banned => button_class("default", "icon-sm", ""),
                    "unban" => button_class("outline", "icon-sm", "text-primary hover:text-primary"),
                    _ => button_class("outline", "icon-sm", ""),
                };
                let face = match button.action.as_str() {
                    "warn" => icons::triangle_alert("").into_any(),
                    "ban" => icons::ban("").into_any(),
                    "unban" => icons::circle_check("").into_any(),
                    _ => multiyap_core::hotkeys::format_duration(button.duration_seconds.unwrap_or(0)).into_any(),
                };
                let needs_dialog = button.action == "warn" || (button.action == "timeout" && dialog.timeout) || (button.action == "ban" && dialog.ban);
                let target = target.clone();
                view! {
                    <button type="button" data-slot="button" class=class aria-label=title.clone() title=title
                        on:click=move |event: web_sys::MouseEvent| {
                            if needs_dialog {
                                let Some(element) = event.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) else { return };
                                layer.reason.set(Some(ReasonState {
                                    anchor: Rect::of(&element),
                                    action: button.action.clone(),
                                    duration_seconds: button.duration_seconds,
                                    target: target.clone(),
                                }));
                            } else {
                                run_mod_action(&target, &button.action, button.duration_seconds, button.fixed_reason.clone());
                            }
                        }>
                        {face}
                    </button>
                }
            }).collect_view();
            if timeouts {
                view! { <div class="flex flex-wrap items-center gap-1">{views}</div> }.into_any()
            } else {
                view! { <div role="group" data-slot="button-group" data-orientation="horizontal" class=ICON_GROUP>{views}</div> }.into_any()
            }
        })
    };

    view! {
        <div class="flex flex-col gap-3">
            <div class="flex items-center gap-3">
                <button type="button" class="shrink-0 rounded-full outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
                    aria-label=open_label title=tr("Kanal im Browser öffnen", "Open channel in the browser")
                    on:click=move |_| { let _ = window().open_with_url_and_target_and_features(&channel_url, "_blank", "noopener,noreferrer"); }>
                    {move || match data.with(|d| d.as_ref().and_then(|d| d.user.as_ref().map(|u| u.profile_image_url.clone()))) {
                        Some(src) => view! { <img src=src alt="" class="size-12 rounded-full" /> }.into_any(),
                        None => view! { <span class="block size-12 rounded-full bg-muted"></span> }.into_any(),
                    }}
                </button>
                <div class="flex min-w-0 flex-col">
                    <div class="flex items-center gap-1.5">
                        <span class="truncate text-sm font-semibold">
                            {move || data.with(|d| d.as_ref().and_then(|d| d.user.as_ref().map(|u| u.display_name.clone()))).unwrap_or_else(|| fallback_name.clone())}
                        </span>
                        {move || data.with(|d| d.as_ref().is_some_and(|d| d.live)).then(|| view! {
                            <span class="size-2 shrink-0 rounded-full bg-live" aria-label="Live" title="Live"></span>
                        })}
                        <button type="button" data-slot="button" class=button_class("ghost", "icon-xs", "") aria-label=tr("Login-Namen kopieren", "Copy login name") title=tr("Login-Namen kopieren", "Copy login name")
                            on:click=move |_| { spawn_local(multiyap_web::chat_tools::copy_text(copy_login.clone())); }>
                            {icons::copy("")}
                        </button>
                    </div>
                    <span class="truncate text-xs text-muted-foreground">{user.login.clone()}</span>
                </div>
                <div class="ml-auto flex shrink-0 self-start">
                    <button type="button" data-slot="button"
                        class=move || button_class("ghost", "icon-xs", if pinned.get() { "text-primary" } else { "" })
                        aria-label=move || if pinned.get() { {tr("Karte lösen", "Unpin card")} } else { {tr("Karte anpinnen", "Pin card")} }
                        title=move || if pinned.get() { {tr("Lösen: schließt beim Klick daneben", "Unpin: closes when clicking outside")} } else { {tr("Anpinnen: bleibt offen", "Pin: stays open")} }
                        on:click=move |_| { multiyap_web::chat_tools::set_card_pinned(&pin_id, !pinned.get_untracked()); }>
                        {move || if pinned.get() { icons::pin_off("").into_any() } else { icons::pin("").into_any() }}
                    </button>
                    <button type="button" data-slot="button" class=button_class("ghost", "icon-xs", "") aria-label=tr("Karte schließen", "Close card") title=tr("Schließen", "Close")
                        on:click=move |_| { multiyap_web::chat_tools::close_card(&close_id); }>
                        {icons::x("")}
                    </button>
                </div>
            </div>

            {move || match data.get() {
                None => view! {
                    <div class="flex justify-center text-muted-foreground">{icons::loader_circle("size-4 animate-spin")}</div>
                }.into_any(),
                Some(d) => {
                    let sub = sub.clone();
                    view! {
                        <dl class="flex flex-col gap-1">
                            {d.user.as_ref().map(|u| {
                                let created = parse_date(&u.created_at);
                                row(tr("Account erstellt", "Account created"), view! {
                                    {format_date(created)}
                                    <span class="block text-muted-foreground">{format_age(created)}</span>
                                })
                            })}
                            {row("Follower", d.followers.map_or_else(|| tr("Unbekannt", "Unknown").to_string(), format_count))}
                            {(moderator.get_untracked() && d.follow_queried).then(|| row(tr("Folgt seit", "Following since"),
                                match d.followed_at.as_deref() {
                                    // Dauer klein darunter wie beim Alter des Accounts.
                                    Some(f) => {
                                        let since = parse_date(f);
                                        view! {
                                            {format_date(since)}
                                            <span class="block text-muted-foreground">{format_age(since)}</span>
                                        }.into_any()
                                    }
                                    None => tr("Folgt nicht", "Not following").into_any(),
                                }))}
                            {row(tr("Abo", "Subscription"), sub)}
                            // Ban-Status nur im eigenen Kanal: Twitch liefert ihn Mods nicht.
                            {broadcaster.get_untracked().then(|| row("Status", d.ban.as_ref().map_or_else(|| tr("Kein Ban oder Timeout", "No ban or timeout").to_string(), ban_text)))}
                            {d.error.clone().map(|e| view! { <p class="text-xs text-destructive">{e}</p> })}
                        </dl>
                    }.into_any()
                }
            }}

            {let top = mod_buttons.clone(); move || top(true)}

            <RecentMessages channel=channel.clone() user_id=user.id.clone() actions=actions />

            {
                let (channel_id, user) = (channel.id.clone(), user.clone());
                move || show_actions.get().then(|| view! { <SuspiciousControls channel_id=channel_id.clone() user=user.clone() /> })
            }

            {move || mod_buttons(false)}

            <ExceptionButton user=user.clone() />

            <ProfileButton channel_login=usercard_channel user_login=usercard_user seventv=seventv />
        </div>
    }
}

/// Einfache Karte für YouTube-Nutzer: YouTube liefert nur Name, Kanal-ID und Rollen. Daher Name, Rollen, letzte
/// Nachrichten in diesem Chat, Ausnahme von Filtern und der Kanal auf YouTube; kostet kein Kontingent. Moderation kommt
/// erst mit der Google-Anmeldung („Coming Soon“).
#[component]
fn YouTubeCardContent(card_id: String, channel: ChannelRef, user: UserRef, badges: Option<Vec<Badge>>, pinned: Signal<bool>, layer: Layer) -> impl IntoView {
    let actions = ChatActions { layer, pause: None };
    let page = multiyap_core::youtube::channel_page(&user.id);
    let open_page_url = page.clone();
    let roles: Vec<String> = badges.unwrap_or_default().iter().filter_map(|b| b.title.clone()).collect();
    let roles = if roles.is_empty() { tr("Keine", "None").to_string() } else { roles.join(", ") };
    let copy_name = user.display_name.clone();
    let (pin_id, close_id) = (card_id.clone(), card_id);
    view! {
        <div class="flex flex-col gap-3">
            <div class="flex items-center gap-3">
                <div class="flex min-w-0 flex-col">
                    <div class="flex items-center gap-1.5">
                        <span class="truncate text-sm font-semibold">{user.display_name.clone()}</span>
                        <button type="button" data-slot="button" class=button_class("ghost", "icon-xs", "") aria-label=tr("Namen kopieren", "Copy name") title=tr("Namen kopieren", "Copy name")
                            on:click=move |_| { spawn_local(multiyap_web::chat_tools::copy_text(copy_name.clone())); }>
                            {icons::copy("")}
                        </button>
                    </div>
                    <span class="truncate text-xs text-muted-foreground">"YouTube"</span>
                </div>
                <div class="ml-auto flex shrink-0 self-start">
                    <button type="button" data-slot="button"
                        class=move || button_class("ghost", "icon-xs", if pinned.get() { "text-primary" } else { "" })
                        aria-label=move || if pinned.get() { {tr("Karte lösen", "Unpin card")} } else { {tr("Karte anpinnen", "Pin card")} }
                        title=move || if pinned.get() { {tr("Lösen: schließt beim Klick daneben", "Unpin: closes when clicking outside")} } else { {tr("Anpinnen: bleibt offen", "Pin: stays open")} }
                        on:click=move |_| { multiyap_web::chat_tools::set_card_pinned(&pin_id, !pinned.get_untracked()); }>
                        {move || if pinned.get() { icons::pin_off("").into_any() } else { icons::pin("").into_any() }}
                    </button>
                    <button type="button" data-slot="button" class=button_class("ghost", "icon-xs", "") aria-label=tr("Karte schließen", "Close card") title=tr("Schließen", "Close")
                        on:click=move |_| { multiyap_web::chat_tools::close_card(&close_id); }>
                        {icons::x("")}
                    </button>
                </div>
            </div>
            <dl class="flex flex-col gap-1">
                {row(tr("Rollen", "Roles"), roles)}
            </dl>
            <RecentMessages channel=channel user_id=user.id.clone() actions=actions />
            <button type="button" data-slot="button" class=button_class("outline", "sm", "w-full") disabled=true>
                {tr("Moderation", "Moderation")}
                <ComingSoonBadge />
            </button>
            <ExceptionButton user=user />
            <button type="button" data-slot="button" class=button_class("outline", "sm", "w-full") title=page
                on:click=move |_| open_page(&open_page_url)>
                {icons::external_link("")}
                {tr("Kanal auf YouTube öffnen", "Open channel on YouTube")}
            </button>
        </div>
    }
}

/// Eine Karte als eigenes Fenster. Verschieben nur am Rand (Innenabstand der Karte), nicht über Inhalt und Knöpfe.
#[component]
fn CardWindowView(card: CardWindow, cards: ReadSignal<Vec<CardWindow>>, layer: Layer) -> impl IntoView {
    let id = card.id.clone();
    let current = Memo::new({
        let id = id.clone();
        move |_| cards.with(|list| list.iter().find(|c| c.id == id).map(|c| (c.x, c.y, c.pinned)))
    });
    let pinned = Signal::derive(move || current.get().is_some_and(|c| c.2));
    let node = NodeRef::<leptos::html::Div>::new();
    let drag = StoredValue::new(None::<(f64, f64)>);
    // Hat man die Karte selbst verschoben, bleibt sie, wo man sie hingelegt hat.
    let moved_by_user = StoredValue::new(false);

    // Ragt die Karte unten aus dem Fenster (Namen weit unten, Inhalte laden nach), rückt sie nach oben.
    let observer = StoredValue::new_local(None::<web_sys::ResizeObserver>);
    Effect::new({
        let id = id.clone();
        move |_| {
            let Some(element) = node.get() else { return };
            let element: web_sys::Element = element.into();
            let fit = {
                let (id, element) = (id.clone(), element.clone());
                move || {
                    if moved_by_user.get_value() {
                        return;
                    }
                    let Some((x, y, _)) = current.get_untracked() else { return };
                    let height = element.get_bounding_client_rect().height();
                    let view_height = window().inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(800.0);
                    let max_top = (view_height - height - EDGE).max(EDGE);
                    if y > max_top {
                        multiyap_web::chat_tools::move_card(&id, x, max_top);
                    }
                }
            };
            fit();
            let callback = Closure::<dyn FnMut()>::new(fit).into_js_value();
            if let Ok(watcher) = web_sys::ResizeObserver::new(callback.unchecked_ref()) {
                watcher.observe(&element);
                observer.set_value(Some(watcher));
            }
        }
    });
    on_cleanup(move || {
        if let Some(watcher) = observer.try_get_value().flatten() {
            watcher.disconnect();
        }
    });

    let (down_id, move_id) = (id.clone(), id.clone());
    let label = multiyap_core::tf!("Nutzerkarte {}", "User card {}", card.user.display_name);
    view! {
        <div node_ref=node data-user-card="" role="group" aria-label=label
            class="fixed z-50 cursor-move rounded-lg bg-popover p-2.5 text-xs text-popover-foreground shadow-md ring-1 ring-foreground/10"
            style:left=move || format!("{}px", current.get().map_or(0.0, |c| c.0))
            style:top=move || format!("{}px", current.get().map_or(0.0, |c| c.1))
            style:width=format!("{CARD_WIDTH}px")
            on:pointerdown=move |event: web_sys::PointerEvent| {
                multiyap_web::chat_tools::focus_card(&down_id);
                let on_border = event.target().zip(event.current_target()).is_some_and(|(t, c)| t == c);
                if !on_border || event.button() != 0 {
                    return;
                }
                event.prevent_default();
                if let Some(element) = event.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                    let _ = element.set_pointer_capture(event.pointer_id());
                }
                let (x, y, _) = current.get_untracked().unwrap_or((0.0, 0.0, false));
                drag.set_value(Some((f64::from(event.client_x()) - x, f64::from(event.client_y()) - y)));
                moved_by_user.set_value(true);
            }
            on:pointermove=move |event: web_sys::PointerEvent| {
                if let Some((dx, dy)) = drag.get_value() {
                    let (x, y) = (f64::from(event.client_x()) - dx, f64::from(event.client_y()) - dy);
                    multiyap_web::chat_tools::move_card(&move_id, x, y);
                }
            }
            on:pointerup=move |_| drag.set_value(None)>
            <div class="cursor-auto">
                {if card.channel.platform == multiyap_core::youtube::PLATFORM {
                    view! { <YouTubeCardContent card_id=id channel=card.channel user=card.user badges=card.badges pinned=pinned layer=layer /> }.into_any()
                } else {
                    view! { <CardContent card_id=id channel=card.channel user=card.user badges=card.badges pinned=pinned layer=layer /> }.into_any()
                }}
            </div>
        </div>
    }
}

/// Ebene für alle offenen Nutzerkarten (einmal in der App). Klick daneben und Esc schließen nicht angepinnte Karten;
/// Menüs, Fenster und Hinweise der Karte zählen nicht als „daneben“.
#[component]
pub fn UserCardLayer() -> impl IntoView {
    let read_cards = || crate::bridge::convert::<Vec<CardWindow>>(&multiyap_web::chat_tools::card_windows()).unwrap_or_default();
    let (cards, set_cards) = signal(read_cards());
    let cards_listener = multiyap_web::chat_tools::subscribe_cards(Rc::new(move || {
        set_cards.try_set(read_cards());
    }));
    on_cleanup(move || multiyap_web::chat_tools::unsubscribe_cards(cards_listener));
    let layer = Layer::new();
    let listeners = WindowListeners::new();
    listeners.add(
        "pointerdown",
        Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
            let inside = event.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()).is_some_and(|t| t.closest(INSIDE_SELECTOR).ok().flatten().is_some());
            if !inside {
                multiyap_web::chat_tools::close_unpinned_cards();
            }
        })
        .into_js_value(),
    );
    // Klicks in den Twitch-Player (iframe) erreichen das Fenster nicht; dort zählt, dass er den Fokus bekommt.
    listeners.add(
        "blur",
        Closure::<dyn FnMut(web_sys::Event)>::new(|_| {
            let into_frame = window().document().and_then(|d| d.active_element()).is_some_and(|e| e.tag_name().eq_ignore_ascii_case("iframe"));
            if into_frame {
                multiyap_web::chat_tools::close_unpinned_cards();
            }
        })
        .into_js_value(),
    );
    listeners.add(
        "keydown",
        Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(|event: web_sys::KeyboardEvent| {
            if event.key() != "Escape" || event.default_prevented() {
                return;
            }
            // Offene Menüs und Fenster schließen zuerst sich selbst.
            let open = window().document().and_then(|d| d.query_selector("[role=\"menu\"], [role=\"dialog\"], [role=\"alertdialog\"]").ok().flatten());
            if open.is_none() {
                multiyap_web::chat_tools::close_unpinned_cards();
            }
        })
        .into_js_value(),
    );
    on_cleanup({
        let listeners = listeners.clone();
        move || listeners.clear()
    });
    view! {
        <For each=move || cards.get() key=|card| card.id.clone() let:card>
            <CardWindowView card=card cards=cards layer=layer />
        </For>
        <LayerView layer=layer />
    }
}
