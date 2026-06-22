// Tauri-backed ConnectRPC transport.
//
// This is the desktop host's implementation of the connect `Transport` interface
// (registered via `registerTransport` before the UI bootstraps). Instead of HTTP,
// it dispatches against the in-process executors in the Tauri backend through
// `invoke`, so the UI's ConnectRPC clients (QueryService, Tags, Files) work
// unchanged — they never learn they're talking to Rust over IPC.
//
// Routing by service:
//   - Tags  (portal.tags.v1.*)      → `connect_unary` (generic dispatcher, JSON)
//   - Query (hydrofoil.query.v1.*)  → `connect_stream` (generic dispatcher; each
//                                      chunk is a raw protobuf RunQueryResponse
//                                      streamed over a Tauri Channel)
//   - Files (portal.files.v1.*)     → the native `files_*` commands (the backend
//                                      serves Files directly off the FileStore, not
//                                      through the dispatcher), see ./tauri-files.
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
  toJsonString,
} from "@bufbuild/protobuf";
import type { Transport } from "@connectrpc/connect";
import { Channel, invoke } from "@tauri-apps/api/core";
import { filesUnary } from "./tauri-files";

const TAGS_PREFIX = "portal.tags.v1.";
const QUERY_PREFIX = "hydrofoil.query.v1.";
const FILES_PREFIX = "portal.files.v1.";

/** The dispatcher service group a method belongs to (matches the Rust `service` arg). */
function serviceGroup(typeName: string): "tags" | "query" | "files" {
  if (typeName.startsWith(TAGS_PREFIX)) return "tags";
  if (typeName.startsWith(QUERY_PREFIX)) return "query";
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
      // The only Files streaming RPCs (UploadFile / DownloadFile / ListDirectory
      // Stream) are served via dedicated files_* commands, not the generic
      // streaming transport. Wire them in ./tauri-files if/when the UI needs them.
      throw new Error(
        `tauri-transport: streaming Files RPC ${method.name} not routed; use the files_* command path`,
      );
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
      path: methodPath(method),
      message: toJsonString(method.input, requestMessage),
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
