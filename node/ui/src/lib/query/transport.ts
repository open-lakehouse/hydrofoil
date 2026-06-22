import { createConnectTransport } from "@connectrpc/connect-web";
import { clientFetch } from "@/lib/client/registry";

// ConnectRPC transport for hydrofoil's QueryService (server-streaming SQL).
//
// Connect-web speaks the Connect protocol over plain fetch — server-streaming
// works in the browser without gRPC-web or HTTP/2. The transport prepends the
// RPC path (`/hydrofoil.query.v1.QueryService/RunQuery`) to `baseUrl`, so an
// empty base resolves against the dev origin; the Vite dev proxy forwards that
// prefix to the Envoy gateway / hydrofoil (see vite.config.ts). Override the
// base with VITE_QUERY_API_URL if hydrofoil is reached on a different origin.
//
// The fetch is routed through `clientFetch` (lib/client/registry.ts), exactly
// like the Unity Catalog client (lib/api.ts), so the Tauri desktop host's
// registered fetch intercepts these calls too — the UI stays host-agnostic.
//
// `useBinaryFormat` keeps the `arrow_ipc` bytes as binary protobuf rather than
// base64-in-JSON, which matters for large result chunks.
export const queryTransport = createConnectTransport({
  baseUrl: import.meta.env.VITE_QUERY_API_URL ?? "/",
  fetch: clientFetch,
  useBinaryFormat: true,
});
