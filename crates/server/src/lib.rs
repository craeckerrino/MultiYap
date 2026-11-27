//! Lokaler Webserver für MultiYap (Ersetzt server/server-core.mjs).
//!
//! Liefert den fertigen Build aus, setzt die Sicherheits-Header und hört ausschließlich auf der eigenen Maschine
//! (127.0.0.1 und, wenn vorhanden, ::1). Genutzt von der Browser-Version (`multiyap-server`) und vom Desktop-Programm
//! (src-tauri). Verhalten wie bisher:.

pub mod update;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Fest, weil die Twitch-App genau diese Rücksprungadresse kennt. Meldungen des Programms in der Sprache des Systems
/// (Deutsch, sonst Englisch), bis die Seite ihre Sprache meldet (Parameter `lang` bei Updates,
/// crates/web/src/updates.rs).
pub fn use_system_language() {
    if let Some(locale) = sys_locale::get_locale() {
        multiyap_core::i18n::set(multiyap_core::i18n::Lang::from_code(&locale));
    }
}

/// Sprache aus dem Parameter `lang` einer Adresse der Seite übernehmen, wenn vorhanden.
pub fn use_page_language(url: &str) {
    let query = url.split_once('?').map(|(_, query)| query).unwrap_or_default();
    if let Some(code) = query.split('&').find_map(|pair| pair.strip_prefix("lang=")) {
        multiyap_core::i18n::set(multiyap_core::i18n::Lang::from_code(code));
    }
}

pub const PORT: u16 = 8475;
pub const APP_URL: &str = "http://localhost:8475";

/// Kennung in jeder Antwort, damit ein zweiter Start erkennt, ob auf dem Port wirklich MultiYap läuft
/// (Sicherheitscheck, 16.11.2026).
pub const MARKER_HEADER: &str = "X-MultiYap";

/// Content-Security-Policy, unverändert aus server-core.mjs übernommen. Die Adresse des MultiYap-Servers für YouTube
/// hängt `csp()` an, falls beim Bauen einer angegeben ist.
const CSP_BASE: &str = concat!(
    "default-src 'self'; ",
    // WebAssembly (Rust-Fachlogik) braucht 'wasm-unsafe-eval'; normales eval bleibt verboten.
    "script-src 'self' 'wasm-unsafe-eval'; ",
    "style-src 'self' 'unsafe-inline'; ",
    // Twitch: Emotes und Abzeichen, Bilder für Bits (Cheermotes); Emotes von 7TV, BTTV und FFZ
    "img-src 'self' data: blob: https://static-cdn.jtvnw.net https://d3aqoihi2n8ty8.cloudfront.net https://cdn.7tv.app https://cdn.betterttv.net https://cdn.frankerfacez.com; ",
    "font-src 'self'; ",
    "media-src 'self' blob:; ",
    // Twitch: Anmeldung (id), Helix-API (api), EventSub (eventsub.wss), IRC nur lesend (irc-ws); Verlauf:
    // recent-messages; Emote-Listen von 7TV, BTTV und FFZ; 7TV-Abzeichen und Namensfarben (EventAPI)
    "connect-src 'self' https://id.twitch.tv https://api.twitch.tv wss://eventsub.wss.twitch.tv wss://irc-ws.chat.twitch.tv https://recent-messages.robotty.de https://7tv.io wss://events.7tv.io https://api.betterttv.net https://api.frankerfacez.com; ",
    // Stream-Tab: offizieller Twitch-Player
    "frame-src https://player.twitch.tv; ",
    "object-src 'none'; ",
    "base-uri 'self'; ",
    "form-action 'self'; ",
    "frame-ancestors 'none'",
);

/// Content-Security-Policy, mit der Adresse des MultiYap-Servers für YouTube (`multiyap_core::youtube::SERVER`),
/// falls es einen gibt (YouTube über `/api/youtube/`, crates/web/src/youtube.rs).
pub fn csp() -> &'static str {
    static CSP: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CSP.get_or_init(|| match multiyap_core::youtube::SERVER {
        Some(server) => CSP_BASE.replacen("https://api.frankerfacez.com;", &format!("https://api.frankerfacez.com {server};"), 1),
        None => CSP_BASE.to_string(),
    })
}

/// Kopfzeilen jeder Antwort.
pub fn base_headers() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Content-Security-Policy", csp()),
        ("X-Content-Type-Options", "nosniff"),
        ("Referrer-Policy", "strict-origin-when-cross-origin"),
        ("Cross-Origin-Opener-Policy", "same-origin"),
        (MARKER_HEADER, "1"),
    ]
}

/// Woher die Dateien des Builds kommen: aus einem Ordner (Browser-Version) oder fest eingebaut (Programm).
pub trait FileSource: Send + Sync + 'static {
    /// `path` ist bereits geprüft: relativ, mit `/` getrennt, ohne `..`. Leerer Pfad ist kein gültiger Name.
    fn read(&self, path: &str) -> Option<Vec<u8>>;
}

/// Build aus einem Ordner auf der Festplatte.
pub struct DiskSource {
    root: PathBuf,
}

impl DiskSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl FileSource for DiskSource {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        let mut full = self.root.clone();
        for segment in path.split('/') {
            full.push(segment);
        }
        // Nur echte Dateien, keine Ordner.
        if !full.is_file() {
            return None;
        }
        std::fs::read(full).ok()
    }
}

/// Antwort unabhängig vom HTTP-Server, damit sie sich testen lässt.
#[derive(Debug, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn text(status: u16, body: &str) -> Self {
        let mut headers = owned(base_headers());
        headers.push(("Content-Type".into(), "text/plain; charset=utf-8".into()));
        Reply { status, headers, body: body.as_bytes().to_vec() }
    }
}

fn owned(headers: Vec<(&'static str, &'static str)>) -> Vec<(String, String)> {
    headers.into_iter().map(|(key, value)| (key.to_string(), value.to_string())).collect()
}

/// Inhaltstyp nach Dateiendung (wie server-core.mjs).
pub fn mime_type(path: &str) -> &'static str {
    let extension = extension_of(path).map(|e| e.to_ascii_lowercase());
    match extension.as_deref() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("ogg") => "audio/ogg",
        Some("txt") => "text/plain; charset=utf-8",
        Some("webmanifest") => "application/manifest+json",
        // WebAssembly (Rust-Fachlogik): nur mit diesem Typ lädt der Browser es direkt beim Herunterladen.
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}

/// Endung des letzten Pfadteils wie Node `extname` (ein führender Punkt zählt nicht als Endung).
fn extension_of(path: &str) -> Option<&str> {
    let last = path.rsplit('/').next().unwrap_or(path);
    match last.rfind('.') {
        Some(index) if index > 0 && index + 1 < last.len() => Some(&last[index + 1..]),
        Some(index) if index > 0 => Some(""),
        _ => None,
    }
}

/// Prozent-Kodierung auflösen. None bei ungültiger Kodierung oder ungültigem UTF-8.
fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = bytes.get(index + 1..index + 3)?;
            let value = u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
            out.push(value);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Wandelt den URL-Pfad in einen sicheren, relativen Pfad innerhalb des Builds um. None: führt hinaus oder ist
/// ungültig (Antwort 400). Leerer String: Wurzel.
pub fn safe_path(url_path: &str) -> Option<String> {
    let decoded = percent_decode(url_path)?;
    if decoded.contains('\0') {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for segment in decoded.split(['/', '\\']) {
        match segment {
            "" | "." => continue,
            // Kein Schritt nach oben, keine Laufwerksangaben (zum Beispiel C:).
            ".." => return None,
            other if other.contains(':') => return None,
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// Beantwortet eine Anfrage (Methode und Adresse) aus `source`.
pub fn respond(method: &str, url: &str, source: &dyn FileSource) -> Reply {
    if method != "GET" && method != "HEAD" {
        let mut reply = Reply::text(405, "Methode nicht erlaubt");
        reply.headers.push(("Allow".into(), "GET, HEAD".into()));
        return reply;
    }

    let url_path = url.split(['?', '#']).next().unwrap_or("/");
    let Some(path) = safe_path(url_path) else {
        return Reply::text(400, "Ungültige Adresse");
    };

    if !path.is_empty() {
        if let Some(body) = source.read(&path) {
            return file_reply(&path, body);
        }
        // Fehlende Dateien mit Endung sind echte 404. Alles andere ist eine App-Adresse und bekommt index.html.
        if extension_of(&path).is_some() {
            return Reply::text(404, "Nicht gefunden");
        }
    }

    match source.read("index.html") {
        Some(body) => file_reply("index.html", body),
        None => Reply::text(500, "Build fehlt. Bitte start.cmd verwenden."),
    }
}

fn file_reply(path: &str, body: Vec<u8>) -> Reply {
    let mut headers = owned(base_headers());
    headers.push(("Content-Type".into(), mime_type(path).into()));
    // Dateien unter assets/ tragen einen Inhalts-Hash im Namen und dürfen dauerhaft zwischengespeichert werden.
    let cache = if path.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    headers.push(("Cache-Control".into(), cache.into()));
    Reply { status: 200, headers, body }
}

/// Fehler beim Start.
#[derive(Debug)]
pub enum StartError {
    /// Port 8475 ist belegt.
    InUse,
    /// index.html fehlt im Build.
    NoBuild,
    Other(String),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartError::InUse => f.write_str(&multiyap_core::tf!("Port {PORT} ist schon belegt.", "Port {PORT} is already in use.")),
            StartError::NoBuild => f.write_str(multiyap_core::i18n::tr("Der Build fehlt (index.html).", "The build is missing (index.html).")),
            StartError::Other(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for StartError {}

/// Laufender Server. Beim Verwerfen oder mit `stop` wird er beendet.
pub struct ServerHandle {
    servers: Vec<Arc<tiny_http::Server>>,
}

impl ServerHandle {
    pub fn stop(&self) {
        for server in &self.servers {
            server.unblock();
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Anfragen, die gleichzeitig bearbeitet werden (Dateien sind klein, mehr ist nicht nötig).
const WORKERS: usize = 4;

/// Startet den Server auf 127.0.0.1 und, wenn vorhanden, ::1, immer auf Port 8475.
pub fn start(source: Arc<dyn FileSource>) -> Result<ServerHandle, StartError> {
    start_on(source, PORT)
}

/// Wie `start`, mit wählbarem Port (nur für Tests, damit ein laufendes MultiYap nicht stört).
fn start_on(source: Arc<dyn FileSource>, port: u16) -> Result<ServerHandle, StartError> {
    if source.read("index.html").is_none() {
        return Err(StartError::NoBuild);
    }
    let mut servers = Vec::new();
    for (index, address) in ["127.0.0.1", "[::1]"].into_iter().enumerate() {
        let socket: SocketAddr = format!("{address}:{port}")
            .parse()
            .map_err(|_| StartError::Other("Ungültige Adresse".into()))?;
        match tiny_http::Server::http(socket) {
            Ok(server) => servers.push(Arc::new(server)),
            Err(error) => {
                let in_use = error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::AddrInUse);
                // IPv6 ist nicht auf jedem System verfügbar. Das ist kein Fehler, solange IPv4 läuft.
                if index == 1 && !in_use {
                    continue;
                }
                for server in &servers {
                    server.unblock();
                }
                return Err(if in_use { StartError::InUse } else { StartError::Other(error.to_string()) });
            }
        }
    }
    for server in &servers {
        for _ in 0..WORKERS {
            let server = Arc::clone(server);
            let source = Arc::clone(&source);
            thread::spawn(move || serve(&server, source.as_ref()));
        }
    }
    Ok(ServerHandle { servers })
}

fn serve(server: &tiny_http::Server, source: &dyn FileSource) {
    while let Ok(request) = server.recv() {
        let header = |name: &str| {
            request.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name)).map(|h| h.value.as_str().to_string())
        };
        let path = request.url().split(['?', '#']).next().unwrap_or("/");
        let reply = if path == "/api/update" && request.method().as_str() == "GET" {
            // Nur Anfragen von MultiYap selbst: fremde Seiten senden immer Origin oder Sec-Fetch-Site mit.
            let same_origin = header("Sec-Fetch-Site").is_none_or(|site| site == "same-origin")
                && header("Origin").is_none_or(|origin| origin == APP_URL);
            update::reply(request.url(), same_origin)
        } else {
            respond(request.method().as_str(), request.url(), source)
        };
        let mut response = tiny_http::Response::from_data(reply.body).with_status_code(reply.status);
        for (key, value) in &reply.headers {
            if let Ok(header) = tiny_http::Header::from_bytes(key.as_bytes(), value.as_bytes()) {
                response.add_header(header);
            }
        }
        // Ein abgebrochener Abruf (Tab geschlossen) ist kein Fehler.
        let _ = request.respond(response);
    }
}

/// Prüft, ob auf dem Port wirklich MultiYap antwortet (Sicherheitscheck, 16.11.2026): Ist er von einem anderen
/// Programm belegt, darf der Browser nicht dorthin geöffnet werden, weil diese Adresse Zugriff auf die
/// gespeicherte Anmeldung hätte.
pub fn is_multiyap_server(host: &str, port: u16) -> bool {
    let Ok(mut addresses) = (host, port).to_socket_addrs() else {
        return false;
    };
    let Some(address) = addresses.next() else {
        return false;
    };
    let timeout = Duration::from_secs(3);
    let Ok(mut stream) = TcpStream::connect_timeout(&address, timeout) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    let request = format!("HEAD / HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut response = Vec::new();
    let _ = stream.take(16 * 1024).read_to_end(&mut response);
    let text = String::from_utf8_lossy(&response);
    let head = text.split("\r\n\r\n").next().unwrap_or("");
    head.lines().skip(1).any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.trim().eq_ignore_ascii_case(MARKER_HEADER) && value.trim() == "1"
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct MapSource(HashMap<&'static str, &'static str>);

    impl FileSource for MapSource {
        fn read(&self, path: &str) -> Option<Vec<u8>> {
            self.0.get(path).map(|content| content.as_bytes().to_vec())
        }
    }

    fn build() -> MapSource {
        MapSource(HashMap::from([
            ("index.html", "<!doctype html>"),
            ("assets/app-abc.js", "console.log(1)"),
            ("favicon.svg", "<svg/>"),
        ]))
    }

    #[test]
    fn liefert_dateien_mit_typ_und_cache() {
        let reply = respond("GET", "/assets/app-abc.js?v=1", &build());
        assert_eq!(reply.status, 200);
        assert_eq!(reply.header("content-type"), Some("text/javascript; charset=utf-8"));
        assert_eq!(reply.header("cache-control"), Some("public, max-age=31536000, immutable"));
        assert_eq!(reply.body, b"console.log(1)");

        let svg = respond("GET", "/favicon.svg", &build());
        assert_eq!(svg.header("content-type"), Some("image/svg+xml"));
        assert_eq!(svg.header("cache-control"), Some("no-cache"));
    }

    #[test]
    fn jede_antwort_hat_sicherheits_header_und_kennung() {
        for url in ["/", "/assets/app-abc.js", "/fehlt.js", "/../x"] {
            let reply = respond("GET", url, &build());
            assert_eq!(reply.header("content-security-policy"), Some(csp()), "{url}");
            assert_eq!(reply.header("x-content-type-options"), Some("nosniff"), "{url}");
            assert_eq!(reply.header(MARKER_HEADER), Some("1"), "{url}");
        }
    }

    #[test]
    fn app_adressen_bekommen_index_html() {
        for url in ["/", "", "/einstellungen", "/#access_token=x", "/a/b/c"] {
            let reply = respond("GET", url, &build());
            assert_eq!(reply.status, 200, "{url}");
            assert_eq!(reply.body, b"<!doctype html>", "{url}");
            assert_eq!(reply.header("content-type"), Some("text/html; charset=utf-8"));
        }
    }

    #[test]
    fn fehlende_dateien_mit_endung_sind_404() {
        assert_eq!(respond("GET", "/fehlt.js", &build()).status, 404);
        assert_eq!(respond("GET", "/assets/x.css", &build()).status, 404);
    }

    #[test]
    fn kein_weg_aus_dem_build_heraus() {
        for url in [
            "/../secret.txt",
            "/assets/../../x",
            "/%2e%2e/x",
            "/%2E%2E%2Fx",
            "/..%5cx",
            "/C:/Windows/win.ini",
            "/x%00.js",
            "/%zz",
        ] {
            assert_eq!(respond("GET", url, &build()).status, 400, "{url}");
        }
        assert_eq!(safe_path("/assets/./app.js").as_deref(), Some("assets/app.js"));
        assert_eq!(safe_path("//assets//app.js").as_deref(), Some("assets/app.js"));
    }

    #[test]
    fn nur_get_und_head() {
        let reply = respond("POST", "/", &build());
        assert_eq!(reply.status, 405);
        assert_eq!(reply.header("allow"), Some("GET, HEAD"));
        assert_eq!(respond("HEAD", "/", &build()).status, 200);
    }

    #[test]
    fn ohne_build_fehler_500() {
        let empty = MapSource(HashMap::new());
        assert_eq!(respond("GET", "/", &empty).status, 500);
    }

    #[test]
    fn endungen_wie_node() {
        assert_eq!(extension_of("a/b.js"), Some("js"));
        assert_eq!(extension_of(".hidden"), None);
        assert_eq!(extension_of("ordner/datei"), None);
        assert_eq!(mime_type("x.WOFF2"), "font/woff2");
        assert_eq!(mime_type("assets/multiyap_wasm_bg-abc.wasm"), "application/wasm");
        assert_eq!(mime_type("x.unbekannt"), "application/octet-stream");
    }

    /// Echte HTTP-Anfrage an den laufenden Server, Antwort als Text.
    fn fetch(port: u16, request: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn echter_server_liefert_aus_dem_ordner() {
        let dir = std::env::temp_dir().join(format!("multiyap-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("index.html"), "<!doctype html><title>t</title>").unwrap();
        std::fs::write(dir.join("assets").join("app.js"), "1").unwrap();
        // Eine Datei außerhalb des Builds, die nie ausgeliefert werden darf.
        std::fs::write(dir.with_extension("geheim"), "geheim").unwrap();

        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = free.local_addr().unwrap().port();
        drop(free);
        let server = start_on(Arc::new(DiskSource::new(&dir)), port).unwrap();

        let page = fetch(port, "GET /einstellungen HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        assert!(page.starts_with("HTTP/1.1 200"), "{page}");
        assert!(page.contains("X-MultiYap: 1"));
        assert!(page.contains("Content-Security-Policy: default-src 'self'"));
        assert!(page.ends_with("<!doctype html><title>t</title>"));

        let head = fetch(port, "HEAD /assets/app.js HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert!(head.contains("immutable"));
        assert!(head.ends_with("\r\n\r\n"), "HEAD ohne Inhalt: {head}");

        let escape = fetch(port, "GET /..%2f..%2fmultiyap-test.geheim HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        assert!(escape.starts_with("HTTP/1.1 400"), "{escape}");
        assert!(!escape.contains("geheim\r\n") && !escape.ends_with("geheim"));

        assert!(is_multiyap_server("127.0.0.1", port));
        assert!(matches!(start_on(Arc::new(DiskSource::new(&dir)), port), Err(StartError::InUse)));

        server.stop();
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(dir.with_extension("geheim"));
    }

    #[test]
    fn erkennt_multiyap_nur_an_der_kennung() {
        use std::net::TcpListener;
        // Fremdes Programm ohne Kennung.
        let foreign = TcpListener::bind("127.0.0.1:0").unwrap();
        let foreign_port = foreign.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            if let Ok((mut stream, _)) = foreign.accept() {
                let mut buffer = [0u8; 512];
                let _ = stream.read(&mut buffer);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
            }
        });
        assert!(!is_multiyap_server("127.0.0.1", foreign_port));
        handle.join().unwrap();

        // Antwort mit Kennung.
        let marked = TcpListener::bind("127.0.0.1:0").unwrap();
        let marked_port = marked.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            if let Ok((mut stream, _)) = marked.accept() {
                let mut buffer = [0u8; 512];
                let _ = stream.read(&mut buffer);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nX-MultiYap: 1\r\nContent-Length: 0\r\n\r\n");
            }
        });
        assert!(is_multiyap_server("127.0.0.1", marked_port));
        handle.join().unwrap();

        // Niemand auf dem Port.
        let free = TcpListener::bind("127.0.0.1:0").unwrap();
        let free_port = free.local_addr().unwrap().port();
        drop(free);
        assert!(!is_multiyap_server("127.0.0.1", free_port));
    }
}
