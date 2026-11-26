//! MultiYap als Windows-Programm (Tauri; ersetzt electron/main.mjs).
//!
//! Startet den lokalen Server (crates/server) im Programm selbst, mit dem Build fest eingebaut, und zeigt
//! http://localhost:8475 im Fenster. Verhalten wie bisher: Installer, das X beendet das Programm, Links und
//! Twitch-Seiten öffnen im normalen Browser, nur eine Instanz.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod private_browser;
mod rules;
mod web_update;

use std::path::PathBuf;
use std::sync::Arc;

use multiyap_server::{start, FileSource, ServerHandle, StartError, PORT};
use tauri::webview::{NewWindowResponse, PermissionKind, PermissionResponse};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use url::Url;

/// Der Build kommt aus Tauris eingebautem Dateispeicher (frontendDist), nicht von der Festplatte.
struct TauriAssets(AppHandle);

impl FileSource for TauriAssets {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.0.asset_resolver().get(path.to_string()).map(|asset| asset.bytes().to_vec())
    }
}

/// Hält den Server am Leben, solange das Programm läuft.
struct Server(#[allow(dead_code)] ServerHandle);

fn open_externally(app: &AppHandle, url: &Url) {
    if rules::may_open_externally(url) {
        let _ = tauri_plugin_opener::OpenerExt::opener(app).open_url(url.as_str(), None::<&str>);
    }
}

/// Update aus dem lokalen Versionsordner installieren. Nur eine neuere Version mit vorhandenem Installer
/// `vX.Y.Z\MultiYap-Setup-X.Y.Z.exe`. Der Installer läuft mit `/P` (nur Fortschrittsfenster, keine Klicks), `/UPDATE` (überspringt die
/// Seite „bereits installiert“ und behält Verknüpfungen, wie Tauris eigener Updater; geprüft am 25.11.2026) und `/R` (startet MultiYap
/// danach neu; Tauris NSIS-Vorlage, geprüft am 23.11.2026). MultiYap beendet sich, damit der Installer die Dateien ersetzen kann.
fn install_update(app: &AppHandle, folder: &str, version: &str) {
    if !multiyap_server::update::is_newer(version, multiyap_server::update::CURRENT_VERSION) {
        return;
    }
    let Some(installer) = multiyap_server::update::installer_path(std::path::Path::new(folder.trim()), version) else {
        return;
    };
    if std::process::Command::new(installer).args(["/P", "/UPDATE", "/R"]).spawn().is_ok() {
        app.exit(0);
    }
}

/// Daten des Fensters (Einstellungen, Layout, Anmeldung) in „%APPDATA%\MultiYap\WebView2“, getrennt vom Browser.
fn data_directory() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|appdata| PathBuf::from(appdata).join("MultiYap").join("WebView2"))
}

fn fail(app: &AppHandle, text: String) {
    let handle = app.clone();
    app.dialog()
        .message(text)
        .title("MultiYap")
        .kind(MessageDialogKind::Error)
        .show(move |_| handle.exit(1));
}

fn main() {
    multiyap_server::use_system_language();
    tauri::Builder::default()
        // Nur eine Instanz: ein zweiter Start holt das vorhandene Fenster nach vorn.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        // Fenstergröße und Position merken.
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let handle = app.handle().clone();
            match start(Arc::new(TauriAssets(handle.clone()))) {
                Ok(server) => {
                    app.manage(Server(server));
                }
                Err(error) => {
                    let text = match error {
                        StartError::InUse => multiyap_core::tf!(
                            "Port {PORT} ist schon belegt. Läuft MultiYap bereits im Browser (start.cmd)? Bitte das beenden und MultiYap neu starten.",
                            "Port {PORT} is already in use. Is MultiYap already running in the browser (start.cmd)? Please close it and start MultiYap again."
                        ),
                        other => multiyap_core::tf!("MultiYap konnte nicht starten: {other}", "MultiYap could not start: {other}"),
                    };
                    fail(&handle, text);
                    return Ok(());
                }
            }

            let navigation_handle = handle.clone();
            let window_handle = handle.clone();
            let mut builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(rules::app_origin()))
                .title("MultiYap")
                .inner_size(1280.0, 800.0)
                .min_inner_size(480.0, 360.0)
                // Tauri ersetzt sonst unter Windows das Ziehen und Ablegen von WebView2; dann kommt HTML-Ziehen
                // (Tabs verschieben, teilen, Schnellaktionen) in der Seite nicht an (Tauri-Doku, geprüft 23.11.2026).
                .disable_drag_drop_handler()
                // Töne der Benachrichtigungen ohne vorherigen Klick abspielen. Die von Tauri sonst gesetzten
                // Schalter müssen dann selbst mit angegeben werden.
                .additional_browser_args(
                    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required",
                )
                // Links und fremde Seiten im normalen Browser.
                .on_navigation(move |url| {
                    if let Some(target) = rules::private_target(url) {
                        if !private_browser::open(&target) {
                            open_externally(&navigation_handle, &target);
                        }
                        return false;
                    }
                    if rules::stays_in_app(url) {
                        return true;
                    }
                    open_externally(&navigation_handle, url);
                    false
                })
                // Neue Fenster (Links, Twitch-Nutzerkarte, Player-Popout, Anmeldung per Code) im normalen Browser.
                // Chat-Links bei eingeschalteter Einstellung im privaten Fenster des Standardbrowsers; wird der
                // Browser nicht erkannt, öffnet der Link normal.
                .on_new_window(move |url, _features| {
                    // Update-Adressen tragen die Sprache der Seite (lang=…) für die Meldungen des Programms.
                    if rules::is_app_page(&url) {
                        multiyap_server::use_page_language(url.as_str());
                    }
                    if let Some((folder, version)) = rules::update_request(&url) {
                        install_update(&window_handle, &folder, &version);
                        return NewWindowResponse::Deny;
                    }
                    if let Some(version) = rules::download_request(&url) {
                        web_update::install(window_handle.clone(), version);
                        return NewWindowResponse::Deny;
                    }
                    if let Some(target) = rules::private_target(&url) {
                        if !private_browser::open(&target) {
                            open_externally(&window_handle, &target);
                        }
                        return NewWindowResponse::Deny;
                    }
                    open_externally(&window_handle, &url);
                    NewWindowResponse::Deny
                })
                // Rechte nur für MultiYap selbst: installierte Schriften für die Chat-Schriftart.
                .on_permission_request(|webview, kind| {
                    let is_app = webview.url().is_ok_and(|url| rules::is_app_page(&url));
                    match kind {
                        PermissionKind::LocalFonts if is_app => PermissionResponse::Allow,
                        _ => PermissionResponse::Deny,
                    }
                });
            if let Some(directory) = data_directory() {
                builder = builder.data_directory(directory);
            }
            builder.build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("MultiYap konnte nicht starten");
}
