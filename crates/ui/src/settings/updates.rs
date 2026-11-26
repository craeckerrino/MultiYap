//! Updates von multiyap.app und aus dem lokalen Versionsordner (Prüfen und Installieren in crates/web/src/updates.rs).
//! Änderungen gelten sofort.

use multiyap_core::i18n::tr;
use leptos::prelude::*;
use multiyap_web::updates;

use crate::bridge::{save_setting, use_setting};
use crate::components::{icons, Button, CommitInput, SettingGroup, SettingRow, Switch};
use crate::store::watch_version;

const FOLDER: &str = "updates.folder";
const FROM_WEBSITE: &str = "updates.fromWebsite";

#[component]
pub fn UpdatesSection() -> impl IntoView {
    let folder = use_setting(FOLDER, String::new());
    let from_website = use_setting(FROM_WEBSITE, true);
    let no_source = Signal::derive(move || !from_website.get() && folder.get().trim().is_empty());
    let version = watch_version(updates::subscribe, updates::unsubscribe);
    let available = Signal::derive(move || {
        version.track();
        updates::available()
    });
    let status = move || {
        version.track();
        if no_source.get() {
            tr("Keine Quelle eingeschaltet.", "No source switched on.").to_string()
        } else if updates::checking() {
            tr("Wird geprüft …", "Checking …").to_string()
        } else if let Some(error) = updates::error() {
            error
        } else if let Some(next) = updates::available() {
            let from = if updates::source() == Some(updates::Source::Website) { tr("von multiyap.app", "from multiyap.app") } else { tr("aus dem Ordner", "from the folder") };
            multiyap_core::tf!("Version {next} ist bereit ({from}).", "Version {next} is ready ({from}).")
        } else {
            tr("MultiYap ist aktuell.", "MultiYap is up to date.").to_string()
        }
    };

    view! {
        <div class="flex flex-col gap-8">
            <SettingGroup
                title="Updates"
                description=tr("MultiYap sucht auf multiyap.app und, falls angegeben, im Ordner mit den freigegebenen Versionen nach einer neueren Version. Beim Installieren schließt sich MultiYap und startet danach neu.", "MultiYap looks for a newer version on multiyap.app and, if set, in the folder with the released versions. While installing, MultiYap closes and then restarts.")
            >
                <SettingRow label=tr("Installierte Version", "Installed version")>
                    <span class="text-sm text-muted-foreground">{env!("CARGO_PKG_VERSION")}</span>
                </SettingRow>
                <SettingRow label=tr("Updates von multiyap.app", "Updates from multiyap.app") description=tr("Prüft beim Start und alle 6 Stunden. Der Installer wird erst nach Prüfung von Größe und Prüfsumme gestartet.", "Checks at startup and every 6 hours. The installer only starts after its size and checksum have been verified.") html_for="updates-website">
                    <Switch id="updates-website" checked=from_website on_change=Callback::new(|checked: bool| save_setting(FROM_WEBSITE, &checked)) />
                </SettingRow>
                <SettingRow label=tr("Ordner mit den Versionen", "Folder with the versions") description=tr("Optional, für Versionen auf diesem Rechner. Vollständiger Pfad, zum Beispiel D:\\Ordner\\Versionen.", "Optional, for versions on this computer. Full path, for example D:\\Folder\\Versions.") html_for="updates-folder">
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
