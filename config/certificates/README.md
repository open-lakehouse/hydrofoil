# OCI Artifact Signing with Notation

This directory contains the configuration files needed to generate certificates for signing OCI artifacts using [Notation](https://notaryproject.dev/) with [Zot](https://zotregistry.io/) registry.

## Overview

This setup implements a **Certificate Authority (CA) hierarchy** for signing OCI artifacts in the Open Lakehouse demo environment:

- **Root CA**: Self-signed certificate authority that signs the signing certificate
- **Signing Certificate**: Code signing certificate used by notation to sign OCI artifacts
- **Trust Store**: Certificates uploaded to zot for signature verification

```
┌─────────────────────────────────────────────────────────────┐
│                    Certificate Hierarchy                     │
└─────────────────────────────────────────────────────────────┘

    Root CA (ca.crt)
         │
         │ signs
         ↓
    Signing Certificate (notation-signing.crt)
         │
         │ used to sign
         ↓
    OCI Artifacts (container images, policies, etc.)
```

## Directory Structure

After generating certificates, the directory will contain:

```
config/certificates/
├── README.md                          # This file
├── .gitignore                         # Protects all certificate files
├── openssl-ca.cnf                     # OpenSSL config for Root CA
├── openssl-signing.cnf                # OpenSSL config for signing cert
├── ca/                                # Root CA files (git-ignored)
│   ├── ca.key                        # CA private key (KEEP SECURE!)
│   ├── ca.crt                        # CA certificate
│   └── ca.srl                        # Serial number file
├── signing/                           # Signing certificate files (git-ignored)
│   ├── notation-signing.key          # Signing private key (KEEP SECURE!)
│   └── notation-signing.crt          # Signing certificate
└── trust/                            # Certificates for zot upload (git-ignored)
    ├── ca.crt                        # Copy of CA cert for zot
    └── notation-signing.crt          # Copy of signing cert (optional)
```

## Quick Start

### 1. Generate Certificates

```bash
./scripts/generate-notation-certs.sh
```

This will:
- Create a Root CA with a 4096-bit RSA key
- Generate a signing certificate signed by the Root CA
- Configure proper certificate extensions for notation compliance
- Create certificates valid for 1 year
- Set up the directory structure

### 2. Start Docker Compose with OCI Profile

```bash
docker compose --profile oci up -d
```

The `oci_cert_uploader` init container will automatically upload the CA certificate to zot's trust store.

### 3. Setup Notation Client

```bash
./scripts/setup-notation-client.sh
```

This will:
- Add the signing key to notation
- Configure the trust policy
- Set up the trust store
- Verify the configuration

### 4. Test the Signing Workflow

```bash
./scripts/test-notation-signing.sh
```

This end-to-end test will:
- Build a test image
- Push it to the registry
- Sign it with notation
- Verify the signature
- Check signature metadata in zot

## Certificate Details

### Root CA Certificate

**File**: `ca/ca.crt`

**Configuration**: `openssl-ca.cnf`

**Properties**:
- Type: Self-signed X.509 certificate
- Key: RSA 4096 bits
- Digest: SHA-384
- Validity: 1 year
- Extensions:
  - `basicConstraints: critical, CA:TRUE, pathlen:0`
  - `keyUsage: critical, keyCertSign, cRLSign`

**Purpose**: Acts as the root of trust for the signing certificate. Uploaded to zot's trust store.

### Signing Certificate

**File**: `signing/notation-signing.crt`

**Configuration**: `openssl-signing.cnf`

**Properties**:
- Type: X.509 certificate signed by Root CA
- Key: RSA 2048 bits
- Digest: SHA-256
- Validity: 1 year
- Extensions (notation-compliant):
  - `basicConstraints: critical, CA:FALSE`
  - `keyUsage: critical, digitalSignature`
  - `extendedKeyUsage: critical, codeSigning`

**Purpose**: Used by notation CLI to sign OCI artifacts. Meets all [notation certificate requirements](https://github.com/notaryproject/specifications/blob/main/specs/signature-specification.md#certificate-requirements).

## How It Works

### Signing Flow

```
┌──────────┐     ┌──────────┐     ┌─────────┐     ┌──────────┐
│Developer │────▶│  Docker  │────▶│   Zot   │────▶│ Notation │
│          │     │          │     │Registry │     │   Sign   │
└──────────┘     └──────────┘     └─────────┘     └──────────┘
                                                         │
                 ┌─────────────────────────────────────┘
                 │
                 ▼
           ┌──────────────┐
           │  Signature   │
           │   Stored in  │
           │  OCI Registry│
           └──────────────┘
```

1. **Build & Push**: Developer builds and pushes image to zot registry
2. **Sign**: Notation signs the image using the signing certificate
3. **Store**: Signature is stored in the OCI registry alongside the image
4. **Verify**: Zot validates signature against CA in trust store

### Verification Flow

```
┌──────────┐     ┌─────────┐     ┌──────────────┐
│  Client  │────▶│   Zot   │────▶│ Trust Store  │
│          │     │         │     │   (CA Cert)  │
└──────────┘     └─────────┘     └──────────────┘
    │                 │                  │
    │  Pull Image     │  Verify          │  Check
    │                 │  Signature       │  Chain
    ▼                 ▼                  ▼
 Image         Signature            Valid? ✓/✗
```

1. **Pull**: Client requests image from zot
2. **Fetch Signature**: Zot retrieves signature from storage
3. **Verify Chain**: Zot validates signature using CA from trust store
4. **Report**: Zot returns image with signature verification status

## Security Considerations

### Private Keys

**CRITICAL**: Private keys are never committed to git!

- `ca/ca.key` - Root CA private key
- `signing/notation-signing.key` - Signing private key

These files are protected by `.gitignore`. Keep them secure and back them up safely.

### Certificate Validity

Certificates are valid for **1 year** from generation. You'll need to regenerate them before expiration:

```bash
# Check certificate expiration
openssl x509 -in config/certificates/ca/ca.crt -noout -dates
openssl x509 -in config/certificates/signing/notation-signing.crt -noout -dates

# Regenerate when approaching expiration
./scripts/generate-notation-certs.sh
```

### Trust Model

This setup uses **self-signed certificates** appropriate for:
- ✓ Development environments
- ✓ Demo scenarios
- ✓ Internal testing
- ✗ Production deployments (use CA-signed certificates)

## Integration with Zot

### Trust Policy Configuration

Zot's notation extension is configured in `config/services/oci_registry/config.json`:

```json
{
  "extensions": {
    "trust": {
      "enable": true,
      "cosign": true,
      "notation": true
    }
  }
}
```

### Certificate Upload

Certificates are automatically uploaded to zot via the `oci_cert_uploader` init container in `compose.yaml`.

Manual upload (if needed):

```bash
./scripts/upload-certs-to-zot.sh
```

Or via API:

```bash
curl --data-binary @config/certificates/trust/ca.crt \
  -X POST "http://localhost:10100/v2/_zot/ext/notation?truststoreType=ca"
```

### Trust Store Location

In zot's storage (SeaweedFS S3):

```
s3://oci-registry/_notation/
├── trustpolicy.json              # Auto-generated by zot
└── truststore/
    └── x509/
        └── ca/
            └── default/
                └── ca.crt        # Your uploaded CA certificate
```

## Notation Client Configuration

### Key Configuration

Notation stores keys in: `~/.config/notation/`

Configuration created by `setup-notation-client.sh`:

```
~/.config/notation/
├── trustpolicy.json              # Trust policy for verification
├── signingkeys.json              # Signing key configuration (manually created)
└── truststore/
    └── x509/
        └── ca/
            └── open-lakehouse/
                └── ca.crt        # Copy of CA certificate
```

**Important Note about Local Keys**: Notation v1.3+ doesn't support `notation key add` with local key files via CLI. The `setup-notation-client.sh` script uses a workaround by manually creating `signingkeys.json` with the following structure:

```json
{
  "default": "open-lakehouse-demo",
  "keys": [
    {
      "name": "open-lakehouse-demo",
      "keyPath": "/path/to/signing/notation-signing.key",
      "certPath": "/path/to/signing/notation-signing.crt"
    }
  ]
}
```

This workaround is documented in [notation issue #539](https://github.com/notaryproject/notation/issues/539). Native CLI support for local keys is planned for a future release.

### Trust Policy

```json
{
  "version": "1.0",
  "trustPolicies": [
    {
      "name": "open-lakehouse-policy",
      "registryScopes": ["localhost:10100/*", "localhost:5000/*"],
      "signatureVerification": {
        "level": "strict"
      },
      "trustStores": ["ca:open-lakehouse"],
      "trustedIdentities": ["*"]
    }
  ]
}
```

**Verification Levels**:
- `strict`: Enforces all validations (default)
- `permissive`: Allows some validation failures
- `audit`: Logs validation results
- `skip`: Disables verification

## Usage Examples

### Sign an Image

```bash
# Build and push
docker build -t localhost:10100/myapp:v1.0 .
docker push localhost:10100/myapp:v1.0

# Sign with notation
notation sign localhost:10100/myapp:v1.0
```

### Verify a Signature

```bash
notation verify localhost:10100/myapp:v1.0
```

### List Signatures

```bash
notation list localhost:10100/myapp:v1.0
```

### Check Signature in Zot

```bash
curl -X POST http://localhost:10100/v2/_zot/ext/search \
  -H "Content-Type: application/json" \
  -d '{"query": "{Image(image:\"myapp:v1.0\"){IsSigned SignatureInfo{Tool IsTrusted Author}}}"}' \
  | jq .
```

### View Zot UI

Open in browser: http://localhost:10100/

The Zot web UI shows signature verification status for all images.

## Troubleshooting

### Certificate Generation Fails

**Error**: `openssl: command not found`

**Solution**: Install OpenSSL:
```bash
# macOS
brew install openssl

# Ubuntu/Debian
sudo apt-get install openssl
```

### Notation Key Add Fails

**Error**: `certificate verification failed`

**Solution**: Verify certificate chain:
```bash
openssl verify -CAfile config/certificates/ca/ca.crt \
  config/certificates/signing/notation-signing.crt
```

### Signature Verification Fails

**Possible causes**:

1. **Certificate not uploaded to zot**
   ```bash
   ./scripts/upload-certs-to-zot.sh
   ```

2. **Certificate expired**
   ```bash
   openssl x509 -in config/certificates/signing/notation-signing.crt -noout -dates
   ```

3. **Trust policy mismatch**
   ```bash
   notation policy show
   ```

4. **Registry scope mismatch**: Ensure registry URL in trust policy matches

### Zot Not Accepting Certificates

**Check zot logs**:
```bash
docker compose logs oci_registry
```

**Verify extensions enabled**:
```bash
curl -s http://localhost:10100/v2/_zot/ext/search | jq .
```

## Advanced Topics

### Certificate Rotation

When certificates approach expiration:

1. Generate new certificates
2. Upload new CA to zot
3. Update notation client configuration
4. Re-sign critical images
5. Remove old certificates from trust store

### Multiple Signing Keys

Add additional signing keys for different teams:

```bash
notation key add \
  --name team-a-signing \
  --plugin default \
  --id team-a \
  --key team-a.key \
  team-a.crt
```

### Custom Certificate Attributes

Modify `openssl-signing.cnf` to add custom attributes:

```ini
[ dn ]
CN = My Custom Signing Certificate
O = My Organization
OU = Development Team
L = San Francisco
C = US
emailAddress = security@example.com
```

### Timestamping

Add timestamp authority (TSA) support for long-term verification:

```bash
notation sign --timestamp-url https://timestamp.example.com \
  localhost:10100/myapp:v1.0
```

## Production Considerations

For production deployments:

1. **Use CA-signed certificates** from trusted Certificate Authorities (DigiCert, Let's Encrypt, etc.)
2. **Implement key management** using HSM or cloud KMS (AWS KMS, Azure Key Vault, GCP KMS)
3. **Enable timestamping** to extend signature validity beyond certificate expiration
4. **Rotate certificates regularly** (e.g., annually)
5. **Implement admission control** to enforce signature verification
6. **Audit signature events** with centralized logging
7. **Use separate certificates** per environment (dev/staging/prod)
8. **Restrict registry access** with authentication and RBAC
9. **Enable HTTPS/TLS** for registry communication
10. **Monitor certificate expiration** with alerting

## References

- [Notation Documentation](https://notaryproject.dev/)
- [Notation Specifications](https://github.com/notaryproject/specifications)
- [Zot Registry Documentation](https://zotregistry.io/latest/)
- [Zot Notation Extension](https://zotregistry.io/latest/admin-guide/admin-configuration/#extensions)
- [OpenSSL Documentation](https://www.openssl.org/docs/)
- [OCI Distribution Spec](https://github.com/opencontainers/distribution-spec)

## Support

For issues or questions:

1. Check the [Troubleshooting](#troubleshooting) section
2. Review logs: `docker compose logs oci_registry`
3. Test with the provided scripts
4. Consult the official documentation linked above

## License

This configuration is part of the Open Lakehouse project.
