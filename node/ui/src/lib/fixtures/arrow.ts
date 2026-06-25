// Arrow-IPC fixtures for the query/result surfaces (DataGrid, ResultsPane, ingest
// preview). Query results travel as self-contained Arrow IPC byte streams (see
// hydrofoil.query.v1.RunQueryResponse.arrow_ipc), so the fixtures here are real
// Arrow tables serialized to IPC — the exact bytes the store decodes in the app.
//
// There is no JSON Schema to validate these against (the wire field is opaque
// `bytes`); instead fixtures.test.ts proves correctness by round-tripping each
// one back through `tableFromIPC` and asserting row/column shape. Building a
// store from these exercises the same decode path the live app uses.

import { type Table, tableFromArrays, tableToIPC } from "apache-arrow";
import { ArrowResultStore } from "@/features/data-grid";

/** Serialize an Arrow table to a self-contained IPC stream (schema + batch). */
export function tableToIpc(table: Table): Uint8Array {
  return tableToIPC(table, "stream");
}

/** Build an {@link ArrowResultStore} from one or more IPC chunks — the same
 *  shape the streaming QueryService produces, ready to hand to <DataGrid>. */
export function storeFromIpc(...chunks: Uint8Array[]): ArrowResultStore {
  const store = new ArrowResultStore();
  for (const chunk of chunks) store.append(chunk);
  return store;
}

// ── Canned result sets ─────────────────────────────────────────────────────────

/** A small, mixed-type result — the everyday case for the grid showcase. Mirrors
 *  a "top customers by revenue" query against main.sales. */
export const topCustomersTable: Table = tableFromArrays({
  customer_id: Int32Array.from([1001, 1002, 1003, 1004, 1005]),
  full_name: [
    "Ada Lovelace",
    "Alan Turing",
    "Grace Hopper",
    "Edsger Dijkstra",
    "Barbara Liskov",
  ],
  orders: Int32Array.from([42, 37, 51, 29, 64]),
  revenue_usd: Float64Array.from([12450.5, 9870.0, 15320.75, 7600.0, 20110.25]),
});

/** An empty result (valid schema, zero rows) — exercises the grid's empty state. */
export const emptyTable: Table = tableFromArrays({
  customer_id: Int32Array.from([]),
  full_name: [] as string[],
  revenue_usd: Float64Array.from([]),
});

/** A wider result with more columns — exercises horizontal scroll / sizing. */
export const tripsTable: Table = tableFromArrays({
  vendor_id: Int32Array.from([1, 2, 1, 2, 1, 2]),
  passenger_count: Int32Array.from([1, 2, 1, 3, 1, 4]),
  trip_distance: Float64Array.from([1.2, 3.8, 0.9, 7.4, 2.1, 5.5]),
  fare_amount: Float64Array.from([7.5, 14.0, 6.0, 28.5, 9.0, 22.25]),
  tip_amount: Float64Array.from([1.5, 2.0, 0.0, 5.0, 1.8, 4.0]),
  payment_type: ["card", "cash", "card", "card", "cash", "card"],
});

/** IPC byte streams for each canned table (the wire form). */
export const topCustomersIpc = tableToIpc(topCustomersTable);
export const emptyIpc = tableToIpc(emptyTable);
export const tripsIpc = tableToIpc(tripsTable);
