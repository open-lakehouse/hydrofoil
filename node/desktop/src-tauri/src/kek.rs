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
//! stores it in the OS-native secret store (`keyring`: macOS Keychain, Windows
//! Credential Manager, Linux Secret Service). The key is injected into the
//! sidecar via the `OPEN_LAKEHOUSE_UC_KEK` env var (the UC config references it as
//! `{ env: OPEN_LAKEHOUSE_UC_KEK }`), so the material never lands on disk.
//!
//! A small persisted record (`key.json`, sibling to `config.yaml` in the env's UC
//! dir) tracks the *provider choice* and *key id* — never the secret itself — so
//! the UI can show key status without starting the environment, and `write_uc_config`
//! can stamp a stable `id:` into every sealed secret (enabling future rotation).
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyProvider {
    Keychain,
    Remote,
}

impl Default for KeyProvider {
    fn default() -> Self {
        KeyProvider::Keychain
    }
}

/// The persisted per-environment key record (`key.json`). Holds only the provider
/// choice and the stable key id — **never** the secret material.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyConfig {
    pub provider: KeyProvider,
    /// Stable identifier stamped into `config.yaml`'s `encryption.active.id` and
    /// recorded in every sealed secret. Defaults to the environment id.
    #[serde(rename = "keyId")]
    pub key_id: String,
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

/// Build the keychain entry for an environment (service `open-lakehouse`, account
/// = env id).
fn keychain_entry(env_id: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, env_id)
        .map_err(|e| format!("opening keychain entry for {env_id}: {e}"))
}

/// Generate a fresh 32-byte (AES-256) KEK from the OS CSPRNG, base64-encoded in
/// the form the UC server expects.
fn generate_kek() -> Result<String, String> {
    use base64::Engine as _;
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|e| format!("CSPRNG unavailable: {e}"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Get-or-create the keychain-stored KEK for an environment. Idempotent: the same
/// environment always resolves to the same key across restarts. Surfaces keychain
/// errors rather than falling back to a shared key.
pub fn ensure_kek(env_id: &str) -> Result<String, String> {
    let entry = keychain_entry(env_id)?;
    match entry.get_password() {
        Ok(kek) => Ok(kek),
        Err(keyring::Error::NoEntry) => {
            let kek = generate_kek()?;
            entry
                .set_password(&kek)
                .map_err(|e| format!("storing KEK in keychain for {env_id}: {e}"))?;
            Ok(kek)
        }
        Err(e) => Err(format!("reading KEK from keychain for {env_id}: {e}")),
    }
}

/// Resolve the current key status for an environment without starting it.
pub fn status(env_id: &str, uc_dir: &Path) -> KeyStatus {
    match read_key_config(uc_dir).map(|c| c.provider) {
        Some(KeyProvider::Remote) => KeyStatus::Remote,
        Some(KeyProvider::Keychain) | None => match keychain_entry(env_id) {
            // Probe the entry: present → configured; absent → not yet minted;
            // backend error → the OS secret store is unreachable.
            Ok(entry) => match entry.get_password() {
                Ok(_) => KeyStatus::Keychain,
                Err(keyring::Error::NoEntry) => KeyStatus::Unconfigured,
                Err(_) => KeyStatus::Unavailable,
            },
            Err(_) => KeyStatus::Unavailable,
        },
    }
}

/// Configure the key provider for an environment, persisting `key.json`. For the
/// keychain provider, eagerly mints the KEK so a broken keychain surfaces here
/// (at configure time) rather than at start time. Returns the resulting status.
pub fn configure(env_id: &str, uc_dir: &Path, provider: KeyProvider) -> Result<KeyStatus, String> {
    match provider {
        KeyProvider::Keychain => {
            // Mint (or confirm) the key first; if this fails the keychain is
            // unavailable — report it without persisting a keychain choice that
            // can't be honored.
            ensure_kek(env_id)?;
        }
        KeyProvider::Remote => {
            // Remote wrap/unwrap is not wired yet; the choice is persisted so the
            // UI reflects it, but the provider has no concrete backend.
        }
    }
    let cfg = KeyConfig {
        provider,
        key_id: env_id.to_string(),
        remote: None,
    };
    write_key_config(uc_dir, &cfg)?;
    Ok(status(env_id, uc_dir))
}

/// Best-effort removal of an environment's key material and record. Used when an
/// environment is deleted. Ignores a missing keychain entry.
#[allow(dead_code)] // wired in once a delete_environment command exists
pub fn delete_key(env_id: &str, uc_dir: &Path) {
    if let Ok(entry) = keychain_entry(env_id) {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => eprintln!("[kek] deleting keychain entry for {env_id}: {e}"),
        }
    }
    let _ = std::fs::remove_file(key_config_path(uc_dir));
}
