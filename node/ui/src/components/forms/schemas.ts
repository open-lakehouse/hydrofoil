// Typed access to the JSON Schemas generated from the Unity Catalog protobufs
// (see uc-client/scripts/gen-form-schemas.mjs). Centralizing the imports keeps
// the `as RJSFSchema` casts in one place.
import type { RJSFSchema } from "@rjsf/utils";

import createCatalog from "@/lib/forms/schemas/create-catalog.json";
import createCredential from "@/lib/forms/schemas/create-credential.json";
import createExternalLocation from "@/lib/forms/schemas/create-external-location.json";
import createSchema from "@/lib/forms/schemas/create-schema.json";
import updateCredential from "@/lib/forms/schemas/update-credential.json";
import updateExternalLocation from "@/lib/forms/schemas/update-external-location.json";

export const formSchemas = {
  createCatalog: createCatalog as RJSFSchema,
  createSchema: createSchema as RJSFSchema,
  createCredential: createCredential as RJSFSchema,
  updateCredential: updateCredential as RJSFSchema,
  createExternalLocation: createExternalLocation as RJSFSchema,
  updateExternalLocation: updateExternalLocation as RJSFSchema,
} as const;

/**
 * Deep clone a generated schema so a dialog can tailor it at runtime (hide
 * fields, inject dynamic enums, tighten `required`) without mutating the shared
 * imported object.
 */
export function cloneSchema(schema: RJSFSchema): RJSFSchema {
  return structuredClone(schema);
}
