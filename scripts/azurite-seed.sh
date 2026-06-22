#!/usr/bin/env bash
# Seed + verify the Azurite credential-vending path end-to-end.
#
# Prereqs:
#   1. Azurite running:  just env-up azurite   (compose; blob on localhost:10000)
#   2. Rust UC server running on the host from the sibling unitycatalog-rs
#      checkout, e.g.:
#        cargo run -p unitycatalog-cli -- server --rest --port 8081 \
#          --config environments/config/azurite/uc-config.yaml
#
# This script registers an `azure_storage_key` storage credential + an external
# location pointing at the Azurite container, then proves the full vend path:
#   - vends a READ-WRITE SAS and uses it to PUT + GET a blob (201 / 200), and
#   - vends a READ-only SAS and confirms it can GET (200) but not PUT (403).
#
# No STS / online token service is involved: the SAS is signed offline from the
# storage account key. This is the local-vendable path that an S3 emulator
# (SeaweedFS/MinIO) cannot provide.
#
# Env overrides: UC_PORT (8081), AZURITE_PORT (10000), CONTAINER (lakehouse).
set -euo pipefail

UC_PORT="${UC_PORT:-8081}"
AZURITE_PORT="${AZURITE_PORT:-10000}"
CONTAINER="${CONTAINER:-lakehouse}"
UC="http://localhost:${UC_PORT}/api/2.1/unity-catalog"
H='content-type: application/json'
# The well-known Azurite account + key (local dev only).
ACCOUNT="devstoreaccount1"
KEY="Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw=="
PREFIX="sales/orders"
BLOB="http://localhost:${AZURITE_PORT}/${ACCOUNT}/${CONTAINER}/${PREFIX}/vended.txt"

log() { printf '\033[36m[azurite-seed]\033[0m %s\n' "$*" >&2; }
fail() { printf '\033[31m[azurite-seed] FAIL:\033[0m %s\n' "$*" >&2; exit 1; }

sas_of() { # $1 = operation (PATH_READ | PATH_READ_WRITE)
  curl -fsS -X POST "$UC/temporary-path-credentials" -H "$H" \
    -d "{\"url\":\"azurite://${CONTAINER}/${PREFIX}\",\"operation\":\"$1\"}" \
  | python3 -c "import sys,json;print(json.load(sys.stdin)['azure_user_delegation_sas']['sas_token'])"
}

log "Registering storage credential 'azurite_key'..."
curl -fsS -X POST "$UC/credentials" -H "$H" -d "{
  \"name\":\"azurite_key\",\"purpose\":\"STORAGE\",
  \"azure_storage_key\":{\"account_name\":\"${ACCOUNT}\",\"account_key\":\"${KEY}\"},
  \"skip_validation\":true
}" -o /dev/null || fail "credential create failed"

log "Registering external location 'azurite_loc' (azurite://${CONTAINER})..."
curl -fsS -X POST "$UC/external-locations" -H "$H" -d "{
  \"name\":\"azurite_loc\",\"url\":\"azurite://${CONTAINER}\",\"credential_name\":\"azurite_key\"
}" -o /dev/null || fail "external location create failed"

log "Vending READ-WRITE credentials + writing/reading a blob..."
RW="$(sas_of PATH_READ_WRITE)"
[[ -n "$RW" ]] || fail "no read-write SAS vended"
code=$(curl -sS -X PUT "$BLOB?$RW" -H "x-ms-blob-type: BlockBlob" \
  --data-binary "written-with-vended-sas" -o /dev/null -w "%{http_code}")
[[ "$code" == "201" ]] || fail "RW PUT expected 201, got $code"
body=$(curl -fsS "$BLOB?$RW")
[[ "$body" == "written-with-vended-sas" ]] || fail "RW GET returned unexpected body: $body"
log "  RW PUT 201, GET ok ✓"

log "Vending READ-only credentials + confirming scope..."
RO="$(sas_of PATH_READ)"
[[ -n "$RO" ]] || fail "no read-only SAS vended"
code=$(curl -sS "$BLOB?$RO" -o /dev/null -w "%{http_code}")
[[ "$code" == "200" ]] || fail "RO GET expected 200, got $code"
code=$(curl -sS -X PUT "${BLOB}.deny?$RO" -H "x-ms-blob-type: BlockBlob" \
  --data-binary x -o /dev/null -w "%{http_code}")
[[ "$code" == "403" ]] || fail "RO PUT expected 403 (write denied), got $code"
log "  RO GET 200, RO PUT 403 (write denied) ✓"

log "SUCCESS: credential vending works end-to-end against Azurite (no STS)."
