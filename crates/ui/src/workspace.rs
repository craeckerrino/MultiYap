//! Arbeitsfläche in Leptos (Nachbau von workspace.tsx, add-tab-dialog.tsx und close-tab-confirm.tsx): Seiten-Leiste
//! oben und Bereiche mit eigener Kopfzeile (crates/ui/src/layout.rs), „Neuer Tab“ (Abschnitt 5.2), die Bestätigung vor
//! dem Schließen eines Tabs oder einer Seite und die Tastenkürzel der Arbeitsfläche. Den Stand hält
//! crates/web/src/workspace.rs (prüfen, speichern, Änderungen nacheinander).

use multiyap_core::i18n::tr;
use std::collections::HashMap;
use std::rc::Rc;

use multiyap_core::channel_input;
use multiyap_core::modules::{tab_kinds, Module, PLATFORMS};
use multiyap_web::workspace as web;
use leptos::prelude::*;
use serde::Deserialize;
use serde_json::Value;
use wasm_bindgen::prelude::*;
use web_sys::KeyboardEvent;

use crate::components::button::button_class;
use crate::components::dialog::{Dialog, DIALOG_FOOTER, LABEL};
use crate::components::input::INPUT_CLASS;
use crate::components::overlay::{window, ConfirmDialog, WindowListeners};
use crate::components::ComingSoonBadge;
use crate::layout::{area_tab, run, tabs_in, Node, WorkspaceLayout};

/// Gewählte Tab-Art: grüner Rand und heller grüner Hintergrund, auch über die dunklen Werte von „outline“ hinweg.
const SELECTED_KIND: &str = "border-primary bg-primary/20 hover:bg-primary/25 dark:border-primary dark:bg-primary/20 dark:hover:bg-primary/25";

/// Ein Tab, wie ihn die Oberfläche braucht (gespeichert in crates/web/src/workspace.rs).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TabInfo {
    pub id: String,
    /// "channel", "stream" oder "highlights".
    pub kind: String,
    /// Plattform (Kanal- und Stream-Tabs): "twitch" oder "youtube".
    pub platform: String,
    /// Login des Kanals (Kanal- und Stream-Tabs), bei YouTube der Schlüssel „@name“ oder „UC…“.
    pub channel: String,
    /// Eigener Anzeigename.
    pub nickname: Option<String>,
    /// Anzeige-Auswahl (Hervorhebungen-Tab).
    pub show: Value,
    /// Zähler ungesehener Treffer (Hervorhebungen-Tab).
    pub counter: bool,
}

/// Eine Seite der oberen Leiste (wie crates/core/src/workspace.rs).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct PageView {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub root: Node,
}

/// Stand der Arbeitsfläche: Seiten mit ihrem Layout, aktive Seite, zuletzt benutzter Bereich und Tabs.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceView {
    #[serde(default)]
    pub pages: Vec<PageView>,
    #[serde(default)]
    pub active_page_id: String,
    pub focused_area_id: String,
    #[serde(default)]
    pub tabs: Vec<TabInfo>,
}

impl WorkspaceView {
    fn read() -> Self {
        serde_json::to_value(web::state()).ok().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
    }

    pub fn page(&self, id: &str) -> Option<&PageView> {
        self.pages.iter().find(|p| p.id == id)
    }

    /// Tabs einer Seite in Lesereihenfolge.
    pub fn page_tabs(&self, page: &str) -> Vec<String> {
        self.page(page).map(|p| tabs_in(&p.root)).unwrap_or_default()
    }

    /// Die gerade sichtbaren Tabs (alle auf der aktiven Seite).
    pub fn active_tabs(&self) -> Vec<String> {
        self.page_tabs(&self.active_page_id)
    }

    /// Tab im zuletzt benutzten Bereich.
    fn focused_tab(&self) -> Option<String> {
        area_tab(&self.page(&self.active_page_id)?.root, &self.focused_area_id).flatten()
    }
}

/// Was ein Tab-Eintrag über seinen Tab meldet, damit der Seiten-Tab oben Namen, Live-Punkt, „ungesehen“ und Blinken
/// seiner Chats zeigen kann.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TabStatus {
    pub name: String,
    pub live: bool,
    pub unread: bool,
    /// Klasse zum Pulsieren, leer: kein Pulsieren.
    pub blink: String,
}

/// Bestätigung vor dem Schließen: ein Tab (ID, Name) oder eine Seite mit mehreren Chats (ID, Name, Anzahl).
#[derive(Clone, Debug, PartialEq)]
enum CloseRequest {
    Tab(String, String),
    Page(String, String, usize),
}

/// Gemeinsamer Stand der Arbeitsfläche für Layout, Tab-Einträge und Inhalte.
#[derive(Clone, Copy)]
pub struct WorkspaceCtx {
    pub state: RwSignal<WorkspaceView>,
    /// Ziel von „Neuer Tab“: ein Bereich (der neue Chat kommt daneben) oder "" für eine neue Seite.
    pub add_target: RwSignal<Option<String>>,
    /// Wovor beim Schließen gefragt wird.
    close_request: RwSignal<Option<CloseRequest>>,
    /// Gemeldeter Zustand je Tab (mit Nummer des meldenden Eintrags, siehe `publish_status`).
    status: RwSignal<HashMap<String, (u64, TabStatus)>>,
    /// Welcher Tab gerade gezogen wird. Browser verraten die Daten eines Zugs erst beim Loslassen, die Zielanzeige
    /// braucht sie aber schon während des Ziehens.
    pub dragged: RwSignal<Option<String>>,
}

impl WorkspaceCtx {
    /// Fragt nach, bevor ein Tab geschlossen wird (X, Mittelklick und „Tab schließen“ im ⋮-Menü).
    pub fn request_close(&self, tab_id: String, name: String) {
        self.close_request.set(Some(CloseRequest::Tab(tab_id, name)));
    }

    /// Fragt nach, bevor eine Seite geschlossen wird. Mit nur einem Chat ist das dasselbe wie den Tab zu schließen.
    pub fn request_close_page(&self, page_id: String, name: String) {
        let tabs = self.state.with_untracked(|s| s.page_tabs(&page_id));
        match tabs.as_slice() {
            [] => {}
            [tab] => self.request_close(tab.clone(), name),
            _ => self.close_request.set(Some(CloseRequest::Page(page_id, name, tabs.len()))),
        }
    }

    /// Meldet den Zustand eines Tabs, solange der Eintrag besteht. Wird ein Tab verschoben, meldet der neue Eintrag
    /// schon, bevor der alte abgebaut ist; die Nummer verhindert, dass der alte den neuen Stand löscht.
    pub fn publish_status(&self, tab_id: String, status: impl Fn() -> TabStatus + 'static) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let token = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let store = self.status;
        let id = tab_id.clone();
        Effect::new(move |_| {
            let next = (token, status());
            if store.with_untracked(|s| s.get(&id) != Some(&next)) {
                store.update(|s| {
                    s.insert(id.clone(), next);
                });
            }
        });
        on_cleanup(move || {
            store.try_update(|s| {
                if s.get(&tab_id).is_some_and(|(t, _)| *t == token) {
                    s.remove(&tab_id);
                }
            });
        });
    }

    /// Gemeldeter Zustand eines Tabs (verfolgt Änderungen).
    pub fn status(&self, tab_id: &str) -> Option<TabStatus> {
        self.status.with(|s| s.get(tab_id).map(|(_, status)| status.clone()))
    }

    pub fn start_drag(&self, tab_id: String) {
        self.dragged.set(Some(tab_id));
    }

    pub fn end_drag(&self) {
        if self.dragged.with_untracked(Option::is_some) {
            self.dragged.set(None);
        }
    }

    /// Logins der offenen Kanal-Tabs (Twitch-Login oder YouTube-Schlüssel), jeder einmal (verfolgt Änderungen).
    pub fn open_channels(&self) -> Vec<String> {
        self.state.with(|s| {
            let mut list: Vec<String> = Vec::new();
            for tab in s.tabs.iter().filter(|t| t.kind == "channel") {
                if !list.contains(&tab.channel) {
                    list.push(tab.channel.clone());
                }
            }
            list
        })
    }

    /// Tab nach ID (verfolgt Änderungen).
    pub fn tab(&self, id: &str) -> Option<TabInfo> {
        self.state.with(|s| s.tabs.iter().find(|t| t.id == id).cloned())
    }
}

pub fn workspace_ctx() -> WorkspaceCtx {
    expect_context::<WorkspaceCtx>()
}

#[derive(Clone, Debug, PartialEq)]
struct Choice {
    id: &'static str,
    label: &'static str,
    description: Option<&'static str>,
    available: bool,
}

fn choices(list: &[Module]) -> Vec<Choice> {
    list.iter().map(|m| Choice { id: m.id, label: m.label, description: (!m.description.is_empty()).then_some(m.description), available: m.available() }).collect()
}

/// Knopf einer Auswahl (Tab-Art oder Plattform): outline, gewählt grün hinterlegt, nicht verfügbar mit „Coming Soon“.
fn choice_button(choice: Choice, selected: Signal<bool>, on_pick: impl Fn() + 'static) -> impl IntoView {
    let available = choice.available;
    view! {
        <button type="button" data-slot="button" title=choice.description disabled=!available
            aria-pressed=move || selected.get().to_string()
            class=move || button_class("outline", "default", if selected.get() { SELECTED_KIND } else { "" })
            on:click=move |_| on_pick()>
            {choice.label}
            {(!available).then(|| view! { <ComingSoonBadge /> })}
        </button>
    }
}

/// Hinweis vor dem Öffnen eines YouTube-Kanals (YouTube API Services Developer Policies, III.A.1 und III.A.2): Mit dem
/// Öffnen gelten die Nutzungsbedingungen von YouTube, dazu die Datenschutzerklärungen von Google und MultiYap
/// (Abschnitt 7 auf multiyap.app). Links öffnen im Browser.
fn youtube_consent() -> impl IntoView {
    let en = multiyap_core::i18n::is_en();
    let privacy = format!("{}{}", multiyap_core::youtube::WEBSITE, if en { "/en/privacy/" } else { "/de/datenschutz/" });
    let link = |href: String, text: &'static str| {
        view! { <a class="text-primary underline-offset-4 hover:underline" href=href target="_blank" rel="noopener noreferrer">{text}</a> }
    };
    view! {
        <p class="text-xs leading-relaxed text-muted-foreground">
            {tr("YouTube-Chats sind vorerst nur zum Lesen, ohne Google-Konto. MultiYap nutzt die YouTube API Services. Mit dem Öffnen stimmst du den ", "YouTube chats are read-only for now, no Google account needed. MultiYap uses the YouTube API Services. By opening a channel you agree to the ")}
            {link("https://www.youtube.com/t/terms".into(), tr("Nutzungsbedingungen von YouTube", "YouTube Terms of Service"))}
            {tr(" zu. Es gelten die ", ". The ")}
            {link("http://www.google.com/policies/privacy".into(), tr("Datenschutzerklärung von Google", "Google Privacy Policy"))}
            {tr(" und die ", " and the ")}
            {link(privacy, tr("Datenschutzerklärung von MultiYap", "MultiYap privacy policy"))}
            {tr(" (Abschnitt 7).", " (section 7) apply.")}
        </p>
    }
}

/// „+“-Dialog: Tab-Art wählen, bei „Kanal“ und „Stream“ Plattform und Kanalname, bei „Hervorhebungen“ direkt öffnen.
#[component]
fn AddTabDialog() -> impl IntoView {
    let ctx = workspace_ctx();
    let target = ctx.add_target;
    let kinds = StoredValue::new(choices(&tab_kinds()));
    let platforms = StoredValue::new(choices(&PLATFORMS));
    let open = RwSignal::new(false);
    let kind = RwSignal::new("channel".to_string());
    let platform = RwSignal::new("twitch".to_string());
    let input = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let field_ref = NodeRef::<leptos::html::Input>::new();
    let open_ref = NodeRef::<leptos::html::Button>::new();
    // Bereich, neben den der neue Tab kommt; ohne Bereich eine neue Seite.
    let area = move || target.get_untracked().filter(|a| !a.is_empty());

    // Geöffnet von außen (Bereich gewählt); beim Öffnen zurücksetzen.
    Effect::new(move |_| {
        let is_open = target.with(Option::is_some);
        if is_open && !open.get_untracked() {
            kind.set("channel".into());
            platform.set("twitch".into());
            input.set(String::new());
            error.set(None);
        }
        open.set(is_open);
    });
    // Geschlossen über X, Esc oder Klick daneben.
    Effect::new(move |_| {
        if !open.get() && target.with_untracked(Option::is_some) {
            target.set(None);
        }
    });
    // Wie autoFocus: Kanalfeld bzw. „Öffnen“ bekommt den Fokus.
    // Erst im nächsten Bild: Der Inhalt zur gewählten Art steht dann im Dokument.
    Effect::new(move |_| {
        kind.track();
        if !open.get() {
            return;
        }
        request_animation_frame(move || {
            // Eine NodeRef kann noch auf ein entferntes Element zeigen; nur Elemente im Dokument zählen. Ist das Fenster
            // bis zum nächsten Bild schon entsorgt, gibt es nichts mehr zu tun (try_, sonst Absturz).
            if let Some(field) = field_ref.try_get_untracked().flatten().filter(|f| f.is_connected()) {
                let _ = field.focus();
            } else if let Some(button) = open_ref.try_get_untracked().flatten().filter(|b| b.is_connected()) {
                let _ = button.focus();
            }
        });
    });

    // YouTube gibt es zuerst nur als Kanal-Tab (lesen), noch nicht als Stream-Tab.
    let platform_offered = move |id: &str| platforms.with_value(|list| list.iter().find(|p| p.id == id).is_some_and(|p| p.available)) && !(id == "youtube" && kind.get() == "stream");
    let platform_available = move || platform_offered(&platform.get());

    let content = move || {
        let kind_buttons = kinds
            .get_value()
            .into_iter()
            .map(|choice| {
                let id = choice.id;
                let selected = Signal::derive(move || kind.get() == id);
                choice_button(choice, selected, move || kind.set(id.to_string()))
            })
            .collect_view();
        let body = match kind.get().as_str() {
            "highlights" => view! {
                <div class="flex flex-col gap-4">
                    <p class="text-sm text-muted-foreground">
                        {tr("Sammelt Filter-Treffer, Erwähnungen, Chat-Ereignisse, Mod-Aktionen und AutoMod aus allen offenen Kanälen. Was angezeigt wird, lässt sich danach im ⋮-Menü des Tabs unter „Anzeigen“ wählen.", "Collects filter matches, mentions, chat events, mod actions and AutoMod from all open channels. What is shown can then be chosen in the tab’s ⋮ menu under “Show”.")}
                    </p>
                    <div data-slot="dialog-footer" class=DIALOG_FOOTER>
                        <button node_ref=open_ref type="button" data-slot="button" class=button_class("default", "default", "")
                            on:click=move |_| {
                                let area = area();
                                run(async move { web::add_highlights_tab(area).await });
                                open.set(false);
                            }>
                            {tr("Öffnen", "Open")}
                        </button>
                    </div>
                </div>
            }
            .into_any(),
            _ => {
                let platform_buttons = platforms
                    .get_value()
                    .into_iter()
                    .map(|choice| {
                        let id = choice.id;
                        let selected = Signal::derive(move || platform.get() == id);
                        let choice = Choice { available: platform_offered(id), ..choice };
                        choice_button(choice, selected, move || platform.set(id.to_string()))
                    })
                    .collect_view();
                view! {
                    <form class="flex flex-col gap-4" on:submit=move |event: web_sys::SubmitEvent| {
                        event.prevent_default();
                        if !platform_available() {
                            return;
                        }
                        let area = area();
                        if platform.get_untracked() == "youtube" {
                            let Some(channel) = channel_input::youtube_channel(&input.get_untracked()) else {
                                error.set(Some(tr("Bitte einen gültigen YouTube-Kanal eingeben: @name, Kanal-ID oder Link zum Kanal.", "Please enter a valid YouTube channel: @name, channel ID or link to the channel.").into()));
                                return;
                            };
                            run(async move { web::add_youtube_tab(&channel, area).await });
                            open.set(false);
                            return;
                        }
                        let Some(channel) = channel_input::twitch_channel(&input.get_untracked()) else {
                            error.set(Some(tr("Bitte einen gültigen Twitch-Kanalnamen oder Link eingeben.", "Please enter a valid Twitch channel name or link.").into()));
                            return;
                        };
                        if kind.get_untracked() == "stream" {
                            run(async move { web::add_stream_tab(&channel, area).await.map(|_| ()) });
                        } else {
                            run(async move { web::add_channel_tab(&channel, area).await });
                        }
                        open.set(false);
                    }>
                        {(kind.get() == "stream").then(|| view! {
                            <p class="text-sm text-muted-foreground">
                                {tr("Zeigt den offiziellen Twitch-Player des Kanals. Der Tab lässt sich wie jeder andere frei im Layout anordnen.", "Shows the channel’s official Twitch player. The tab can be arranged freely in the layout like any other.")}
                            </p>
                        })}
                        <div class="flex flex-col gap-2">
                            <label data-slot="label" class=LABEL>{tr("Plattform", "Platform")}</label>
                            <div class="flex gap-2">{platform_buttons}</div>
                        </div>
                        <div class="flex flex-col gap-2">
                            <label data-slot="label" for="channel-input" class=LABEL>{tr("Kanalname oder Link", "Channel name or link")}</label>
                            <input node_ref=field_ref id="channel-input" data-slot="input" class=INPUT_CLASS autocomplete="off" spellcheck="false"
                                aria-invalid=move || error.with(Option::is_some).then_some("true")
                                prop:value=move || input.get()
                                on:input=move |event| {
                                    let value = event_target_value(&event);
                                    // Eingefügte Links wählen die Plattform vor.
                                    if let Some(p) = channel_input::parse(&value).platform {
                                        platform.set(p.to_string());
                                    }
                                    input.set(value);
                                    error.set(None);
                                } />
                            {move || error.get().map(|e| view! { <p class="text-sm text-destructive">{e}</p> })}
                            {move || (!platform_available()).then(|| {
                                let label = platforms.with_value(|list| list.iter().find(|p| p.id == platform.get()).map(|p| p.label)).unwrap_or_default();
                                view! { <p class="text-sm text-muted-foreground">{multiyap_core::tf!("{label} ist noch nicht verfügbar.", "{label} is not available yet.")}</p> }
                            })}
                        </div>
                        {move || (platform.get() == "youtube" && platform_available()).then(youtube_consent)}
                        <div data-slot="dialog-footer" class=DIALOG_FOOTER>
                            <button type="submit" data-slot="button" class=button_class("default", "default", "")
                                disabled=move || !platform_available() || input.with(|i| i.trim().is_empty())>
                                {tr("Öffnen", "Open")}
                            </button>
                        </div>
                    </form>
                }
                .into_any()
            }
        };
        view! {
            <div class="grid grid-cols-2 gap-2">{kind_buttons}</div>
            {body}
        }
    };

    view! {
        <Dialog open=open title=tr("Neuer Tab", "New tab") description=Signal::derive(|| tr("Art des Tabs wählen.", "Choose the kind of tab.").to_string()) max_width="sm:max-w-md">
            {content}
        </Dialog>
    }
}

/// Fenster der Arbeitsfläche: „Neuer Tab“ und „Tab schließen?“.
#[component]
fn WorkspaceDialogs() -> impl IntoView {
    let ctx = workspace_ctx();
    let request = ctx.close_request;
    view! {
        <AddTabDialog />
        <ConfirmDialog
            open=Signal::derive(move || matches!(request.get(), Some(CloseRequest::Tab(..))))
            title=tr("Tab schließen?", "Close tab?")
            description=Signal::derive(move || match request.get() {
                Some(CloseRequest::Tab(_, name)) => multiyap_core::tf!("„{name}“ wird geschlossen. Über „Zuletzt geschlossenen Tab wiederherstellen“ lässt er sich zurückholen.", "“{name}” will be closed. It can be brought back with “Restore last closed tab”."),
                _ => String::new(),
            })
            action=tr("Schließen", "Close")
            on_confirm=Callback::new(move |_| {
                if let Some(CloseRequest::Tab(id, _)) = request.get_untracked() {
                    run(async move { web::close_tab(&id).await });
                }
                request.set(None);
            })
            on_cancel=Callback::new(move |_| request.set(None)) />
        <ConfirmDialog
            open=Signal::derive(move || matches!(request.get(), Some(CloseRequest::Page(..))))
            title=tr("Seite schließen?", "Close page?")
            description=Signal::derive(move || match request.get() {
                Some(CloseRequest::Page(_, name, count)) => multiyap_core::tf!("„{name}“ wird mit allen {count} Chats geschlossen. Über „Zuletzt geschlossenen Tab wiederherstellen“ lässt sie sich zurückholen.", "“{name}” will be closed with all {count} chats. It can be brought back with “Restore last closed tab”."),
                _ => String::new(),
            })
            action=tr("Schließen", "Close")
            on_confirm=Callback::new(move |_| {
                if let Some(CloseRequest::Page(id, _, _)) = request.get_untracked() {
                    run(async move { web::close_page(&id).await });
                }
                request.set(None);
            })
            on_cancel=Callback::new(move |_| request.set(None)) />
    }
}

/// Arbeitsfläche: Bereiche mit Tab-Leisten. Erst sichtbar, wenn der Stand geladen ist.
#[component]
pub fn Workspace() -> impl IntoView {
    let state = RwSignal::new(WorkspaceView::read());
    let loaded = RwSignal::new(web::loaded());
    let subscription = web::subscribe(Rc::new(move || {
        let next = WorkspaceView::read();
        if state.with_untracked(|s| *s != next) {
            state.try_set(next);
        }
        // Nur bei echtem Wechsel setzen: Jedes Setzen meldet eine Änderung und baute sonst bei jeder Tab-Änderung die
        // ganze Arbeitsfläche samt Fenstern neu auf (Absturz, wenn ein Fenster dabei noch einen Fokus geplant hatte).
        let now = web::loaded();
        if loaded.try_get_untracked().is_some_and(|before| before != now) {
            loaded.try_set(now);
        }
    }));
    on_cleanup(move || web::unsubscribe(subscription));
    let ctx = WorkspaceCtx { state, add_target: RwSignal::new(None), close_request: RwSignal::new(None), status: RwSignal::new(HashMap::new()), dragged: RwSignal::new(None) };
    provide_context(ctx);

    // Tastenkürzel „Neuer Tab“: Dialog für eine neue Seite.
    let command = multiyap_web::app_commands::on("newTab", Rc::new(move || {
        ctx.add_target.try_set(Some(String::new()));
    }));
    on_cleanup(move || multiyap_web::app_commands::off(command));

    let listeners = WindowListeners::new();
    listeners.add(
        "keydown",
        Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| {
            let key = event.key().to_lowercase();
            // Zuletzt geschlossenen Tab wiederherstellen (Strg+Umschalt+T, sofern der Browser die Taste durchlässt).
            if event.ctrl_key() && event.shift_key() && key == "t" {
                event.prevent_default();
                run(web::restore_last_closed_tab());
                return;
            }
            // Suche im aktuellen Tab: aktiver Tab im fokussierten Bereich. Ohne Tab bleibt die Browser-Suche. In
            // offenen Fenstern (Einstellungen) ebenfalls die Browser-Suche.
            if event.ctrl_key() && !event.shift_key() && !event.alt_key() && key == "f" {
                let dialog_open = window().document().and_then(|d| d.query_selector("[role=\"dialog\"], [role=\"alertdialog\"]").ok().flatten()).is_some();
                if dialog_open {
                    return;
                }
                let Some(tab) = state.with_untracked(WorkspaceView::focused_tab) else { return };
                event.prevent_default();
                multiyap_web::chat_tools::open_search(&tab);
            }
        })
        .into_js_value(),
    );
    // Abgebrochenes Ziehen (zum Beispiel Esc oder außerhalb losgelassen) beenden.
    listeners.add("dragend", Closure::<dyn FnMut()>::new(move || ctx.end_drag()).into_js_value());
    on_cleanup({
        let listeners = listeners.clone();
        move || listeners.clear()
    });

    view! {
        {move || loaded.get().then(|| view! {
            <div class="flex min-h-0 flex-1 flex-col">
                <WorkspaceLayout />
                <WorkspaceDialogs />
            </div>
        })}
    }
}
