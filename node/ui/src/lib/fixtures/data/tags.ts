// Governed-tag fixtures (portal Tags domain) for the Storybook showcase.
//
// Typed against the generated ConnectRPC message types (camelCase). The
// validation test serializes these through protobuf `toJson` to the canonical
// snake_case wire shape before checking them against ../schemas/{tag-policy,
// entity-tag-assignment}.json.
//
// Referential integrity: every assignment's tag_key references a policy defined
// here, any value references one of that policy's allowed values, and every
// entity_name references an entity in ./catalog.ts.

import { create } from "@bufbuild/protobuf";
import {
  type EntityTagAssignment,
  EntityTagAssignmentSchema,
  type TagPolicy,
  TagPolicySchema,
} from "@/gen/portal/tags/v1/models_pb";

const CREATED = 1705320000000n;
const UPDATED = 1718870400000n;

export const tagPolicies: TagPolicy[] = [
  create(TagPolicySchema, {
    tagKey: "data_classification",
    description: "Sensitivity tier governing access and masking.",
    values: [
      { name: "public" },
      { name: "internal" },
      { name: "confidential" },
    ],
    id: "tag-classification-0001",
    createdAt: CREATED,
    updatedAt: UPDATED,
  }),
  create(TagPolicySchema, {
    tagKey: "domain",
    description: "Owning business domain.",
    values: [{ name: "sales" }, { name: "marketing" }, { name: "finance" }],
    id: "tag-domain-0002",
    createdAt: CREATED,
    updatedAt: CREATED,
  }),
  // A policy with no allowed values — free-form tag.
  create(TagPolicySchema, {
    tagKey: "cost_center",
    description: "Free-form cost-center code (no value restriction).",
    values: [],
    id: "tag-costcenter-0003",
    createdAt: CREATED,
    updatedAt: CREATED,
  }),
];

export const tagAssignments: EntityTagAssignment[] = [
  create(EntityTagAssignmentSchema, {
    entityType: "tables",
    entityName: "main.sales.orders",
    tagKey: "data_classification",
    tagValue: "confidential",
  }),
  create(EntityTagAssignmentSchema, {
    entityType: "tables",
    entityName: "main.sales.orders",
    tagKey: "domain",
    tagValue: "sales",
  }),
  create(EntityTagAssignmentSchema, {
    entityType: "schemas",
    entityName: "main.marketing",
    tagKey: "domain",
    tagValue: "marketing",
  }),
  create(EntityTagAssignmentSchema, {
    entityType: "columns",
    entityName: "main.sales.customers.email",
    tagKey: "data_classification",
    tagValue: "confidential",
  }),
  // Free-form value (cost_center policy has no allowed-value set).
  create(EntityTagAssignmentSchema, {
    entityType: "catalogs",
    entityName: "main",
    tagKey: "cost_center",
    tagValue: "CC-1042",
  }),
];
