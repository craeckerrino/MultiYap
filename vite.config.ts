import tailwindcss from '@tailwindcss/vite'
import { defineConfig } from 'vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [tailwindcss()],
  build: {
    // Nichts als data:-Adresse einbetten: Die CSP erlaubt Schriften nur von der eigenen Adresse (font-src 'self'),
    // eingebettete kleine Schriftteile (zum Beispiel von JetBrains Mono) würden sonst blockiert.
    assetsInlineLimit: 0,
  },
  server: {
    port: 8475,
    strictPort: true,
  },
  preview: {
    port: 8475,
    strictPort: true,
  },
})
