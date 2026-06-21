// Normalize errors thrown by openapi-fetch / the Unity Catalog REST API into a
// human-readable string. UC error bodies look like
// `{ "error_code": "...", "message": "..." , "details": [...] }`, but network or
// client failures surface as plain `Error`s, so we defensively handle both.

interface UcErrorBody {
  message?: string;
  error_code?: string;
  detail?: string;
}

export function parseUcError(
  error: unknown,
  fallback = "Request failed.",
): string {
  if (!error) return fallback;

  if (typeof error === "string") return error || fallback;

  if (typeof error === "object") {
    const body = error as UcErrorBody & { error?: UcErrorBody };
    // openapi-fetch puts the parsed JSON error body under `error` on the result,
    // but our query layer throws that body directly, so check both shapes.
    const candidate = body.error ?? body;
    const message = candidate.message ?? candidate.detail;
    if (message) {
      return candidate.error_code
        ? `${candidate.error_code}: ${message}`
        : message;
    }
    if (error instanceof Error && error.message) return error.message;
  }

  return fallback;
}
