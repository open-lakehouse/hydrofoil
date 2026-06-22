/// <reference types="vite/client" />

// Mirrors ../ui/env.d.ts so the aliased UI source type-checks under this
// (desktop) tsconfig. In a packaged Tauri build (no Vite dev proxy) set
// VITE_API_URL to the absolute gateway URL so the UI's relative /api base
// reaches the server.
interface ImportMetaEnv {
  readonly VITE_API_URL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
