//! App data-dir layout: where the environments registry and per-environment data
//! live on disk. Centralized here because several modules ([`crate::env`],
//! [`crate::uc`], [`crate::modules`], [`crate::notebook`]) resolve paths under the
//! same root.

use std::path::PathBuf;

/// The app working directory (gitignored, in-repo for this iteration so it's
/// inspectable): holds `environments.json` and per-environment data under
/// `envs/<id>/`.
///
/// .../node/desktop/src-tauri/../.open-lakehouse → node/desktop/.open-lakehouse
pub(crate) fn app_data_dir() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.open-lakehouse")
}

/// The UC data dir for a given environment: `.open-lakehouse/envs/<id>/uc`,
/// holding `config.yaml`, `catalog.db`, and `storage/`.
pub(crate) fn env_uc_dir(id: &str) -> PathBuf {
    app_data_dir().join("envs").join(id).join("uc")
}

/// The local "home" volume dir for an environment: `.open-lakehouse/envs/<id>/home`.
/// Backs the editor's always-available home volume (served as `/home/...`).
pub(crate) fn env_home_dir(id: &str) -> PathBuf {
    app_data_dir().join("envs").join(id).join("home")
}
