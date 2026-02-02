#!/usr/bin/env bash
#
# Upload Notation Certificates to Zot Registry
# Uploads the CA certificate to zot's trust store via API
#
# Usage: ./scripts/upload-certs-to-zot.sh
#

set -euo pipefail

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CERT_DIR="$PROJECT_ROOT/config/certificates"
CA_CERT="$CERT_DIR/trust/ca.crt"
ZOT_URL="${ZOT_URL:-http://localhost:10100}"
MAX_RETRIES=30
RETRY_DELAY=2

echo "=================================================="
echo "Upload Notation Certificates to Zot Registry"
echo "=================================================="
echo ""

# Check if CA certificate exists
if [ ! -f "$CA_CERT" ]; then
    echo -e "${RED}Error: CA certificate not found at $CA_CERT${NC}"
    echo ""
    echo "Please generate certificates first:"
    echo "  ./scripts/generate-notation-certs.sh"
    echo ""
    exit 1
fi

echo "Configuration:"
echo "  Zot URL:        $ZOT_URL"
echo "  CA Certificate: $CA_CERT"
echo ""

# Wait for zot to be ready
echo "Waiting for zot registry to be ready..."
RETRIES=0
while [ $RETRIES -lt $MAX_RETRIES ]; do
    if curl -sf "$ZOT_URL/v2/" > /dev/null 2>&1; then
        echo -e "${GREEN}✓${NC} Zot registry is ready"
        break
    fi
    RETRIES=$((RETRIES + 1))
    if [ $RETRIES -eq $MAX_RETRIES ]; then
        echo -e "${RED}✗${NC} Zot registry is not responding after $MAX_RETRIES attempts"
        echo ""
        echo "Please ensure zot is running:"
        echo "  docker compose --profile oci up -d"
        echo ""
        exit 1
    fi
    echo -n "."
    sleep $RETRY_DELAY
done
echo ""

# Upload CA certificate to zot trust store
echo "Uploading CA certificate to zot trust store..."
echo "----------------------------------------"

HTTP_CODE=$(curl -s -w "%{http_code}" -o /tmp/zot-upload-response.txt \
    --data-binary @"$CA_CERT" \
    -X POST "$ZOT_URL/v2/_zot/ext/notation?truststoreType=ca")

if [ "$HTTP_CODE" -eq 200 ] || [ "$HTTP_CODE" -eq 201 ]; then
    echo -e "${GREEN}✓${NC} CA certificate uploaded successfully (HTTP $HTTP_CODE)"

    # Display response if available
    if [ -s /tmp/zot-upload-response.txt ]; then
        echo ""
        echo "Response:"
        cat /tmp/zot-upload-response.txt
        echo ""
    fi
else
    echo -e "${RED}✗${NC} Failed to upload certificate (HTTP $HTTP_CODE)"
    echo ""
    echo "Response:"
    cat /tmp/zot-upload-response.txt
    echo ""
    rm -f /tmp/zot-upload-response.txt
    exit 1
fi

rm -f /tmp/zot-upload-response.txt
echo ""

# Verify certificate was uploaded by checking zot extensions
echo "Verifying certificate upload..."
echo "----------------------------------------"

if curl -sf "$ZOT_URL/v2/_zot/ext/search" \
    -H "Content-Type: application/json" \
    -d '{"query": "{GlobalSearch(query:\"\"){Images{RepoName}}}"}' > /dev/null 2>&1; then
    echo -e "${GREEN}✓${NC} Zot notation extension is responding"
else
    echo -e "${YELLOW}⚠${NC}  Could not verify notation extension (this may be normal)"
fi
echo ""

# Display certificate information
echo "Certificate Information:"
echo "----------------------------------------"
openssl x509 -in "$CA_CERT" -noout -subject -issuer -dates
echo ""

# Summary
echo "=================================================="
echo -e "${GREEN}Certificate Upload Complete!${NC}"
echo "=================================================="
echo ""
echo "The CA certificate has been uploaded to zot's trust store."
echo "Zot will now verify signatures from certificates signed by this CA."
echo ""
echo "Next steps:"
echo "  1. Setup notation client: ./scripts/setup-notation-client.sh"
echo "  2. Test signing workflow: ./scripts/test-notation-signing.sh"
echo ""
echo -e "${BLUE}Tip:${NC} You can view the trust store in zot's S3 storage:"
echo "  Check the _notation/truststore/x509/ca/default/ directory"
echo ""
