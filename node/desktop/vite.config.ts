import path from "node:path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig, type ProxyOptions } from "vite";

// The desktop app reuses the UI wholesale. `@` points at the UI's source tree so
// the UI's own `@/...` imports resolve when compiled from this (desktop) root, and
// so this entry can import the UI bootstrap (`@/main`) and the fetch registry
// (`@/lib/client/registry`) directly — without @open-lakehouse/ui exposing an
// `exports` surface.
const UI_SRC = path.resolve(__dirname, "../ui/src");

// In dev the Tauri webview loads this Vite server, so it needs the same gateway
// proxy the UI uses (see ../ui/vite.config.ts) for the Unity Catalog REST API and
// the embedded MLflow/marimo iframes. Mirrored here so desktop dev talks to one
// origin too.
const GATEWAY_URL = process.env.GATEWAY_URL ?? "http://localhost:9080";

// Hydrofoil's ConnectRPC QueryService (see ../ui/vite.config.ts). For host-run
// dev point QUERY_URL at hydrofoil's HTTP port (default :9082).
const QUERY_URL = process.env.QUERY_URL ?? GATEWAY_URL;

function serviceProxy(): ProxyOptions {
  return {
    target: GATEWAY_URL,
    changeOrigin: true,
    ws: true,
    autoRewrite: true,
    configure: (proxy) => {
      proxy.on("proxyRes", (proxyRes) => {
        proxyRes.headers["x-frame-options"] = undefined;
        proxyRes.headers["content-security-policy"] = undefined;
      });
    },
  };
}

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { "@": UI_SRC },
  },
  // Tauri expects a fixed port and quiet output; surface its env to the build.
  clearScreen: false,
  envPrefix: ["VITE_", "TAURI_ENV_"],
  server: {
    port: 3003,
    strictPort: true,
    proxy: {
      "/api": {
        target: GATEWAY_URL,
        changeOrigin: true,
      },
      "/hydrofoil.query.v1.QueryService": {
        target: QUERY_URL,
        changeOrigin: true,
      },
      "/mlflow": serviceProxy(),
      "/marimo": serviceProxy(),
    },
  },
});
