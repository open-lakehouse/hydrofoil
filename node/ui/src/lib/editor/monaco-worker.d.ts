// monaco-editor 0.55 ships these worker-bootstrap modules without `.d.ts` files.
// Declare the exports we use from each.

// The blessed worker bootstrap (used by monaco's own language workers). Performs
// the two-message handshake and wires `createClient(ctx, createData)`'s result as
// the worker's foreign module.
declare module "monaco-editor/esm/vs/common/initialize" {
  import type { worker } from "monaco-editor";
  export function initialize<T, D>(
    createClient: (ctx: worker.IWorkerContext, createData: D) => T,
  ): void;
  export function isWorkerInitialized(): boolean;
}
