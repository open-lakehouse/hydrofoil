// Tauri-specific fetch implementation. This is the ONLY place on the JS side that
// imports `@tauri-apps/api`; keeping it isolated here is what lets node/ui stay
// completely Tauri-free (the UI only sees a generic fetch via its registry).
//
// The implementation decides, per request, whether to handle it locally through a
// Tauri Rust command (`invoke`) or to fall through to the network. For now it
// routes NOTHING through Rust — `shouldRouteThroughRust` always returns false — so
// every request goes over HTTP exactly as in the browser. This is the inert seam:
// the wiring is in place, the Rust command exists, but no real service is bridged
// yet. Bridging one later is a matter of returning true here for selected requests
// and implementing the corresponding `proxy_request` handler in src-tauri.
import { invoke } from "@tauri-apps/api/core";

/** Response shape returned by the Rust `proxy_request` command. */
interface ProxyResponse {
  status: number;
  body: string;
  headers: [string, string][];
}

/**
 * Decide whether a request should be served by the Tauri backend instead of the
 * network. Currently always false (HTTP fallback for everything). The single
 * place to add a routing allowlist when bridging a real service.
 *
 * Takes the URL + method (not a Request) so the routing decision never has to
 * construct a Request and therefore never touches the body stream — building a
 * Request from the call args can lock the body, which breaks the fallthrough
 * fetch of a POST ("the request body is disturbed or locked").
 */
function shouldRouteThroughRust(_url: string, _method: string): boolean {
  return false;
}

export const tauriFetch: typeof globalThis.fetch = async (input, init) => {
  const url =
    typeof input === "string"
      ? input
      : input instanceof URL
        ? input.href
        : input.url;
  const method = (
    init?.method ?? (input instanceof Request ? input.method : "GET")
  ).toUpperCase();

  if (!shouldRouteThroughRust(url, method)) {
    // Fall through to the platform fetch with the ORIGINAL input/init, untouched
    // — no Request was constructed, so no body stream was disturbed.
    return globalThis.fetch(input as RequestInfo, init);
  }

  // Routing through Rust: now it's safe to consume the body — the original
  // input/init are not used past this point.
  const request = new Request(input as RequestInfo, init);
  const body = request.body ? await request.text() : "";
  const response = await invoke<ProxyResponse>("proxy_request", {
    method: request.method,
    url: request.url,
    headers: [...request.headers] as [string, string][],
    body,
  });

  return new Response(response.body, {
    status: response.status,
    headers: response.headers,
  });
};
