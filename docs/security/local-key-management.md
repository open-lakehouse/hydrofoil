# Local key management for desktop environments

> Status: **Implemented** (2026-06) in the desktop shell
> (`node/desktop/src-tauri/src/kek.rs`, wired into `lib.rs`), with the UI surface in
> `node/ui/src/components/environment`. See
> [`docs/adr/0016-local-environment-key-management.md`](../adr/0016-local-environment-key-management.md)
> for the decision record.

## Problem

A desktop "environment" runs a local Unity Catalog server (the `uc` sidecar, SQLite
backend, local `file://` managed storage). UC **envelope-encrypts storage credentials at
rest** in its catalog: a per-secret data-encryption key (DEK) encrypts the credential, and
a long-lived **key-encryption key (KEK)** wraps the DEK. The KEK is the root secret — hold
it and you can decrypt every credential in the catalog.

Historically the desktop shell wrote the **same hardcoded dev KEK into every generated
`config.yaml`**, in plaintext on disk:

```yaml
encryption:
  active:
    id: dev
    key: AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=   #  0x00 0x01 … 0x1f
```

A shared, source-committed, plaintext-on-disk KEK means the credentials it protects are
effectively unprotected. This document describes how the desktop app instead mints fresh
per-environment key material and manages it in the OS-native secret store.

## Scope and threat model

- **In scope:** local desktop (Tauri). The trust anchor is the **OS-native secret store**
  (macOS Keychain, Windows Credential Manager, Linux Secret Service). The goal is that a
  KEK is never written to disk and never shared between environments, so a stolen
  `config.yaml` / catalog file does not yield decryptable credentials.
- **Out of scope (deferred):** remote KMS (cloud key vaults), key rotation flows, and
  multi-user key sharing. These are reflected in the data model but not wired (see
  *Future work*).

## Design

### Per-environment KEK in the OS keychain

When an environment is **created**, the shell generates a fresh **32-byte (AES-256) KEK**
from the OS CSPRNG (`getrandom`), base64-encodes it, and stores it in the OS keychain via
the [`keyring`](https://crates.io/crates/keyring) crate:

- **service** = `open-lakehouse`
- **account** = the environment id (a stable, filesystem-safe slug)
- **secret** = the base64 KEK string (the exact form UC expects)

One entry per environment ⇒ keys are isolated. Resolution is **get-or-create** and
idempotent: the same environment always resolves to the same key across restarts.

### The key never lands on disk

UC's config schema accepts an **environment-variable indirection** for the key
(`ConfigValue` is `#[serde(untagged)]`: either an inline base64 string or `{ env: VAR }`).
The generated `config.yaml` therefore references the key indirectly:

```yaml
encryption:
  active:
    id: <key id>                 # stable per-environment id, recorded in every sealed secret
    key:
      env: OPEN_LAKEHOUSE_UC_KEK
```

At **start**, the shell resolves the KEK from the keychain and injects it into the `uc`
sidecar process via the `OPEN_LAKEHOUSE_UC_KEK` env var (the `tauri-plugin-shell` command
`.env(...)`). The material lives only in the keychain and in the child process's
environment — never in `config.yaml`, never logged.

### Persisted key record (`key.json`)

Each environment keeps a small record beside its `config.yaml` (in the env's UC dir),
holding only the *provider choice* and *key id* — **never** the secret:

```jsonc
{ "provider": "keychain", "keyId": "<env-id>", "remote": null }
```

This drives the UI status and stamps `encryption.active.id`. The `id` makes future key
rotation possible (UC supports an `encryption.retired: [...]` list) without a migration.

### Status model and UI

The shell exposes a key status the UI renders without starting the environment:

| Status         | Meaning                                                                 |
| -------------- | ----------------------------------------------------------------------- |
| `unconfigured` | No key minted yet (freshly created / legacy env).                       |
| `keychain`     | Provider is the OS keychain and the key exists.                         |
| `remote`       | Provider is a remote key store.                                         |
| `unavailable`  | Keychain provider, but the OS secret store can't be reached.            |

The environment management view (`EnvironmentDetail` Overview tab) shows an **Encryption
key** card with this status and a **Configure key** action (idle-only — the key is fixed
for a running environment). An `unconfigured`/`unavailable` status is a **blocking
warning**: `start` refuses to spawn the sidecar in that state (no silent fall-through to a
shared key).

### Lifecycle

```
create → (mint KEK in keychain, write key.json) → idle
       → [Configure key]  (choose provider; re-mint if needed)
       → start  (gate on status; resolve KEK; inject env var; spawn sidecar)
```

Creation does **not** hard-fail if the keychain is unavailable — the environment is
created in `unavailable` status so the UI can warn and let the user pick a provider.

## Key components

- `node/desktop/src-tauri/src/kek.rs` — key generation, keychain access, `key.json`,
  status, and the `KeyProvider`/`KeyStatus` types.
- `node/desktop/src-tauri/src/lib.rs` — `create_environment` mints the key;
  `write_uc_config` writes the env-var reference + per-env `id`; `spawn_uc_sidecar`
  injects `OPEN_LAKEHOUSE_UC_KEK`; `start_environment` gates on `unavailable`; commands
  `environment_key_status` / `configure_environment_key`.
- `node/ui/src/lib/client/environments.ts` — `keyStatus` / `configureKey` on the
  framework-agnostic `EnvironmentHost`; `node/desktop/src/tauri-environments.ts` maps them
  to the Tauri commands.
- `node/ui/src/components/environment/{manager/EnvironmentDetail.tsx,ConfigureKeyDialog.tsx}`
  — the status card and provider-select dialog.
- UC schema (read-only reference): `unitycatalog-rs/crates/cli/src/config.rs`
  (`ConfigValue` / `KeyConfig` / `EncryptionConfig`),
  `unitycatalog-rs/crates/common/src/services/encryption.rs` (envelope scheme).

## Decisions

- **OS keychain, hard-required.** No silent fallback to a shared plaintext key; an
  unreachable keychain is surfaced (`unavailable`) and blocks start.
- **Fresh key, no migration.** Environments created under the old dev KEK get a fresh
  per-env key on next start; secrets previously sealed under the dev key become unreadable.
  Acceptable for local dev (at most test credentials).
- **Remote provider is scaffolded, not wired.** The choice is modeled and shown in the UI
  but cannot yet be committed.

## Future work

- **Remote KMS providers.** UC's `KeyProvider` trait
  (`unitycatalog-rs/crates/common/src/services/encryption.rs`) is async and built for
  remote wrap/unwrap (Azure Key Vault, AWS/GCP KMS). Wiring a concrete remote provider +
  the `remote` config in `key.json` is the path to the `remote` status being selectable.
- **Struct-based config generation.** `write_uc_config` still hand-formats the YAML, which
  is fragile around the snake_case/kebab-case field-casing trap. The intended follow-up is
  to factor UC's config types into a small dependency-light `unitycatalog-config` crate
  (the `unitycatalog-cli` crate is binary-only and pulls the whole server/TUI stack, so it
  can't be depended on from the deliberately-standalone desktop crate), then build the
  `Config` struct and `serde_yml::to_string` it. This eliminates the casing traps by
  construction. Tracked as a cross-repo change in `unitycatalog-rs`.
- **Key rotation UI.** UC already supports `encryption.retired` keys + lazy re-wrap; the
  per-env `id` we stamp makes adding a rotation flow non-breaking.
