//! ConnectRPC service implementations, backed by a [`crate::store`].
//!
//! Each generated service trait is implemented for [`AppState`]. Handlers read
//! request fields off the zero-copy view, copy out owned data before crossing
//! an `.await`, delegate to the store, and return owned response messages.

mod files;
mod tag_assignments;
mod tag_policies;

use std::sync::Arc;

use connectrpc::Router;

use crate::services::files::v1::FilesServiceExt;
use crate::services::tags::v1::{EntityTagAssignmentsServiceExt, TagPoliciesServiceExt};
use crate::store::{FileStore, TagStore};

/// Shared, cheaply-cloneable handler state. Holds the backing stores; one value
/// implements all three portal service traits. Files and tags are backed
/// independently so the files backend (e.g. Unity Catalog volumes) can be swapped
/// without affecting tag storage.
#[derive(Clone)]
pub struct AppState {
    pub(crate) files: Arc<dyn FileStore>,
    pub(crate) tags: Arc<dyn TagStore>,
}

impl AppState {
    pub fn new(files: Arc<dyn FileStore>, tags: Arc<dyn TagStore>) -> Self {
        Self { files, tags }
    }

    /// Register all portal services onto a ConnectRPC router.
    pub fn register_all(self, router: Router) -> Router {
        let state = Arc::new(self);
        let router = TagPoliciesServiceExt::register(Arc::clone(&state), router);
        let router = EntityTagAssignmentsServiceExt::register(Arc::clone(&state), router);
        FilesServiceExt::register(state, router)
    }
}
