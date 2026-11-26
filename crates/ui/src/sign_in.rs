//! Anmeldeseite und Anmeldung per Code in Leptos (Nachbau von sign-in-screen.tsx und device-login-dialog.tsx).
//! Token werden nie gelesen.

use multiyap_core::i18n::tr;
use leptos::prelude::*;
use serde::Deserialize;

use crate::components::button::button_class;
use crate::components::dialog::{Dialog, DIALOG_FOOTER};
use crate::components::icons;
use crate::components::overlay::{window, Alert};
use crate::settings::account::ManualTokenForm;

const CARD: &str = "group/card flex flex-col gap-(--card-spacing) overflow-hidden rounded-xl bg-card py-(--card-spacing) text-sm text-card-foreground ring-1 ring-foreground/10 [--card-spacing:--spacing(4)] has-data-[slot=card-footer]:pb-0 has-[>img:first-child]:pt-0 data-[size=sm]:[--card-spacing:--spacing(3)] data-[size=sm]:has-data-[slot=card-footer]:pb-0 *:[img:first-child]:rounded-t-xl *:[img:last-child]:rounded-b-xl w-full max-w-sm";
const CARD_HEADER: &str = "group/card-header @container/card-header grid auto-rows-min items-start gap-1 rounded-t-xl px-(--card-spacing) has-data-[slot=card-action]:grid-cols-[1fr_auto] has-data-[slot=card-description]:grid-rows-[auto_auto] [.border-b]:pb-(--card-spacing)";

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
struct AuthInfo {
    status: String,
    #[serde(default)]
    error: Option<String>,
}

/// Anmeldeseite: Knopf „Mit Twitch anmelden“, andere Wege (Weiterleitung, Token) zum Aufklappen.
#[component]
pub fn SignInScreen() -> impl IntoView {
    let auth = crate::bridge::use_auth(AuthInfo::default());
    let more = RwSignal::new(false);
    let redirect = multiyap_web::auth::device_login_available();
    view! {
        <div class="flex flex-1 items-center justify-center p-4">
            <div data-slot="card" class=CARD>
                <div data-slot="card-header" class=CARD_HEADER>
                    <div data-slot="card-title" class="text-base leading-snug font-medium group-data-[size=sm]/card:text-sm">{tr("Anmelden", "Sign in")}</div>
                    <div data-slot="card-description" class="text-sm text-muted-foreground">
                        {tr("MultiYap braucht Zugriff auf dein Twitch-Konto, um Chats zu lesen, zu schreiben und zu moderieren.", "MultiYap needs access to your Twitch account to read, write and moderate chats.")}
                    </div>
                </div>
                <div data-slot="card-content" class="px-(--card-spacing) flex flex-col gap-4">
                    {move || auth.with(|a| a.error.clone()).map(|error| view! {
                        <Alert destructive=true title=tr("Anmeldung fehlgeschlagen", "Sign-in failed") icon=icons::triangle_alert("").into_any()>{error}</Alert>
                    })}
                    <button type="button" data-slot="button" class=button_class("default", "lg", "") on:click=|_| multiyap_web::auth::sign_in()>
                        {tr("Mit Twitch anmelden", "Sign in with Twitch")}
                    </button>
                    <div data-slot="collapsible" data-state=move || if more.get() { "open" } else { "closed" } class="flex flex-col gap-2">
                        <button type="button" data-slot="collapsible-trigger" data-state=move || if more.get() { "open" } else { "closed" }
                            aria-expanded=move || more.get().to_string()
                            class=button_class("ghost", "sm", "group/trigger self-start") on:click=move |_| more.update(|m| *m = !*m)>
                            {tr("Andere Anmeldewege", "Other ways to sign in")}
                            {icons::chevron_down("transition-transform group-data-[state=open]/trigger:rotate-180")}
                        </button>
                        {move || more.get().then(|| view! {
                            <div data-slot="collapsible-content" class="flex flex-col gap-3">
                                {redirect.then(|| view! {
                                    <button type="button" data-slot="button" class=button_class("outline", "sm", "self-start") on:click=|_| multiyap_web::auth::sign_in_with_redirect()>
                                        {tr("Mit Weiterleitung anmelden", "Sign in with redirect")}
                                    </button>
                                })}
                                <ManualTokenForm id="manual-token" />
                            </div>
                        })}
                    </div>
                </div>
            </div>
        </div>
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceLogin {
    status: String,
    #[serde(default)]
    user_code: Option<String>,
    #[serde(default)]
    verification_uri: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

/// Anmeldung per Code: zeigt den Code, öffnet twitch.tv/activate im normalen Browser und wartet, bis die Person dort
/// bestätigt. Danach schließt es sich von selbst.
#[component]
pub fn DeviceLoginDialog() -> impl IntoView {
    let login = crate::bridge::use_device_login(DeviceLogin { status: "idle".into(), ..Default::default() });
    let open = RwSignal::new(false);
    let copied = RwSignal::new(false);
    Effect::new(move |_| open.set(login.with(|l| l.status != "idle")));
    // Geschlossen über X, Esc oder Klick daneben: abbrechen.
    Effect::new(move |_| {
        if !open.get() && login.with_untracked(|l| l.status != "idle") {
            multiyap_web::auth::cancel_device_login();
        }
    });
    let content = move || {
        let state = login.get();
        let copy_code = state.user_code.clone().unwrap_or_default();
        let uri = state.verification_uri.clone().unwrap_or_default();
        let error = state.status == "error";
        view! {
            {(state.status == "requesting").then(|| view! {
                <div class="flex items-center justify-center gap-2 py-6 text-sm text-muted-foreground">
                    {icons::loader_circle("size-4 animate-spin")}
                    {tr("Code wird angefordert…", "Requesting code…")}
                </div>
            })}
            {(state.status == "waiting").then(|| view! {
                <div class="flex flex-col items-center gap-4 py-2">
                    <div class="flex items-center gap-2">
                        <span class="rounded-lg border bg-muted px-4 py-2 font-mono text-2xl font-semibold tracking-widest">{state.user_code.clone()}</span>
                        <button type="button" data-slot="button" class=button_class("ghost", "icon-sm", "") aria-label=tr("Code kopieren", "Copy code")
                            on:click=move |_| {
                                let code = copy_code.clone();
                                let clipboard = window().navigator().clipboard();
                                let promise = clipboard.write_text(&code);
                                leptos::task::spawn_local(async move {
                                    let ok = wasm_bindgen_futures::JsFuture::from(promise).await.is_ok();
                                    copied.try_set(ok);
                                    if ok {
                                        set_timeout(move || { copied.try_set(false); }, std::time::Duration::from_millis(1500));
                                    }
                                });
                            }>
                            {move || if copied.get() { icons::check("").into_any() } else { icons::copy("").into_any() }}
                        </button>
                    </div>
                    <button type="button" data-slot="button" class=button_class("default", "lg", "w-full")
                        on:click=move |_| { let _ = window().open_with_url_and_target_and_features(&uri, "_blank", "noopener,noreferrer"); }>
                        {icons::external_link("")}
                        {tr("Bei Twitch bestätigen", "Confirm on Twitch")}
                    </button>
                    <p class="flex items-center gap-2 text-sm text-muted-foreground">
                        {icons::loader_circle("size-4 animate-spin")}
                        {tr("Warte auf Bestätigung…", "Waiting for confirmation…")}
                    </p>
                </div>
            })}
            {error.then(|| view! { <p class="py-2 text-sm text-destructive">{state.message.clone()}</p> })}
            <div data-slot="dialog-footer" class=DIALOG_FOOTER>
                <button type="button" data-slot="button" class=button_class("outline", "default", "") on:click=|_| { multiyap_web::auth::cancel_device_login(); }>
                    {if error { {tr("Schließen", "Close")} } else { {tr("Abbrechen", "Cancel")} }}
                </button>
                {error.then(|| view! {
                    <button type="button" data-slot="button" class=button_class("default", "default", "") on:click=|_| leptos::task::spawn_local(multiyap_web::auth::start_device_login())>
                        {tr("Erneut versuchen", "Try again")}
                    </button>
                })}
            </div>
        }
    };
    view! {
        <Dialog open=open title=tr("Mit Twitch anmelden", "Sign in with Twitch") description=Signal::derive(|| tr("Bei Twitch bestätigen, MultiYap meldet dich danach von selbst an.", "Confirm on Twitch, MultiYap then signs you in by itself.").to_string()) max_width="sm:max-w-md">
            {content}
        </Dialog>
    }
}
