//! Konto & Anmeldung (account-section.tsx) samt Token manuell einfügen (manual-token-form.tsx). Hier nur Stand und Knöpfe.
//! Das Token wird nie angezeigt und nie aus der Brücke gelesen.

use multiyap_core::i18n::tr;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Deserialize;
use wasm_bindgen::JsValue;

use crate::components::input::INPUT_CLASS;
use crate::components::{icons, Button, ComingSoonBadge, SettingGroup, SettingRow, Switch};

#[derive(Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthInfo {
    status: String,
    login: Option<String>,
    source: Option<String>,
    expires_at: Option<f64>,
}

fn date_de(ms: f64) -> String {
    let date = js_sys::Date::new(&JsValue::from_f64(ms));
    date.to_locale_date_string(tr("de-DE", "en-US"), &JsValue::UNDEFINED).as_string().unwrap_or_default()
}

fn describe(auth: &AuthInfo) -> Option<String> {
    auth.login.as_ref()?;
    Some(if auth.status == "expired" {
        tr("Die Anmeldung ist abgelaufen. Bitte neu anmelden.", "The sign-in has expired. Please sign in again.").into()
    } else if auth.source.as_deref() == Some("device") {
        tr("Angemeldet per Code, verlängert sich automatisch.", "Signed in with a code, renews automatically.").into()
    } else {
        let how = if auth.source.as_deref() == Some("manual") { tr("per eingefügtem Token", "with a pasted token") } else { tr("über Twitch", "via Twitch") };
        let until = auth.expires_at.map(|ms| multiyap_core::tf!(", gültig bis {}", ", valid until {}", date_de(ms))).unwrap_or_default();
        multiyap_core::tf!("Angemeldet {how}{until}.", "Signed in {how}{until}.")
    })
}

/// Ausweichweg: Zugriffstoken manuell einfügen.
#[component]
pub(crate) fn ManualTokenForm(#[prop(into)] id: String) -> impl IntoView {
    let token = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let submit = move || {
        busy.set(true);
        error.set(None);
        let value = token.get_untracked();
        spawn_local(async move {
            match multiyap_web::auth::sign_in_with_token(&value).await {
                Some(message) => error.set(Some(message)),
                None => token.set(String::new()),
            }
            busy.set(false);
        });
    };
    view! {
        <form class="flex flex-col gap-2" on:submit=move |event| { event.prevent_default(); submit(); }>
            <label for=id.clone() data-slot="label" class="flex items-center gap-2 text-sm leading-none font-medium select-none group-data-[disabled=true]:pointer-events-none group-data-[disabled=true]:opacity-50 peer-disabled:cursor-not-allowed peer-disabled:opacity-50">
                {tr("Zugriffstoken", "Access token")}
            </label>
            <input
                data-slot="input"
                id=id
                type="password"
                autocomplete="off"
                spellcheck="false"
                class=INPUT_CLASS
                placeholder="oauth:…"
                aria-invalid=move || error.get().map(|_| "true")
                prop:value=move || token.get()
                on:input=move |event| token.set(event_target_value(&event))
            />
            {move || error.get().map(|text| view! { <p class="text-sm text-destructive">{text}</p> })}
            <p class="text-sm text-muted-foreground">{tr("Das Token bleibt nur in MultiYap auf diesem PC gespeichert.", "The token is only stored in MultiYap on this PC.")}</p>
            <Button submit=true variant="secondary" disabled=Signal::derive(move || busy.get() || token.get().trim().is_empty())>
                {move || busy.get().then(|| icons::loader_circle("animate-spin"))}
                {tr("Token übernehmen", "Use token")}
            </Button>
        </form>
    }
}

#[component]
pub fn AccountSection() -> impl IntoView {
    let auth = crate::bridge::use_auth(AuthInfo::default());
    let device_login = multiyap_web::auth::device_login_available();
    let signed_in = Signal::derive(move || auth.get().login.is_some());

    view! {
        <div class="flex flex-col gap-8">
            <SettingGroup title=tr("Angemeldetes Konto", "Signed-in account")>
                {move || {
                    let info = auth.get();
                    view! {
                        <SettingRow label=info.login.clone().unwrap_or_else(|| tr("Nicht angemeldet", "Not signed in").into()) description=describe(&info).unwrap_or_default()>
                            <div class="flex gap-2">
                                <Button size="sm" variant="secondary" on_click=Callback::new(|_| multiyap_web::auth::sign_in())>
                                    {move || if signed_in.get() { {tr("Neu anmelden", "Sign in again")} } else { {tr("Mit Twitch anmelden", "Sign in with Twitch")} }}
                                </Button>
                                {move || signed_in.get().then(|| view! {
                                    <Button size="sm" variant="outline" on_click=Callback::new(|_| crate::bridge::sign_out())>
                                        {icons::log_out("")}
                                        {tr("Abmelden", "Sign out")}
                                    </Button>
                                })}
                            </div>
                        </SettingRow>
                    }
                }}
                <SettingRow label=tr("Mehrere Accounts", "Multiple accounts") description=tr("Zum Beispiel Haupt- und Mod-Account.", "For example a main and a mod account.")>
                    <div class="flex items-center gap-2">
                        <ComingSoonBadge />
                        <Switch id="multiple-accounts" checked=false on_change=Callback::new(|_: bool| {}) disabled=true />
                    </div>
                </SettingRow>
            </SettingGroup>

            <SettingGroup title=tr("Andere Anmeldewege", "Other ways to sign in") description=tr("Falls die Anmeldung über Twitch nicht klappt.", "In case signing in via Twitch does not work.")>
                {device_login.then(|| view! {
                    <SettingRow label=tr("Mit Weiterleitung anmelden", "Sign in with redirect") description=tr("Der bisherige Weg, ohne Code.", "The previous way, without a code.")>
                        <Button size="sm" variant="outline" on_click=Callback::new(|_| multiyap_web::auth::sign_in_with_redirect())>{tr("Anmelden", "Sign in")}</Button>
                    </SettingRow>
                })}
                <div class="flex flex-col gap-2 py-3">
                    <span class="text-sm font-medium">{tr("Token manuell einfügen", "Paste token manually")}</span>
                    <ManualTokenForm id="settings-manual-token" />
                </div>
            </SettingGroup>
        </div>
    }
}
