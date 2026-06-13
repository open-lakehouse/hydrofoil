# S04 — unitycatalog-rs: Delta client surface (typed errors, missing endpoints, robustness)

| | |
|---|---|
| Target repo | `unitycatalog-rs` (sibling checkout, e.g. `~/code/unitycatalog-rs`) |
| Follow-through | `open-lakehouse` (remove status-code workarounds; needs that checkout) |
| Depends on | — |
| Scope | One PR in unitycatalog-rs (+ small follow-up in open-lakehouse) |
| Findings | A4 (major), A9 (major), A10-partial (minor) |

## Mission

You are working in `unitycatalog-rs`. `crates/client/src/delta_v1.rs` is the
hand-written client for the UC `/delta/v1` API; its request/response models live in
`crates/common/src/models/delta/v1.rs` and already mirror the spec faithfully
(kebab-case, tagged unions — round-trip tests pinned to spec examples exist at
`crates/common/src/models/delta/v1.rs:603-868`). This session brings the client's
*coverage and error handling* up to spec.

Spec reference (download first):

```sh
mkdir -p /tmp/uc-specs
curl -sL https://raw.githubusercontent.com/unitycatalog/unitycatalog/main/api/delta.yaml -o /tmp/uc-specs/delta.yaml
```

## Findings to fix

### A4 [major] Delta error envelope is never parsed

- `crates/client/src/error.rs:149-173` — the generic `ApiErrorBody { error_code,
  message }` parser is used by all four `DeltaV1Client` methods
  (`crates/client/src/delta_v1.rs:57, 81, 104, 129`).
- The correct model already exists, unused:
  `DeltaErrorResponse`/`DeltaErrorModel`/`DeltaErrorType` at
  `crates/common/src/models/delta/v1.rs:586-601`.
- The server in this same repo emits the spec envelope
  (`crates/server/src/rest/routers/delta/models.rs:74-81`).

Spec: delta.yaml ~lines 1538–1546 — every non-2xx `/delta/v1` body is
`{"error": {message, type, code}}` with `type ∈ DeltaErrorType`. Today every Delta API
error falls through to `UcApiError::Other { status, error_code: "", message: <raw
JSON> }`; typed predicates (`is_not_found`, `is_already_exists`) return false, and
`CommitVersionConflictException` / `UpdateRequirementConflictException` /
`ResourceExhaustedException` / `TooManyRequestsException` /
`CommitStateUnknownException` are indistinguishable except by HTTP status. Both
hydrofoil and lineage-service carry status-code workarounds because of this.

**Fix:** add a `parse_delta_error_response` path that deserializes
`DeltaErrorResponse` and maps it into a typed error variant (e.g.
`Error::Delta(DeltaErrorModel)`), wired into all `DeltaV1Client` methods. Make the
typed predicates (`is_not_found`, `is_already_exists`, plus new ones for the commit
error types) work for Delta errors. Also add the missing optional `stack:
Option<Vec<String>>` field to `DeltaErrorModel` (spec ~lines 1532–1536; currently
dropped on deserialize).

**Follow-through (open-lakehouse, if checkout available):** replace the status-code
workarounds with the typed predicates —
`crates/lineage-service/src/writer/unity.rs:56-59` and the committer's workaround at
`unitycatalog-rs/crates/datafusion/src/managed/committer.rs:289-294`. Bump the
open-lakehouse dependency pin if needed.

### A9 [major] 8 of 12 delta.yaml operations missing from `DeltaV1Client`

Implemented today: createStagingTable, createTable, loadTable, updateTable. Missing
(spec line ranges in delta.yaml): `getConfig` (53–112), `deleteTable` (294–309),
`tableExists` HEAD (310–321), `renameTable` (424–455), `getTableCredentials`
(323–357), `getStagingTableCredentials` (391–422), `getTemporaryPathCredentials`
(457–493), `reportMetrics` (359–389). Models for rename/metrics/credentials already
exist in `crates/common/src/models/delta/v1.rs` (e.g. `DeltaRenameTableRequest`,
`DeltaCredentialsResponse`, `DeltaCatalogConfig`, `DeltaReportMetricsRequest` — all
currently dead code).

Two have protocol consequences: `getConfig` is how a client negotiates
`protocol-versions` and discovers endpoint support (S05's read-path fallback wants
this); `getStagingTableCredentials` is how staging-phase credentials get refreshed
(S06 needs it).

**Fix:** implement all eight as methods on `DeltaV1Client`, following the style of
the existing four, using the existing models. Note: credentials are currently fetched
via the legacy `temporary-table-credentials` UC endpoint — do not rip that out here;
adding the Delta-native credential methods is sufficient (S06 migrates the staging
path).

### A10-partial [minor] Client robustness

1. **URL path segments are not percent-encoded** —
   `crates/client/src/delta_v1.rs:53-55, 77-79, 100-102, 125-127` build paths with
   `format!` + `Url::join`; names containing `/ ? # space ..` mis-route. Encode each
   segment (`percent-encoding` crate) before joining. Apply to the new endpoints too.
2. **`Uuid::parse_str(&table_id).unwrap()` panic** —
   `crates/client/src/temporary_credentials.rs:265`; a malformed server-supplied id
   panics the client. Return an error like the volume path does (`:338-339`).
3. **Empty session token serialized as `Some("")`** —
   `crates/object-store/src/credential.rs:204-213` (`as_aws` always sets
   `token: Some(session_token)`). Map empty → `None` to avoid signing with an empty
   `x-amz-security-token` on stores that reject it.
4. **`table_id` resolved with `unwrap_or_default()`** —
   `crates/datafusion/src/managed/append.rs:47-52`; a table missing
   `io.unitycatalog.tableId` produces an empty committer id that only fails later
   with a confusing mismatch error. Return a clear error immediately (the property is
   required by ManagedTablesSpec §table properties).

## Constraints

- Crates are unpublished: change APIs freely (e.g. error enum shape), no
  compatibility shims.
- Match the existing hand-written client style; don't introduce a generator.
- Stage your changes and propose a commit message, but do **not** run `git commit` —
  the user signs commits. Attribute AI work as "AI assisted by Isaac" if attribution
  is included.

## Verification

- New tests: (1) wiremock-style test of `DeltaV1Client` against canned
  `DeltaErrorResponse` bodies for each `DeltaErrorType`, asserting the typed mapping
  and predicates (this repo's server emits the envelope — reuse its fixtures if
  convenient); (2) round-trip/request-shape tests for each newly added endpoint
  pinned to the delta.yaml examples; (3) URL-encoding test with a name containing
  `/` and a space; (4) malformed-uuid returns error, not panic.
- `cargo test` for the client/common/object-store/datafusion crates;
  `cargo clippy` clean on touched code.
