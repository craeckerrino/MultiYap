//! Update von der Website installieren. Die Seite fragt nur nach der Version; alles Weitere prüft das Programm selbst
//! und traut den Angaben der Seite nicht:
//!
//! 1. `latest.json` von multiyap.app neu laden (nur HTTPS) und prüfen (crates/core/src/updates.rs): Version wie
//!    angefragt und neuer als die installierte, Dateiname passend zur Version, Größe und SHA-256 vorhanden.
//! 2. Installer laden, höchstens die angegebene Größe, und Größe und SHA-256 vergleichen.
//! 3. Erst dann in den Temp-Ordner schreiben und wie beim Update aus dem Versionsordner mit `/P /UPDATE /R`
//!    starten (nur Fortschrittsfenster, ohne Seite „bereits installiert“, startet MultiYap danach neu); MultiYap beendet
//!    sich.
//!
//! Bei einem Fehler bleibt MultiYap offen und meldet den Grund.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use multiyap_core::i18n::tr;
use multiyap_core::tf;
use multiyap_core::updates::{self, Release};
use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

/// Nur ein Download zur Zeit (doppelter Klick).
static RUNNING: AtomicBool = AtomicBool::new(false);

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().https_only(true).timeout_global(Some(Duration::from_secs(300))).build().into()
}

fn latest(agent: &ureq::Agent, version: &str) -> Result<Release, String> {
    let text = agent
        .get(updates::LATEST_URL)
        .call()
        .map_err(|e| tf!("Website nicht erreichbar ({e}).", "Website not reachable ({e})."))?
        .body_mut()
        .with_config()
        .limit(64 * 1024)
        .read_to_string()
        .map_err(|e| tf!("Angaben zur Version nicht lesbar ({e}).", "Version details not readable ({e})."))?;
    let json: serde_json::Value = serde_json::from_str(&text).map_err(|_| tr("Angaben zur Version nicht lesbar.", "Version details not readable.").to_string())?;
    let release = updates::parse_release(&json).ok_or(tr("Angaben zur Version sind ungültig.", "Version details are invalid."))?;
    if release.version != version {
        return Err(tf!("Auf der Website liegt inzwischen Version {}. Bitte erneut prüfen.", "The website now has version {}. Please check again.", release.version));
    }
    if !updates::is_newer(&release.version, multiyap_server::update::CURRENT_VERSION) {
        return Err(tr("Diese Version ist nicht neuer als die installierte.", "This version is not newer than the installed one.").into());
    }
    Ok(release)
}

fn download(agent: &ureq::Agent, release: &Release) -> Result<std::path::PathBuf, String> {
    let bytes = agent
        .get(&release.download_url())
        .call()
        .map_err(|e| tf!("Download nicht möglich ({e}).", "Download not possible ({e})."))?
        .body_mut()
        .with_config()
        .limit(release.size + 1)
        .read_to_vec()
        .map_err(|e| tf!("Download abgebrochen ({e}).", "Download aborted ({e})."))?;
    if !updates::matches(release, &bytes) {
        return Err(tr("Der geladene Installer stimmt nicht mit der Prüfsumme überein. Er wurde nicht gestartet.", "The downloaded installer does not match the checksum. It was not started.").into());
    }
    let folder = std::env::temp_dir().join("MultiYap-Update");
    std::fs::create_dir_all(&folder).map_err(|_| tr("Temp-Ordner nicht beschreibbar.", "Temp folder not writable.").to_string())?;
    let path = folder.join(&release.file_name);
    std::fs::write(&path, &bytes).map_err(|_| tr("Installer konnte nicht gespeichert werden.", "Installer could not be saved.").to_string())?;
    Ok(path)
}

fn run(app: &AppHandle, version: &str) -> Result<(), String> {
    let agent = agent();
    let release = latest(&agent, version)?;
    let installer = download(&agent, &release)?;
    std::process::Command::new(&installer).args(["/P", "/UPDATE", "/R"]).spawn().map_err(|_| tr("Installer konnte nicht gestartet werden.", "Installer could not be started.").to_string())?;
    app.exit(0);
    Ok(())
}

/// Lädt und installiert im Hintergrund, damit das Fenster weiter reagiert.
pub fn install(app: AppHandle, version: String) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        if let Err(message) = run(&app, &version) {
            app.dialog().message(tf!("Das Update konnte nicht installiert werden: {message}", "The update could not be installed: {message}")).title("MultiYap").kind(MessageDialogKind::Error).show(|_| {});
        }
        RUNNING.store(false, Ordering::SeqCst);
    });
}
