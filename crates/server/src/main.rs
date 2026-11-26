//! MultiYap im Browser (start.cmd): liefert den Build aus dem Ordner `dist` unter http://localhost:8475 aus und öffnet
//! den Browser. Ersetzt server/serve.mjs.
//!
//! Aufruf: `multiyap-server [--no-browser] [--root <Ordner>]`, Standard für den Ordner ist `dist`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use multiyap_core::i18n::tr;
use multiyap_server::{is_multiyap_server, start, DiskSource, StartError, APP_URL, PORT};

fn open_browser() {
    #[cfg(windows)]
    let result = {
        use std::os::windows::process::CommandExt;
        // Leerer Titel, damit `start` die Adresse nicht als Fenstertitel nimmt.
        std::process::Command::new("cmd").raw_arg(format!("/c start \"\" {APP_URL}")).spawn()
    };
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(APP_URL).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(APP_URL).spawn();
    if result.is_err() {
        eprintln!("{}", multiyap_core::tf!("Der Browser konnte nicht geöffnet werden. Bitte {APP_URL} selbst öffnen.", "The browser could not be opened. Please open {APP_URL} yourself."));
    }
}

fn main() -> ExitCode {
    multiyap_server::use_system_language();
    let mut args = std::env::args().skip(1);
    let mut no_browser = false;
    let mut root = PathBuf::from("dist");
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--no-browser" => no_browser = true,
            "--root" => match args.next() {
                Some(path) => root = PathBuf::from(path),
                None => {
                    eprintln!("{}", tr("Nach --root fehlt der Ordner.", "The folder is missing after --root."));
                    return ExitCode::FAILURE;
                }
            },
            other => {
                eprintln!("{}", multiyap_core::tf!("Unbekannte Angabe: {other}", "Unknown argument: {other}"));
                return ExitCode::FAILURE;
            }
        }
    }

    let source = Arc::new(DiskSource::new(root));
    let _server = match start(source) {
        Ok(server) => server,
        Err(StartError::NoBuild) => {
            eprintln!("{}", tr("Der Build fehlt (dist/index.html). Bitte über start.cmd starten.", "The build is missing (dist/index.html). Please start via start.cmd."));
            return ExitCode::FAILURE;
        }
        Err(StartError::InUse) => {
            // Nur öffnen, wenn dort wirklich MultiYap läuft (sonst hätte ein fremdes Programm Zugriff auf die
            // Anmeldung).
            if is_multiyap_server("127.0.0.1", PORT) {
                println!("{}", multiyap_core::tf!("MultiYap läuft schon unter {APP_URL}.", "MultiYap is already running at {APP_URL}."));
                if !no_browser {
                    open_browser();
                }
                return ExitCode::SUCCESS;
            }
            eprintln!(
                "{}",
                multiyap_core::tf!(
                    "Port {PORT} ist von einem anderen Programm belegt. Bitte dieses Programm oder das MultiYap-Programm beenden und erneut starten.",
                    "Port {PORT} is used by another program. Please close that program or the MultiYap program and start again."
                )
            );
            return ExitCode::FAILURE;
        }
        Err(error) => {
            eprintln!("{}", multiyap_core::tf!("Start fehlgeschlagen: {error}", "Start failed: {error}"));
            return ExitCode::FAILURE;
        }
    };

    println!("{}", multiyap_core::tf!("MultiYap läuft unter {APP_URL}", "MultiYap is running at {APP_URL}"));
    println!("{}", tr("Dieses Fenster offen lassen. Schließen beendet MultiYap.", "Keep this window open. Closing it ends MultiYap."));
    if !no_browser {
        open_browser();
    }

    // Läuft, bis das Fenster geschlossen oder Strg+C gedrückt wird.
    loop {
        std::thread::park();
    }
}
