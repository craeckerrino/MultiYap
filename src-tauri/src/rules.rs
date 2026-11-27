//! Regeln für das Programmfenster (übernommen aus electron/main.mjs und electron/permissions.mjs).

use url::Url;

/// Adresse von MultiYap im Fenster (lokaler Server).
pub fn app_origin() -> Url {
    Url::parse(multiyap_server::APP_URL).expect("feste Adresse ist gültig")
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme() && a.host_str() == b.host_str() && a.port_or_known_default() == b.port_or_known_default()
}

/// Seiten, die im Fenster selbst laufen dürfen: MultiYap und die Twitch-Anmeldung mit Weiterleitung (springt
/// danach zurück auf http://localhost:8475). Alles andere geht im normalen Browser auf.
pub fn stays_in_app(url: &Url) -> bool {
    if same_origin(url, &app_origin()) {
        return true;
    }
    url.scheme() == "https"
        && url
            .host_str()
            .is_some_and(|host| host == "twitch.tv" || host.ends_with(".twitch.tv"))
}

/// Nur http und https dürfen im normalen Browser geöffnet werden.
pub fn may_open_externally(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

/// Rechte bekommt nur MultiYap selbst, nie eine Twitch-Seite im Fenster (Sicherheitscheck, 16.11.2026).
pub fn is_app_page(url: &Url) -> bool {
    same_origin(url, &app_origin())
}

/// Chat-Link, der im privaten Fenster aufgehen soll: MultiYap öffnet dafür `/open-private?url=…` als neues Fenster
/// (crates/ui/src/chat/line.rs, `open_link_private`). Nur http und https.
pub fn private_target(url: &Url) -> Option<Url> {
    if !same_origin(url, &app_origin()) || url.path() != "/open-private" {
        return None;
    }
    let (_, target) = url.query_pairs().find(|(key, _)| key == "url")?;
    Url::parse(&target).ok().filter(may_open_externally)
}

/// Update installieren: MultiYap öffnet `/update-install?folder=…&version=…` als neues Fenster
/// (crates/web/src/updates.rs). Gibt Ordner und Version zurück; geprüft wird beim Installieren.
pub fn update_request(url: &Url) -> Option<(String, String)> {
    if !same_origin(url, &app_origin()) || url.path() != "/update-install" {
        return None;
    }
    let value = |name: &str| url.query_pairs().find(|(key, _)| key == name).map(|(_, value)| value.into_owned());
    Some((value("folder")?, value("version")?))
}

/// Programmdatei aus dem Öffnen-Befehl eines Browsers, zum Beispiel
/// `"C:\Program Files\Mozilla Firefox\firefox.exe" -osint -url "%1"`.
pub fn browser_executable(command: &str) -> Option<String> {
    let command = command.trim();
    let path = match command.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?,
        None => {
            let end = command.to_ascii_lowercase().find(".exe").map(|i| i + 4).unwrap_or(command.len());
            &command[..end]
        }
    };
    let path = path.trim();
    path.to_ascii_lowercase().ends_with(".exe").then(|| path.to_string())
}

/// Schalter für ein privates Fenster je nach Browser (aus dem Dateinamen der Programmdatei).
pub fn private_flag(executable: &str) -> Option<&'static str> {
    let name = executable.rsplit(['\\', '/']).next()?.to_ascii_lowercase();
    match name.as_str() {
        "msedge.exe" => Some("--inprivate"),
        "chrome.exe" | "brave.exe" | "vivaldi.exe" | "chromium.exe" => Some("--incognito"),
        "opera.exe" => Some("--private"),
        // Opera startet über „launcher.exe“ im Opera-Ordner.
        "launcher.exe" if executable.to_ascii_lowercase().contains("opera") => Some("--private"),
        "firefox.exe" | "librewolf.exe" | "waterfox.exe" => Some("-private-window"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn multiyap_und_twitch_anmeldung_bleiben_im_fenster() {
        assert!(stays_in_app(&url("http://localhost:8475/")));
        assert!(stays_in_app(&url("http://localhost:8475/#access_token=x")));
        assert!(stays_in_app(&url("https://id.twitch.tv/oauth2/authorize?x=1")));
        assert!(stays_in_app(&url("https://www.twitch.tv/login")));
        assert!(stays_in_app(&url("https://twitch.tv/")));
    }

    #[test]
    fn alles_andere_geht_nach_draussen() {
        for text in [
            "http://localhost:9999/",
            "http://127.0.0.1:8475/",
            "http://www.twitch.tv/",
            "https://twitch.tv.evil.example/",
            "https://eviltwitch.tv/",
            "https://example.com/",
        ] {
            assert!(!stays_in_app(&url(text)), "{text}");
        }
    }

    #[test]
    fn nur_http_und_https_nach_draussen() {
        assert!(may_open_externally(&url("https://example.com/")));
        assert!(may_open_externally(&url("http://example.com/")));
        assert!(!may_open_externally(&url("file:///C:/Windows/win.ini")));
        assert!(!may_open_externally(&url("javascript:alert(1)")));
        assert!(!may_open_externally(&url("ms-settings:privacy")));
    }

    #[test]
    fn private_links() {
        let target = private_target(&url("http://localhost:8475/open-private?url=https%3A%2F%2Fexample.com%2Fa%3Fb%3D1")).unwrap();
        assert_eq!(target.as_str(), "https://example.com/a?b=1");
        assert!(private_target(&url("http://localhost:8475/open-private?url=file%3A%2F%2F%2FC%3A%2Fx")).is_none());
        assert!(private_target(&url("http://localhost:8475/open-private")).is_none());
        assert!(private_target(&url("http://localhost:9999/open-private?url=https%3A%2F%2Fexample.com")).is_none());
        assert!(private_target(&url("https://example.com/open-private?url=https%3A%2F%2Fexample.com")).is_none());
    }

    #[test]
    fn update_adresse() {
        let request = update_request(&url("http://localhost:8475/update-install?folder=D%3A%5CVersionen&version=0.94.0")).unwrap();
        assert_eq!(request, (r"D:\Versionen".to_string(), "0.94.0".to_string()));
        assert!(update_request(&url("http://localhost:8475/update-install?folder=D%3A%5C")).is_none());
        assert!(update_request(&url("https://www.twitch.tv/update-install?folder=a&version=1.0.0")).is_none());
    }

    #[test]
    fn browser_befehl() {
        let edge = r#""C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe" --single-argument %1"#;
        assert_eq!(browser_executable(edge).as_deref(), Some(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"));
        assert_eq!(browser_executable(r"C:\Tools\Browser\chrome.exe %1").as_deref(), Some(r"C:\Tools\Browser\chrome.exe"));
        assert_eq!(browser_executable("rundll32 url.dll"), None);
        assert_eq!(private_flag(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"), Some("--inprivate"));
        assert_eq!(private_flag(r"C:\Program Files\Google\Chrome\Application\chrome.exe"), Some("--incognito"));
        assert_eq!(private_flag(r"C:\Program Files\Mozilla Firefox\FIREFOX.EXE"), Some("-private-window"));
        assert_eq!(private_flag(r"C:\Unbekannt\browser.exe"), None);
    }

    #[test]
    fn rechte_nur_fuer_multiyap() {
        assert!(is_app_page(&url("http://localhost:8475/")));
        assert!(!is_app_page(&url("https://www.twitch.tv/login")));
        assert!(!is_app_page(&url("http://localhost:9999/")));
    }
}
