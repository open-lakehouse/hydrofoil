# DuckDB ↔ Unity Catalog managed Delta tables (findings)

> Status: **blocked — root cause confirmed, not yet fixed.** DuckDB can read and
> append to UC *managed* (catalog-managed / coordinated-commits) Delta tables in
> principle, but against our `roeap/unitycatalog` fork every commits call fails
> with `400 Bad Request`. The cause is a content-type mismatch in DuckDB's HTTP
> client, proven on the wire (below). This documents the evidence and the
> mitigation options so we can pick one deliberately.

## Goal

A notebook (`notebooks/uc_duckdb.py`, `just uc-duckdb`) that uses DuckDB to read a
UC **managed** Delta table and `INSERT` more rows into it — the same live AWS
setup as `notebooks/uc_managed.py` (UC at `localhost:8081`, bucket `olai-demo-1`,
`eu-central-1`).

DuckDB cannot do UC DDL (no `CREATE`/`DROP`), so the table is created + seeded by a
Spark cell first (the proven path from `uc_managed.py`); DuckDB only reads + appends.

## What works

- **DuckDB version / write support.** DuckDB 1.5.3, extension `unity_catalog`
  `ad54a34`, `delta` `2d76d40`. This is post-PR
  [duckdb/unity_catalog#75](https://github.com/duckdb/unity_catalog/pull/75)
  ("Add catalog managed commits"), so the coordinated-commits code path is present.
  DuckDB ≥ 1.5.x can append (INSERT) to Delta — it is **append-only**: no
  `UPDATE`/`DELETE`/`MERGE`/`OVERWRITE`, and no DDL.
- **Endpoint / attach.** The `unity_catalog` extension builds request URLs by
  concatenating `ENDPOINT + path`, so the secret's `ENDPOINT` must carry a scheme
  and a host that resolves. Working form:

  ```sql
  CREATE OR REPLACE SECRET uc (
      TYPE unity_catalog, TOKEN '',
      ENDPOINT 'http://127.0.0.1:8081',   -- scheme REQUIRED; 127.0.0.1 avoids localhost/IPv6 quirks
      AWS_REGION 'eu-central-1'
  );
  -- a *named* secret is only used if ATTACH references it by name;
  -- OSS UC can't auto-detect the default schema (that probe hits a Databricks-only endpoint).
  ATTACH 'demo' AS uc_demo (TYPE unity_catalog, SECRET uc, DEFAULT_SCHEMA 'managed_demo');
  ```

  Earlier failures (`Could not resolve hostname`, host dropped from the URL) were
  all caused by a missing scheme / an unreferenced named secret — now fixed.
- **Credential vending.** UC vends short-lived per-table S3 creds; the extension
  wires them in automatically. No separate S3 secret needed for the reads.

## The blocker

Once attached, reading the managed table makes DuckDB call the Delta
coordinated-commits endpoint to list commits before it can build the log tail:

```
GET /api/2.1/unity-catalog/delta/preview/commits
→ 400 Bad Request
  {"error_code":"INVALID_ARGUMENT",
   "message":"No suitable request converter found for a @RequestObject 'DeltaGetCommits'"}
```

`/delta/preview/commits` **is** the correct, current path — the `preview` segment is
intentional (upstream flags the API experimental). The path is not the problem.

### Protocol shape (both sides agree on paper)

The server route (`server/.../service/DeltaCommitsService.java`, identical in
`roeap/unitycatalog` and upstream `unitycatalog/unitycatalog`):

- `getCommits` is annotated `@Get("")` and binds its argument from the **JSON request
  body** via a `@RequestObject` + `JacksonRequestConverterFunction`. I.e. it is a
  **GET-with-a-body**.
- `commit` is `@Post("")`, body → `DeltaCommit`.

The DuckDB client (`src/uc_api.cpp` `GetCommits`) builds a POST body and sets
`send_post_as_get_request = true` — i.e. it sends a **GET carrying a JSON body**,
which is exactly what the server's contract expects. So the shapes match.

### Root cause: missing `Content-Type: application/json` on DuckDB's GET

Captured DuckDB's actual request via a logging proxy in front of UC:

```
>>> GET /api/2.1/unity-catalog/delta/preview/commits | bodylen=218 |
    body={"start_version": 0, "table_id": "...", "table_uri": "s3://olai-demo-1/managed/..."}
```

The body is well-formed. Replaying that exact request against the real UC server
while varying only the content-type isolates the cause:

| Request (DuckDB's exact GET + body) | `Content-Type`            | Result |
|-------------------------------------|---------------------------|--------|
| same body                           | `application/json`        | **200 OK** — returns `commits[]` + `latest_table_version` |
| same body                           | (none)                    | 400 `No suitable request converter for 'DeltaGetCommits'` |
| same body                           | `text/plain`              | 400 (same) |
| same body                           | form-urlencoded (curl default) | 400 (same) |

So the server's Jackson converter only deserializes the body into `DeltaGetCommits`
when `Content-Type: application/json` is present. **DuckDB's `unity_catalog`
extension omits that header on its GET-with-body**, so the server can't bind the
request object → `400`. The request shape is correct; only the content-type header
is missing.

Conclusion: **client-side bug** (DuckDB / its httpfs `send_post_as_get_request`
path drops the JSON content-type). The fork and the notebook are otherwise correct,
and the notebook cannot work around it — it doesn't control the header DuckDB emits.

### Verified environment

- DuckDB 1.5.3; `unity_catalog` `ad54a34`, `delta` `2d76d40`, `httpfs` `52afb42`.
- UC server `ghcr.io/roeap/unitycatalog:v0.0.0-dev-3`, Armeria 1.28.4.
- A manual `curl` GET-with-body + `Content-Type: application/json` returns `200`
  with a real commit row, confirming the table and server are otherwise healthy.

## Mitigation options

1. **Patch the fork's converter (server-side).** Make `DeltaCommitsService.getCommits`
   bind `DeltaGetCommits` from the body regardless of (or tolerant of a missing)
   content-type — e.g. register the Jackson request converter for that route without
   gating on `application/json`, or default a missing/`text/plain` content-type to
   JSON for this GET. Rebuild + retag the `roeap/unitycatalog` image. Unblocks
   managed tables for *any* DuckDB client. Aligns with how other UC gaps in this
   repo have been fixed at the fork layer. Smallest blast radius for our setup.
2. **Use an EXTERNAL UC table instead.** An external Delta table (explicit
   `LOCATION`, normal `_delta_log`) is *not* catalog-managed, so DuckDB never calls
   `/delta/preview/commits` (`IsCCV2()` is false → no `BuildLogTail`). DuckDB read +
   append work today with no server change. Trade-off: loses the "managed table"
   demonstration, and external writes to SeaweedFS hit the session-token gap (real
   AWS S3 is fine). `notebooks/uc_crud.py` already creates external tables.
3. **Patch the DuckDB extension (upstream).** Fix `src/uc_api.cpp` `GetCommits` to set
   `Content-Type: application/json` on the GET-with-body. Correct long-term home and
   fixes it for everyone, but requires building the C++ extension and is slow to land.

### Note on scope

DuckDB's coordinated-commits support (PR #75) was developed and tested against
**Databricks-hosted UC**, not OSS UC. Even once the content-type issue is resolved,
expect further OSS-specific gaps on the managed-table write path; both sides label
this API a proof-of-concept / `preview`.

## References

- `notebooks/uc_duckdb.py`, `notebooks/uc_managed.py`, `notebooks/uc_crud.py`
- DuckDB blog, "Delta Grows Up: Writes, Unity Catalog and Time Travel" (2026-05-07):
  <https://duckdb.org/2026/05/07/delta-uc-updates>
- DuckDB Unity Catalog extension: <https://duckdb.org/docs/stable/core_extensions/unity_catalog>
- `duckdb/unity_catalog` `src/uc_api.cpp` (`GetCommits`/`PostCommit`),
  `src/storage/uc_table_set.cpp` (`IsCCV2`, `BuildLogTail`, `InternalAttach`); PR #75.
- `unitycatalog/unitycatalog` `server/.../service/DeltaCommitsService.java`
  (`@Get("")` getCommits / `@Post("")` commit); OpenAPI `api/all.yaml`
  `/delta/preview/commits`.
