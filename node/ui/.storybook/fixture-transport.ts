// An in-memory ConnectRPC transport that serves the hydrofoil + portal services
// from the curated fixtures, registered via `registerTransport` so the UI's RPC
// clients (QueryService, Ingest, Tags, Files) work in Storybook with no backend.
//
// Built with `createRouterTransport` (the official connect-es testing tool):
// implementations are written against the generated schema types and run nearly
// the same code path a real server would — including server-streaming, which the
// query/preview results need. This is the same "register a fake transport at the
// seam" pattern node/desktop uses for tauriTransport, but in-process.

import { createRouterTransport, type Transport } from "@connectrpc/connect";

import { IngestService } from "@/gen/hydrofoil/ingest/v1/svc_pb";
import { QueryService } from "@/gen/hydrofoil/query/v1/svc_pb";
import { FilesService } from "@/gen/portal/files/v1/svc_pb";
import {
  EntityTagAssignmentsService,
  TagPoliciesService,
} from "@/gen/portal/tags/v1/svc_pb";
import {
  arrow,
  fileMetadata,
  homeEntries,
  queryEntries,
  rawFilesEntries,
  tagAssignments,
  tagPolicies,
} from "@/lib/fixtures";

// Directory contents keyed by path, so listDirectoryContents serves the right
// children for the fixture tree.
const DIRECTORIES: Record<string, typeof homeEntries> = {
  "/home": homeEntries,
  "/home/queries": queryEntries,
  "/Volumes/main/sales/raw_files": rawFilesEntries,
};

export const fixtureTransport: Transport = createRouterTransport(
  ({ service }) => {
    // Query: stream the canned Arrow result as one IPC chunk. A trivial dialect
    // sniff picks the wider "trips" result for taxi queries, else top-customers.
    service(QueryService, {
      async *runQuery(req) {
        const sql = (req.sql ?? "").toLowerCase();
        const ipc = sql.includes("trip")
          ? arrow.tripsIpc
          : arrow.topCustomersIpc;
        const table = arrow.storeFromIpc(ipc);
        yield { arrowIpc: ipc, numRows: BigInt(table.rowCount) };
      },
    });

    // Ingest: preview returns the schema + a sample as Arrow IPC.
    service(IngestService, {
      previewFile() {
        return {
          schemaIpc: arrow.topCustomersIpc,
          sampleIpc: arrow.topCustomersIpc,
          totalRowsEstimate: 5n,
        };
      },
      // Client-streaming: drain the request stream, then return one response.
      async ingestTable(reqs) {
        for await (const _ of reqs) {
          // consume the streamed Arrow chunks
        }
        return {
          rowsWritten: 5n,
          qualifiedName: "main.sales.imported",
          created: true,
        };
      },
    });

    // Tags.
    service(TagPoliciesService, {
      listTagPolicies() {
        return { tagPolicies, nextPageToken: undefined };
      },
      getTagPolicy(req) {
        const policy = tagPolicies.find((p) => p.tagKey === req.tagKey);
        if (!policy) throw new Error(`tag policy not found: ${req.tagKey}`);
        return policy;
      },
    });

    service(EntityTagAssignmentsService, {
      listEntityTagAssignments(req) {
        const assignments = tagAssignments.filter(
          (a) =>
            (!req.entityType || a.entityType === req.entityType) &&
            (!req.entityName || a.entityName === req.entityName),
        );
        return { tagAssignments: assignments, nextPageToken: undefined };
      },
    });

    // Files.
    service(FilesService, {
      listDirectoryContents(req) {
        const entries = DIRECTORIES[req.path] ?? [];
        return { contents: entries, nextPageToken: undefined };
      },
      getFileMetadata() {
        return fileMetadata;
      },
    });
  },
);
