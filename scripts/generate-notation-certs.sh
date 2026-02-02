#!/usr/bin/env bash
#
# Generate Notation Signing Certificates with CA Hierarchy
# Creates a root CA and a signing certificate signed by that CA
#
# Usage: ./scripts/generate-notation-certs.sh
#

set -euo pipefail

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Directories
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
CERT_DIR="$PROJECT_ROOT/config/certificates"
CA_DIR="$CERT_DIR/ca"
SIGNING_DIR="$CERT_DIR/signing"
TRUST_DIR="$CERT_DIR/trust"

# Certificate validity in days (1 year)
VALIDITY_DAYS=365

echo "=================================================="
echo "Notation Certificate Generation for Open Lakehouse"
echo "=================================================="
echo ""

# Check if certificates already exist
if [ -f "$CA_DIR/ca.crt" ] || [ -f "$SIGNING_DIR/notation-signing.crt" ]; then
    echo -e "${YELLOW}Warning: Certificates already exist${NC}"
    echo ""
    echo "Found existing certificates in:"
    [ -f "$CA_DIR/ca.crt" ] && echo "  - $CA_DIR/ca.crt"
    [ -f "$SIGNING_DIR/notation-signing.crt" ] && echo "  - $SIGNING_DIR/notation-signing.crt"
    echo ""
    read -p "Do you want to regenerate them? This will overwrite existing certificates. [y/N] " -n 1 -r
    echo ""
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo "Aborted. Keeping existing certificates."
        exit 0
    fi
    echo ""
fi

# Create directory structure
echo "Creating directory structure..."
mkdir -p "$CA_DIR" "$SIGNING_DIR" "$TRUST_DIR"
echo -e "${GREEN}✓${NC} Directories created"
echo ""

# Step 1: Generate Root CA
echo "Step 1: Generating Root CA certificate..."
echo "----------------------------------------"

openssl req \
    -x509 \
    -new \
    -nodes \
    -newkey rsa:4096 \
    -keyout "$CA_DIR/ca.key" \
    -out "$CA_DIR/ca.crt" \
    -config "$CERT_DIR/openssl-ca.cnf" \
    -days "$VALIDITY_DAYS" \
    -sha384

echo -e "${GREEN}✓${NC} Root CA certificate generated"
echo "  Private key: $CA_DIR/ca.key"
echo "  Certificate: $CA_DIR/ca.crt"
echo ""

# Step 2: Generate Signing Certificate Request
echo "Step 2: Generating signing certificate..."
echo "----------------------------------------"

# Generate private key for signing certificate
openssl genrsa -out "$SIGNING_DIR/notation-signing.key" 2048

# Generate Certificate Signing Request (CSR)
openssl req \
    -new \
    -key "$SIGNING_DIR/notation-signing.key" \
    -out "$SIGNING_DIR/notation-signing.csr" \
    -config "$CERT_DIR/openssl-signing.cnf"

echo -e "${GREEN}✓${NC} Signing certificate request generated"
echo ""

# Step 3: Sign the certificate with the CA
echo "Step 3: Signing certificate with Root CA..."
echo "----------------------------------------"

# Create a temporary extension file that includes authorityKeyIdentifier
cat > "$SIGNING_DIR/signing-extensions.cnf" << EOF
[ v3_req ]
basicConstraints = critical, CA:FALSE
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid:always
EOF

openssl x509 \
    -req \
    -in "$SIGNING_DIR/notation-signing.csr" \
    -CA "$CA_DIR/ca.crt" \
    -CAkey "$CA_DIR/ca.key" \
    -CAcreateserial \
    -out "$SIGNING_DIR/notation-signing.crt" \
    -days "$VALIDITY_DAYS" \
    -sha256 \
    -extensions v3_req \
    -extfile "$SIGNING_DIR/signing-extensions.cnf"

# Clean up CSR and temporary extension file
rm -f "$SIGNING_DIR/notation-signing.csr"
rm -f "$SIGNING_DIR/signing-extensions.cnf"

echo -e "${GREEN}✓${NC} Signing certificate created and signed by CA"
echo "  Private key: $SIGNING_DIR/notation-signing.key"
echo "  Certificate: $SIGNING_DIR/notation-signing.crt"
echo ""

# Step 4: Copy certificates to trust directory for zot upload
echo "Step 4: Preparing certificates for zot trust store..."
echo "----------------------------------------"

cp "$CA_DIR/ca.crt" "$TRUST_DIR/ca.crt"
cp "$SIGNING_DIR/notation-signing.crt" "$TRUST_DIR/notation-signing.crt"

echo -e "${GREEN}✓${NC} Certificates copied to trust directory"
echo "  Trust dir: $TRUST_DIR"
echo ""

# Step 5: Verify certificates
echo "Step 5: Verifying certificate chain..."
echo "----------------------------------------"

# Verify signing certificate against CA
if openssl verify -CAfile "$CA_DIR/ca.crt" "$SIGNING_DIR/notation-signing.crt" > /dev/null 2>&1; then
    echo -e "${GREEN}✓${NC} Certificate chain verification successful"
else
    echo -e "${RED}✗${NC} Certificate chain verification failed"
    exit 1
fi
echo ""

# Step 6: Display certificate details
echo "Step 6: Certificate Details"
echo "----------------------------------------"

echo ""
echo "Root CA Certificate:"
openssl x509 -in "$CA_DIR/ca.crt" -noout -subject -issuer -dates -ext basicConstraints,keyUsage
echo ""

echo "Signing Certificate:"
openssl x509 -in "$SIGNING_DIR/notation-signing.crt" -noout -subject -issuer -dates -ext basicConstraints,keyUsage,extendedKeyUsage
echo ""

# Summary
echo "=================================================="
echo -e "${GREEN}Certificate Generation Complete!${NC}"
echo "=================================================="
echo ""
echo "Next steps:"
echo "  1. Start docker compose: docker compose --profile oci up -d"
echo "  2. Upload certificates to zot: ./scripts/upload-certs-to-zot.sh"
echo "  3. Setup notation client: ./scripts/setup-notation-client.sh"
echo "  4. Test signing: ./scripts/test-notation-signing.sh"
echo ""
echo "Certificate locations:"
echo "  CA Certificate:      $CA_DIR/ca.crt"
echo "  Signing Certificate: $SIGNING_DIR/notation-signing.crt"
echo "  Trust Store:         $TRUST_DIR/"
echo ""
echo -e "${YELLOW}Important:${NC} Keep the private keys secure!"
echo "  CA Private Key:      $CA_DIR/ca.key"
echo "  Signing Private Key: $SIGNING_DIR/notation-signing.key"
echo ""
echo "Certificates are valid for $VALIDITY_DAYS days (until: $(date -v+${VALIDITY_DAYS}d '+%Y-%m-%d' 2>/dev/null || date -d "+${VALIDITY_DAYS} days" '+%Y-%m-%d' 2>/dev/null || echo "N/A"))"
echo ""
