//! App in Leptos: Kopfleiste (Name links, Konto, Schloss und Zahnrad rechts), darunter Hinweise zum Speicher, die
//! Anmeldung oder die Arbeitsfläche; dazu Nutzerkarten, Anmeldung per Code und Meldungen über allem. Auch die Hülle
//! selbst (vorher App.tsx); Speicherstand aus crates/web/src/startup.rs.

use multiyap_core::i18n::tr;
use leptos::prelude::*;
use serde::Deserialize;
use wasm_bindgen::JsCast;

use crate::bridge::{convert, save_setting, use_setting};
use crate::chat::layer::{Layer, LayerView};
use crate::components::button::button_class;
use crate::components::icons;
use crate::components::overlay::{Alert, DropdownMenu, MenuEntry};
use crate::settings::dialog::SettingsDialog;
use crate::sign_in::{DeviceLoginDialog, SignInScreen};
use crate::toaster::Toaster;
use crate::usercard::UserCardLayer;
use crate::workspace::Workspace;

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthInfo {
    status: String,
    login: Option<String>,
    #[serde(default)]
    missing_scopes: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
struct PersistenceStatus {
    state: String,
    #[serde(default)]
    message: Option<String>,
}

/// Stand des Speichers als Signal (crates/web/src/startup.rs).
fn use_persistence() -> ReadSignal<PersistenceStatus> {
    let read = || convert::<PersistenceStatus>(&multiyap_web::startup::persistence_status()).unwrap_or_default();
    let (value, set_value) = signal(read());
    let id = multiyap_web::startup::subscribe_persistence(std::rc::Rc::new(move || {
        set_value.try_set(read());
    }));
    on_cleanup(move || multiyap_web::startup::unsubscribe_persistence(id));
    value
}

fn tooltip_events(layer: Layer, text: impl Fn() -> String + Clone + 'static) -> (impl Fn(web_sys::PointerEvent) + Clone, impl Fn(web_sys::PointerEvent) + Clone) {
    let enter = move |event: web_sys::PointerEvent| {
        if let Some(element) = event.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
            layer.show_tip(&element, text(), false);
        }
    };
    (enter, move |_| layer.hide_tip())
}

/// Angemeldetes Konto in der Kopfleiste mit Abmelden.
#[component]
fn AccountMenu(auth: ReadSignal<AuthInfo>) -> impl IntoView {
    move || {
        let info = auth.get();
        if info.status != "signed-in" && info.status != "expired" {
            return None;
        }
        let login = info.login.unwrap_or_default();
        let label = login.clone();
        let entries = Signal::derive(move || {
            vec![
                MenuEntry::Label(multiyap_core::tf!("Angemeldet als {label}", "Signed in as {label}")),
                MenuEntry::Separator,
                MenuEntry::IconItem {
                    icon: || icons::log_out("").into_any(),
                    label: tr("Abmelden", "Sign out").into(),
                    on_select: Callback::new(|_| {
                        crate::bridge::sign_out();
                    }),
                },
            ]
        });
        Some(view! {
            <DropdownMenu align_end=true trigger_variant="ghost" width_class="" entries=entries>
                {login}
                {icons::chevron_down("")}
            </DropdownMenu>
        })
    }
}

/// Kopfleiste: Name links, Konto, Schloss und Zahnrad rechts. Schloss und Zahnrad erst, wenn der Speicher bereit ist.
#[component]
pub fn AppHeader() -> impl IntoView {
    let auth = crate::bridge::use_auth(AuthInfo::default());
    let persistence = use_persistence();
    let locked = use_setting("layout.locked", false);
    let layer = Layer::new();
    let ready = move || persistence.with(|p| p.state == "ready");
    // Schloss: gesperrt geschlossenes, ganz weißes Schloss; offen graues offenes Schloss.
    let lock_tip = move || {
        if locked.get_untracked() { tr("Layout gesperrt: Tabs schließen und hinzufügen geht weiterhin", "Layout locked: closing and adding tabs still works").to_string() } else { tr("Layout sperren", "Lock layout").to_string() }
    };
    let (lock_enter, lock_leave) = tooltip_events(layer, lock_tip);
    let (gear_enter, gear_leave) = tooltip_events(layer, || tr("Einstellungen", "Settings").to_string());
    view! {
        <header class="flex h-12 shrink-0 items-center justify-between gap-4 border-b px-4">
            <span class="font-heading text-sm font-medium">"MultiYap"</span>
            <div class="flex items-center gap-2">
                <AccountMenu auth=auth />
                {move || ready().then(|| {
                    let (lock_enter, lock_leave, gear_enter, gear_leave) = (lock_enter.clone(), lock_leave.clone(), gear_enter.clone(), gear_leave.clone());
                    view! {
                        <UpdateButton />
                        <button type="button" data-slot="button"
                            class=move || button_class("ghost", "icon-sm", if locked.get() { "text-foreground" } else { "text-muted-foreground" })
                            aria-label=move || if locked.get() { {tr("Layout entsperren", "Unlock layout")} } else { {tr("Layout sperren", "Lock layout")} }
                            aria-pressed=move || locked.get().to_string()
                            on:pointerenter=lock_enter on:pointerleave=lock_leave
                            on:click=move |_| {
                                layer.hide_tip();
                                save_setting("layout.locked", &!locked.get_untracked());
                            }>
                            {move || if locked.get() { icons::lock("[&_rect]:fill-current").into_any() } else { icons::lock_open("").into_any() }}
                        </button>
                        <button type="button" data-slot="button" class=button_class("ghost", "icon-sm", "") aria-label=tr("Einstellungen", "Settings")
                            on:pointerenter=gear_enter on:pointerleave=gear_leave
                            on:click=move |_| {
                                layer.hide_tip();
                                multiyap_web::app_commands::run("openSettings");
                            }>
                            {icons::settings("")}
                        </button>
                        // Einstellungsfenster; öffnet über den Befehl „openSettings“ (Zahnrad, Tastenkürzel).
                        <SettingsDialog />
                    }
                })}
            </div>
            <LayerView layer=layer />
        </header>
    }
}

/// Knopf in der Kopfleiste, wenn auf multiyap.app oder im Versionsordner ein Update bereitliegt. Prüfen in
/// crates/web/src/updates.rs.
#[component]
fn UpdateButton() -> impl IntoView {
    multiyap_web::updates::start();
    let version = crate::store::watch_version(multiyap_web::updates::subscribe, multiyap_web::updates::unsubscribe);
    move || {
        version.track();
        multiyap_web::updates::available().map(|next| {
            view! {
                <button type="button" data-slot="button" class=button_class("secondary", "sm", "")
                    title=tr("MultiYap schließt sich, installiert das Update und startet neu", "MultiYap closes, installs the update and restarts")
                    on:click=move |_| multiyap_web::updates::install()>
                    {icons::download("")}
                    {format!("Update {next}")}
                </button>
            }
        })
    }
}

/// Probleme mit dem Speicher; ist alles in Ordnung, wird nichts angezeigt.
#[component]
pub fn PersistenceNotice() -> impl IntoView {
    let status = use_persistence();
    move || {
        let current = status.get();
        let warn = || icons::triangle_alert("").into_any();
        Some(match current.state.as_str() {
            "ready" => return None,
            "blocked" => view! {
                <Alert title=tr("MultiYap ist noch in einem anderen Tab geöffnet", "MultiYap is still open in another tab") icon=warn()>
                    {tr("Bitte die anderen MultiYap-Tabs schließen oder neu laden. Danach geht es hier automatisch weiter.", "Please close or reload the other MultiYap tabs. It then continues here automatically.")}
                </Alert>
            }
            .into_any(),
            "outdated" => view! {
                <Alert destructive=true title=tr("Neue Version in einem anderen Tab", "New version in another tab") icon=warn()>
                    {tr("Ein anderer Tab hat eine neuere Version von MultiYap geöffnet. Bitte diesen Tab neu laden.", "Another tab opened a newer version of MultiYap. Please reload this tab.")}
                </Alert>
            }
            .into_any(),
            "newer" => view! {
                <Alert destructive=true title=tr("Gespeicherte Daten sind neuer als diese Version", "Saved data is newer than this version") icon=warn()>
                    {tr("Die Daten wurden mit einer neueren Version von MultiYap gespeichert. Sie bleiben unverändert. Bitte die neueste Version starten.", "The data was saved with a newer version of MultiYap. It stays unchanged. Please start the newest version.")}
                </Alert>
            }
            .into_any(),
            "error" => view! {
                <Alert destructive=true title=tr("Speicher konnte nicht geöffnet werden", "Storage could not be opened") icon=warn()>
                    {current.message.clone().unwrap_or_default()}
                </Alert>
            }
            .into_any(),
            _ => view! { <Alert title=tr("Daten werden geladen", "Loading data") icon=icons::loader_circle("animate-spin").into_any() /> }.into_any(),
        })
    }
}

/// Hinweise zur Anmeldung oberhalb des Inhalts: abgelaufenes Token, fehlende Berechtigungen.
#[component]
pub fn AuthNotice() -> impl IntoView {
    let auth = crate::bridge::use_auth(AuthInfo::default());
    let sign_in = |_| {
        multiyap_web::auth::sign_in();
    };
    move || {
        let info = auth.get();
        if info.status == "expired" {
            return Some(
                view! {
                    <Alert destructive=true title=tr("Bitte neu anmelden", "Please sign in again") icon=icons::triangle_alert("").into_any()
                        action=view! { <button type="button" data-slot="button" class=button_class("default", "sm", "") on:click=sign_in>{tr("Neu anmelden", "Sign in again")}</button> }.into_any()>
                        {tr("Die Anmeldung bei Twitch ist abgelaufen oder wurde widerrufen.", "The Twitch sign-in has expired or was revoked.")}
                    </Alert>
                }
                .into_any(),
            );
        }
        if info.status == "signed-in" && info.missing_scopes > 0 {
            let count = info.missing_scopes;
            let text = if count == 1 { tr("Dem Token fehlt eine Berechtigung.", "The token is missing a permission.").to_string() } else { multiyap_core::tf!("Dem Token fehlen {count} Berechtigungen.", "The token is missing {count} permissions.") };
            return Some(
                view! {
                    <Alert title=tr("Es fehlen Berechtigungen", "Permissions are missing") icon=icons::info("").into_any()
                        action=view! { <button type="button" data-slot="button" class=button_class("secondary", "sm", "") on:click=sign_in>{tr("Mit Twitch anmelden", "Sign in with Twitch")}</button> }.into_any()>
                        {multiyap_core::tf!("{text} Manche Funktionen sind deshalb nicht verfügbar. Eine Anmeldung über „Mit Twitch anmelden“ behebt das.", "{text} Some features are therefore not available. Signing in with “Sign in with Twitch” fixes this.")}
                    </Alert>
                }
                .into_any(),
            );
        }
        None
    }
}

/// Was die App unter der Kopfleiste zeigt.
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    /// Speicher nicht bereit: Hinweis.
    Storage,
    /// Anmeldung wird geprüft.
    Loading,
    SignedOut,
    /// Angemeldet (auch mit abgelaufenem Token): Arbeitsfläche.
    Main,
}

/// Die ganze App (Vorher App.tsx). Die Arbeitsfläche bleibt eingebunden, solange angemeldet ist, damit Chats bei
/// Hinweisen zur Anmeldung nicht neu aufgebaut werden.
#[component]
pub fn App() -> impl IntoView {
    let persistence = use_persistence();
    let auth = crate::bridge::use_auth(AuthInfo::default());
    let phase = Memo::new(move |_| {
        if persistence.with(|p| p.state != "ready") {
            return Phase::Storage;
        }
        match auth.with(|a| a.status.clone()).as_str() {
            "loading" => Phase::Loading,
            "signed-out" => Phase::SignedOut,
            _ => Phase::Main,
        }
    });
    let notice = Memo::new(move |_| auth.with(|a| a.status == "expired" || (a.status == "signed-in" && a.missing_scopes > 0)));
    view! {
        <div class="flex h-svh flex-col">
            <AppHeader />
            {move || match phase.get() {
                Phase::Storage => view! { <div class="p-4"><PersistenceNotice /></div> }.into_any(),
                Phase::Loading => view! {
                    <div class="flex flex-1 items-center justify-center text-muted-foreground">{icons::loader_circle("size-5 animate-spin")}</div>
                }
                .into_any(),
                Phase::SignedOut => view! { <SignInScreen /> }.into_any(),
                Phase::Main => view! {
                    <main class="flex min-h-0 flex-1 flex-col">
                        {move || notice.get().then(|| view! { <div class="shrink-0 p-2"><AuthNotice /></div> })}
                        <Workspace />
                    </main>
                }
                .into_any(),
            }}
        </div>
        // Nutzerkarten als eigene Fenster über allem (anpinnbar, verschiebbar).
        <UserCardLayer />
        // Anmeldung per Code: Fenster mit dem Code, von überall über „Mit Twitch anmelden“.
        <DeviceLoginDialog />
        <Toaster />
    }
}
