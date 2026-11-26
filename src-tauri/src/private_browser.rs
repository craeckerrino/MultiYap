//! Links im privaten Fenster des Standardbrowsers öffnen.
//!
//! Der Standardbrowser steht unter Windows in der Registry: `HKCU\Software\Microsoft\Windows\Shell\Associations\
//! UrlAssociations\https\UserChoice` (Wert `ProgId`), dessen Öffnen-Befehl unter `HKCR\<ProgId>\shell\open\command`.
//! Aus der Programmdatei ergibt sich der Schalter für ein privates Fenster (src-tauri/src/rules.rs).

use std::process::Command;

use url::Url;

use crate::rules;

#[cfg(windows)]
fn default_browser_command() -> Option<String> {
    use winreg::enums::{HKEY_CLASSES_ROOT, HKEY_CURRENT_USER};
    use winreg::RegKey;

    let user = RegKey::predef(HKEY_CURRENT_USER);
    let prog_id: String = ["https", "http"].iter().find_map(|scheme| {
        let path = format!(r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\{scheme}\UserChoice");
        user.open_subkey(path).ok()?.get_value::<String, _>("ProgId").ok().filter(|id| !id.is_empty())
    })?;
    RegKey::predef(HKEY_CLASSES_ROOT)
        .open_subkey(format!(r"{prog_id}\shell\open\command"))
        .ok()?
        .get_value::<String, _>("")
        .ok()
}

#[cfg(not(windows))]
fn default_browser_command() -> Option<String> {
    None
}

/// Startet den Standardbrowser mit privatem Fenster. false, wenn der Browser nicht erkannt wurde oder nicht startet.
pub fn open(url: &Url) -> bool {
    if !rules::may_open_externally(url) {
        return false;
    }
    let Some(executable) = default_browser_command().as_deref().and_then(rules::browser_executable) else {
        return false;
    };
    let Some(flag) = rules::private_flag(&executable) else {
        return false;
    };
    // Die Adresse ist ein eigenes Argument (von url geprüft und kodiert), nie Teil eines Befehls.
    Command::new(executable).arg(flag).arg(url.as_str()).spawn().is_ok()
}
