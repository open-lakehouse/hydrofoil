// File-tree fixtures (portal Files domain) for the Storybook showcase: a small
// realistic volume tree with directories and files, plus standalone file/dir
// metadata. Typed against the generated ConnectRPC message types (camelCase);
// the validation test serializes them through protobuf `toJson` to the canonical
// snake_case wire shape before checking against ../schemas/{file-metadata,
// directory-entry,directory-metadata}.json.
//
// The tree is rooted at the Home volume (/home) the desktop host serves, and at
// the UC volume main.sales.raw_files defined in ./catalog.ts, so the fixtures
// line up with the catalog world.

import { create } from "@bufbuild/protobuf";
import {
  type DirectoryEntry,
  DirectoryEntrySchema,
  type DirectoryMetadata,
  DirectoryMetadataSchema,
  type FileMetadata,
  FileMetadataSchema,
} from "@/gen/portal/files/v1/svc_pb";

const MODIFIED = 1718870400000n;

/** Top-level entries under the Home volume (/home). */
export const homeEntries: DirectoryEntry[] = [
  create(DirectoryEntrySchema, {
    path: "/home/notebooks",
    isDirectory: true,
    fileSize: 0n,
    lastModified: MODIFIED,
  }),
  create(DirectoryEntrySchema, {
    path: "/home/queries",
    isDirectory: true,
    fileSize: 0n,
    lastModified: MODIFIED,
  }),
  create(DirectoryEntrySchema, {
    path: "/home/README.md",
    isDirectory: false,
    fileSize: 412n,
    lastModified: MODIFIED,
  }),
];

/** Contents of /home/queries. */
export const queryEntries: DirectoryEntry[] = [
  create(DirectoryEntrySchema, {
    path: "/home/queries/top_customers.sql",
    isDirectory: false,
    fileSize: 287n,
    lastModified: MODIFIED,
  }),
  create(DirectoryEntrySchema, {
    path: "/home/queries/daily_orders.sql",
    isDirectory: false,
    fileSize: 533n,
    lastModified: MODIFIED,
  }),
];

/** Contents of the UC volume main.sales.raw_files. */
export const rawFilesEntries: DirectoryEntry[] = [
  create(DirectoryEntrySchema, {
    path: "/Volumes/main/sales/raw_files/2024-06-20",
    isDirectory: true,
    fileSize: 0n,
    lastModified: MODIFIED,
  }),
  create(DirectoryEntrySchema, {
    path: "/Volumes/main/sales/raw_files/orders_2024-06-20.parquet",
    isDirectory: false,
    fileSize: 1048576n,
    lastModified: MODIFIED,
  }),
];

/** Standalone metadata for a single file. */
export const fileMetadata: FileMetadata = create(FileMetadataSchema, {
  path: "/home/queries/top_customers.sql",
  fileSize: 287n,
  lastModified: MODIFIED,
  contentType: "application/sql",
  etag: '"d41d8cd98f00b204e9800998ecf8427e"',
});

/** Standalone metadata for a directory. */
export const directoryMetadata: DirectoryMetadata = create(
  DirectoryMetadataSchema,
  {
    path: "/home/queries",
    lastModified: MODIFIED,
  },
);
