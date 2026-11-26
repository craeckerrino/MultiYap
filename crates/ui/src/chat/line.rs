//! Chatzeile in Leptos (Nachbau von chat-line.tsx, event-line.tsx, automod-line.tsx, quick-buttons.tsx): alle
//! Zeilenarten mit Bedienung. Name und „@Name“ öffnen die Nutzerkarte, Rechtsklick öffnet das Menü, Schnellbuttons
//! und AutoMod-Knöpfe lösen Mod-Aktionen aus (geprüft zentral in TypeScript). Chat-Inhalte erscheinen nur als Text
//! oder Bild, nie als HTML. Ohne `ChatActions` (Vorschau in „Darstellung“) ist die Zeile nur Anzeige.

use multiyap_core::i18n::tr;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use multiyap_core::chat_text::{
    badge_label, describe_mod_action, is_protected_sender, mention_name, removed_note, split_names, suspicious_label, NamePart,
};
use multiyap_core::model::{AutoModItem, Badge, ChannelRef, ChatEventItem, ChatMessage, Emote, MessagePart, ModActionItem, UserRef};
use multiyap_core::render::{expand_emotes, has_link, stack_overlays, with_links, RenderPart};
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};
use wasm_bindgen::{JsCast, JsValue};

use super::item::{ChatItem, DividerItem, Row, SystemNotice};
use super::layer::{run_mod_action, Layer, MenuState, ModTarget, ReasonState, Rect};
use crate::bridge::toast;
use crate::components::button::button_class;
use crate::components::icons;
use crate::settings::model::{QuickButton, ReasonDialog};

/// Darstellung entfernter Nachrichten (Einstellung chat.removedDisplay).
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct RemovedDisplay {
    pub mode: String,
    pub strikethrough: bool,
    pub dimmed: bool,
    pub italic: bool,
}

impl Default for RemovedDisplay {
    fn default() -> Self {
        RemovedDisplay { mode: "styled".into(), strikethrough: true, dimmed: true, italic: false }
    }
}

/// Einstellungen, die jede Zeile braucht (auch die Vorschau).
#[derive(Clone, Debug, PartialEq)]
pub struct LineOptions {
    pub time_format: String,
    pub removed_display: RemovedDisplay,
    pub readable_names: bool,
    /// Erste Nachrichten markieren (Hinterlegung und Hinweis „Erste Nachricht“).
    pub mark_first_message: bool,
    /// Nachrichten mit Links leicht hinterlegen.
    pub mark_links: bool,
}

/// Bedienung aus der Chat-Liste: schwebende Ebene und Pause des Tabs (ohne Pause, zum Beispiel in der Nutzerkarte,
/// läuft der Chat weiter).
#[derive(Clone, Copy)]
pub struct ChatActions {
    pub layer: Layer,
    pub pause: Option<super::pause::ChatPause>,
}

impl ChatActions {
    /// Hält die Chat-Anzeige an; liefert die Funktion, die die Pause wieder aufhebt.
    pub fn hold(&self) -> JsValue {
        self.pause.map_or(JsValue::UNDEFINED, |pause| pause.hold())
    }

    /// Nutzerkarte unter dem Element öffnen; der Chat steht still, bis sie zu ist.
    pub fn open_card(&self, channel: &ChannelRef, user: &UserRef, badges: Option<&[Badge]>, element: &web_sys::Element) {
        let rect = Rect::of(element);
        open_card_at(channel, user, badges, rect, self.hold());
    }
}

/// Nutzerkarte unter dem Element öffnen; `release` hebt die Chat-Pause wieder auf (crates/web/src/chat_tools.rs).
fn open_card_at(channel: &ChannelRef, user: &UserRef, badges: Option<&[Badge]>, rect: Rect, release: JsValue) {
    multiyap_web::chat_tools::open_card(channel.clone(), user.clone(), badges.map(<[Badge]>::to_vec), rect.left, rect.bottom + 4.0, release.dyn_into::<js_sys::Function>().ok());
}

/// Markierung einer Zeile: Erwähnung oder Filter-Kategorie (berechnet in der Chat-Liste).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LineInfo {
    pub mention: bool,
    pub term: Option<TermInfo>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TermInfo {
    pub name: String,
    pub color: String,
    pub scroll_marker: bool,
}

/// Für alle Zeilen einer Liste gleich.
#[derive(Clone)]
pub struct LineEnv {
    pub options: LineOptions,
    pub reply_button: bool,
    pub quick_buttons_in_chat: bool,
    pub quick_buttons: Rc<Vec<QuickButton>>,
    pub reason_dialog: ReasonDialog,
    pub actions: Option<ChatActions>,
    /// Zähler der Spam-Erkennung (Nachrichten-ID → Anzahl). Ein eigenes Signal, damit bei neuen Nachrichten nur die
    /// kleinen Zähler neu zeichnen und nicht jede Zeile.
    pub spam_counts: Option<RwSignal<Rc<HashMap<String, u32>>, LocalStorage>>,
    /// Ab dieser Anzahl rot hervorgehoben.
    pub spam_highlight_at: f64,
}

/// Je Zeile verschieden.
#[derive(Clone, Default)]
pub struct RowEnv {
    pub can_moderate: bool,
    pub channel_label: Option<String>,
    pub info: LineInfo,
    pub emotes: Option<Rc<HashMap<String, Emote>>>,
}

thread_local! {
    static FORMATTERS: RefCell<HashMap<String, js_sys::Function>> = RefCell::new(HashMap::new());
    static AVATARS: RefCell<HashMap<String, Option<String>>> = RefCell::new(HashMap::new());
}

/// Uhrzeit wie Intl.DateTimeFormat('de-DE', …) in TypeScript: HH:MM oder HH:MM:SS.
/// Link im privaten Fenster öffnen, wenn die Einstellung an ist: Das Programmfenster fängt die Adresse
/// `/open-private?url=…` beim Öffnen als neues Fenster ab und startet den Standardbrowser privat
/// (src-tauri/src/private_browser.rs). Sonst öffnet der Link wie gewohnt.
fn open_link_private(event: &web_sys::MouseEvent, url: &str) {
    let private = multiyap_web::settings::get("chat.openLinksPrivate").and_then(|v| v.as_bool()).unwrap_or(false);
    if !private {
        return;
    }
    event.prevent_default();
    let marker = format!("/open-private?url={}", js_sys::encode_uri_component(url));
    if let Some(window) = web_sys::window() {
        let _ = window.open_with_url_and_target_and_features(&marker, "_blank", "noopener,noreferrer");
    }
}

pub fn format_time(timestamp: f64, format: &str) -> String {
    let function = FORMATTERS.with(|cache| {
        cache
            .borrow_mut()
            .entry(format.to_string())
            .or_insert_with(|| {
                let options = js_sys::Object::new();
                let _ = js_sys::Reflect::set(&options, &"hour".into(), &"2-digit".into());
                let _ = js_sys::Reflect::set(&options, &"minute".into(), &"2-digit".into());
                if format == "HH:MM:SS" {
                    let _ = js_sys::Reflect::set(&options, &"second".into(), &"2-digit".into());
                }
                let locales = js_sys::Array::of1(&tr("de-DE", "en-US").into());
                js_sys::Intl::DateTimeFormat::new(&locales, &options).format()
            })
            .clone()
    });
    function.call1(&JsValue::NULL, &JsValue::from_f64(timestamp)).ok().and_then(|v| v.as_string()).unwrap_or_default()
}

fn dark_theme() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
        .is_none_or(|root| root.class_list().contains("dark"))
}

/// Namensfarbe, auf Wunsch aufgehellt (lesbare Benutzernamen). Ohne eigene Farbe eine feste Standardfarbe von Twitch.
pub fn name_color(color: Option<&str>, user_id: &str, readable: bool) -> String {
    let color = color.filter(|c| !c.is_empty()).unwrap_or_else(|| multiyap_core::color::default_name_color(user_id));
    if readable { multiyap_core::color::readable_name_color(color, dark_theme()) } else { color.to_string() }
}

/// Farbklassen der Filter-Kategorien (Tailwind braucht die Namen ausgeschrieben).
pub fn term_row_class(color: &str) -> &'static str {
    match color {
        "term-1" => "bg-term-1/15 border-term-1",
        "term-2" => "bg-term-2/15 border-term-2",
        "term-3" => "bg-term-3/15 border-term-3",
        "term-4" => "bg-term-4/15 border-term-4",
        "term-5" => "bg-term-5/15 border-term-5",
        _ => "bg-term-6/15 border-term-6",
    }
}

pub fn term_marker_class(color: &str) -> &'static str {
    match color {
        "term-1" => "bg-term-1",
        "term-2" => "bg-term-2",
        "term-3" => "bg-term-3",
        "term-4" => "bg-term-4",
        "term-5" => "bg-term-5",
        _ => "bg-term-6",
    }
}

const EMOTE_SOURCE: [(&str, &str); 4] = [("twitch", "Twitch"), ("seventv", "7TV"), ("bttv", "BTTV"), ("ffz", "FFZ")];
const NAME_BUTTON: &str = "font-semibold outline-none hover:underline focus-visible:underline";

/// Name und Quelle eines Emotes, zum Beispiel „Kappa (Twitch)“.
fn emote_label(emote: &Emote) -> String {
    let source = EMOTE_SOURCE.iter().find(|(id, _)| *id == emote.source).map_or(emote.source.as_str(), |(_, label)| label);
    format!("{} ({source})", emote.name)
}

/// Emote-Bild in Chatgröße.
fn emote_image(emote: &Emote, class: &str) -> impl IntoView {
    let srcset = emote.image_url2x.as_ref().map(|url2x| format!("{} 1x, {url2x} 2x", emote.image_url));
    view! {
        <img class=format!("h-7 w-auto {class}") src=emote.image_url.clone() srcset=srcset alt=emote.name.clone() loading="lazy" decoding="async" />
    }
}

/// Emote mit seinem Namen beim Drüberfahren, als Tooltip aus dem Designsystem wie bei den Abzeichen; ohne Bedienung
/// (zum Beispiel in der Nutzerkarte) als einfacher Titel.
fn emote_view(label: String, class: &'static str, actions: Option<ChatActions>, content: AnyView) -> AnyView {
    let title = actions.is_none().then(|| label.clone());
    view! {
        <span class=class title=title
            on:pointerenter=move |event: web_sys::PointerEvent| {
                if let (Some(actions), Some(element)) = (actions, event.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok())) {
                    actions.layer.show_tip(&element, label.clone(), false);
                }
            }
            on:pointerleave=move |_| if let Some(actions) = actions { actions.layer.hide_tip() }>
            {content}
        </span>
    }
    .into_any()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FoundUser {
    user_id: String,
    login: String,
    display_name: String,
}

/// „@Name“: Klick öffnet die Nutzerkarte. Die Erwähnung kennt nur den Login; die Person wird beim Klick nachgeschlagen
/// (in TypeScript für die Sitzung zwischengespeichert).
fn mention_button(text: String, login: String, channel: ChannelRef, actions: ChatActions) -> AnyView {
    view! {
        <button type="button" class="font-medium outline-none hover:underline focus-visible:underline" on:click=move |event: web_sys::MouseEvent| {
            let Some(element) = event.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) else { return };
            let rect = Rect::of(&element);
            let release = actions.hold();
            let (login, channel) = (login.clone(), channel.clone());
            spawn_local(async move {
                let released = || {
                    if let Some(release) = release.dyn_ref::<js_sys::Function>() {
                        let _ = release.call0(&JsValue::NULL);
                    }
                };
                match crate::bridge::user_by_login(&login).await {
                    Ok(value) => match value.and_then(|v| serde_json::from_value::<FoundUser>(v).ok()) {
                        Some(found) => {
                            let user = UserRef { id: found.user_id, login: found.login, display_name: found.display_name };
                            open_card_at(&channel, &user, None, rect, release.clone());
                        }
                        None => {
                            released();
                            toast("error", &multiyap_core::tf!("Nutzer „{login}“ nicht gefunden.", "User “{login}” not found."));
                        }
                    },
                    Err(_) => {
                        released();
                        toast("error", tr("Nutzerkarte konnte nicht geladen werden.", "User card could not be loaded."));
                    }
                }
            });
        }>
            {text}
        </button>
    }
    .into_any()
}

/// Bits: Bild der Stufe und Anzahl in ihrer Farbe. Ist das Präfix kein Cheermote des Kanals (oder sind die Bilder noch
/// nicht geladen), bleibt der Text stehen.
fn cheer_part(text: &str, prefix: &str, bits: u64, channel: Option<&ChannelRef>, interactive: bool) -> AnyView {
    let image = channel
        .filter(|_| interactive)
        .and_then(|c| multiyap_web::emotes::cheer_image(&c.id, prefix, bits as f64));
    let Some(image) = image else { return text.to_string().into_any() };
    let label = format!("{bits} {}", if bits == 1 { "Bit" } else { "Bits" });
    view! {
        <span class="inline-flex items-center align-middle font-semibold" style:color=image.color title=label>
            <img class="inline-block h-7 w-auto" src=image.url.clone() srcset=format!("{} 1x, {} 2x", image.url, image.url2x) alt=text.to_string() />
            {bits}
        </span>
    }
    .into_any()
}

/// Nachrichtentext mit Emotes, Links, Erwähnungen und Bits.
pub fn parts_view(parts: &[MessagePart], emotes: Option<&HashMap<String, Emote>>, channel: Option<&ChannelRef>, actions: Option<ChatActions>) -> AnyView {
    let linked = with_links(parts);
    let expanded = match emotes {
        Some(map) if !map.is_empty() => expand_emotes(linked, |name| map.get(name).cloned()),
        _ => linked,
    };
    stack_overlays(expanded)
        .into_iter()
        .map(|entry| match entry {
            RenderPart::Stack { base, overlays } => {
                let label = std::iter::once(&base).chain(overlays.iter()).map(emote_label).collect::<Vec<_>>().join(" + ");
                let content = view! {
                    {emote_image(&base, "col-start-1 row-start-1")}
                    {overlays.iter().map(|o| emote_image(o, "col-start-1 row-start-1 place-self-center")).collect_view()}
                };
                emote_view(label, "inline-grid align-middle", actions, content.into_any())
            }
            RenderPart::Part(part) => match part {
                MessagePart::Text { text } => text.into_any(),
                MessagePart::Mention { text, login } => match (channel, actions) {
                    (Some(channel), Some(actions)) => mention_button(text, login, channel.clone(), actions),
                    _ => view! { <span class="font-medium">{text}</span> }.into_any(),
                },
                // Link-Erkennung: neuer Tab, ohne Verweis auf MultiYap; beim Drüberfahren die echte Zieladresse.
                // Auf Wunsch im privaten Fenster (Einstellung beim Klick gelesen, gilt also sofort).
                MessagePart::Link { text, url } => {
                    let (click_url, aux_url) = (url.clone(), url.clone());
                    view! {
                        <a href=url.clone() target="_blank" rel="noopener noreferrer" title=url class="text-link underline underline-offset-2 hover:opacity-80"
                            on:click=move |event: web_sys::MouseEvent| open_link_private(&event, &click_url)
                            on:auxclick=move |event: web_sys::MouseEvent| if event.button() == 1 { open_link_private(&event, &aux_url) }>{text}</a>
                    }
                    .into_any()
                }
                MessagePart::Emote { emote } => emote_view(emote_label(&emote), "inline-block align-middle", actions, emote_image(&emote, "").into_any()),
                MessagePart::Cheer { text, prefix, bits } => cheer_part(&text, &prefix, bits, channel, actions.is_some()),
                // Emoji-Codes von YouTube ohne Bild: abgeschwächt.
                MessagePart::Shortcode { text } => view! { <span class="text-xs text-muted-foreground">{text}</span> }.into_any(),
            },
        })
        .collect_view()
        .into_any()
}

/// Uhrzeit am Zeilenanfang; im Hervorhebungen-Tab davor Kanal und Plattform (innerhalb der Zeile).
fn time_view(timestamp: f64, env: &LineEnv, row: &RowEnv) -> impl IntoView {
    view! {
        {row.channel_label.clone().map(|label| view! { <span class="mr-1.5 text-xs text-muted-foreground not-italic" title=label.clone()>{label.clone()}</span> })}
        <span class="mr-1.5 text-muted-foreground not-italic tabular-nums">{format_time(timestamp, &env.options.time_format)}</span>
    }
}

/// Abzeichen mit Namen beim Drüberfahren, als Tooltip aus dem Designsystem.
fn badge_icon(src: String, src2x: Option<String>, label: String, actions: Option<ChatActions>) -> impl IntoView {
    let srcset = src2x.map(|url2x| format!("{src} 1x, {url2x} 2x"));
    let tip = label.clone();
    view! {
        <img class="mr-1 inline-block size-4.5 align-middle" src=src srcset=srcset alt=label
            on:pointerenter=move |event: web_sys::PointerEvent| {
                if let (Some(actions), Some(element)) = (actions, event.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok())) {
                    actions.layer.show_tip(&element, tip.clone(), false);
                }
            }
            on:pointerleave=move |_| if let Some(actions) = actions { actions.layer.hide_tip() } />
    }
}

/// Rolle bei YouTube als Symbol (YouTube liefert keine Abzeichen-Bilder), Name beim Drüberfahren wie bei Abzeichen.
fn role_icon(badge: &Badge, actions: Option<ChatActions>) -> Option<AnyView> {
    let icon = match badge.set_id.as_str() {
        "broadcaster" => icons::video("size-3.5").into_any(),
        "moderator" => icons::wrench("size-3.5").into_any(),
        "member" => icons::star("size-3.5").into_any(),
        "verified" => icons::badge_check("size-3.5").into_any(),
        _ => return None,
    };
    let label = badge_label(badge);
    let tip = label.clone();
    Some(
        view! {
            <span role="img" aria-label=label class="mr-1 inline-flex size-4.5 items-center justify-center align-middle text-muted-foreground"
                on:pointerenter=move |event: web_sys::PointerEvent| {
                    if let (Some(actions), Some(element)) = (actions, event.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok())) {
                        actions.layer.show_tip(&element, tip.clone(), false);
                    }
                }
                on:pointerleave=move |_| if let Some(actions) = actions { actions.layer.hide_tip() }>
                {icon}
            </span>
        }
        .into_any(),
    )
}

/// Aussehen eines Namens in einer Aktionszeile: 7TV-Namensfarbe, sonst die Namensfarbe der Person.
#[derive(Clone, Debug, PartialEq)]
enum NameLook {
    /// Noch unbekannt (wird nachgefragt): Farbe der Zeile.
    Plain,
    Color(String),
    Paint { image: String, filter: Option<String> },
}

/// Anklickbarer Name (öffnet die Nutzerkarte); ohne Bedienung nur Text. Bei Twitch in der 7TV-Namensfarbe oder, falls
/// keine da ist, in der Namensfarbe der Person wie im Chat; die Farbe kommt aus den Chatnachrichten der Sitzung oder
/// wird nachgefragt (crates/web/src/name_colors.rs).
fn name_button(channel: &ChannelRef, user: UserRef, badges: Option<Vec<Badge>>, actions: Option<ChatActions>, readable: bool) -> impl IntoView {
    use multiyap_web::{name_colors, seventv};
    let channel = channel.clone();
    let name = user.display_name.clone();
    let twitch = channel.platform == "twitch";
    let user_id = user.id.clone();
    if twitch && name_colors::color(&user_id).is_none() {
        name_colors::request(&user_id);
    }
    let colors = crate::store::watch_version(name_colors::subscribe, name_colors::unsubscribe);
    let paints = crate::store::watch_version(seventv::subscribe, seventv::unsubscribe);
    let look = Memo::new(move |_| {
        colors.track();
        paints.track();
        if !twitch {
            return NameLook::Plain;
        }
        if let Some(paint) = seventv::paint(&user_id, readable) {
            return NameLook::Paint { image: paint.style.background_image, filter: paint.style.filter };
        }
        match name_colors::color(&user_id) {
            Some(color) => NameLook::Color(name_color(Some(&color), &user_id, readable)),
            None => NameLook::Plain,
        }
    });
    let painted = move || matches!(look.get(), NameLook::Paint { .. });
    view! {
        <button type="button" class=NAME_BUTTON
            style:color=move || match look.get() {
                NameLook::Color(color) => Some(color),
                NameLook::Paint { .. } => Some("transparent".to_string()),
                NameLook::Plain => None,
            }
            style:background-image=move || match look.get() { NameLook::Paint { image, .. } => Some(image), _ => None }
            style:background-size=move || painted().then_some("100% 100%")
            style:background-clip=move || painted().then_some("text")
            style:-webkit-background-clip=move || painted().then_some("text")
            style:filter=move || match look.get() { NameLook::Paint { filter, .. } => filter, _ => None }
            on:click=move |event: web_sys::MouseEvent| {
            if let (Some(actions), Some(element)) = (actions, event.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok())) {
                actions.open_card(&channel, &user, badges.as_deref(), &element);
            }
        }>
            {name}
        </button>
    }
}

struct SevenTvBadge {
    tooltip: String,
    image_url: String,
    image_url2x: String,
}

struct SevenTvPaint {
    background_image: String,
    filter: Option<String>,
}

/// Profilbild des Ursprungskanals bei Shared Chat, für die Sitzung gemerkt.
fn source_avatar(channel: &ChannelRef) -> impl IntoView {
    let label = multiyap_core::tf!("Aus dem Chat von {}", "From {}’s chat", channel.display_name);
    let known = AVATARS.with(|cache| cache.borrow().get(&channel.id).cloned());
    let url = RwSignal::new(known.clone().flatten());
    if known.is_none() {
        let id = channel.id.clone();
        spawn_local(async move {
            let found = multiyap_web::chatters::avatar(&id).await;
            AVATARS.with(|cache| cache.borrow_mut().insert(id, found.clone()));
            url.try_set(found);
        });
    }
    move || match url.get() {
        Some(src) => view! { <img src=src alt=label.clone() title=label.clone() class="mr-1 inline-block size-4.5 rounded-full align-middle" loading="lazy" /> }.into_any(),
        None => view! { <span title=label.clone() class="mr-1 inline-block size-4.5 rounded-full bg-muted align-middle"></span> }.into_any(),
    }
}

// ---- Schnellbuttons ----

fn quick_title(button: &QuickButton) -> String {
    match button.action.as_str() {
        "timeout" => format!("Timeout {}", multiyap_core::hotkeys::format_duration(button.duration_seconds.unwrap_or(0))),
        other => crate::settings::model::action_label(other).to_string(),
    }
}

/// Schnellbuttons direkt vor dem Namen. Immer sichtbar, keine Sicherheitsabfrage. Ob sie erscheinen, entscheidet der
/// Aufrufer über die zentrale Mod-Prüfung.
fn quick_buttons(target: ModTarget, env: &LineEnv, actions: ChatActions) -> impl IntoView {
    let dialog = env.reason_dialog;
    let buttons: Vec<QuickButton> = env
        .quick_buttons
        .iter()
        // Löschen braucht eine Nachricht (bei AutoMod-Zeilen gibt es keine im Chat).
        .filter(|b| b.action != "delete" || target.message_id.is_some())
        .cloned()
        .collect();
    view! {
        <span class="mr-1 inline-flex items-center align-middle">
            {buttons.into_iter().map(|button| {
                let timeout = button.action == "timeout";
                // Enger als die Standardgröße, damit die Buttons im Chat wenig Platz brauchen.
                let class = if timeout {
                    button_class("ghost", "xs", "align-middle text-muted-foreground hover:text-foreground text-xs").replacen("h-6 ", "h-4 ", 1).replacen(" px-2 ", " px-0.5 ", 1)
                } else {
                    button_class("ghost", "icon-xs", "align-middle text-muted-foreground hover:text-foreground [&_svg]:size-3").replacen("size-6 ", "size-4 ", 1)
                };
                let title = quick_title(&button);
                let face = match button.action.as_str() {
                    "delete" => icons::trash("").into_any(),
                    "warn" => icons::triangle_alert("").into_any(),
                    "ban" => icons::ban("").into_any(),
                    "unban" => icons::circle_check("").into_any(),
                    _ => multiyap_core::hotkeys::format_duration(button.duration_seconds.unwrap_or(0)).into_any(),
                };
                // Verwarnen hat immer ein Fenster; Timeout und Ban je nach Einstellung; Löschen und Unban nie.
                let needs_dialog = button.action == "warn" || (timeout && dialog.timeout) || (button.action == "ban" && dialog.ban);
                let target = target.clone();
                view! {
                    <button type="button" data-slot="button" class=class aria-label=title.clone() title=title
                        on:click=move |event: web_sys::MouseEvent| {
                            if needs_dialog {
                                let Some(element) = event.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) else { return };
                                actions.layer.reason.set(Some(ReasonState {
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
            }).collect_view()}
        </span>
    }
}

// ---- Zeilenarten ----

/// Nachrichtentext; bei entfernten Nachrichten je nach Einstellung gestaltet oder verborgen (Klick zeigt Text).
fn message_body(message: &ChatMessage, env: &LineEnv, row: &RowEnv) -> AnyView {
    let render = {
        let message = message.clone();
        let emotes = row.emotes.clone();
        let actions = env.actions;
        send_wrapper::SendWrapper::new(move || parts_view(&message.parts, emotes.as_deref(), Some(&message.channel), actions))
    };
    if message.flags.removed.is_none() {
        return render();
    }
    let display = &env.options.removed_display;
    match display.mode.as_str() {
        "normal" => render(),
        "hidden" => {
            let revealed = RwSignal::new(false);
            view! {
                {move || if revealed.get() {
                    render()
                } else {
                    view! {
                        <button type="button" class="text-muted-foreground italic underline-offset-2 hover:underline" on:click=move |_| revealed.set(true)>
                            {tr("Nachricht entfernt, anzeigen", "Message removed, show")}
                        </button>
                    }.into_any()
                }}
            }
            .into_any()
        }
        _ => {
            let mut classes = Vec::new();
            if display.strikethrough {
                classes.push("line-through");
            }
            if display.dimmed {
                classes.push("opacity-50");
            }
            if display.italic {
                classes.push("italic");
            }
            view! { <span class=classes.join(" ")>{render()}</span> }.into_any()
        }
    }
}

fn message_view(message: &ChatMessage, js: JsValue, env: &LineEnv, row: &RowEnv) -> AnyView {
    let actions = env.actions;
    // Schnellbuttons: nur mit Mod-Rechten, nicht bei eigenen Nachrichten, Broadcaster, Mods und Lead Mods.
    let show_quick = actions.is_some() && row.can_moderate && env.quick_buttons_in_chat && message.flags.own != Some(true) && !is_protected_sender(&message.badges);
    let removed = message.flags.removed.clone();
    let mention = row.info.mention;
    // Eine Erwähnung hat Vorrang vor der Farbe einer Begriffs-Kategorie.
    let term = if mention { None } else { row.info.term.clone() };
    // Eingeschränkt oder überwacht: nur in Mod-Kanälen sichtbar.
    let suspicious = if row.can_moderate { message.flags.suspicious.clone() } else { None };

    let mut classes = vec!["group/line chat-line border-l-2 border-transparent px-3 text-chat break-words"];
    // Nachrichten mit Links leicht markiert; andere Hervorhebungen gehen vor.
    let first_message = env.options.mark_first_message && message.flags.first_message == Some(true);
    if env.options.mark_links && has_link(&message.parts) {
        classes.push("bg-link/10");
    }
    if first_message {
        classes.push("bg-first-message/10");
    }
    if let Some(term) = &term {
        classes.push(term_row_class(&term.color));
    }
    if mention {
        classes.push("border-mention bg-mention/15");
    }
    // Eingeschränkt oder überwacht: ganze Zeile in der Farbe (überwacht seit 23.11.2026).
    match suspicious.as_ref().map(|s| s.status.as_str()) {
        Some("restricted") => classes.push("border-restricted bg-restricted/15"),
        Some(_) => classes.push("border-monitored bg-monitored/15"),
        None => {}
    }
    if message.flags.from_history == Some(true) {
        classes.push("opacity-60");
    }
    let title = if mention { Some(tr("Erwähnung", "Mention").to_string()) } else { term.as_ref().map(|t| format!("Filter: {}", t.name)) };

    // 7TV-Kosmetik. 7TV kennt nur Twitch-Konten.
    let twitch = message.channel.platform == "twitch";
    let seven_badge = actions.filter(|_| twitch).and_then(|_| multiyap_web::seventv::badge(&message.sender.id)).map(|b| SevenTvBadge {
        tooltip: b.tooltip,
        image_url: b.image_url,
        image_url2x: b.image_url_2x,
    });
    let paint = actions
        .filter(|_| twitch)
        .and_then(|_| multiyap_web::seventv::paint(&message.sender.id, env.options.readable_names))
        .map(|p| SevenTvPaint { background_image: p.style.background_image, filter: p.style.filter });
    let color = if paint.is_none() { Some(name_color(message.sender.color.as_deref(), &message.sender.id, env.options.readable_names)) } else { None };
    let (paint_image, paint_filter) = paint.map_or((None, None), |p| (Some(p.background_image), p.filter));
    let painted = paint_image.is_some();

    let target = ModTarget {
        channel_id: message.channel.id.clone(),
        user_id: message.sender.id.clone(),
        display_name: message.sender.display_name.clone(),
        message_id: Some(message.id.clone()),
    };
    let user = UserRef { id: message.sender.id.clone(), login: message.sender.login.clone(), display_name: message.sender.display_name.clone() };
    let channel = message.channel.clone();
    let badges = message.badges.clone();
    let shared = Rc::new(message.clone());
    let can_moderate = row.can_moderate;
    // „@Name“ ins Eingabefeld und Antworten gibt es nur bei Twitch (YouTube zuerst nur lesen, ohne Eingabefeld).
    let menu_js = js.clone();
    let reply_message = shared.clone();
    let (login, mention_as) = (message.channel.login.clone(), mention_name(&message.sender.login, &message.sender.display_name));

    view! {
        <div class=classes.join(" ") title=title
            on:contextmenu=move |event: web_sys::MouseEvent| {
                // Rechtsklick-Menü.
                let Some(actions) = actions else { return };
                event.prevent_default();
                actions.layer.menu.set(Some(MenuState {
                    x: f64::from(event.client_x()),
                    y: f64::from(event.client_y()),
                    message: shared.clone(),
                    js: menu_js.clone(),
                    can_moderate,
                }));
            }>
            {message.reply.as_ref().map(|reply| view! {
                <div class="truncate text-xs text-muted-foreground">{multiyap_core::tf!("Antwort an @{}: {}", "Reply to @{}: {}", reply.sender_display_name, reply.text)}</div>
            })}
            {time_view(message.timestamp, env, row)}
            {(show_quick).then(|| actions.map(|a| quick_buttons(target.clone(), env, a)))}
            {message.source_channel.as_ref().map(source_avatar)}
            {message.badges.iter().filter_map(|badge| match badge.image_url.clone() {
                Some(src) => Some(badge_icon(src, badge.image_url2x.clone(), badge_label(badge), actions).into_any()),
                None if badge.source == multiyap_core::youtube::PLATFORM => role_icon(badge, actions),
                None => None,
            }).collect_view()}
            {seven_badge.map(|b| badge_icon(b.image_url, Some(b.image_url2x), format!("{} (7TV)", b.tooltip), actions))}
            <button type="button" class=NAME_BUTTON
                style:color=move || if painted { Some("transparent".to_string()) } else { color.clone() }
                style:background-image=paint_image.clone()
                style:background-size=painted.then_some("100% 100%")
                style:background-clip=painted.then_some("text")
                style:-webkit-background-clip=painted.then_some("text")
                style:filter=paint_filter.clone()
                on:click=move |event: web_sys::MouseEvent| {
                    if let (Some(actions), Some(element)) = (actions, event.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok())) {
                        actions.open_card(&channel, &user, Some(&badges), &element);
                    }
                }
                on:contextmenu=move |event: web_sys::MouseEvent| {
                    // Rechtsklick auf den Namen fügt „@Name “ ins Eingabefeld ein.
                    if actions.is_none() || !twitch {
                        return;
                    }
                    event.prevent_default();
                    event.stop_propagation();
                    multiyap_web::chat_tools::mention_in_input(&login, &mention_as);
                }>
                {message.sender.display_name.clone()}
            </button>
            <span class="text-muted-foreground">": "</span>
            {message_body(message, env, row)}
            // Spam-Zähler „(5x)“ (Nur Mod-Kanäle).
{env.spam_counts.map(|counts| {
                let id = message.id.clone();
                let highlight_at = env.spam_highlight_at;
                move || {
                    let count = counts.with(|map| map.get(&id).copied()).filter(|c| *c > 0)?;
                    let class = if f64::from(count) >= highlight_at {
                        "ml-1.5 rounded-sm px-1 align-middle text-xs tabular-nums bg-spam font-semibold text-background"
                    } else {
                        "ml-1.5 rounded-sm px-1 align-middle text-xs tabular-nums text-spam"
                    };
                    Some(view! { <span class=class title=multiyap_core::tf!("Ähnliche Nachricht {count}x in kurzer Zeit", "Similar message {count}x in a short time")>{format!("({count}x)")}</span> })
                }
            })}
            {first_message.then(|| view! {
                <span class="ml-1.5 rounded-sm border border-first-message/50 px-1 align-middle text-xs text-first-message">{tr("Erste Nachricht", "First message")}</span>
            })}
            {suspicious.map(|s| {
                let class = if s.status == "restricted" {
                    "ml-1.5 rounded-sm px-1 align-middle text-xs bg-restricted font-semibold text-background"
                } else {
                    "ml-1.5 rounded-sm px-1 align-middle text-xs border border-monitored/50 text-monitored"
                };
                let reasons = if s.reasons.is_empty() { String::new() } else { format!(": {}", s.reasons.join(", ")) };
                view! { <span class=class>{suspicious_label(&s.status)}{reasons}</span> }
            })}
            {removed.as_ref().filter(|_| row.can_moderate).map(|r| view! {
                <span class="ml-1.5 text-xs text-mod-action">{format!("({})", removed_note(&r.reason, r.by.as_deref(), r.duration_seconds))}</span>
            })}
            // Antwort-Button am Ende der Nachricht, erscheint beim Überfahren der Zeile.
            {(env.reply_button && twitch && removed.is_none() && actions.is_some()).then(|| view! {
                <button type="button" data-slot="button" class=button_class("ghost", "icon-xs", "ml-1 align-middle text-muted-foreground opacity-0 group-hover/line:opacity-100 focus-visible:opacity-100")
                    aria-label=multiyap_core::tf!("{} antworten", "Reply to {}", message.sender.display_name) title=tr("Antworten", "Reply")
                    on:click=move |_| { multiyap_web::chat_tools::start_reply(&reply_message.channel.login, &reply_message.id, &reply_message.sender.display_name, &reply_message.text); }>
                    {icons::message_square_reply("")}
                </button>
            })}
        </div>
    }
    .into_any()
}

fn notice_text(notice_type: &str) -> &'static str {
    match notice_type {
        "connecting" => tr("Verbinde…", "Connecting…"),
        "connected" => tr("Verbunden", "Connected"),
        "joined" => tr("Kanal beigetreten", "Joined channel"),
        "disconnected" => tr("Verbindung getrennt", "Disconnected"),
        "reconnected" => tr("Erneut verbunden", "Reconnected"),
        "reconnect-requested" => tr("Twitch hat eine neue Verbindung angefordert (zum Beispiel wegen Wartung), sie wird ohne Unterbrechung übernommen", "Twitch requested a new connection (for example for maintenance), it is taken over without interruption"),
        "history-failed" => tr("Verlauf konnte nicht geladen werden", "History could not be loaded"),
        "load-failed" => tr("Konnte nicht geladen werden", "Could not be loaded"),
        "seventv-added" => tr("7TV-Emote hinzugefügt", "7TV emote added"),
        "seventv-removed" => tr("7TV-Emote entfernt", "7TV emote removed"),
        "seventv-renamed" => tr("7TV-Emote umbenannt", "7TV emote renamed"),
        "mod-events-unavailable" => tr("Mod-Aktionen und AutoMod nicht verfügbar", "Mod actions and AutoMod not available"),
        "mod-events-restored" => tr("Mod-Aktionen und AutoMod wieder verfügbar", "Mod actions and AutoMod available again"),
        "stream-online" => tr("Stream ist jetzt live", "Stream is live now"),
        "stream-offline" => tr("Stream ist jetzt offline", "Stream is offline now"),
        _ => "",
    }
}

fn notice_view(notice: &SystemNotice, env: &LineEnv, row: &RowEnv) -> AnyView {
    view! {
        <div class="chat-line px-3 text-chat text-muted-foreground italic">
            {time_view(notice.timestamp, env, row)}
            {notice_text(&notice.notice_type)}
            {notice.detail.as_ref().map(|d| format!(": {d}"))}
        </div>
    }
    .into_any()
}

/// Dezente Systemzeile: welcher Mod welche Aktion durchgeführt hat. Der Name der betroffenen Person ist anklickbar.
fn mod_action_view(item: &ModActionItem, env: &LineEnv, row: &RowEnv) -> AnyView {
    let actions = env.actions;
    let text = describe_mod_action(item);
    let target_part = item.target.as_ref().and_then(|target| text.find(&target.display_name).map(|index| (index, target.clone())));
    let body = match target_part {
        Some((index, target)) => {
            let (before, after) = (text[..index].to_string(), text[index + target.display_name.len()..].to_string());
            view! { {before}{name_button(&item.channel, target, None, actions, env.options.readable_names)}{after} }.into_any()
        }
        None => text.into_any(),
    };
    let class = if item.from_history == Some(true) { "chat-line px-3 text-chat text-mod-action opacity-60" } else { "chat-line px-3 text-chat text-mod-action" };
    view! {
        <div class=class>
            {time_view(item.timestamp, env, row)}
            {item.moderator.clone().map(|m| view! { {name_button(&item.channel, m, None, actions, env.options.readable_names)}" " })}
            {body}
            {item.reason.as_ref().map(|r| format!(": {r}"))}
            {item.source_channel.as_ref().map(|s| format!(" (in {s})"))}
        </div>
    }
    .into_any()
}

fn automod_status(status: &str) -> &'static str {
    match status {
        "approved" => "zugelassen",
        "denied" => "abgelehnt",
        "expired" => "abgelaufen",
        "invalid" => tr("ungültig", "invalid"),
        _ => "offen",
    }
}

/// Zurückgehaltene Nachricht: auffällige Zeile mit Nutzer, Nachricht und Grund. „Zulassen“ grün fett, „Ablehnen“ rot
/// fett (fest). Kein Ton, kein Blinken. Die Zeile aktualisiert sich.
fn automod_view(item: &AutoModItem, _js: JsValue, env: &LineEnv, row: &RowEnv) -> AnyView {
    let actions = env.actions;
    let busy = RwSignal::new(false);
    let pending = item.status == "pending";
    let (automod_channel, automod_id) = (item.channel.id.clone(), item.id.clone());
    let resolve = move |allow: bool| {
        let (channel, id) = (automod_channel.clone(), automod_id.clone());
        busy.set(true);
        spawn_local(async move {
            multiyap_web::chat_tools::resolve_automod(channel, id, allow).await;
            busy.try_set(false);
        });
    };
    let approve = resolve.clone();
    let target = ModTarget { channel_id: item.channel.id.clone(), user_id: item.sender.id.clone(), display_name: item.sender.display_name.clone(), message_id: None };
    view! {
        <div class="mx-2 my-1 flex flex-col gap-1 rounded-md border bg-muted px-2 py-1.5 text-chat leading-relaxed">
            <div class="text-xs text-muted-foreground">
                {if item.own { item.reason.clone() } else { multiyap_core::tf!("Von AutoMod zurückgehalten: {}", "Held by AutoMod: {}", item.reason) }}
                {(!pending).then(|| view! {
                    <span class="font-medium text-foreground">
                        {format!(" · {}{}", automod_status(&item.status), item.resolved_by.as_ref().map(|b| multiyap_core::tf!(" von {b}", " by {b}")).unwrap_or_default())}
                    </span>
                })}
            </div>
            <div class="break-words">
                {env.quick_buttons_in_chat.then(|| actions.map(|a| quick_buttons(target, env, a)))}
                {time_view(item.timestamp, env, row)}
                {name_button(&item.channel, item.sender.clone(), None, actions, env.options.readable_names)}
                <span class="text-muted-foreground">": "</span>
                {parts_view(&item.parts, row.emotes.as_deref(), Some(&item.channel), actions)}
            </div>
            {(pending && !item.own && actions.is_some()).then(|| view! {
                <div class="flex gap-2">
                    <button type="button" data-slot="button" disabled=move || busy.get()
                        class=button_class("plain", "sm", "bg-automod-approve font-bold text-background hover:bg-automod-approve/80")
                        on:click=move |_| approve(true)>{tr("Zulassen", "Allow")}</button>
                    <button type="button" data-slot="button" disabled=move || busy.get()
                        class=button_class("plain", "sm", "bg-automod-deny font-bold text-background hover:bg-automod-deny/80")
                        on:click=move |_| resolve(false)>{tr("Ablehnen", "Deny")}</button>
                </div>
            })}
        </div>
    }
    .into_any()
}

/// Trennlinie: „Live ab hier“ (Farbe über das Token live-marker) oder Datumstrenner.
fn divider_view(item: &DividerItem) -> AnyView {
    let live = item.variant == "live";
    let (text, line) = if live { ("text-live-marker", "h-px flex-1 bg-live-marker") } else { ("text-muted-foreground", "h-px flex-1 bg-border") };
    view! {
        <div role="separator" class=format!("flex items-center gap-2 px-3 py-1 text-xs {text}")>
            <span class=line></span>
            <span>{item.label.clone()}</span>
            <span class=line></span>
        </div>
    }
    .into_any()
}

/// Chat-Ereignis: einheitlich in leichtem Rosa, ohne Einstellungen, ohne Ton. Namen der Beteiligten öffnen die
/// Nutzerkarte; Community-Geschenke lassen sich für die Empfänger aufklappen.
fn event_view(item: &ChatEventItem, env: &LineEnv, row: &RowEnv) -> AnyView {
    let actions = env.actions;
    let text = view! {
        <span class="font-medium">
            {split_names(&item.text, item.users.as_deref().unwrap_or_default()).into_iter().map(|part| match part {
                NamePart::Text(text) => view! { <span>{text}</span> }.into_any(),
                NamePart::User(user) => name_button(&item.channel, user, None, actions, env.options.readable_names).into_any(),
            }).collect_view()}
        </span>
    };
    let body = view! { {time_view(item.timestamp, env, row)}{text} };
    let recipients = item.recipients.clone().unwrap_or_default();
    let open = RwSignal::new(false);
    let head = if recipients.is_empty() {
        view! { <div>{body}</div> }.into_any()
    } else {
        let count = recipients.len();
        let list = recipients.join(", ");
        view! {
            <div data-slot="collapsible" data-state=move || if open.get() { "open" } else { "closed" }>
                // Nur Anzahl und Pfeil klappen auf, damit die Namen im Text eigene Knöpfe bleiben.
                <div class="flex w-full items-center gap-1">
                    <span class="min-w-0 flex-1">{body}</span>
                    <button type="button" data-slot="collapsible-trigger" aria-expanded=move || open.get().to_string()
                        data-state=move || if open.get() { "open" } else { "closed" }
                        class="group/event flex shrink-0 items-center gap-1 text-xs opacity-80 hover:opacity-100"
                        on:click=move |_| open.update(|o| *o = !*o)>
                        {multiyap_core::tf!("{count} Empfänger", "{count} recipients")}
                        {icons::chevron_down("size-3.5 transition-transform group-data-[state=open]/event:rotate-180")}
                    </button>
                </div>
                {move || open.get().then(|| view! {
                    <div data-slot="collapsible-content"><p class="pt-1 text-xs">{list.clone()}</p></div>
                })}
            </div>
        }
        .into_any()
    };
    let class = if item.from_history == Some(true) { "chat-line px-3 opacity-60" } else { "chat-line px-3" };
    view! {
        <div class=class>
            <div class="rounded-md bg-chat-event px-2 py-1 text-chat break-words text-chat-event-foreground">
                {head}
                {item.message.as_ref().map(|parts| view! {
                    <div class="pt-0.5">{parts_view(parts, row.emotes.as_deref(), Some(&item.channel), actions)}</div>
                })}
                // Link am Ende (zum Beispiel „Raid beitreten“), öffnet im normalen Browser.
                {item.link.clone().map(|link| view! {
                    <div class="pt-1">
                        <a href=link.url target="_blank" rel="noopener noreferrer" data-slot="button"
                            class=button_class("secondary", "xs", "")>
                            {link.label}
                            {icons::external_link("")}
                        </a>
                    </div>
                })}
            </div>
        </div>
    }
    .into_any()
}

/// Eine Zeile der Chat-Liste. Mod-Aktionen und AutoMod nur in Mod-Kanälen.
pub fn chat_line(row_data: &Row, env: &LineEnv, row: &RowEnv) -> Option<AnyView> {
    Some(match &*row_data.item {
        ChatItem::Divider(item) => divider_view(item),
        ChatItem::Event(item) => event_view(item, env, row),
        ChatItem::Message(message) => message_view(message, row_data.js.clone(), env, row),
        ChatItem::System(notice) => notice_view(notice, env, row),
        ChatItem::ModAction(item) => {
            if !row.can_moderate {
                return None;
            }
            mod_action_view(item, env, row)
        }
        ChatItem::AutoMod(item) => {
            if !row.can_moderate {
                return None;
            }
            automod_view(item, row_data.js.clone(), env, row)
        }
    })
}

/// Nachrichtenzeile für die Vorschau in „Darstellung“ (ohne Bedienung).
pub fn message_line(message: &ChatMessage, options: &LineOptions) -> impl IntoView {
    let env = LineEnv {
        options: options.clone(),
        reply_button: false,
        quick_buttons_in_chat: false,
        quick_buttons: Rc::new(Vec::new()),
        reason_dialog: ReasonDialog { timeout: false, ban: false },
        actions: None,
        spam_counts: None,
        spam_highlight_at: f64::INFINITY,
    };
    message_view(message, JsValue::UNDEFINED, &env, &RowEnv::default())
}
