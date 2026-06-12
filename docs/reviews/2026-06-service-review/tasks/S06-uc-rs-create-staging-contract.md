# S06 — unitycatalog-rs: create/staging contract, staging credentials, non-spec fields

| | |
|---|---|
| Target repo | `unitycatalog-rs` (sibling checkout, e.g. `~/code/unitycatalog-rs`) |
| Follow-through | `open-lakehouse` ADR 0010 wording fix (needs that checkout) |
| Depends on | S04 (`getStagingTableCredentials` client method) — implement it here if S04 hasn't landed |
| Scope | One PR in unitycatalog-rs (+ doc-only edit in open-lakehouse) |
| Findings | A7 (major), A8 (major), A11 (minor) |

## Mission

You are working in `unitycatalog-rs`. Managed-table creation
(`crates/datafusion/src/managed/create.rs`) follows the staging flow: POST
`createStagingTable` → write `0.json` (put-if-absent) to the staged location → POST
`createTable`. The staging response carries a *contract*
(`required-protocol`, `required-properties`, `suggested-protocol`,
`suggested-properties`) that the client currently ignores, and the staging-phase
object store is a static S3-only construction. This session makes create honor the
contract, makes staging credentials cloud-agnostic and refreshable, and resolves the
one non-spec field the client sends.

Spec references (download first):

```sh
mkdir -p /tmp/uc-specs
curl -sL https://raw.githubusercontent.com/unitycatalog/unitycatalog/main/api/delta.yaml -o /tmp/uc-specs/delta.yaml
curl -sL https://raw.githubusercontent.com/unitycatalog/unitycatalog/main/spec/protocols/ManagedTablesSpec.md -o /tmp/uc-specs/ManagedTablesSpec.md
```

## Findings to fix

### A7 [major] Staging contract ignored; disk↔catalog metadata divergence

- `crates/datafusion/src/managed/create.rs:121-122` —
  `get_required_properties_for_disk` is hardcoded; the deserialized
  `DeltaStagingTableResponse.required_protocol / required_properties /
  suggested_protocol / suggested_properties` fields
  (`crates/common/src/models/delta/v1.rs:342-356`) are never read.
- `create.rs:180-194` vs `:208-213` — `delta.checkpointPolicy=v2` is deliberately
  NOT written to the Delta log but IS added to the UC `createTable` properties
  payload, so catalog and `_delta_log` metadata disagree.

Spec: delta.yaml ~lines 651–678 — `required-protocol`: "client must write an initial
commit with at least this protocol"; `required-properties`: "table properties that
must be set". ManagedTablesSpec line 60: `0.json` should include all table features
and properties required by the spec; line 576: metadata posted to the catalog is the
desired final state and must match the Delta log.

**Fix:**
1. Derive the `0.json` protocol and properties from the staging response: union the
   hardcoded baseline with `required-protocol`/`required-properties`; intersect
   `suggested-*` with kernel-supported features. Fail fast with a clear error naming
   any *required* feature the kernel cannot honor.
2. Make disk and catalog identical for `delta.checkpointPolicy`: either set it in the
   kernel-created `0.json` metadata (the kernel already writes the `v2Checkpoint`
   feature — check whether the buoyant kernel's `create_table` accepts table
   properties; if it genuinely cannot, stop sending the property to UC and rely on
   the feature flag). Whichever direction, the two sources must agree.

### A8 [major] Staging object store: S3-only, static, expiry ignored

- `crates/datafusion/src/managed/create.rs:317-355` (`build_staging_store`) — honors
  only `s3.*` config keys (creation impossible on Azure/GCS); `expiration_time_ms`
  never consulted (an expired token mid-create yields an opaque storage error);
  credential selection is first-prefix-match-else-first instead of the spec's
  longest-prefix rule; region only from `AWS_REGION` env.

Spec: delta.yaml `DeltaStorageCredential` (~lines 543–588) — multi-cloud config keys,
`expiration-time-ms`, "clients should choose the most specific prefix (longest
prefix)".

**Fix:** rebuild the staging store on the existing refresh machinery:
`UCCredentialProvider` in `crates/object-store/src/credential.rs` already does vend +
`TokenCache` refresh for table credentials — reuse it with a staging-credentials
re-vend via `DeltaV1Client::get_staging_table_credentials` (from S04; implement that
one method here if S04 hasn't landed, spec ~lines 391–422). Support Azure/GCS config
keys, longest-prefix credential selection, and honor `expiration-time-ms`.

### A11 [minor] `data_source_format` in `/delta/v1` createTable is not a spec field

- `crates/common/src/models/delta/v1.rs:365-372` — `DeltaCreateTableRequest` carries
  `data_source_format: Option<DeltaDataSourceFormat>` (skip-if-none); the doc comment
  correctly explains it: required by the pinned Java server image
  (`ghcr.io/roeap/unitycatalog:v0.0.0-dev-3`) whose createTable handler reads it from
  the body, while newer server code hardcodes DELTA and ignores it, and delta.yaml's
  `DeltaCreateTableRequest` schema (~lines 687–738) omits the field entirely.
- `crates/datafusion/src/managed/create.rs:160` always sends `Some(Delta)`.
- The field is legitimately required in the *legacy route-level tables API*
  (ManagedTablesSpec ~lines 177, 343) — that is where the confusion comes from; the
  legacy-API usages in `crates/client/src/codegen/` are correct and out of scope.

A field-by-field check of all `/delta/v1` request models against delta.yaml found
this to be the **only** non-spec field sent.

**Fix:**
1. Check which server image the consuming stack currently pins
   (open-lakehouse `environments/`): if it no longer requires the field in the
   `/delta/v1` body, remove the field from the model and the `create.rs` call site.
   If it still does, keep it optional-with-comment and add a tracking note (issue or
   TODO) to remove it when the pin moves; consider reporting the spec/impl drift
   upstream.
2. **open-lakehouse doc fix:** `docs/adr/0010-catalog-managed-table-writes.md` line 56
   states createTable requests "must carry `data-source-format: \"DELTA\"`" — reword
   to attribute the requirement to the pinned server implementation, not the Delta
   API spec, citing the delta.yaml schema omission.

## Constraints

- Crates are unpublished: change APIs freely, no compatibility shims.
- Fail-fast errors over silent best-guesses for the create flow (a wrongly-created
  managed table is expensive to clean up).
- Stage changes per repo and propose commit messages, but do **not** run
  `git commit` — the user signs commits. Attribute AI work as "AI assisted by Isaac"
  if attribution is included.

## Verification

- Unit tests: (1) staging response with extra `required-protocol` features /
  `required-properties` lands them in `0.json`; (2) a required feature the kernel
  can't honor produces the fail-fast error; (3) longest-prefix credential selection;
  (4) Azure/GCS key handling (construct-only test is fine); (5) catalog and disk
  property payloads are identical for `checkpointPolicy`.
- The `#[ignore]` live create test in `crates/datafusion/tests/managed_table.rs`
  still compiles; run against a live stack if available.
- `cargo test` + `cargo clippy` clean on touched crates.
