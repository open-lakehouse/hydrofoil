// Pluggable query-execution registry — the seam that lets a host environment run
// SQL somewhere other than the ConnectRPC `QueryService`, WITHOUT the UI taking
// on any dependency on that host or its execution mechanism.
//
// This is the *execution* analogue of `lib/client/registry.ts` (which swaps the
// HTTP fetch). The default runner here speaks ConnectRPC over `clientFetch`, so
// the web build behaves exactly as a direct client call. But a host (e.g. the
// Tauri desktop shell in node/desktop) may execute queries an entirely different
// way — a Rust `invoke` command, an embedded engine — bypassing the wire. Such a
// host calls `registerQueryRunner` before the UI bootstraps to swap in its own
// implementation.
//
// The rest of the data layer (useRunQuery, the Arrow store, the grid) depends
// ONLY on the `QueryRunner` type and `queryRunner` below — never on `createClient`
// or the generated `QueryService`. This file is the single place that imports
// them, so the wire path is fully replaceable.
//
// Deliberately framework-agnostic: no Tauri, no `import.meta.env`, no globals
// beyond the default runner's transport (which itself routes through clientFetch).

import { createClient } from "@connectrpc/connect";
import { QueryService } from "@/gen/hydrofoil/query/v1/svc_pb.js";
import { queryTransport } from "./transport";

/** One streamed result chunk: a self-contained Arrow IPC stream + its row count. */
export interface QueryChunk {
  /** Arrow IPC stream bytes for one record batch (schema + batch + EOS). */
  arrowIpc: Uint8Array;
  /** Rows in this chunk, for progress display without decoding. */
  numRows: number;
}

/** A query to execute. Mirrors the proto `RunQueryRequest`. */
export interface QueryRequest {
  sql: string;
  limit?: number;
  catalog?: string;
  schema?: string;
}

/**
 * Executes a query and yields Arrow IPC chunks as they are produced. The
 * implementation is host-chosen; aborting `opts.signal` must tear the execution
 * down (the default runner drops the response body, which aborts the server-side
 * stream).
 */
export type QueryRunner = (
  req: QueryRequest,
  opts: { signal: AbortSignal },
) => AsyncIterable<QueryChunk>;

// The default web runner: hydrofoil's ConnectRPC QueryService over `clientFetch`.
// The connect client is created once and reused across runs.
const client = createClient(QueryService, queryTransport);

/** Default runner — server-streaming SQL via the ConnectRPC QueryService. */
export const connectQueryRunner: QueryRunner = async function* (req, opts) {
  const stream = client.runQuery(
    {
      sql: req.sql,
      limit: req.limit,
      catalog: req.catalog,
      schema: req.schema,
    },
    { signal: opts.signal },
  );
  for await (const chunk of stream) {
    // `numRows` is a proto uint64 (bigint on the wire); narrow for the UI.
    yield { arrowIpc: chunk.arrowIpc, numRows: Number(chunk.numRows) };
  }
};

let current: QueryRunner = connectQueryRunner;

/** Install a custom runner. Hosts call this once, before the UI bootstraps. */
export function registerQueryRunner(runner: QueryRunner): void {
  current = runner;
}

/** The runner currently in effect (the registered one, or the default). */
export function getQueryRunner(): QueryRunner {
  return current;
}

// Stable reference the data layer always calls. It dereferences `current` on
// every call (late binding), so a host can register its runner before OR after
// this module is evaluated and still take effect — no ordering constraint.
export const queryRunner: QueryRunner = (req, opts) => current(req, opts);
