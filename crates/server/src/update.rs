//! Updates aus einem lokalen Ordner. Der Ordner enthält je
//! freigegebener Version `vX.Y.Z\MultiYap-Setup-X.Y.Z.exe` (Ablauf „übertragen“). Die Seite fragt `/api/update?folder=…&lang=…` ab; installiert wird im
//! Programmfenster (src-tauri/src/main.rs), das dieselben Prüfungen nutzt.

use std::path::{Path, PathBuf};

use multiyap_core::i18n::tr;

use crate::{base_headers, owned, percent_decode, Reply};

/// Version dieses Programms (gemeinsame Version aller Teile, Cargo.toml `[workspace.package]`).
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// „X.Y.Z“ als Zahlen; nur Ziffern und zwei Punkte.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.split('.');
    let mut next = || -> Option<u64> {
        let part = parts.next()?;
        (!part.is_empty() && part.len() <= 9 && part.bytes().all(|b| b.is_ascii_digit())).then(|| part.parse().ok())?
    };
    let version = (next()?, next()?, next()?);
    parts.next().is_none().then_some(version)
}

/// Ist `candidate` neuer als `current`?
pub fn is_newer(candidate: &str, current: &str) -> bool {
    matches!((parse_version(candidate), parse_version(current)), (Some(a), Some(b)) if a > b)
}

/// Pfad des Installers einer Version im Ordner, wenn es ihn gibt.
pub fn installer_path(folder: &Path, version: &str) -> Option<PathBuf> {
    parse_version(version)?;
    if !folder.is_absolute() {
        return None;
    }
    let path = folder.join(format!("v{version}")).join(format!("MultiYap-Setup-{version}.exe"));
    path.is_file().then_some(path)
}

/// Neueste Version im Ordner mit vorhandenem Installer.
pub fn newest(folder: &Path) -> Result<Option<String>, &'static str> {
    if !folder.is_absolute() {
        return Err(tr(
            "Bitte einen vollständigen Pfad angeben (zum Beispiel D:\\Ordner\\Versionen).",
            "Please enter a full path (for example D:\\Folder\\Versions).",
        ));
    }
    let entries = std::fs::read_dir(folder).map_err(|_| tr("Ordner nicht gefunden.", "Folder not found."))?;
    let best = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str().and_then(|name| name.strip_prefix('v')).map(str::to_string))
        .filter(|version| installer_path(folder, version).is_some())
        .max_by_key(|version| parse_version(version));
    Ok(best)
}

fn json_reply(status: u16, body: String) -> Reply {
    let mut headers = owned(base_headers());
    headers.push(("Content-Type".into(), "application/json; charset=utf-8".into()));
    headers.push(("Cache-Control".into(), "no-store".into()));
    Reply { status, headers, body: body.into_bytes() }
}

/// Antwort auf `/api/update?folder=…`: `{"version":"X.Y.Z"}`, wenn im Ordner eine neuere Version liegt, sonst
/// `{"version":null}`, bei einem Problem zusätzlich `"error"`. Nur für MultiYap selbst (`same_origin`), damit fremde
/// Seiten im Browser den Rechner nicht nach Ordnern abfragen können.
pub fn reply(url: &str, same_origin: bool) -> Reply {
    if !same_origin {
        return json_reply(403, r#"{"error":"Nicht erlaubt."}"#.into());
    }
    crate::use_page_language(url);
    let folder = url
        .split_once('?')
        .map(|(_, query)| query)
        .unwrap_or_default()
        .split('&')
        .find_map(|pair| pair.strip_prefix("folder="))
        .and_then(|value| percent_decode(&value.replace('+', " ")))
        .unwrap_or_default();
    match newest(Path::new(folder.trim())) {
        // Versionen bestehen nur aus Ziffern und Punkten, Fehlertexte sind fest: kein Maskieren nötig.
        Ok(Some(version)) if is_newer(&version, CURRENT_VERSION) => json_reply(200, format!(r#"{{"version":"{version}"}}"#)),
        Ok(_) => json_reply(200, r#"{"version":null}"#.into()),
        Err(message) => json_reply(200, format!(r#"{{"version":null,"error":"{}"}}"#, message.replace('\\', "\\\\"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versionen_vergleichen() {
        assert_eq!(parse_version("0.93.1"), Some((0, 93, 1)));
        assert_eq!(parse_version("0.93"), None);
        assert_eq!(parse_version("0.93.1.2"), None);
        assert_eq!(parse_version("0.93.x"), None);
        assert_eq!(parse_version("../1.2.3"), None);
        assert!(is_newer("0.94.0", "0.93.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.93.1", "0.93.1"));
        assert!(is_newer("0.10.0", "0.9.0"));
    }

    #[test]
    fn neueste_version_im_ordner() {
        let folder = std::env::temp_dir().join(format!("multiyap-update-test-{}", std::process::id()));
        for (dir, file) in [("v0.9.0", Some("MultiYap-Setup-0.9.0.exe")), ("v0.10.0", Some("MultiYap-Setup-0.10.0.exe")), ("v0.11.0", None), ("anderes", None)] {
            std::fs::create_dir_all(folder.join(dir)).unwrap();
            if let Some(file) = file {
                std::fs::write(folder.join(dir).join(file), b"x").unwrap();
            }
        }
        assert_eq!(newest(&folder), Ok(Some("0.10.0".into())));
        assert!(installer_path(&folder, "0.10.0").is_some());
        assert!(installer_path(&folder, "0.11.0").is_none());
        assert!(installer_path(Path::new("relativ"), "0.10.0").is_none());
        assert!(newest(Path::new("relativ")).is_err());
        assert!(newest(&folder.join("fehlt")).is_err());
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn nur_fuer_multiyap_selbst() {
        assert_eq!(reply("/api/update?folder=C%3A%5C", false).status, 403);
        let body = String::from_utf8(reply("/api/update?folder=relativ", true).body).unwrap();
        assert!(body.contains(r#""version":null"#) && body.contains("error"));
    }
}
