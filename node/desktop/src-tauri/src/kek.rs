//! Local key management for the per-environment Unity Catalog KEK.
//!
//! The UC sidecar envelope-encrypts storage credentials at rest in its SQLite
//! catalog under a **key-encryption key (KEK)**. The KEK must be 32 bytes
//! (AES-256), supplied to the server base64-encoded. Historically the desktop
//! shell hardcoded a single dev KEK in every generated `config.yaml` — a shared,
//! source-committed, plaintext-on-disk key, which leaves those credentials
//! effectively unprotected.
//!
//! This module mints a **fresh, per-environment** KEK from the OS CSPRNG and
//! stores it in the OS-native secret store. The key is injected into the sidecar
//! via the `OPEN_LAKEHOUSE_UC_KEK` env var (the UC config references it as
//! `{ env: OPEN_LAKEHOUSE_UC_KEK }`), so the material never lands on disk.
//!
//! ## Secret store
//!
//! The default (non-biometric) KEK uses the portable `keyring` crate on every
//! platform — the proven path. On **macOS** an optional **Touch ID** mode stores
//! the same key bytes with a biometric `SecAccessControl` via `security-framework`
//! instead; toggling it moves the key between the two stores without the material
//! ever changing (no rotation/re-encryption). On **Windows / Linux** there is no
//! biometric path and the Touch ID toggle reports it as unsupported.
//!
//! ### Touch ID requires a signed app
//!
//! Attaching a `kSecAttrAccessControl` flag to a keychain item requires the app to
//! be code-signed with a `keychain-access-groups` entitlement — **even on the
//! legacy keychain**. Any unsigned build (all `cargo`/dev builds, and the current
//! packaged app, which has no signing config) fails the biometric write with
//! `errSecMissingEntitlement` (-34018). Until the desktop app ships with signing +
//! that entitlement, enabling Touch ID surfaces a clear "needs a signed build"
//! message and leaves the existing plain key untouched. See [`store::biometric`].
//!
//! ## Records and immutability
//!
//! A small persisted record (`key.json`, sibling to `config.yaml` in the env's UC
//! dir) tracks the *provider choice*, *key id*, and whether *biometric* protection
//! is on — never the secret itself — so the UI can show key status without
//! starting the environment, and `write_uc_config` can stamp a stable `id:` into
//! every sealed secret (enabling future rotation).
//!
//! The key material is **minted once** and never regenerated: there is no
//! rotation/re-encryption path, so the storage provider is locked after creation
//! and only the biometric flag is mutable.
//!
//! Hard requirement: the OS keychain is the trust anchor. If it is unavailable we
//! surface an error / `Unavailable` status — we never silently fall back to a
//! shared plaintext key.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The keychain "service" all environment KEKs live under. The per-environment
/// id is the "account" within this service.
const KEYCHAIN_SERVICE: &str = "open-lakehouse";

/// The env var the spawned UC sidecar resolves its KEK from (matched by the
/// `key: { env: OPEN_LAKEHOUSE_UC_KEK }` reference written into `config.yaml`).
pub const KEK_ENV_VAR: &str = "OPEN_LAKEHOUSE_UC_KEK";

/// Where the KEK material is kept. `keychain` stores it in the OS secret store;
/// `remote` defers to an external key store (scaffolded — not yet wired).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyProvider {
    #[default]
    Keychain,
    Remote,
}

/// The persisted per-environment key record (`key.json`). Holds only the provider
/// choice, the stable key id, and the biometric flag — **never** the secret
/// material.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyConfig {
    pub provider: KeyProvider,
    /// Stable identifier stamped into `config.yaml`'s `encryption.active.id` and
    /// recorded in every sealed secret. Defaults to the environment id.
    #[serde(rename = "keyId")]
    pub key_id: String,
    /// Whether the keychain item is gated behind Touch ID (macOS only). Orthogonal
    /// to the provider: the same key bytes can be (un)protected in place. Older
    /// records predate this field, hence `default`.
    #[serde(default)]
    pub biometric: bool,
    /// Reserved for remote-store configuration (endpoint / key reference). `None`
    /// today — the remote provider is scaffolding.
    #[serde(default)]
    pub remote: Option<serde_json::Value>,
}

/// Key status the UI can render without starting the environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyStatus {
    /// No `key.json` and no keychain entry yet (freshly created / legacy env).
    Unconfigured,
    /// Provider is the OS keychain and the key exists (or can be created).
    Keychain,
    /// Provider is the OS keychain and the key is additionally gated behind
    /// Touch ID. Reads (i.e. environment starts) prompt for biometry every time.
    #[serde(rename = "keychain-biometric")]
    KeychainBiometric,
    /// Provider is a remote key store.
    Remote,
    /// Provider is the keychain but the OS secret store can't be reached
    /// (e.g. headless Linux with no Secret Service). The environment cannot
    /// start until this is resolved or a different provider is chosen.
    Unavailable,
}

/// Path to the persisted key record for an environment, given its UC data dir
/// (`.open-lakehouse/envs/<id>/uc`). Sits beside `config.yaml`.
pub fn key_config_path(uc_dir: &Path) -> PathBuf {
    uc_dir.join("key.json")
}

/// Read the persisted key record, or `None` when absent / unparsable.
pub fn read_key_config(uc_dir: &Path) -> Option<KeyConfig> {
    let bytes = std::fs::read(key_config_path(uc_dir)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Persist the key record (`key.json`), creating the UC dir if needed.
fn write_key_config(uc_dir: &Path, cfg: &KeyConfig) -> Result<(), String> {
    std::fs::create_dir_all(uc_dir).map_err(|e| format!("creating {uc_dir:?}: {e}"))?;
    let json = serde_json::to_vec_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(key_config_path(uc_dir), json).map_err(|e| format!("writing key.json: {e}"))
}

/// Generate a fresh 32-byte (AES-256) KEK from the OS CSPRNG, base64-encoded in
/// the form the UC server expects.
fn generate_kek() -> Result<String, String> {
    use base64::Engine as _;
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|e| format!("CSPRNG unavailable: {e}"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Platform-specific keychain backend. The public API in this module is written
/// against these three operations; the rest (records, mint-once, status) is
/// platform-agnostic.
mod store {
    use super::KEYCHAIN_SERVICE;

    /// Outcome of a read attempt: the stored secret, a definitive "no such item",
    /// or a backend error (store unreachable / biometry cancelled).
    pub enum Read {
        Found(String),
        Missing,
        Error(String),
    }

    /// Whether this platform can gate an item behind biometry at all. The UI uses
    /// this to enable/disable the Touch ID switch.
    pub const fn biometric_supported() -> bool {
        cfg!(target_os = "macos")
    }

    /// Read the KEK for `env_id`. Tries the biometric store first on macOS (which
    /// prompts for Touch ID when an item is there), then the plain store, so a key
    /// is found regardless of which protection it currently carries.
    pub fn read(env_id: &str) -> Read {
        #[cfg(target_os = "macos")]
        match biometric::read(env_id) {
            Read::Missing => {}
            other => return other,
        }
        plain::read(env_id)
    }

    /// Write `kek` for `env_id` with no extra protection (the portable path on all
    /// platforms). The biometric variant lives in [`set_biometric_protection`].
    pub fn write(env_id: &str, kek: &str) -> Result<(), String> {
        plain::write(env_id, kek)
    }

    /// Whether a *plain* (non-biometric) KEK exists for `env_id`, with no decrypt.
    /// Used by status probing, which must never trigger a Touch ID prompt — for a
    /// biometric key the caller trusts the persisted record instead of reading it.
    pub fn plain_exists(env_id: &str) -> Read {
        plain::read(env_id)
    }

    /// Best-effort removal of the KEK for `env_id` from every store it might be in.
    pub fn delete(env_id: &str) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        biometric::delete(env_id)?;
        plain::delete(env_id)
    }

    /// macOS error code surfaced when a biometric `kSecAttrAccessControl` write is
    /// attempted by a binary lacking the `keychain-access-groups` entitlement
    /// (i.e. any unsigned build). `Err(MISSING_ENTITLEMENT_MARKER…)` is recognized
    /// by the caller to show a "needs a signed build" message.
    pub const MISSING_ENTITLEMENT: &str = "MISSING_KEYCHAIN_ENTITLEMENT";

    /// Move the KEK for `env_id` into (or out of) the biometric-protected store,
    /// rewriting the same `kek` bytes. macOS only — errors elsewhere. On enabling,
    /// the plain copy is removed only after the biometric write succeeds, so a
    /// failure (e.g. missing entitlement) leaves the existing plain key intact.
    #[cfg(target_os = "macos")]
    pub fn set_biometric_protection(env_id: &str, kek: &str, enabled: bool) -> Result<(), String> {
        if enabled {
            biometric::write(env_id, kek)?;
            // Now that the protected copy exists, drop the plain one.
            plain::delete(env_id)?;
        } else {
            plain::write(env_id, kek)?;
            biometric::delete(env_id)?;
        }
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    pub fn set_biometric_protection(_env_id: &str, _kek: &str, _enabled: bool) -> Result<(), String> {
        Err("biometric protection is not supported on this platform".into())
    }

    /// The plain (no access control) store, used on every platform for
    /// non-biometric KEKs. `keyring` keeps this portable and unchanged from before
    /// the Touch ID work.
    mod plain {
        use super::{Read, KEYCHAIN_SERVICE};

        fn entry(env_id: &str) -> Result<keyring::Entry, String> {
            keyring::Entry::new(KEYCHAIN_SERVICE, env_id)
                .map_err(|e| format!("opening keychain entry for {env_id}: {e}"))
        }

        pub fn read(env_id: &str) -> Read {
            let entry = match entry(env_id) {
                Ok(e) => e,
                Err(e) => return Read::Error(e),
            };
            match entry.get_password() {
                Ok(kek) => Read::Found(kek),
                Err(keyring::Error::NoEntry) => Read::Missing,
                Err(e) => Read::Error(format!("reading KEK from keychain for {env_id}: {e}")),
            }
        }

        pub fn write(env_id: &str, kek: &str) -> Result<(), String> {
            entry(env_id)?
                .set_password(kek)
                .map_err(|e| format!("storing KEK in keychain for {env_id}: {e}"))
        }

        pub fn delete(env_id: &str) -> Result<(), String> {
            match entry(env_id) {
                Ok(entry) => match entry.delete_credential() {
                    Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                    Err(e) => Err(format!("deleting keychain entry for {env_id}: {e}")),
                },
                Err(e) => Err(e),
            }
        }
    }

    /// The biometric (Touch ID) store on macOS. Items here carry a
    /// `kSecAttrAccessControl` flag, which the OS enforces on read.
    ///
    /// IMPORTANT: attaching that flag requires the app to be code-signed with a
    /// `keychain-access-groups` entitlement (true even for the legacy keychain) —
    /// any unsigned build fails the write with `errSecMissingEntitlement` (-34018).
    /// Until the desktop app ships with signing + that entitlement, enabling Touch
    /// ID surfaces a clear "needs a signed build" message rather than a raw error.
    #[cfg(target_os = "macos")]
    mod biometric {
        use super::{Read, KEYCHAIN_SERVICE, MISSING_ENTITLEMENT};
        use security_framework::passwords::{
            delete_generic_password_options, generic_password, set_generic_password_options,
            AccessControlOptions, PasswordOptions,
        };

        const ERR_ITEM_NOT_FOUND: i32 = -25300;
        const ERR_MISSING_ENTITLEMENT: i32 = -34018;

        fn options(env_id: &str) -> PasswordOptions {
            PasswordOptions::new_generic_password(KEYCHAIN_SERVICE, env_id)
        }

        pub fn read(env_id: &str) -> Read {
            match generic_password(options(env_id)) {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(s) => Read::Found(s),
                    Err(e) => Read::Error(format!("KEK for {env_id} is not valid UTF-8: {e}")),
                },
                Err(e) if e.code() == ERR_ITEM_NOT_FOUND => Read::Missing,
                Err(e) => Read::Error(format!("reading biometric KEK for {env_id}: {e}")),
            }
        }

        pub fn write(env_id: &str, kek: &str) -> Result<(), String> {
            let mut opts = options(env_id);
            // Touch ID / Face ID for the *currently enrolled* set, with a
            // device-passcode fallback so a user whose biometry isn't working can
            // still unlock. `BIOMETRY_CURRENT_SET` invalidates the item if the
            // enrolled fingerprints change — acceptable, since we can re-mint.
            opts.set_access_control_options(
                AccessControlOptions::BIOMETRY_CURRENT_SET
                    | AccessControlOptions::DEVICE_PASSCODE
                    | AccessControlOptions::OR,
            );
            set_generic_password_options(kek.as_bytes(), opts).map_err(|e| {
                if e.code() == ERR_MISSING_ENTITLEMENT {
                    MISSING_ENTITLEMENT.to_string()
                } else {
                    format!("storing biometric KEK for {env_id}: {e}")
                }
            })
        }

        pub fn delete(env_id: &str) -> Result<(), String> {
            match delete_generic_password_options(options(env_id)) {
                Ok(()) => Ok(()),
                Err(e) if e.code() == ERR_ITEM_NOT_FOUND => Ok(()),
                // A missing entitlement means we could never have written here, so
                // there's nothing to delete — treat as success.
                Err(e) if e.code() == ERR_MISSING_ENTITLEMENT => Ok(()),
                Err(e) => Err(format!("deleting biometric KEK for {env_id}: {e}")),
            }
        }
    }
}

/// Get-or-create the keychain-stored KEK for an environment. Idempotent: the same
/// environment always resolves to the same key across restarts — the material is
/// **minted once** and never regenerated. New keys are written with the biometric
/// flag from `key.json` (default off). Surfaces keychain errors rather than
/// falling back to a shared key.
pub fn ensure_kek(env_id: &str, _uc_dir: &Path) -> Result<String, String> {
    match store::read(env_id) {
        store::Read::Found(kek) => Ok(kek),
        store::Read::Missing => {
            // Mint into the plain store. Biometric protection (if requested) is
            // applied as a separate in-place move via `set_biometric`, so minting
            // never depends on the keychain entitlement that biometry needs.
            let kek = generate_kek()?;
            store::write(env_id, &kek)?;
            Ok(kek)
        }
        store::Read::Error(e) => Err(e),
    }
}

/// Resolve the current key status for an environment without starting it. Does not
/// force a biometric read: the record's `biometric` flag is the source of truth
/// for whether Touch ID is on, and we only probe the store for presence.
pub fn status(env_id: &str, uc_dir: &Path) -> KeyStatus {
    let cfg = read_key_config(uc_dir);
    match cfg.as_ref().map(|c| c.provider) {
        Some(KeyProvider::Remote) => KeyStatus::Remote,
        Some(KeyProvider::Keychain) | None => {
            // A biometric key lives in the protected store, which prompts on every
            // read — so we must NOT probe it just to render status. The persisted
            // record is the source of truth that Touch ID is on; trust it.
            if cfg.as_ref().map(|c| c.biometric).unwrap_or(false) {
                return KeyStatus::KeychainBiometric;
            }
            match store::plain_exists(env_id) {
                store::Read::Found(_) => KeyStatus::Keychain,
                store::Read::Missing => KeyStatus::Unconfigured,
                store::Read::Error(_) => KeyStatus::Unavailable,
            }
        }
    }
}

/// Configure the key provider for an environment, persisting `key.json`. For the
/// keychain provider, eagerly mints the KEK so a broken keychain surfaces here
/// (at configure time) rather than at start time. Returns the resulting status.
///
/// The key material is minted once; this is intended for *initial* provisioning.
/// Changing the provider after a key exists is rejected at the command layer.
pub fn configure(env_id: &str, uc_dir: &Path, provider: KeyProvider) -> Result<KeyStatus, String> {
    let biometric = read_key_config(uc_dir).map(|c| c.biometric).unwrap_or(false);
    match provider {
        KeyProvider::Keychain => {
            // Mint (or confirm) the key first; if this fails the keychain is
            // unavailable — report it without persisting a keychain choice that
            // can't be honored.
            ensure_kek(env_id, uc_dir)?;
        }
        KeyProvider::Remote => {
            // Remote wrap/unwrap is not wired yet; the choice is persisted so the
            // UI reflects it, but the provider has no concrete backend.
        }
    }
    let cfg = KeyConfig {
        provider,
        key_id: env_id.to_string(),
        biometric,
        remote: None,
    };
    write_key_config(uc_dir, &cfg)?;
    Ok(status(env_id, uc_dir))
}

/// Turn Touch ID protection on or off for a keychain-stored KEK, in place. Reads
/// the existing key bytes (prompting once if it is currently biometric), rewrites
/// the same item with/without the biometric access-control flag, and records the
/// new state in `key.json`. The key material is unchanged, so already-sealed
/// credentials keep decrypting — no rotation involved.
///
/// Only valid for the keychain provider on a platform that supports biometry.
pub fn set_biometric(env_id: &str, uc_dir: &Path, enabled: bool) -> Result<KeyStatus, String> {
    if enabled && !store::biometric_supported() {
        return Err("biometric protection is not supported on this platform".into());
    }
    let cfg = read_key_config(uc_dir);
    if matches!(cfg.as_ref().map(|c| c.provider), Some(KeyProvider::Remote)) {
        return Err("biometric protection applies to keychain-stored keys only".into());
    }

    // Obtain the current key bytes (mints on first use; prompts if already
    // biometric), then move the same bytes into/out of the biometric store.
    let kek = ensure_kek(env_id, uc_dir)?;
    store::set_biometric_protection(env_id, &kek, enabled).map_err(|e| {
        // Unsigned builds can't attach a biometric access control to a keychain
        // item. Surface that as actionable guidance instead of a raw OS error.
        if e == store::MISSING_ENTITLEMENT {
            "Touch ID requires a code-signed build of the app (a `keychain-access-groups` \
             entitlement). It can't be enabled in this unsigned build."
                .to_string()
        } else {
            e
        }
    })?;

    let cfg = KeyConfig {
        provider: KeyProvider::Keychain,
        key_id: cfg.map(|c| c.key_id).unwrap_or_else(|| env_id.to_string()),
        biometric: enabled,
        remote: None,
    };
    write_key_config(uc_dir, &cfg)?;
    Ok(status(env_id, uc_dir))
}

/// Best-effort removal of an environment's key material and record. Used when an
/// environment is deleted. Ignores a missing keychain entry.
#[allow(dead_code)] // wired in once a delete_environment command exists
pub fn delete_key(env_id: &str, uc_dir: &Path) {
    if let Err(e) = store::delete(env_id) {
        eprintln!("[kek] {e}");
    }
    let _ = std::fs::remove_file(key_config_path(uc_dir));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip the plain (non-biometric) keychain store, which every platform
    /// uses for the default KEK: a freshly minted key reads back identically across
    /// "restarts", mint-once never regenerates, and delete is idempotent. The
    /// biometric store is exercised separately and only in a signed build (its
    /// write needs the keychain entitlement), so it isn't covered here.
    #[test]
    fn store_roundtrip_and_mint_once() {
        // A test-only env id so we never collide with a real environment's item.
        let env_id = "test-kek-roundtrip-DO-NOT-USE";
        // Clean any leftover from a previously failed run.
        let _ = store::delete(env_id);

        // Missing before first write.
        assert!(matches!(store::read(env_id), store::Read::Missing));

        // Write then read back the exact bytes.
        let kek = generate_kek().expect("generate");
        store::write(env_id, &kek).expect("write");
        match store::read(env_id) {
            store::Read::Found(got) => assert_eq!(got, kek),
            other => panic!(
                "expected Found, got a different read outcome: {}",
                match other {
                    store::Read::Missing => "Missing",
                    store::Read::Error(_) => "Error",
                    store::Read::Found(_) => unreachable!(),
                }
            ),
        }

        // ensure_kek with an existing item returns it unchanged (mint-once): use a
        // temp uc_dir so key.json side effects don't escape the test.
        let dir = std::env::temp_dir().join(env_id);
        let again = ensure_kek(env_id, &dir).expect("ensure_kek");
        assert_eq!(again, kek, "ensure_kek must not regenerate an existing key");

        // Delete is real, then idempotent.
        store::delete(env_id).expect("delete");
        assert!(matches!(store::read(env_id), store::Read::Missing));
        store::delete(env_id).expect("delete of missing item is ok");
    }
}
