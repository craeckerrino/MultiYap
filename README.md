# MultiYap

Chat-App für Twitch und YouTube unter Windows: beliebig viele Kanäle in einem Fenster, Stream-Ansicht als eigener Tab und Moderationswerkzeuge für Kanäle, in denen du Mod oder Broadcaster bist.

Download und Infos: [multiyap.app](https://multiyap.app)

*A Twitch and YouTube chat app for Windows: many channels in one window, the stream as its own tab and moderation tools for channels you moderate. Download at [multiyap.app](https://multiyap.app).*

## Technik

Geschrieben in Rust: [Tauri 2](https://tauri.app) für das Programmfenster, [Leptos](https://leptos.dev) (WebAssembly) für die Oberfläche. Die App nutzt das in Windows eingebaute WebView2.

| Ordner | Inhalt |
| --- | --- |
| `crates/core` | Fachlogik ohne Oberfläche |
| `crates/ui` | Oberfläche in Leptos |
| `crates/web` | Dienste im Browser: Speicher, Twitch, YouTube |
| `crates/wasm` | Einstieg für WebAssembly |
| `crates/server` | lokaler Webserver |
| `src-tauri` | Desktop-Programm |

## Bauen

Voraussetzungen: Windows, [Rust](https://rustup.rs) mit dem Ziel `wasm32-unknown-unknown`, `wasm-bindgen-cli` in Version 0.2.130 und [Node.js](https://nodejs.org).

```powershell
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130
npm ci
npm run app:dist
```

Der Installer liegt danach unter `target/release/bundle/nsis`. Tests: `npm test`.

## Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io), certificate by [SignPath Foundation](https://signpath.org).

- Committers and reviewers: [craeckerrino](https://github.com/craeckerrino)
- Approvers: [craeckerrino](https://github.com/craeckerrino)

Only the installer built by [GitHub Actions](.github/workflows/build.yml) from this repository is signed. Every release is approved by hand before signing.

### Privacy

MultiYap only connects to the services it needs for the features you use: Twitch (chat, sign in, moderation), the emote services 7TV, BetterTTV and FrankerFaceZ, recent-messages.robotty.de (recent chat history when a channel is opened) and multiyap.app (YouTube chats and the update check, which can be turned off under Settings → Updates). There is no telemetry, no tracking and no advertising. Details: [privacy policy](https://multiyap.app/en/privacy/) ([Deutsch](https://multiyap.app/de/datenschutz/)).

## Lizenz

[GPL 3.0](LICENSE)
