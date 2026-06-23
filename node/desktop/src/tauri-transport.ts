// Tauri-backed ConnectRPC transport.
//
// This is the desktop host's implementation of the connect `Transport` interface
// (registered via `registerTransport` before the UI bootstraps). Instead of HTTP,
// it dispatches against the in-process executors in the Tauri backend through
// `invoke`, so the UI's ConnectRPC clients (QueryService, Tags, Files) work
// unchanged — they never learn they're talking to Rust over IPC.
//
// Routing by service:
//   - Tags   (portal.tags.v1.*)      → `connect_unary` (generic dispatcher, JSON)
//   - Query  (hydrofoil.query.v1.*)  → `connect_stream` (generic dispatcher; each
//                                       chunk is a raw protobuf RunQueryResponse
//                                       streamed over a Tauri Channel)
//   - Ingest (hydrofoil.ingest.v1.*) → `connect_unary_proto` (PreviewFile: proto
//                                       in/out, carrying Arrow IPC) and
//                                       `query_ingest` (IngestTable: client-stream,
//                                       request frames sent as a list of proto bytes)
//   - Files  (portal.files.v1.*)     → the native `files_*` commands (the backend
//                                       serves Files directly off the FileStore, not
//                                       through the dispatcher), see ./tauri-files.
//
// Alongside `@tauri-apps`, this file is one of the only places that imports Tauri;
// keeping it in node/desktop is what lets node/ui stay Tauri-free.
import {
  create,
  type DescMessage,
  type DescMethodStreaming,
  type DescMethodUnary,
  fromBinary,
  fromJsonString,
  type MessageInitShape,
  type MessageShape,
  toBinary,
  toJsonString,
} from "@bufbuild/protobuf";
import type { Transport } from "@connectrpc/connect";
import { Channel, invoke } from "@tauri-apps/api/core";
import { filesStream, filesUnary } from "./tauri-files";

const TAGS_PREFIX = "portal.tags.v1.";
const QUERY_PREFIX = "hydrofoil.query.v1.";
const INGEST_PREFIX = "hydrofoil.ingest.v1.";
const FILES_PREFIX = "portal.files.v1.";

/** The dispatcher service group a method belongs to (matches the Rust `service` arg). */
function serviceGroup(typeName: string): "tags" | "query" | "ingest" | "files" {
  if (typeName.startsWith(TAGS_PREFIX)) return "tags";
  if (typeName.startsWith(QUERY_PREFIX)) return "query";
  if (typeName.startsWith(INGEST_PREFIX)) return "ingest";
  if (typeName.startsWith(FILES_PREFIX)) return "files";
  throw new Error(`tauri-transport: no route for service ${typeName}`);
}

/** The Connect method path, e.g. `portal.tags.v1.TagPoliciesService/ListTagPolicies`. */
function methodPath(method: {
  parent: { typeName: string };
  name: string;
}): string {
  return `${method.parent.typeName}/${method.name}`;
}

/** Flatten a `HeadersInit` to the `[name, value][]` shape the Rust commands take. */
function headerPairs(header: HeadersInit | undefined): [string, string][] {
  if (!header) return [];
  return [...new Headers(header).entries()];
}

/** Turn a stream of init-shapes into a stream of full messages. */
async function* materialize<I extends DescMessage>(
  schema: I,
  input: AsyncIterable<MessageInitShape<I>>,
): AsyncIterable<MessageShape<I>> {
  for await (const msg of input) yield create(schema, msg);
}

export const tauriTransport: Transport = {
  async unary<I extends DescMessage, O extends DescMessage>(
    method: DescMethodUnary<I, O>,
    _signal: AbortSignal | undefined,
    _timeoutMs: number | undefined,
    header: HeadersInit | undefined,
    input: MessageInitShape<I>,
  ) {
    const group = serviceGroup(method.parent.typeName);
    const message = create(method.input, input);

    let out: MessageShape<O>;
    if (group === "files") {
      // Files bypasses the dispatcher: the desktop backend serves it off the
      // FileStore directly via native files_* commands.
      out = await filesUnary(method, message);
    } else if (group === "ingest") {
      // IngestService unary (PreviewFile): proto in / proto out, since the
      // response carries Arrow IPC bytes that JSON would base64-bloat.
      const responseBytes = await invoke<number[]>("connect_unary_proto", {
        service: group,
        path: methodPath(method),
        message: Array.from(toBinary(method.input, message)),
        headers: headerPairs(header),
      });
      out = fromBinary(method.output, new Uint8Array(responseBytes));
    } else {
      // Tags (and any future unary RPC): JSON in, JSON out through the generic
      // dispatcher command.
      const responseJson = await invoke<string>("connect_unary", {
        service: group,
        path: methodPath(method),
        message: toJsonString(method.input, message),
        headers: headerPairs(header),
      });
      out = fromJsonString(method.output, responseJson);
    }

    return {
      stream: false as const,
      service: method.parent,
      method,
      header: new Headers(),
      message: out,
      trailer: new Headers(),
    };
  },

  async stream<I extends DescMessage, O extends DescMessage>(
    method: DescMethodStreaming<I, O>,
    signal: AbortSignal | undefined,
    _timeoutMs: number | undefined,
    header: HeadersInit | undefined,
    input: AsyncIterable<MessageInitShape<I>>,
  ) {
    const group = serviceGroup(method.parent.typeName);
    if (group === "files") {
      // Files streaming RPCs (DownloadFile / UploadFile) are served off the
      // FileStore via the native files_download / files_upload commands. The
      // bridge in ./tauri-files materializes the request frames and adapts the
      // native command to the connect output-message stream.
      const messages = filesStream(method, materialize(method.input, input));
      return {
        stream: true as const,
        service: method.parent,
        method,
        header: new Headers(),
        message: messages,
        trailer: new Headers(),
      };
    }

    if (group === "ingest") {
      // IngestService client-streaming (IngestTable): drain every request frame,
      // proto-encode each, and hand the whole list to the `query_ingest` command,
      // which builds a RequestStream and returns the single response. Mirrors the
      // files_upload client-stream pattern; on desktop the bulk data rides via the
      // first frame's source_path, so the frame list is small.
      const frames: number[][] = [];
      for await (const msg of input) {
        frames.push(
          Array.from(toBinary(method.input, create(method.input, msg))),
        );
      }
      const responseBytes = await invoke<number[]>("query_ingest", {
        service: group,
        path: methodPath(method),
        frames,
        headers: headerPairs(header),
      });
      const response = fromBinary(method.output, new Uint8Array(responseBytes));
      async function* single(): AsyncIterable<MessageShape<O>> {
        yield response;
      }
      return {
        stream: true as const,
        service: method.parent,
        method,
        header: new Headers(),
        message: single(),
        trailer: new Headers(),
      };
    }

    // Server-streaming only (QueryService.RunQuery): take the single request
    // message, then receive raw protobuf response frames over a Tauri Channel.
    let first: MessageInitShape<I> | undefined;
    for await (const msg of input) {
      first = msg;
      break;
    }
    const requestMessage = create(
      method.input,
      first ?? ({} as unknown as MessageInitShape<I>),
    );

    const channel = new Channel<ArrayBuffer>();
    // Bridge the Channel (push) into an async iterable (pull) the connect client
    // consumes. The invoke promise resolving == end-of-stream (Channel delivery
    // is ordered and the command returns only after every send).
    const queue: ArrayBuffer[] = [];
    let notify: (() => void) | undefined;
    let done = false;
    let failure: unknown;
    channel.onmessage = (buf) => {
      queue.push(buf);
      notify?.();
    };

    const completion = invoke<void>("connect_stream", {
      service: group,
      // Proto-encoded request bytes: the streaming dispatcher uses one codec for
      // both request and the binary (Arrow) response, so the request must be
      // Proto, not JSON. The Rust side takes a Vec<u8>.
      message: Array.from(toBinary(method.input, requestMessage)),
      path: methodPath(method),
      headers: headerPairs(header),
      onChunk: channel,
    })
      .then(() => {
        done = true;
        notify?.();
      })
      .catch((err) => {
        failure = err;
        done = true;
        notify?.();
      });

    async function* messages(): AsyncIterable<MessageShape<O>> {
      while (true) {
        if (signal?.aborted) throw signal.reason ?? new Error("aborted");
        while (queue.length > 0) {
          const buf = queue.shift() as ArrayBuffer;
          yield fromBinary(method.output, new Uint8Array(buf));
        }
        if (done) {
          await completion; // surface a command error, if any
          if (failure) throw failure;
          return;
        }
        await new Promise<void>((resolve) => {
          notify = resolve;
        });
        notify = undefined;
      }
    }

    return {
      stream: true as const,
      service: method.parent,
      method,
      header: new Headers(),
      message: messages(),
      trailer: new Headers(),
    };
  },
};
