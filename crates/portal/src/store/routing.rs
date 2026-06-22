//! A [`FileStore`] that routes calls to one of two backing stores by path prefix.
//!
//! The desktop editor addresses two kinds of volume through the one Files API:
//!   - `/home/...` — the always-available local "home" volume ([`LocalFileStore`]).
//!   - `/Volumes/<catalog>/<schema>/<volume>/...` — Unity Catalog volumes
//!     ([`super::unity::UnityVolumeStore`]).
//!
//! Keeping the prefix dispatch in this one wrapper means the Tauri `files_*`
//! commands and the Connect Files adapter still see a single `dyn FileStore` and
//! need no changes.
//!
//! The home store is handed paths with the `/home` prefix stripped (so it sandboxes
//! against its own root); any path it echoes back in a response (entry/metadata
//! `path` fields) is re-prefixed with `/home` so the UI keeps addressing it the
//! same way.

use std::sync::Arc;

use crate::error::{StoreError, StoreResult};
use crate::proto::files::v1::{DirectoryEntry, DirectoryMetadata, FileMetadata};
use crate::store::{ByteStream, EntryStream, FileStore, ListOpts, Page};

/// The logical prefix for the local home volume.
pub const HOME_PREFIX: &str = "/home";
/// The logical prefix for Unity Catalog volumes.
pub const VOLUMES_PREFIX: &str = "/Volumes";

/// Routes file operations to the home store or the volumes store by path prefix.
pub struct RoutingFileStore {
    home: Arc<dyn FileStore>,
    volumes: Arc<dyn FileStore>,
}

/// Which backing store a path routes to, plus the path as that store expects it.
enum Route {
    /// Home store; path has `/home` stripped (always begins with `/`).
    Home(String),
    /// Volumes store; path passed through unchanged.
    Volumes(String),
}

impl RoutingFileStore {
    pub fn new(home: Arc<dyn FileStore>, volumes: Arc<dyn FileStore>) -> Self {
        Self { home, volumes }
    }

    fn route(&self, path: &str) -> StoreResult<Route> {
        if let Some(rest) = strip_prefix(path, HOME_PREFIX) {
            // `/home` → "/", `/home/queries` → "/queries".
            let inner = if rest.is_empty() { "/".to_string() } else { rest };
            Ok(Route::Home(inner))
        } else if path == VOLUMES_PREFIX || path.starts_with(&format!("{VOLUMES_PREFIX}/")) {
            Ok(Route::Volumes(path.to_string()))
        } else {
            Err(StoreError::InvalidArgument(format!(
                "unrecognized volume path {path:?} (expected {HOME_PREFIX}/… or {VOLUMES_PREFIX}/…)"
            )))
        }
    }
}

/// Strip `prefix` from `path` only at a path boundary: `/home` and `/home/x`
/// match, but `/homely` does not. Returns the remainder (with leading `/` for the
/// sub-path case, or empty for an exact match).
fn strip_prefix(path: &str, prefix: &str) -> Option<String> {
    if path == prefix {
        return Some(String::new());
    }
    path.strip_prefix(prefix)
        .filter(|rest| rest.starts_with('/'))
        .map(str::to_string)
}

/// Re-attach the `/home` prefix to a path the home store echoed back.
fn rehome(inner: &str) -> String {
    let trimmed = inner.trim_start_matches('/');
    if trimmed.is_empty() {
        HOME_PREFIX.to_string()
    } else {
        format!("{HOME_PREFIX}/{trimmed}")
    }
}

#[async_trait::async_trait]
impl FileStore for RoutingFileStore {
    async fn put_file(
        &self,
        path: &str,
        content_type: Option<String>,
        contents: Vec<u8>,
    ) -> StoreResult<FileMetadata> {
        match self.route(path)? {
            Route::Home(p) => {
                let mut meta = self.home.put_file(&p, content_type, contents).await?;
                meta.path = rehome(&meta.path);
                Ok(meta)
            }
            Route::Volumes(p) => self.volumes.put_file(&p, content_type, contents).await,
        }
    }

    async fn put_file_stream(
        &self,
        path: &str,
        content_type: Option<String>,
        chunks: ByteStream,
    ) -> StoreResult<FileMetadata> {
        match self.route(path)? {
            Route::Home(p) => {
                let mut meta = self.home.put_file_stream(&p, content_type, chunks).await?;
                meta.path = rehome(&meta.path);
                Ok(meta)
            }
            Route::Volumes(p) => self.volumes.put_file_stream(&p, content_type, chunks).await,
        }
    }

    async fn read_file(
        &self,
        path: &str,
        offset: Option<i64>,
        length: Option<i64>,
    ) -> StoreResult<Vec<u8>> {
        match self.route(path)? {
            Route::Home(p) => self.home.read_file(&p, offset, length).await,
            Route::Volumes(p) => self.volumes.read_file(&p, offset, length).await,
        }
    }

    async fn read_file_stream(
        &self,
        path: &str,
        offset: Option<i64>,
        length: Option<i64>,
    ) -> StoreResult<ByteStream> {
        match self.route(path)? {
            Route::Home(p) => self.home.read_file_stream(&p, offset, length).await,
            Route::Volumes(p) => self.volumes.read_file_stream(&p, offset, length).await,
        }
    }

    async fn stat_file(&self, path: &str) -> StoreResult<FileMetadata> {
        match self.route(path)? {
            Route::Home(p) => {
                let mut meta = self.home.stat_file(&p).await?;
                meta.path = rehome(&meta.path);
                Ok(meta)
            }
            Route::Volumes(p) => self.volumes.stat_file(&p).await,
        }
    }

    async fn delete_file(&self, path: &str) -> StoreResult<()> {
        match self.route(path)? {
            Route::Home(p) => self.home.delete_file(&p).await,
            Route::Volumes(p) => self.volumes.delete_file(&p).await,
        }
    }

    async fn create_directory(&self, path: &str) -> StoreResult<DirectoryMetadata> {
        match self.route(path)? {
            Route::Home(p) => {
                let mut meta = self.home.create_directory(&p).await?;
                meta.path = rehome(&meta.path);
                Ok(meta)
            }
            Route::Volumes(p) => self.volumes.create_directory(&p).await,
        }
    }

    async fn delete_directory(&self, path: &str) -> StoreResult<()> {
        match self.route(path)? {
            Route::Home(p) => self.home.delete_directory(&p).await,
            Route::Volumes(p) => self.volumes.delete_directory(&p).await,
        }
    }

    async fn stat_directory(&self, path: &str) -> StoreResult<DirectoryMetadata> {
        match self.route(path)? {
            Route::Home(p) => {
                let mut meta = self.home.stat_directory(&p).await?;
                meta.path = rehome(&meta.path);
                Ok(meta)
            }
            Route::Volumes(p) => self.volumes.stat_directory(&p).await,
        }
    }

    async fn list_directory(
        &self,
        path: &str,
        page: Page,
    ) -> StoreResult<(Vec<DirectoryEntry>, Option<String>)> {
        match self.route(path)? {
            Route::Home(p) => {
                let (mut entries, token) = self.home.list_directory(&p, page).await?;
                for e in &mut entries {
                    e.path = rehome(&e.path);
                }
                Ok((entries, token))
            }
            Route::Volumes(p) => self.volumes.list_directory(&p, page).await,
        }
    }

    async fn list_files_opts(&self, path: &str, opts: ListOpts) -> StoreResult<EntryStream> {
        match self.route(path)? {
            Route::Home(p) => {
                use futures::StreamExt;
                let stream = self.home.list_files_opts(&p, opts).await?;
                // Re-home each entry's path as it flows through.
                Ok(stream
                    .map(|item| {
                        item.map(|mut e| {
                            e.path = rehome(&e.path);
                            e
                        })
                    })
                    .boxed())
            }
            Route::Volumes(p) => self.volumes.list_files_opts(&p, opts).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{LocalFileStore, MemoryStore};

    fn routing() -> (tempfile::TempDir, RoutingFileStore) {
        let dir = tempfile::tempdir().unwrap();
        let home = Arc::new(LocalFileStore::new(dir.path()).unwrap());
        // The volumes side is irrelevant for the home-routing tests; a memory
        // store stands in (it won't be hit by `/home` paths).
        let volumes = Arc::new(MemoryStore::new());
        (dir, RoutingFileStore::new(home, volumes))
    }

    #[tokio::test]
    async fn home_paths_round_trip_with_prefix() {
        let (_dir, store) = routing();
        let meta = store
            .put_file("/home/queries/a.sql", None, b"x".to_vec())
            .await
            .unwrap();
        // The response path keeps the /home prefix the UI addressed it by.
        assert_eq!(meta.path, "/home/queries/a.sql");

        let (entries, _) = store
            .list_directory("/home/queries", Page::default())
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "/home/queries/a.sql");

        let bytes = store.read_file("/home/queries/a.sql", None, None).await.unwrap();
        assert_eq!(bytes, b"x");
    }

    #[tokio::test]
    async fn unrecognized_prefix_rejected() {
        let (_dir, store) = routing();
        assert!(matches!(
            store.stat_file("/work/a.sql").await,
            Err(StoreError::InvalidArgument(_))
        ));
        // `/homely` must not match the `/home` prefix.
        assert!(matches!(
            store.stat_file("/homely/a.sql").await,
            Err(StoreError::InvalidArgument(_))
        ));
    }
}
