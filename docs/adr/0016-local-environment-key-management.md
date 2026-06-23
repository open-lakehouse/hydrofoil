# 0016 — Per-environment KEK in the OS keychain for desktop environments

> Status: **Accepted** (2026-06). Implemented in the desktop shell
> (`node/desktop/src-tauri/src/kek.rs`, wired into `lib.rs`) with the UI surface in
> `node/ui/src/components/environment`. Detailed in
> [`docs/security/local-key-management.md`](../security/local-key-management.md). Relates
> to [`0011`](0011-uc-credential-vending-server-token.md) (UC credential vending) and
> [`0015`](0015-client-environment-scope.md) (client-side environment scope).

## Context

A desktop environment runs a local Unity Catalog `uc` sidecar that **envelope-encrypts
storage credentials at rest** under a key-encryption key (KEK). The desktop shell generates
the sidecar's `config.yaml`, and historically wrote the **same hardcoded dev KEK** into
every environment, in plaintext on disk:

```yaml
encryption: { active: { id: dev, key: AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8= } }
```

A shared, source-committed, plaintext-on-disk KEK leaves the credentials it protects
effectively unprotected. We want fresh, per-environment key material managed by a local
secret store, without the key ever touching disk.

Two facts make this clean:
- UC's `KeyConfig.key` is a `ConfigValue` (`#[serde(untagged)]`) accepting either an inline
  base64 string **or** `{ env: VARNAME }` — an env-var indirection
  (`unitycatalog-rs/crates/cli/src/config.rs`).
- The desktop crate is a standalone Tauri crate with no prior crypto deps, and spawns the
  sidecar via `tauri-plugin-shell`, whose command builder supports `.env(...)`.

## Decision

- **One fresh 32-byte (AES-256) KEK per environment**, drawn from the OS CSPRNG
  (`getrandom`) at environment-create time and stored in the **OS-native secret store**
  (`keyring`: macOS Keychain / Windows Credential Manager / Linux Secret Service), keyed by
  service `open-lakehouse` + account = environment id. Resolution is get-or-create and
  idempotent.
- **The KEK never lands on disk.** `config.yaml` references it as
  `key: { env: OPEN_LAKEHOUSE_UC_KEK }`; the shell injects the resolved key into the
  sidecar process env at spawn.
- **A persisted `key.json`** (beside `config.yaml`) records only the provider choice and a
  stable key id (stamped into `encryption.active.id`) — never the secret.
- **OS keychain is hard-required; no silent fallback.** An unreachable keychain yields an
  `unavailable` status that **blocks start** and is surfaced in the UI — we never fall back
  to a shared key. Environment *creation* tolerates an unavailable keychain (created in
  `unavailable` status) so the UI can prompt the user to configure a store.
- **Fresh key, no migration** for environments created under the old dev KEK: they mint a
  new per-env key on next start; secrets sealed under the dev key become unreadable
  (acceptable for local dev).
- **A `remote` key-store provider is modeled and shown in the UI but not wired** — the
  choice cannot yet be committed.

## Consequences

- Stealing an environment's `config.yaml` or catalog file no longer yields a usable KEK;
  keys are isolated per environment and rotatable later (the stamped `id` + UC's
  `encryption.retired` support make rotation non-breaking).
- The desktop crate gains `keyring`, `getrandom`, `base64`. The keychain becomes a
  first-class dependency of starting an environment — hence the explicit `unavailable`
  status and start-gate rather than a degraded fallback.
- `write_uc_config` still **hand-formats** the YAML. This preserves the existing
  snake_case/kebab-case field-casing trap. The intended follow-up is to factor UC's config
  types into a small, dependency-light `unitycatalog-config` crate (the binary-only
  `unitycatalog-cli` pulls the whole server/TUI stack and can't be depended on from the
  standalone desktop crate) and build + `serde_yml::to_string` the real `Config` struct —
  eliminating the trap by construction. Tracked as a cross-repo change in `unitycatalog-rs`.
- The UI now reads key status before start and exposes a configure dialog; the
  framework-agnostic `EnvironmentHost` grows `keyStatus` / `configureKey`, with the web
  build reporting `remote` (server-managed) and treating configuration as a no-op.
