//! Sicherung (backup-section.tsx). Export, Prüfen und Einspielen in einer Transaktion laufen in crates/web/src/backup.rs;
//! die geprüfte Sicherung wartet dort, hier nur die Zusammenfassung.

use multiyap_core::i18n::tr;
use multiyap_web::backup::{self as backup, AutoBackupInfo, Summary};
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

use crate::components::overlay::Alert;
use crate::components::{icons, Button, SettingGroup, SettingRow};

fn format_date_time(value: &JsValue) -> String {
    let date = js_sys::Date::new(value);
    let options = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&options, &"dateStyle".into(), &"medium".into());
    let _ = js_sys::Reflect::set(&options, &"timeStyle".into(), &"short".into());
    date.to_locale_string(tr("de-DE", "en-US"), &options).as_string().unwrap_or_default()
}

#[component]
pub fn BackupSection() -> impl IntoView {
    let busy = RwSignal::new(false);
    let pending = RwSignal::new_local(None::<Summary>);
    let errors = RwSignal::new(Vec::<String>::new());
    let autos = RwSignal::new_local(Vec::<AutoBackupInfo>::new());
    let file_ref = NodeRef::<leptos::html::Input>::new();

    spawn_local(async move {
        let list = backup::auto_list().await;
        autos.try_set(list);
    });

    let export = move || {
        busy.set(true);
        spawn_local(async move {
            backup::export().await;
            busy.set(false);
        });
    };

    let check_file = move |file: web_sys::File| {
        busy.set(true);
        pending.set(None);
        errors.set(Vec::new());
        spawn_local(async move {
            let text = JsFuture::from(file.text()).await.ok().and_then(|t| t.as_string());
            let Some(text) = text else {
                errors.set(vec![tr("Die Datei konnte nicht gelesen werden.", "The file could not be read.").into()]);
                busy.set(false);
                return;
            };
            match backup::check(&text) {
                Ok(summary) => pending.set(Some(summary)),
                Err(found) => errors.set(found),
            }
            busy.set(false);
        });
    };

    let apply = move || {
        busy.set(true);
        spawn_local(async move {
            if !backup::apply_pending().await {
                busy.set(false);
            }
        });
    };

    let restore = move |key: f64| {
        busy.set(true);
        spawn_local(async move {
            match backup::restore(key).await {
                Ok(true) => {}
                Ok(false) => busy.set(false),
                Err(found) => {
                    errors.set(found);
                    busy.set(false);
                }
            }
        });
    };

    view! {
        <div class="flex flex-col gap-8">
            <SettingGroup title=tr("Sicherung", "Backup") description=tr("Einstellungen, Tabs, Vorlagen, Hotkeys und Sounds. Ohne Zugangsdaten.", "Settings, tabs, templates, hotkeys and sounds. Without credentials.")>
                <SettingRow label=tr("Exportieren", "Export") description=tr("Speichert den aktuellen Stand als Datei.", "Saves the current state as a file.")>
                    <Button variant="secondary" size="sm" disabled=busy on_click=Callback::new(move |_| export())>
                        {icons::download("")}
                        {tr("Sicherung exportieren", "Export backup")}
                    </Button>
                </SettingRow>
                <SettingRow label=tr("Importieren", "Import") description=tr("Wird vorher geprüft, der aktuelle Stand wird automatisch gesichert.", "Checked first, the current state is backed up automatically.")>
                    <Button variant="secondary" size="sm" disabled=busy on_click=Callback::new(move |_| {
                        if let Some(input) = file_ref.get_untracked() {
                            input.click();
                        }
                    })>
                        {move || if busy.get() { icons::loader_circle("animate-spin").into_any() } else { icons::upload("").into_any() }}
                        {tr("Datei wählen", "Choose file")}
                    </Button>
                    <input
                        node_ref=file_ref
                        type="file"
                        accept="application/json,.json"
                        class="hidden"
                        on:change=move |event| {
                            let Some(input) = event.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()) else { return };
                            let file = input.files().and_then(|files| files.get(0));
                            input.set_value("");
                            if let Some(file) = file {
                                check_file(file);
                            }
                        }
                    />
                </SettingRow>
            </SettingGroup>

            {move || (!errors.get().is_empty()).then(|| view! {
                <Alert destructive=true title=tr("Sicherung nicht übernommen", "Backup not applied") icon=icons::triangle_alert("").into_any()>
                    <ul class="list-disc pl-4">
                        {errors.get().into_iter().take(10).map(|error| view! { <li>{error}</li> }).collect_view()}
                    </ul>
                    {tr("Es wurde nichts verändert.", "Nothing was changed.")}
                </Alert>
            })}

            {move || pending.get().map(|p| view! {
                <Alert title=tr("Sicherung geprüft", "Backup checked")>
                    <p>
                        {multiyap_core::tf!("Erstellt am {} mit Version {}, {} Einträge. Der aktuelle Stand wird ersetzt und vorher automatisch gesichert.", "Created on {} with version {}, {} entries. The current state will be replaced and backed up automatically first.",
                            format_date_time(&JsValue::from_str(&p.created_at)),
                            p.app_version,
                            p.count
                        )}
                    </p>
                    <div class="mt-3 flex gap-2">
                        <Button size="sm" disabled=busy on_click=Callback::new(move |_| apply())>{tr("Einspielen", "Restore")}</Button>
                        <Button size="sm" variant="outline" disabled=busy on_click=Callback::new(move |_| {
                            backup::cancel();
                            pending.set(None);
                        })>{tr("Abbrechen", "Cancel")}</Button>
                    </div>
                </Alert>
            })}

            <SettingGroup title=tr("Automatische Sicherungen", "Automatic backups") description=tr("Stand vor jedem Import, die letzten 5.", "State before each import, the last 5.")>
                {move || {
                    let list = autos.get();
                    if list.is_empty() {
                        view! { <p class="py-3 text-sm text-muted-foreground">{tr("Noch keine automatischen Sicherungen.", "No automatic backups yet.")}</p> }.into_any()
                    } else {
                        list.into_iter()
                            .map(|backup| {
                                let key = backup.key;
                                view! {
                                    <SettingRow label=format_date_time(&JsValue::from_str(&backup.created_at))>
                                        <Button variant="outline" size="sm" disabled=busy on_click=Callback::new(move |_| restore(key))>
                                            {icons::rotate_ccw("")}
                                            {tr("Wiederherstellen", "Restore")}
                                        </Button>
                                    </SettingRow>
                                }
                            })
                            .collect_view()
                            .into_any()
                    }
                }}
            </SettingGroup>
        </div>
    }
}
