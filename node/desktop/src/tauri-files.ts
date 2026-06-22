// Files RPC → native `files_*` command mapping for the Tauri host.
//
// The desktop backend serves the portal Files service NOT through the ConnectRPC
// dispatcher but off the `FileStore` directly (native types, no proto framing),
// via the `files_*` commands. This module translates the UI's generated Files
// ConnectRPC client calls into those commands and back, so the UI stays unaware.
//
// The unary Files RPCs are mapped in `filesUnary`. The editor also needs the two
// content-transfer streaming RPCs (DownloadFile / UploadFile); `filesStream`
// bridges those onto the native `files_download` / `files_upload` commands so the
// UI's generated Files client works unchanged on desktop. (ListDirectoryStream is
// still unrouted — the UI uses the unary paged ListDirectoryContents instead.)
import {
  create,
  type DescMessage,
  type DescMethodStreaming,
  type DescMethodUnary,
  type MessageInitShape,
  type MessageShape,
} from "@bufbuild/protobuf";
import { Channel, invoke } from "@tauri-apps/api/core";

// The store metadata types (FileMetadata, DirectoryMetadata) are buffa-generated
// and serialize as proto-JSON, i.e. camelCase (`fileSize`, `lastModified`,
// `contentType`, …). The list envelope, by contrast, is hand-built snake_case in
// the Rust `files_list` command (`{ contents, next_page_token }`).
interface FileMetaJson {
  path?: string;
  fileSize?: number;
  lastModified?: number;
  contentType?: string;
  etag?: string;
}
interface DirMetaJson {
  path?: string;
  lastModified?: number;
}
interface ListJson {
  contents?: unknown[];
  next_page_token?: string | null;
}

/** A request message carrying a `path` field (the common Files request shape). */
interface PathRequest {
  path: string;
  maxResults?: number;
  pageToken?: string;
}

/**
 * Dispatch a unary Files RPC to its native command and build the typed response.
 * Throws for an RPC with no native mapping (a programming error — the streaming
 * RPCs must use the dedicated command paths, not this transport).
 */
export async function filesUnary<I extends DescMessage, O extends DescMessage>(
  method: DescMethodUnary<I, O>,
  message: MessageShape<I>,
): Promise<MessageShape<O>> {
  const req = message as unknown as PathRequest;
  const out = method.output;

  switch (method.name) {
    case "GetFileMetadata": {
      const m = await invoke<FileMetaJson>("files_stat", { path: req.path });
      return fileMeta(out, m);
    }
    case "CreateDirectory": {
      const m = await invoke<DirMetaJson>("files_create_dir", {
        path: req.path,
      });
      return dirMeta(out, m);
    }
    case "GetDirectoryMetadata": {
      // No dedicated stat-dir command yet; create_directory is idempotent for the
      // metadata shape. Add a files_stat_dir command if a pure stat is needed.
      const m = await invoke<DirMetaJson>("files_create_dir", {
        path: req.path,
      });
      return dirMeta(out, m);
    }
    case "DeleteFile": {
      await invoke<void>("files_delete", { path: req.path });
      return create(out, init(out, {}));
    }
    case "DeleteDirectory": {
      await invoke<void>("files_delete_dir", { path: req.path });
      return create(out, init(out, {}));
    }
    case "ListDirectoryContents": {
      const res = await invoke<ListJson>("files_list", {
        path: req.path,
        maxResults: req.maxResults,
        pageToken: req.pageToken,
      });
      return create(
        out,
        init(out, {
          contents: res.contents ?? [],
          nextPageToken: res.next_page_token ?? undefined,
        }),
      );
    }
    default:
      throw new Error(`tauri-files: unmapped Files RPC ${method.name}`);
  }
}

// Dynamic message construction: the response shape is known at runtime (it comes
// from a native command's JSON), not to the type system through the generic `O`.
// Funnel the init object through a single `unknown` cast so the call sites stay
// readable.
function init<O extends DescMessage>(
  _out: O,
  obj: object,
): MessageInitShape<O> {
  return obj as unknown as MessageInitShape<O>;
}

function fileMeta<O extends DescMessage>(
  out: O,
  m: FileMetaJson,
): MessageShape<O> {
  return create(
    out,
    init(out, {
      path: m.path ?? "",
      fileSize: BigInt(m.fileSize ?? 0),
      lastModified: BigInt(m.lastModified ?? 0),
      contentType: m.contentType ?? "",
      etag: m.etag ?? "",
    }),
  );
}

function dirMeta<O extends DescMessage>(
  out: O,
  m: DirMetaJson,
): MessageShape<O> {
  return create(
    out,
    init(out, {
      path: m.path ?? "",
      lastModified: BigInt(m.lastModified ?? 0),
    }),
  );
}

/** A DownloadFile request message (the fields we read off it). */
interface DownloadRequest {
  path: string;
  offset?: bigint;
  length?: bigint;
}

/** An UploadFile request message (one frame of the client stream). */
interface UploadFrame {
  path: string;
  contentType?: string;
  chunk: Uint8Array;
}

/**
 * Dispatch a streaming Files RPC to its native command and return an async
 * iterable of the connect output messages.
 *
 *  - DownloadFile (server-stream): invokes `files_download`, which pushes raw
 *    byte chunks over a Channel; each becomes one `DownloadFileResponse { chunk }`.
 *  - UploadFile (client-stream): drains the request frames (first carries path +
 *    content_type, all carry chunk bytes), concatenates them, and invokes
 *    `files_upload` with the bytes as the raw invoke body (path/content_type ride
 *    along as headers, per Tauri's raw-body command convention); yields one
 *    `UploadFileResponse` built from the returned metadata.
 */
export async function* filesStream<
  I extends DescMessage,
  O extends DescMessage,
>(
  method: DescMethodStreaming<I, O>,
  input: AsyncIterable<MessageShape<I>>,
): AsyncGenerator<MessageShape<O>> {
  const out = method.output;

  if (method.name === "DownloadFile") {
    // The single server-stream request message.
    let req: DownloadRequest | undefined;
    for await (const msg of input) {
      req = msg as unknown as DownloadRequest;
      break;
    }
    if (!req) throw new Error("filesStream: DownloadFile missing request");

    // Bridge the Channel (push) into this generator (pull), mirroring the
    // server-streaming bridge in tauri-transport.ts.
    const channel = new Channel<ArrayBuffer>();
    const queue: ArrayBuffer[] = [];
    let notify: (() => void) | undefined;
    let done = false;
    let failure: unknown;
    channel.onmessage = (buf) => {
      queue.push(buf);
      notify?.();
    };
    const completion = invoke<void>("files_download", {
      path: req.path,
      offset: req.offset !== undefined ? Number(req.offset) : undefined,
      length: req.length !== undefined ? Number(req.length) : undefined,
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

    while (true) {
      while (queue.length > 0) {
        const buf = queue.shift() as ArrayBuffer;
        yield create(out, init(out, { chunk: new Uint8Array(buf) }));
      }
      if (done) {
        await completion;
        if (failure) throw failure;
        return;
      }
      await new Promise<void>((resolve) => {
        notify = resolve;
      });
      notify = undefined;
    }
  }

  if (method.name === "UploadFile") {
    let path = "";
    let contentType: string | undefined;
    const parts: Uint8Array[] = [];
    let total = 0;
    let first = true;
    for await (const msg of input) {
      const frame = msg as unknown as UploadFrame;
      if (first) {
        path = frame.path;
        contentType = frame.contentType;
        first = false;
      }
      if (frame.chunk?.length) {
        parts.push(frame.chunk);
        total += frame.chunk.length;
      }
    }
    const body = new Uint8Array(total);
    let offset = 0;
    for (const p of parts) {
      body.set(p, offset);
      offset += p.length;
    }

    // Raw body carries the bytes; the scalar args ride as request headers, keyed
    // by the Rust command's argument names (snake_case), per Tauri v2's raw-body
    // command convention.
    const headers: Record<string, string> = { path };
    if (contentType) headers.content_type = contentType;
    const m = await invoke<FileMetaJson>("files_upload", body, { headers });

    yield create(
      out,
      init(out, {
        path: m.path ?? path,
        fileSize: BigInt(m.fileSize ?? total),
        etag: m.etag ?? "",
      }),
    );
    return;
  }

  throw new Error(`tauri-files: unmapped streaming Files RPC ${method.name}`);
}
