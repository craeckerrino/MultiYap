import './index.css'
import init, { start } from './wasm/pkg/multiyap_wasm'

/**
 * Einstieg: Die App ist ganz in Rust (crates/ui zeichnet, crates/web startet Speicher, Anmeldung und Dienste). Hier
 * bleibt nur, was vor dem Laden von WebAssembly passieren muss: einen Rücksprung von Twitch sofort aus der Adresse
 * entfernen, damit das Token nicht im Verlauf oder in Lesezeichen landet. Ausgewertet wird er in Rust
 * (crates/web/src/auth.rs). Der Ordner `wasm/pkg` entsteht mit `npm run build:wasm`.
 */

/** Zufallswert der Anmeldung mit Rücksprung; Rust legt ihn beim Weiterleiten an (crates/web/src/auth.rs). */
const STATE_KEY = 'multiyap:oauth-state'

interface CapturedRedirect {
  href: string
  expectedState: string | null
}

function captureRedirect(): CapturedRedirect | null {
  const href = window.location.href
  const url = new URL(href)
  const hash = new URLSearchParams(url.hash.startsWith('#') ? url.hash.slice(1) : url.hash)
  if (!hash.has('access_token') && !url.searchParams.has('error') && !hash.has('error')) return null

  // Adresse sofort bereinigen.
  window.history.replaceState(null, '', url.pathname)

  let expectedState: string | null = null
  try {
    expectedState = sessionStorage.getItem(STATE_KEY)
    sessionStorage.removeItem(STATE_KEY)
  } catch {
    expectedState = null
  }
  return { href, expectedState }
}

const redirect = captureRedirect()

init().then(
  () => start(redirect?.href, redirect?.expectedState ?? undefined),
  () => {
    const root = document.getElementById('root')
    if (root) {
      root.textContent = 'MultiYap konnte nicht starten: Die Programmteile in Rust (WebAssembly) ließen sich nicht laden. Bitte neu laden.'
    }
  },
)
