// IngestService client wrapper — the import page's view of the host engine.
//
// Like `lib/query/runner.ts`, this is the single place that imports the generated
// `IngestService` + transport, so the page components stay decoupled from the
// wire. On desktop the registered `tauriTransport` routes these calls to the
// in-process engine (PreviewFile → `connect_unary_proto`, IngestTable →
// `query_ingest`); on web they would speak Connect over fetch.

import { createClient } from "@connectrpc/connect";
import { IngestService } from "@/gen/hydrofoil/ingest/v1/svc_pb.js";
import { clientTransport } from "@/lib/client/registry";

const client = createClient(IngestService, clientTransport);

/** A previewed file: inferred schema + a row sample, both Arrow IPC. */
export interface FilePreview {
  /** Arrow IPC stream of the inferred schema (no batches). */
  schemaIpc: Uint8Array;
  /** Arrow IPC stream of the sample rows (schema + batch + EOS). */
  sampleIpc: Uint8Array;
  /** Best-effort source row count (0 when unknown). */
  totalRows: number;
}

/** Parse a local file and return its inferred schema + a capped row sample. */
export async function previewFile(
  path: string,
  sampleRows?: number,
): Promise<FilePreview> {
  const res = await client.previewFile({ path, sampleRows });
  return {
    schemaIpc: res.schemaIpc,
    sampleIpc: res.sampleIpc,
    totalRows: Number(res.totalRowsEstimate),
  };
}

/** Target for an ingest: the managed table to create/append and its schema. */
export interface IngestTarget {
  catalog: string;
  schema: string;
  table: string;
  /** The user-confirmed Arrow schema, as an Arrow IPC stream. */
  targetSchemaIpc: Uint8Array;
  /** Local file path the host reads the data from (desktop path). */
  sourcePath: string;
  createIfMissing: boolean;
}

/** Result of an ingest. */
export interface IngestResult {
  rowsWritten: number;
  qualifiedName: string;
  created: boolean;
}

/**
 * Create the managed table (if needed) and ingest the file. On desktop the data
 * is read by the host from `sourcePath`, so a single request frame carrying the
 * target + schema is enough; the streaming RPC shape is retained for a future web
 * client that would stream Arrow IPC chunks instead.
 */
export async function ingestTable(target: IngestTarget): Promise<IngestResult> {
  async function* frames() {
    yield {
      catalog: target.catalog,
      schema: target.schema,
      table: target.table,
      targetSchemaIpc: target.targetSchemaIpc,
      sourcePath: target.sourcePath,
      createIfMissing: target.createIfMissing,
    };
  }
  const res = await client.ingestTable(frames());
  return {
    rowsWritten: Number(res.rowsWritten),
    qualifiedName: res.qualifiedName,
    created: res.created,
  };
}
