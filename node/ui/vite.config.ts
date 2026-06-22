import path from "node:path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig, type ProxyOptions } from "vite";

// The Envoy gateway from environments/ fronts every backend service. In dev we
// proxy the same path prefixes Envoy serves so the browser talks to a single
// origin (the Vite dev server), avoiding cross-origin/CSP issues when embedding
// the MLflow and marimo UIs and when calling the Unity Catalog REST API.
const GATEWAY_URL = process.env.GATEWAY_URL ?? "http://localhost:9080";

// Hydrofoil's ConnectRPC QueryService. In the fully-containerized stack it's
// fronted by Envoy (under GATEWAY_URL); for host-run dev (`just hydro`), point
// QUERY_URL straight at hydrofoil's HTTP port (default :9082).
const QUERY_URL = process.env.QUERY_URL ?? GATEWAY_URL;

// Service UIs (MLflow, marimo) are embedded in an <iframe>. Two things break
// that, both fixed here:
//   1. They 30x-redirect the base path to an ABSOLUTE gateway URL
//      (http://localhost:9080/...), which is cross-origin to the dev server.
//      `autoRewrite` rewrites that Location back to the dev origin so the iframe
//      stays same-origin. (We also point the iframe at the trailing-slash path
//      to skip the redirect entirely — see src/lib/services.ts.)
//   2. MLflow sends `x-frame-options: SAMEORIGIN` (and apps may send a CSP
//      `frame-ancestors`), which blocks framing. We strip those response headers
//      so the embedded UI renders.
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
    alias: { "@": path.resolve(__dirname, "src") },
  },
  server: {
    port: 3002,
    proxy: {
      // Unity Catalog REST API (Envoy routes /api/2.1/unity-catalog -> unity-catalog:8081).
      "/api": {
        target: GATEWAY_URL,
        changeOrigin: true,
      },
      // Hydrofoil ConnectRPC QueryService (server-streaming SQL). Connect's RPC
      // paths are rooted at the fully-qualified service name.
      "/hydrofoil.query.v1.QueryService": {
        target: QUERY_URL,
        changeOrigin: true,
      },
      // MLflow web UI (served under --static-prefix /mlflow).
      "/mlflow": serviceProxy(),
      // marimo notebook editor (served under --base-url /marimo, long-lived WebSocket).
      "/marimo": serviceProxy(),
    },
  },
});
