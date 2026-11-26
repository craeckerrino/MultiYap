//! Einstieg für den Browser (WebAssembly): exportiert nur `start` (crates/wasm/src/ui.rs). Fachlogik in crates/core,
//! Dienste in crates/web, Oberfläche in crates/ui. Erzeugt wird der Browser-Teil mit `npm run build:wasm`.

mod ui;
