//! Updates aus dem lokalen Versionsordner (Prüfen und Installieren in crates/web/src/updates.rs).
//! Änderungen gelten sofort.

use multiyap_core::i18n::tr;
use leptos::prelude::*;
use multiyap_web::updates;

use crate::bridge::{save_setting, use_setting};
use crate::components::{icons, Button, CommitInput, SettingGroup, SettingRow};
use crate::store::watch_version;

const FOLDER: &str = "updates.folder";

#[component]
pub fn UpdatesSection() -> impl IntoView {
    let folder = use_setting(FOLDER, String::new());
    let no_source = Signal::derive(move || folder.get().trim().is_empty());
    let version = watch_version(updates::subscribe, updates::unsubscribe);
    let available = Signal::derive(move || {
        version.track();
        updates::available()
    });
    let status = move || {
        version.track();
        if no_source.get() {
            tr("Kein Ordner angegeben.", "No folder set.").to_string()
        } else if updates::checking() {
            tr("Wird geprüft …", "Checking …").to_string()
        } else if let Some(error) = updates::error() {
            error
        } else if let Some(next) = updates::available() {
            multiyap_core::tf!("Version {next} ist bereit.", "Version {next} is ready.")
        } else {
            tr("MultiYap ist aktuell.", "MultiYap is up to date.").to_string()
        }
    };

    view! {
        <div class="flex flex-col gap-8">
            <SettingGroup
                title="Updates"
                description=tr("MultiYap sucht im Ordner mit den freigegebenen Versionen nach einer neueren Version. Beim Installieren schließt sich MultiYap und startet danach neu.", "MultiYap looks for a newer version in the folder with the released versions. While installing, MultiYap closes and then restarts.")
            >
                <SettingRow label=tr("Installierte Version", "Installed version")>
                    <span class="text-sm text-muted-foreground">{env!("CARGO_PKG_VERSION")}</span>
                </SettingRow>
                <SettingRow label=tr("Ordner mit den Versionen", "Folder with the versions") description=tr("Vollständiger Pfad, zum Beispiel D:\\Ordner\\Versionen. Leer: keine Prüfung.", "Full path, for example D:\\Folder\\Versions. Empty: no check.") html_for="updates-folder">
                    <CommitInput
                        id="updates-folder"
                        class="w-80"
                        value=Signal::derive(move || folder.get())
                        max_length=500
                        placeholder=tr("D:\\Ordner\\Versionen", "D:\\Folder\\Versions")
                        aria_label=tr("Ordner mit den Versionen", "Folder with the versions")
                        on_commit=Callback::new(|value: String| {
                            save_setting(FOLDER, &value.trim().to_string());
                            true
                        })
                    />
                </SettingRow>
                <SettingRow label="Status">
                    <div class="flex items-center gap-2">
                        <span class="text-sm text-muted-foreground">{status}</span>
                        <Button variant="secondary" size="sm"
                            disabled=no_source
                            on_click=Callback::new(|_| updates::check())>
                            {icons::refresh_cw("")}
                            {tr("Jetzt prüfen", "Check now")}
                        </Button>
                        {move || available.get().map(|next| view! {
                            <Button size="sm" on_click=Callback::new(|_| updates::install())>
                                {icons::download("")}
                                {multiyap_core::tf!("Version {next} installieren", "Install version {next}")}
                            </Button>
                        })}
                    </div>
                </SettingRow>
            </SettingGroup>
        </div>
    }
}
