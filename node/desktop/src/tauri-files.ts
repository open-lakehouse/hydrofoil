// Files RPC → native `files_*` command mapping for the Tauri host.
//
// The desktop backend serves the portal Files service NOT through the ConnectRPC
// dispatcher but off the `FileStore` directly (native types, no proto framing),
// via the `files_*` commands. This module translates the UI's generated Files
// ConnectRPC client calls into those commands and back, so the UI stays unaware.
//
// Only the unary Files RPCs are mapped here. The streaming ones (UploadFile /
// DownloadFile / ListDirectoryStream) have dedicated raw/Channel command paths
// (files_upload / files_download) wired where the UI needs them; the connect
// streaming transport intentionally does not route them.
import {
  create,
  type DescMessage,
  type DescMethodUnary,
  type MessageInitShape,
  type MessageShape,
} from "@bufbuild/protobuf";
import { invoke } from "@tauri-apps/api/core";

/** Snake_case JSON returned by the Rust `files_*` commands (proto-JSON of the store types). */
interface FileMetaJson {
  path?: string;
  file_size?: number;
  last_modified?: number;
  content_type?: string;
  etag?: string;
}
interface DirMetaJson {
  path?: string;
  last_modified?: number;
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
      fileSize: BigInt(m.file_size ?? 0),
      lastModified: BigInt(m.last_modified ?? 0),
      contentType: m.content_type ?? "",
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
      lastModified: BigInt(m.last_modified ?? 0),
    }),
  );
}
