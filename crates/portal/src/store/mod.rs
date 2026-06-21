//! Resource stores backing the portal services.
//!
//! The first pass keeps everything in memory (see [`memory::MemoryStore`]).
//! The trait surface is intentionally small and purpose-built — the generated
//! proto messages are used directly as the stored values.

pub mod memory;

pub use memory::MemoryStore;

use crate::error::StoreResult;
use crate::proto::files::v1::{DirectoryEntry, DirectoryMetadata, FileMetadata};
use crate::proto::tags::v1::{EntityTagAssignment, TagPolicy};

/// A single page request (max results + opaque token).
#[derive(Debug, Default, Clone)]
pub struct Page {
    pub max_results: Option<usize>,
    pub page_token: Option<String>,
}

/// Storage for governed tag definitions and their assignments to entities.
#[async_trait::async_trait]
pub trait TagStore: Send + Sync + 'static {
    // --- tag policies (keyed by tag_key) ---
    async fn create_policy(&self, policy: TagPolicy) -> StoreResult<TagPolicy>;
    async fn get_policy(&self, tag_key: &str) -> StoreResult<TagPolicy>;
    async fn list_policies(&self, page: Page) -> StoreResult<(Vec<TagPolicy>, Option<String>)>;
    async fn update_policy(
        &self,
        tag_key: &str,
        policy: TagPolicy,
        update_mask: &[String],
    ) -> StoreResult<TagPolicy>;
    async fn delete_policy(&self, tag_key: &str) -> StoreResult<()>;

    // --- assignments (keyed by entity_type + entity_name + tag_key) ---
    async fn create_assignment(
        &self,
        assignment: EntityTagAssignment,
    ) -> StoreResult<EntityTagAssignment>;
    async fn get_assignment(
        &self,
        entity_type: &str,
        entity_name: &str,
        tag_key: &str,
    ) -> StoreResult<EntityTagAssignment>;
    async fn list_assignments(
        &self,
        entity_type: &str,
        entity_name: &str,
        page: Page,
    ) -> StoreResult<(Vec<EntityTagAssignment>, Option<String>)>;
    async fn update_assignment(
        &self,
        entity_type: &str,
        entity_name: &str,
        tag_key: &str,
        assignment: EntityTagAssignment,
    ) -> StoreResult<EntityTagAssignment>;
    async fn delete_assignment(
        &self,
        entity_type: &str,
        entity_name: &str,
        tag_key: &str,
    ) -> StoreResult<()>;
}

/// Storage for files and directories, addressed by path.
#[async_trait::async_trait]
pub trait FileStore: Send + Sync + 'static {
    async fn put_file(
        &self,
        path: &str,
        content_type: Option<String>,
        contents: Vec<u8>,
    ) -> StoreResult<FileMetadata>;
    /// Read a (possibly partial) range of a file's bytes.
    async fn read_file(
        &self,
        path: &str,
        offset: Option<i64>,
        length: Option<i64>,
    ) -> StoreResult<Vec<u8>>;
    async fn stat_file(&self, path: &str) -> StoreResult<FileMetadata>;
    async fn delete_file(&self, path: &str) -> StoreResult<()>;

    async fn create_directory(&self, path: &str) -> StoreResult<DirectoryMetadata>;
    async fn delete_directory(&self, path: &str) -> StoreResult<()>;
    async fn stat_directory(&self, path: &str) -> StoreResult<DirectoryMetadata>;
    async fn list_directory(
        &self,
        path: &str,
        page: Page,
    ) -> StoreResult<(Vec<DirectoryEntry>, Option<String>)>;
}
