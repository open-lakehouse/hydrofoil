// File access for the editor — a thin, typed wrapper over the generated portal
// `FilesService` ConnectRPC client.
//
// This is deliberately NOT a new pluggable registry: the existing transport
// seam (`clientTransport` in lib/client/registry.ts) already is the seam. On the
// web build the client speaks Connect over the network; in the Tauri desktop
// host the registered `tauriTransport` intercepts the same client and serves it
// off the in-process FileStore. So this module stays host-agnostic and never
// imports Tauri — exactly like lib/query/runner.ts does for QueryService.
//
// The store deals in bytes; the editor encodes/decodes text at its boundary, so
// binary files remain possible later. Reads/writes are buffered (not streamed)
// into the editor: Monaco needs the whole string, and a partially-decoded buffer
// would render broken intermediate states.

import { createClient } from "@connectrpc/connect";
import { FilesService } from "@/gen/portal/files/v1/svc_pb";
import { clientTransport } from "@/lib/client/registry";

/** One entry in a directory listing (a file or a subdirectory). */
export interface FileEntry {
  /** Absolute path of the entry. */
  path: string;
  /** Basename (last path segment), for display. */
  name: string;
  isDirectory: boolean;
  /** Size in bytes (0 for directories). Narrowed from the proto int64. */
  size: number;
  /** Last-modified time in epoch milliseconds. Narrowed from int64. */
  lastModified: number;
}

/** File metadata (the proto analog of an HTTP HEAD). */
export interface FileStat {
  path: string;
  size: number;
  lastModified: number;
  contentType: string;
  /** Opaque version tag; used for write-if-match conflict detection. */
  etag: string;
}

export interface ReadResult {
  bytes: Uint8Array;
  stat: FileStat;
}

export interface ListPage {
  entries: FileEntry[];
  nextPageToken?: string;
}

/** Raised when an `ifMatchEtag` write precondition fails (file changed on disk). */
export class ConflictError extends Error {
  constructor(message = "File changed on disk") {
    super(message);
    this.name = "ConflictError";
  }
}

export interface WriteOptions {
  /** Write only if the server's current etag matches (optimistic concurrency). */
  ifMatchEtag?: string;
  contentType?: string;
}

/** The file-access surface the editor depends on. */
export interface FileStore {
  listDirectory(
    path: string,
    opts?: { pageToken?: string; maxResults?: number },
  ): Promise<ListPage>;
  readFile(path: string): Promise<ReadResult>;
  writeFile(
    path: string,
    content: Uint8Array,
    opts?: WriteOptions,
  ): Promise<FileStat>;
  stat(path: string): Promise<FileStat>;
  createDir(path: string): Promise<void>;
  delete(path: string, opts?: { isDirectory?: boolean }): Promise<void>;
}

const client = createClient(FilesService, clientTransport);

/** The last path segment, used as a display name. */
function basename(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  const slash = trimmed.lastIndexOf("/");
  return slash >= 0 ? trimmed.slice(slash + 1) : trimmed;
}

/** Upload the bytes as a single-message client stream (path + content_type + chunk). */
async function* uploadFrames(
  path: string,
  content: Uint8Array,
  contentType?: string,
) {
  // The store does a (multipart) streaming put internally; one message is fine
  // for editor-sized files. Chunk here later if very large saves matter.
  yield { path, contentType, chunk: content };
}

export const connectFileStore: FileStore = {
  async listDirectory(path, opts) {
    const res = await client.listDirectoryContents({
      path,
      maxResults: opts?.maxResults,
      pageToken: opts?.pageToken,
    });
    return {
      entries: res.contents.map((e) => ({
        path: e.path,
        name: basename(e.path),
        isDirectory: e.isDirectory,
        size: Number(e.fileSize),
        lastModified: Number(e.lastModified),
      })),
      nextPageToken: res.nextPageToken,
    };
  },

  async readFile(path) {
    // Concatenate the server-stream chunks into one buffer.
    const chunks: Uint8Array[] = [];
    let total = 0;
    for await (const res of client.downloadFile({ path })) {
      chunks.push(res.chunk);
      total += res.chunk.length;
    }
    const bytes = new Uint8Array(total);
    let offset = 0;
    for (const c of chunks) {
      bytes.set(c, offset);
      offset += c.length;
    }
    const stat = await this.stat(path);
    return { bytes, stat };
  },

  async writeFile(path, content, opts) {
    const headers = opts?.ifMatchEtag
      ? new Headers({ "if-match": opts.ifMatchEtag })
      : undefined;
    try {
      const res = await client.uploadFile(
        uploadFrames(path, content, opts?.contentType),
        { headers },
      );
      return {
        path: res.path,
        size: Number(res.fileSize),
        lastModified: Date.now(),
        contentType: opts?.contentType ?? "",
        etag: res.etag,
      };
    } catch (err) {
      // A precondition failure surfaces as a Connect failed-precondition/aborted;
      // normalize to ConflictError so the editor can offer overwrite/reload.
      if (isPreconditionFailure(err)) throw new ConflictError();
      throw err;
    }
  },

  async stat(path) {
    const m = await client.getFileMetadata({ path });
    return {
      path: m.path,
      size: Number(m.fileSize),
      lastModified: Number(m.lastModified),
      contentType: m.contentType,
      etag: m.etag,
    };
  },

  async createDir(path) {
    await client.createDirectory({ path });
  },

  async delete(path, opts) {
    if (opts?.isDirectory) await client.deleteDirectory({ path });
    else await client.deleteFile({ path });
  },
};

/** Whether a thrown Connect error represents a failed etag precondition. */
function isPreconditionFailure(err: unknown): boolean {
  // ConnectError carries a numeric `code`; FailedPrecondition=9, Aborted=10.
  const code = (err as { code?: unknown })?.code;
  return code === 9 || code === 10;
}
