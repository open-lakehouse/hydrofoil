// Barrel for the curated Storybook fixtures: a single believable lakehouse world
// (UC catalog entities + governed tags + a volume file tree + Arrow result sets)
// shared across the Storybook host fakes and individual stories.
//
// See REGENERATE.md for the agent prompt that (re)authors this dataset, and
// fixtures.test.ts for the validation that keeps it honest against the generated
// JSON Schemas.

import type { ActiveEnvironment } from "@/lib/client/environments";
import { ucVolume } from "@/lib/editor/volumes";

export * as arrow from "./arrow";
export * from "./data/catalog";
export * from "./data/files";
export * from "./data/tags";

/** A mock active environment for ActiveEnvironmentProvider in stories — desktop-
 *  like (has a Home volume) plus the UC volume from the catalog fixtures. */
export const activeEnvironment: ActiveEnvironment = {
  id: "fixture-env",
  name: "Fixtures",
  capabilities: { hasHome: true },
  volumes: [
    ucVolume({ catalog: "main", schema: "sales", volume: "raw_files" }),
  ],
};
