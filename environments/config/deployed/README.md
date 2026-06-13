# Running host-local binaries against the deployed Unity Catalog

Configs in this folder are for running binaries from this workspace **on your host**
against the **deployed** Unity Catalog stack — the ECS Fargate deployment managed by
the sibling [`unitycatalog-quickstart`](../../../../unitycatalog-quickstart) repo
(`just deploy-ecs`). This is the test path before trusting the fully-deployed
pipeline (`just deploy-ecs-lineage` in that repo).

Contrast with the sibling folders:

- `../local/` — host binaries against the local docker-compose stack.
- `../live/` — configs **mounted into** the compose/ECS containers themselves.

## lineage-service

`lineage-service.toml` runs the OpenLineage ingest + Marquez read API locally, writing
events to a UC **catalog-managed** Delta table on the deployed server
(`demo.lineage.events_local` — deliberately separate from the deployed service's
`demo.lineage.events`, so local test events never pollute the live lineage graph).
S3 credentials are vended through UC; no AWS keys are needed locally.

### Prerequisites

1. A deployed UC server: `just deploy-ecs` in unitycatalog-quickstart (writes
   `UC_SERVER` and `UC_ADMIN_TOKEN` into that repo's `.env`).
2. A token for the live server — see "Tokens for the live (ECS) deployment" in the
   quickstart README. Quick path: the admin token (`just deploy-ecs-token`); verify
   any token with `just check-token` (also in the quickstart repo).

### Run

Wire the environment from the quickstart `.env` (note the REST suffix — the service
takes the full REST base, the same derivation the ECS deployment uses):

```bash
set -a; source ~/code/unitycatalog-quickstart/.env; set +a
export UNITY_CATALOG_URL="${UC_SERVER%/}/api/2.1/unity-catalog/"
export UNITY_CATALOG_TOKEN="$UC_ADMIN_TOKEN"   # or a user token from create-user-jwt
export AWS_REGION="${AWS_REGION:-us-west-2}"   # the deployed bucket's region
```

Then, from the repo root:

```bash
just lineage-deployed
# equivalent to:
#   cargo run -p lineage-service -- environments/config/deployed/lineage-service.toml
```

Startup logs `registering unity-managed delta sink: demo.lineage.events_local`, and on
the first run `creating table …` (auto-create). A 401/403 here means the token is bad
or lacks grants on the `demo` catalog — re-check with `just check-token`. If the bind
fails with "address already in use", pick another port: `LINEAGE__PORT=8095 just
lineage-deployed`.

The Marquez read API resolves the same Unity Catalog table the ingest side writes:
for `unity-managed` it reads the catalog-ratified commit tail (so freshly ingested
events are visible immediately, even before the writer backfills the published log);
for `unity-external` it reads the published `_delta_log`. The startup line
`lineage read API resolving … through Unity Catalog` confirms the read path is wired
to the catalog rather than a local path.

### Smoke test

Post a minimal OpenLineage run event, then read it back through the Marquez API:

```bash
curl -s -X POST http://localhost:8091/api/v1/lineage \
  -H 'Content-Type: application/json' \
  -d '{
    "eventType": "COMPLETE",
    "eventTime": "'"$(date -u +%Y-%m-%dT%H:%M:%SZ)"'",
    "run":  { "runId": "'"$(uuidgen | tr 'A-Z' 'a-z')"'" },
    "job":  { "namespace": "deployed-smoke", "name": "local_test" },
    "outputs": [ { "namespace": "deployed-smoke", "name": "demo.lineage.events_local" } ],
    "producer": "https://github.com/roeap/open-lakehouse",
    "schemaURL": "https://openlineage.io/spec/2-0-2/OpenLineage.json#/$defs/RunEvent"
  }'
```

Expect `202 {"status":"accepted"}`. After the flush interval (~500ms) the event is
readable through the Marquez API — its namespace shows up immediately:

```bash
curl -s http://localhost:8091/api/v1/namespaces | jq
```

You can also confirm the commit landed on the deployed server directly —
`latest_table_version` increments with each flush:

```bash
table=$(curl -s -H "Authorization: Bearer $UNITY_CATALOG_TOKEN" \
  "${UNITY_CATALOG_URL%/}/tables/demo.lineage.events_local")
curl -s -X GET "${UNITY_CATALOG_URL%/}/delta/preview/commits" \
  -H "Authorization: Bearer $UNITY_CATALOG_TOKEN" -H 'Content-Type: application/json' \
  -d "{\"table_id\":\"$(jq -r .table_id <<<"$table")\",\"table_uri\":\"$(jq -r .storage_location <<<"$table")\",\"start_version\":0}" | jq
```
