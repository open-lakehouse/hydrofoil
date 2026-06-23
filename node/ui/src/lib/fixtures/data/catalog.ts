// A believable, internally-consistent Unity Catalog world for the Storybook
// component showcase. Hand-curated (not faker) so it reads like a real lakehouse:
// two catalogs, a few schemas, tables with realistic columns, volumes, a
// function, a model, plus metastore-level credentials and external locations.
//
// Typed against the real `@open-lakehouse/uc-client` types (the OpenAPI surface
// the UI consumes) and validated at runtime against the generated JSON Schemas in
// ../schemas/ by fixtures.test.ts. Referential integrity is intentional: every
// child's catalog_name/schema_name resolves to a parent defined here, and
// full_name is the dot-joined three-level name.
//
// Timestamps are fixed epoch-ms constants (no Date.now()) so fixtures are
// deterministic across runs and snapshots.

import type {
  CatalogInfo,
  ColumnInfo,
  CredentialInfo,
  ExternalLocationInfo,
  FunctionInfo,
  RegisteredModelInfo,
  SchemaInfo,
  TableInfo,
  VolumeInfo,
} from "@open-lakehouse/uc-client";

// A couple of fixed instants (2024-01-15 and 2024-06-20, UTC) reused throughout.
const CREATED = 1705320000000;
const UPDATED = 1718870400000;
const OWNER = "ada@lakehouse.dev";

// ── Catalogs ─────────────────────────────────────────────────────────────────

export const catalogs: CatalogInfo[] = [
  {
    name: "main",
    comment: "Primary production catalog for analytics and ML.",
    owner: OWNER,
    created_at: CREATED,
    created_by: OWNER,
    updated_at: UPDATED,
    updated_by: OWNER,
    id: "cat-main-0001",
    storage_root: "s3://lakehouse-prod/main",
    storage_location:
      "s3://lakehouse-prod/main/__unitystorage/catalogs/cat-main-0001",
    properties: { tier: "gold", team: "platform" },
  },
  {
    name: "samples",
    comment: "Read-only example datasets bundled with the lakehouse.",
    owner: OWNER,
    created_at: CREATED,
    created_by: OWNER,
    updated_at: CREATED,
    updated_by: OWNER,
    id: "cat-samples-0002",
    storage_root: "s3://lakehouse-prod/samples",
    storage_location:
      "s3://lakehouse-prod/samples/__unitystorage/catalogs/cat-samples-0002",
  },
];

// ── Schemas ──────────────────────────────────────────────────────────────────

export const schemas: SchemaInfo[] = [
  {
    name: "sales",
    catalog_name: "main",
    full_name: "main.sales",
    comment: "Curated sales facts and dimensions.",
    owner: OWNER,
    created_at: CREATED,
    created_by: OWNER,
    updated_at: UPDATED,
    updated_by: OWNER,
    schema_id: "sch-sales-0001",
  },
  {
    name: "marketing",
    catalog_name: "main",
    full_name: "main.marketing",
    comment: "Campaign and engagement tables.",
    owner: OWNER,
    created_at: CREATED,
    created_by: OWNER,
    updated_at: CREATED,
    updated_by: OWNER,
    schema_id: "sch-marketing-0002",
  },
  {
    name: "nyctaxi",
    catalog_name: "samples",
    full_name: "samples.nyctaxi",
    comment: "NYC TLC trip records (sample).",
    owner: OWNER,
    created_at: CREATED,
    created_by: OWNER,
    updated_at: CREATED,
    updated_by: OWNER,
    schema_id: "sch-nyctaxi-0003",
  },
];

// ── Tables (with columns) ─────────────────────────────────────────────────────

function col(
  position: number,
  name: string,
  typeName: NonNullable<ColumnInfo["type_name"]>,
  typeText: string,
  opts: { nullable?: boolean; comment?: string } = {},
): ColumnInfo {
  return {
    name,
    type_name: typeName,
    type_text: typeText,
    type_json: JSON.stringify({
      name,
      type: typeText,
      nullable: opts.nullable ?? true,
    }),
    position,
    nullable: opts.nullable ?? true,
    comment: opts.comment,
  };
}

export const tables: TableInfo[] = [
  {
    name: "orders",
    catalog_name: "main",
    schema_name: "sales",
    table_type: "MANAGED",
    data_source_format: "DELTA",
    storage_location: "s3://lakehouse-prod/main/sales/orders",
    comment: "One row per customer order.",
    owner: OWNER,
    created_at: CREATED,
    created_by: OWNER,
    updated_at: UPDATED,
    updated_by: OWNER,
    table_id: "tbl-orders-0001",
    columns: [
      col(0, "order_id", "LONG", "bigint", {
        nullable: false,
        comment: "Surrogate key.",
      }),
      col(1, "customer_id", "LONG", "bigint", { nullable: false }),
      col(2, "order_ts", "TIMESTAMP", "timestamp", {
        comment: "Order placed at.",
      }),
      col(3, "status", "STRING", "string"),
      col(4, "total_amount", "DECIMAL", "decimal(12,2)"),
      col(5, "currency", "STRING", "string"),
    ],
  },
  {
    name: "customers",
    catalog_name: "main",
    schema_name: "sales",
    table_type: "MANAGED",
    data_source_format: "DELTA",
    storage_location: "s3://lakehouse-prod/main/sales/customers",
    comment: "Customer dimension.",
    owner: OWNER,
    created_at: CREATED,
    created_by: OWNER,
    updated_at: CREATED,
    updated_by: OWNER,
    table_id: "tbl-customers-0002",
    columns: [
      col(0, "customer_id", "LONG", "bigint", { nullable: false }),
      col(1, "email", "STRING", "string"),
      col(2, "full_name", "STRING", "string"),
      col(3, "signed_up_on", "DATE", "date"),
      col(4, "is_active", "BOOLEAN", "boolean"),
    ],
  },
  {
    name: "campaigns",
    catalog_name: "main",
    schema_name: "marketing",
    table_type: "EXTERNAL",
    data_source_format: "PARQUET",
    storage_location: "s3://lakehouse-ext/marketing/campaigns",
    comment: "Externally-managed campaign exports.",
    owner: OWNER,
    created_at: CREATED,
    created_by: OWNER,
    updated_at: CREATED,
    updated_by: OWNER,
    table_id: "tbl-campaigns-0003",
    columns: [
      col(0, "campaign_id", "STRING", "string", { nullable: false }),
      col(1, "channel", "STRING", "string"),
      col(2, "spend_usd", "DOUBLE", "double"),
      col(3, "started_on", "DATE", "date"),
    ],
  },
  {
    name: "trips",
    catalog_name: "samples",
    schema_name: "nyctaxi",
    table_type: "MANAGED",
    data_source_format: "DELTA",
    storage_location: "s3://lakehouse-prod/samples/nyctaxi/trips",
    comment: "Yellow-cab trip records.",
    owner: OWNER,
    created_at: CREATED,
    created_by: OWNER,
    updated_at: CREATED,
    updated_by: OWNER,
    table_id: "tbl-trips-0004",
    columns: [
      col(0, "vendor_id", "INT", "int", { nullable: false }),
      col(1, "pickup_ts", "TIMESTAMP", "timestamp"),
      col(2, "dropoff_ts", "TIMESTAMP", "timestamp"),
      col(3, "passenger_count", "INT", "int"),
      col(4, "trip_distance", "DOUBLE", "double"),
      col(5, "fare_amount", "DOUBLE", "double"),
    ],
  },
];

// ── Volumes ───────────────────────────────────────────────────────────────────

export const volumes: VolumeInfo[] = [
  {
    name: "raw_files",
    catalog_name: "main",
    schema_name: "sales",
    full_name: "main.sales.raw_files",
    volume_type: "MANAGED",
    storage_location: "s3://lakehouse-prod/main/sales/__volumes/raw_files",
    comment: "Landing zone for raw order extracts.",
    owner: OWNER,
    created_at: CREATED,
    created_by: OWNER,
    updated_at: UPDATED,
    updated_by: OWNER,
    volume_id: "vol-rawfiles-0001",
  },
  {
    name: "exports",
    catalog_name: "main",
    schema_name: "marketing",
    full_name: "main.marketing.exports",
    volume_type: "EXTERNAL",
    storage_location: "s3://lakehouse-ext/marketing/exports",
    comment: "Outbound campaign exports.",
    owner: OWNER,
    created_at: CREATED,
    created_by: OWNER,
    updated_at: CREATED,
    updated_by: OWNER,
    volume_id: "vol-exports-0002",
  },
];

// ── Functions ──────────────────────────────────────────────────────────────────

export const functions: FunctionInfo[] = [
  {
    name: "net_revenue",
    catalog_name: "main",
    schema_name: "sales",
    full_name: "main.sales.net_revenue",
    comment: "Gross amount minus refunds.",
    data_type: "DECIMAL",
    full_data_type: "decimal(12,2)",
    routine_body: "SQL",
    routine_definition: "gross_amount - refund_amount",
    is_deterministic: true,
    sql_data_access: "CONTAINS_SQL",
    is_null_call: false,
    security_type: "DEFINER",
    specific_name: "net_revenue",
    parameter_style: "S",
    owner: OWNER,
    created_at: CREATED,
    created_by: OWNER,
    updated_at: CREATED,
    updated_by: OWNER,
    function_id: "fn-netrevenue-0001",
  },
];

// ── Registered models ──────────────────────────────────────────────────────────

export const models: RegisteredModelInfo[] = [
  {
    name: "churn_classifier",
    catalog_name: "main",
    schema_name: "marketing",
    full_name: "main.marketing.churn_classifier",
    storage_location:
      "s3://lakehouse-prod/main/marketing/__models/churn_classifier",
    comment: "Gradient-boosted churn model.",
    owner: OWNER,
    created_at: CREATED,
    created_by: OWNER,
    updated_at: UPDATED,
    updated_by: OWNER,
    id: "mdl-churn-0001",
  },
];

// ── Metastore-level: credentials + external locations ────────────────────────

export const credentials: CredentialInfo[] = [
  {
    name: "prod-s3-role",
    purpose: "STORAGE",
    comment: "IAM role for the production bucket.",
    owner: OWNER,
    full_name: "prod-s3-role",
    id: "cred-prods3-0001",
    created_at: CREATED,
    created_by: OWNER,
    updated_at: UPDATED,
    updated_by: OWNER,
    aws_iam_role: {
      role_arn: "arn:aws:iam::123456789012:role/lakehouse-prod",
    },
  },
];

export const externalLocations: ExternalLocationInfo[] = [
  {
    name: "marketing-ext",
    url: "s3://lakehouse-ext/marketing",
    credential_name: "prod-s3-role",
    credential_id: "cred-prods3-0001",
    comment: "External landing area for marketing exports.",
    owner: OWNER,
    id: "extloc-marketing-0001",
    created_at: CREATED,
    created_by: OWNER,
    updated_at: CREATED,
    updated_by: OWNER,
  },
];
