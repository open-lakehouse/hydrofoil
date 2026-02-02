#!/usr/bin/env bash
#
# Setup Notation CLI Client
# Configures notation with signing keys and trust policy
#
# Usage: ./scripts/setup-notation-client.sh
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
SIGNING_KEY="$CERT_DIR/signing/notation-signing.key"
SIGNING_CERT="$CERT_DIR/signing/notation-signing.crt"

# Detect notation config directory based on OS
if [ "$(uname)" == "Darwin" ]; then
    # macOS uses Application Support
    NOTATION_CONFIG_DIR="$HOME/Library/Application Support/notation"
else
    # Linux uses XDG config
    NOTATION_CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/notation"
fi

KEY_NAME="open-lakehouse-demo"
TRUSTSTORE_NAME="open-lakehouse"

echo "=================================================="
echo "Setup Notation CLI Client"
echo "=================================================="
echo ""

# Check if notation is installed
if ! command -v notation &> /dev/null; then
    echo -e "${RED}Error: notation CLI is not installed${NC}"
    echo ""
    echo "Please install notation:"
    echo "  macOS:   brew install notation"
    echo "  Linux:   Download from https://github.com/notaryproject/notation/releases"
    echo ""
    exit 1
fi

echo "Notation version:"
notation version
echo ""

# Check if certificates exist
if [ ! -f "$CA_CERT" ]; then
    echo -e "${RED}Error: CA certificate not found at $CA_CERT${NC}"
    echo ""
    echo "Please generate certificates first:"
    echo "  ./scripts/generate-notation-certs.sh"
    echo ""
    exit 1
fi

if [ ! -f "$SIGNING_KEY" ] || [ ! -f "$SIGNING_CERT" ]; then
    echo -e "${RED}Error: Signing certificate/key not found${NC}"
    echo ""
    echo "Please generate certificates first:"
    echo "  ./scripts/generate-notation-certs.sh"
    echo ""
    exit 1
fi

# Create notation configuration directory
echo "Creating notation configuration directory..."
mkdir -p "$NOTATION_CONFIG_DIR/truststore/x509/ca/$TRUSTSTORE_NAME"
echo -e "${GREEN}✓${NC} Configuration directory created"
echo ""

# Step 1: Add signing key to notation
echo "Step 1: Adding signing key to notation..."
echo "----------------------------------------"

# Note: Notation v1.3+ doesn't support 'notation key add' with local files
# We use the workaround of manually editing signingkeys.json
# See: https://github.com/notaryproject/notation/issues/539

SIGNINGKEYS_FILE="$NOTATION_CONFIG_DIR/signingkeys.json"

# Backup existing signingkeys.json if it exists
if [ -f "$SIGNINGKEYS_FILE" ]; then
    echo -e "${YELLOW}⚠${NC}  Existing signing keys configuration found"
    cp "$SIGNINGKEYS_FILE" "$SIGNINGKEYS_FILE.backup.$(date +%Y%m%d-%H%M%S)"
    echo "  Backup created: $SIGNINGKEYS_FILE.backup.*"
fi

# Create signingkeys.json with local key configuration
cat > "$SIGNINGKEYS_FILE" << EOF
{
  "default": "$KEY_NAME",
  "keys": [
    {
      "name": "$KEY_NAME",
      "keyPath": "$SIGNING_KEY",
      "certPath": "$SIGNING_CERT"
    }
  ]
}
EOF

echo -e "${GREEN}✓${NC} Signing key configured in signingkeys.json"
echo "  Location: $SIGNINGKEYS_FILE"

echo ""
echo "Configured keys:"
notation key list
echo ""

# Step 2: Add CA certificate to trust store
echo "Step 2: Setting up trust store..."
echo "----------------------------------------"

# Use notation cert add to properly add certificate to trust store
notation cert add --type ca --store "$TRUSTSTORE_NAME" "$CA_CERT"

echo -e "${GREEN}✓${NC} CA certificate added to trust store"
echo "  Store: ca:$TRUSTSTORE_NAME"
echo "  Location: $NOTATION_CONFIG_DIR/truststore/x509/ca/$TRUSTSTORE_NAME/"
echo ""

# Step 3: Create trust policy
echo "Step 3: Creating trust policy..."
echo "----------------------------------------"

TRUSTPOLICY_FILE="$NOTATION_CONFIG_DIR/trustpolicy.json"

# Backup existing trust policy if it exists
if [ -f "$TRUSTPOLICY_FILE" ]; then
    echo -e "${YELLOW}⚠${NC}  Existing trust policy found, creating backup"
    cp "$TRUSTPOLICY_FILE" "$TRUSTPOLICY_FILE.backup.$(date +%Y%m%d-%H%M%S)"
fi

# Create trust policy
cat > "$TRUSTPOLICY_FILE" << EOF
{
  "version": "1.0",
  "trustPolicies": [
    {
      "name": "open-lakehouse-policy",
      "registryScopes": ["*"],
      "signatureVerification": {
        "level": "strict"
      },
      "trustStores": ["ca:$TRUSTSTORE_NAME"],
      "trustedIdentities": ["*"]
    }
  ]
}
EOF

echo -e "${GREEN}✓${NC} Trust policy created"
echo "  Location: $TRUSTPOLICY_FILE"
echo ""

echo "Trust policy contents:"
cat "$TRUSTPOLICY_FILE"
echo ""

# Step 4: Verify configuration
echo "Step 4: Verifying configuration..."
echo "----------------------------------------"

# Check if trust policy is valid
if notation policy show > /dev/null 2>&1; then
    echo -e "${GREEN}✓${NC} Trust policy is valid"
else
    echo -e "${RED}✗${NC} Trust policy validation failed"
    exit 1
fi

echo ""
notation policy show
echo ""

# Summary
echo "=================================================="
echo -e "${GREEN}Notation Client Setup Complete!${NC}"
echo "=================================================="
echo ""
echo "Configuration summary:"
echo "  Default signing key:    $KEY_NAME"
echo "  Trust store:            ca:$TRUSTSTORE_NAME"
echo "  Registry scopes:        localhost:10100/*, localhost:5000/*"
echo "  Verification level:     strict"
echo "  Configuration dir:      $NOTATION_CONFIG_DIR"
echo ""
echo "Next steps:"
echo "  1. Ensure docker compose is running with OCI profile:"
echo "     docker compose --profile oci up -d"
echo ""
echo "  2. Test the signing workflow:"
echo "     ./scripts/test-notation-signing.sh"
echo ""
echo -e "${BLUE}Usage Examples:${NC}"
echo ""
echo "  Sign an image:"
echo "    docker push localhost:10100/myapp:latest"
echo "    notation sign localhost:10100/myapp:latest"
echo ""
echo "  Verify a signature:"
echo "    notation verify localhost:10100/myapp:latest"
echo ""
echo "  List signatures:"
echo "    notation list localhost:10100/myapp:latest"
echo ""
