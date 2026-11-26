//! Updates von der Website: multiyap.app legt unter `/download/latest.json` die neueste Version mit Dateiname, Größe
//! und SHA-256 des Installers ab (Website/scripts/prepare.mjs). Die Seite prüft damit, ob es eine neuere Version gibt
//! (crates/web/src/updates.rs); das Programmfenster lädt den Installer, prüft Größe und Prüfsumme und startet ihn erst
//! dann (src-tauri/src/web_update.rs). Beide nutzen diese Prüfungen.

use serde_json::Value;

/// Angaben zur neuesten Version auf der Website.
pub const LATEST_URL: &str = "https://multiyap.app/download/latest.json";
/// Installer liegen unter dieser Adresse plus Dateiname.
pub const DOWNLOAD_BASE: &str = "https://multiyap.app/download/";
/// Größer ist kein Installer von MultiYap (der Installer hat rund 4 MB).
pub const MAX_SIZE: u64 = 200 * 1024 * 1024;

/// Neueste Version laut Website, geprüft.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub file_name: String,
    pub size: u64,
    /// SHA-256 als 64 Kleinbuchstaben und Ziffern (hex).
    pub sha256: String,
}

impl Release {
    pub fn download_url(&self) -> String {
        format!("{DOWNLOAD_BASE}{}", self.file_name)
    }
}

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

/// Liest `latest.json`. Nur gültige Version, der erwartete Dateiname zur Version (kein Pfad, keine andere Datei),
/// eine plausible Größe und eine vollständige Prüfsumme.
pub fn parse_release(json: &Value) -> Option<Release> {
    let version = json.get("version")?.as_str()?;
    parse_version(version)?;
    let file_name = json.get("fileName")?.as_str()?;
    if file_name != format!("MultiYap-Setup-{version}.exe") {
        return None;
    }
    let size = json.get("size")?.as_u64().filter(|s| (1..=MAX_SIZE).contains(s))?;
    let sha256 = json.get("sha256")?.as_str()?;
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return None;
    }
    Some(Release { version: version.to_string(), file_name: file_name.to_string(), size, sha256: sha256.to_string() })
}

/// Stimmt der geladene Installer mit den Angaben überein (Größe und SHA-256)?
pub fn matches(release: &Release, bytes: &[u8]) -> bool {
    use sha2::{Digest, Sha256};
    if bytes.len() as u64 != release.size {
        return false;
    }
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    hex == release.sha256
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const HASH_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn latest(version: &str, file: &str, size: u64, hash: &str) -> Value {
        json!({ "version": version, "fileName": file, "size": size, "sha256": hash })
    }

    #[test]
    fn compares_versions() {
        assert_eq!(parse_version("0.97.0"), Some((0, 97, 0)));
        assert_eq!(parse_version("0.97"), None);
        assert_eq!(parse_version("../1.2.3"), None);
        assert!(is_newer("0.98.0", "0.97.9"));
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(!is_newer("0.97.0", "0.97.0"));
        assert!(!is_newer("x", "0.97.0"));
    }

    #[test]
    fn reads_only_valid_releases() {
        let ok = parse_release(&latest("0.98.0", "MultiYap-Setup-0.98.0.exe", 3, HASH_ABC)).unwrap();
        assert_eq!(ok.download_url(), "https://multiyap.app/download/MultiYap-Setup-0.98.0.exe");
        // Dateiname muss zur Version passen, nie ein Pfad oder eine fremde Datei.
        assert!(parse_release(&latest("0.98.0", "MultiYap-Setup-0.97.0.exe", 3, HASH_ABC)).is_none());
        assert!(parse_release(&latest("0.98.0", "../MultiYap-Setup-0.98.0.exe", 3, HASH_ABC)).is_none());
        assert!(parse_release(&latest("0.98", "MultiYap-Setup-0.98.exe", 3, HASH_ABC)).is_none());
        assert!(parse_release(&latest("0.98.0", "MultiYap-Setup-0.98.0.exe", 0, HASH_ABC)).is_none());
        assert!(parse_release(&latest("0.98.0", "MultiYap-Setup-0.98.0.exe", MAX_SIZE + 1, HASH_ABC)).is_none());
        assert!(parse_release(&latest("0.98.0", "MultiYap-Setup-0.98.0.exe", 3, "abc")).is_none());
        assert!(parse_release(&latest("0.98.0", "MultiYap-Setup-0.98.0.exe", 3, &HASH_ABC.to_uppercase())).is_none());
        assert!(parse_release(&json!({})).is_none());
    }

    #[test]
    fn checks_size_and_hash() {
        let release = parse_release(&latest("0.98.0", "MultiYap-Setup-0.98.0.exe", 3, HASH_ABC)).unwrap();
        assert!(matches(&release, b"abc"));
        assert!(!matches(&release, b"abd"));
        assert!(!matches(&release, b"abcd"));
    }
}
