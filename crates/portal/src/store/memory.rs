//! In-memory implementation of the portal stores.
//!
//! Backed by `RwLock`-guarded maps. State is process-local and lost on restart;
//! this exists so the service runs end-to-end before a durable backend lands.

use std::collections::BTreeMap;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use futures::StreamExt;

use crate::error::{StoreError, StoreResult};
use crate::proto::files::v1::{DirectoryEntry, DirectoryMetadata, FileMetadata};
use crate::proto::tags::v1::{EntityTagAssignment, TagPolicy};
use crate::store::{ByteStream, EntryStream, FileStore, ListOpts, Page, TagStore, paginate};

/// Composite key for an entity tag assignment.
type AssignmentKey = (String, String, String);

#[derive(Default)]
struct StoredFile {
    content_type: String,
    last_modified: i64,
    bytes: Vec<u8>,
}

/// Process-local store for tags and files.
#[derive(Default)]
pub struct MemoryStore {
    policies: RwLock<BTreeMap<String, TagPolicy>>,
    assignments: RwLock<BTreeMap<AssignmentKey, EntityTagAssignment>>,
    files: RwLock<BTreeMap<String, StoredFile>>,
    directories: RwLock<BTreeMap<String, i64>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Current time in epoch milliseconds. Defined at call time (not const) — fine
/// for a running service; tests do not assert on exact timestamps.
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[async_trait::async_trait]
impl TagStore for MemoryStore {
    async fn create_policy(&self, mut policy: TagPolicy) -> StoreResult<TagPolicy> {
        if policy.tag_key.is_empty() {
            return Err(StoreError::InvalidArgument("tag_key is required".into()));
        }
        let mut policies = self.policies.write().unwrap();
        if policies.contains_key(&policy.tag_key) {
            return Err(StoreError::AlreadyExists(format!(
                "tag policy {:?}",
                policy.tag_key
            )));
        }
        let ts = now_millis();
        policy.id = Some(new_id());
        policy.created_at = Some(ts);
        policy.updated_at = Some(ts);
        policies.insert(policy.tag_key.clone(), policy.clone());
        Ok(policy)
    }

    async fn get_policy(&self, tag_key: &str) -> StoreResult<TagPolicy> {
        self.policies
            .read()
            .unwrap()
            .get(tag_key)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("tag policy {tag_key:?}")))
    }

    async fn list_policies(&self, page: Page) -> StoreResult<(Vec<TagPolicy>, Option<String>)> {
        let policies = self.policies.read().unwrap();
        // BTreeMap iterates in key order, giving a stable page boundary.
        let all: Vec<TagPolicy> = policies.values().cloned().collect();
        Ok(paginate(all, &page))
    }

    async fn update_policy(
        &self,
        tag_key: &str,
        policy: TagPolicy,
        update_mask: &[String],
    ) -> StoreResult<TagPolicy> {
        let mut policies = self.policies.write().unwrap();
        let existing = policies
            .get_mut(tag_key)
            .ok_or_else(|| StoreError::NotFound(format!("tag policy {tag_key:?}")))?;

        // Empty mask => full replace of the mutable fields. Otherwise only the
        // named fields are touched. tag_key/id/created_at are immutable here.
        let touch = |field: &str| update_mask.is_empty() || update_mask.iter().any(|m| m == field);
        if touch("description") {
            existing.description = policy.description.clone();
        }
        if touch("values") {
            existing.values = policy.values.clone();
        }
        existing.updated_at = Some(now_millis());
        Ok(existing.clone())
    }

    async fn delete_policy(&self, tag_key: &str) -> StoreResult<()> {
        self.policies
            .write()
            .unwrap()
            .remove(tag_key)
            .map(|_| ())
            .ok_or_else(|| StoreError::NotFound(format!("tag policy {tag_key:?}")))
    }

    async fn create_assignment(
        &self,
        assignment: EntityTagAssignment,
    ) -> StoreResult<EntityTagAssignment> {
        if assignment.entity_type.is_empty()
            || assignment.entity_name.is_empty()
            || assignment.tag_key.is_empty()
        {
            return Err(StoreError::InvalidArgument(
                "entity_type, entity_name and tag_key are required".into(),
            ));
        }
        let key = (
            assignment.entity_type.clone(),
            assignment.entity_name.clone(),
            assignment.tag_key.clone(),
        );
        let mut assignments = self.assignments.write().unwrap();
        if assignments.contains_key(&key) {
            return Err(StoreError::AlreadyExists(format!("tag assignment {key:?}")));
        }
        assignments.insert(key, assignment.clone());
        Ok(assignment)
    }

    async fn get_assignment(
        &self,
        entity_type: &str,
        entity_name: &str,
        tag_key: &str,
    ) -> StoreResult<EntityTagAssignment> {
        let key = (
            entity_type.to_string(),
            entity_name.to_string(),
            tag_key.to_string(),
        );
        self.assignments
            .read()
            .unwrap()
            .get(&key)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("tag assignment {key:?}")))
    }

    async fn list_assignments(
        &self,
        entity_type: &str,
        entity_name: &str,
        page: Page,
    ) -> StoreResult<(Vec<EntityTagAssignment>, Option<String>)> {
        let assignments = self.assignments.read().unwrap();
        let matching: Vec<EntityTagAssignment> = assignments
            .iter()
            .filter(|((et, en, _), _)| et == entity_type && en == entity_name)
            .map(|(_, v)| v.clone())
            .collect();
        Ok(paginate(matching, &page))
    }

    async fn update_assignment(
        &self,
        entity_type: &str,
        entity_name: &str,
        tag_key: &str,
        assignment: EntityTagAssignment,
    ) -> StoreResult<EntityTagAssignment> {
        let key = (
            entity_type.to_string(),
            entity_name.to_string(),
            tag_key.to_string(),
        );
        let mut assignments = self.assignments.write().unwrap();
        let existing = assignments
            .get_mut(&key)
            .ok_or_else(|| StoreError::NotFound(format!("tag assignment {key:?}")))?;
        existing.tag_value = assignment.tag_value.clone();
        Ok(existing.clone())
    }

    async fn delete_assignment(
        &self,
        entity_type: &str,
        entity_name: &str,
        tag_key: &str,
    ) -> StoreResult<()> {
        let key = (
            entity_type.to_string(),
            entity_name.to_string(),
            tag_key.to_string(),
        );
        self.assignments
            .write()
            .unwrap()
            .remove(&key)
            .map(|_| ())
            .ok_or_else(|| StoreError::NotFound(format!("tag assignment {key:?}")))
    }
}

#[async_trait::async_trait]
impl FileStore for MemoryStore {
    async fn put_file(
        &self,
        path: &str,
        content_type: Option<String>,
        contents: Vec<u8>,
    ) -> StoreResult<FileMetadata> {
        if path.is_empty() {
            return Err(StoreError::InvalidArgument("path is required".into()));
        }
        let ts = now_millis();
        let file = StoredFile {
            content_type: content_type.unwrap_or_else(|| "application/octet-stream".into()),
            last_modified: ts,
            bytes: contents,
        };
        let meta = file_metadata(path, &file);
        self.files.write().unwrap().insert(path.to_string(), file);
        Ok(meta)
    }

    async fn put_file_stream(
        &self,
        path: &str,
        content_type: Option<String>,
        mut chunks: ByteStream,
    ) -> StoreResult<FileMetadata> {
        // The in-memory backing has no multipart sink, so collect the stream and
        // delegate to `put_file`. (The streaming win lives in the cloud-backed
        // `UnityVolumeStore`; this keeps the trait object usable in tests.)
        let mut contents = Vec::new();
        while let Some(chunk) = chunks.next().await {
            contents.extend_from_slice(&chunk?);
        }
        self.put_file(path, content_type, contents).await
    }

    async fn read_file(
        &self,
        path: &str,
        offset: Option<i64>,
        length: Option<i64>,
    ) -> StoreResult<Vec<u8>> {
        let files = self.files.read().unwrap();
        let file = files
            .get(path)
            .ok_or_else(|| StoreError::NotFound(format!("file {path:?}")))?;
        let start = offset.unwrap_or(0).max(0) as usize;
        if start > file.bytes.len() {
            return Err(StoreError::InvalidArgument(format!(
                "offset {start} is past end of file ({} bytes)",
                file.bytes.len()
            )));
        }
        let end = match length {
            Some(len) if len >= 0 => (start + len as usize).min(file.bytes.len()),
            _ => file.bytes.len(),
        };
        Ok(file.bytes[start..end].to_vec())
    }

    async fn read_file_stream(
        &self,
        path: &str,
        offset: Option<i64>,
        length: Option<i64>,
    ) -> StoreResult<ByteStream> {
        // The bytes already live in memory, so there's nothing to stream from a
        // backend: read the range and hand it back as a single chunk. (The trait
        // contract is satisfied; the streaming benefit is real only for the
        // object-store-backed `UnityVolumeStore`.)
        let bytes = self.read_file(path, offset, length).await?;
        Ok(futures::stream::once(async move { Ok(Bytes::from(bytes)) }).boxed())
    }

    async fn stat_file(&self, path: &str) -> StoreResult<FileMetadata> {
        let files = self.files.read().unwrap();
        files
            .get(path)
            .map(|f| file_metadata(path, f))
            .ok_or_else(|| StoreError::NotFound(format!("file {path:?}")))
    }

    async fn delete_file(&self, path: &str) -> StoreResult<()> {
        self.files
            .write()
            .unwrap()
            .remove(path)
            .map(|_| ())
            .ok_or_else(|| StoreError::NotFound(format!("file {path:?}")))
    }

    async fn create_directory(&self, path: &str) -> StoreResult<DirectoryMetadata> {
        if path.is_empty() {
            return Err(StoreError::InvalidArgument("path is required".into()));
        }
        let ts = now_millis();
        self.directories
            .write()
            .unwrap()
            .insert(path.to_string(), ts);
        Ok(DirectoryMetadata {
            path: path.to_string(),
            last_modified: ts,
            ..Default::default()
        })
    }

    async fn delete_directory(&self, path: &str) -> StoreResult<()> {
        // Databricks semantics: only an empty directory may be deleted; a
        // non-empty one is rejected (the caller must delete its contents first).
        let prefix = if path.ends_with('/') {
            path.to_string()
        } else {
            format!("{path}/")
        };
        let has_child_file = self
            .files
            .read()
            .unwrap()
            .keys()
            .any(|p| p.starts_with(&prefix));
        let has_child_dir = self
            .directories
            .read()
            .unwrap()
            .keys()
            .any(|p| p.as_str() != path && p.starts_with(&prefix));
        if has_child_file || has_child_dir {
            return Err(StoreError::FailedPrecondition(format!(
                "directory {path:?} is not empty; delete its contents first"
            )));
        }
        self.directories
            .write()
            .unwrap()
            .remove(path)
            .map(|_| ())
            .ok_or_else(|| StoreError::NotFound(format!("directory {path:?}")))
    }

    async fn stat_directory(&self, path: &str) -> StoreResult<DirectoryMetadata> {
        self.directories
            .read()
            .unwrap()
            .get(path)
            .map(|&ts| DirectoryMetadata {
                path: path.to_string(),
                last_modified: ts,
                ..Default::default()
            })
            .ok_or_else(|| StoreError::NotFound(format!("directory {path:?}")))
    }

    async fn list_directory(
        &self,
        path: &str,
        page: Page,
    ) -> StoreResult<(Vec<DirectoryEntry>, Option<String>)> {
        let prefix = if path.ends_with('/') {
            path.to_string()
        } else {
            format!("{path}/")
        };
        let mut entries: Vec<DirectoryEntry> = Vec::new();

        let directories = self.directories.read().unwrap();
        for (dir, &ts) in directories.iter() {
            if is_direct_child(&prefix, dir) {
                entries.push(DirectoryEntry {
                    path: dir.clone(),
                    is_directory: true,
                    last_modified: ts,
                    ..Default::default()
                });
            }
        }
        let files = self.files.read().unwrap();
        for (file_path, file) in files.iter() {
            if is_direct_child(&prefix, file_path) {
                entries.push(DirectoryEntry {
                    path: file_path.clone(),
                    is_directory: false,
                    file_size: file.bytes.len() as i64,
                    last_modified: file.last_modified,
                    ..Default::default()
                });
            }
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(paginate(entries, &page))
    }

    async fn list_files_opts(&self, path: &str, opts: ListOpts) -> StoreResult<EntryStream> {
        let prefix = if path.ends_with('/') {
            path.to_string()
        } else {
            format!("{path}/")
        };

        // The in-memory store has no streaming backend, so collect into a sorted
        // vec and replay it as a stream — the trait contract holds; the streaming
        // win is real only for `UnityVolumeStore`.
        //
        // Recursive: every key under the prefix, as file entries (object stores
        // are flat — a recursive listing has no folders). Non-recursive: mirror
        // `list_with_delimiter` — emit direct-child files, and roll deeper keys up
        // into their immediate subfolder as a single `is_directory` entry (the
        // common-prefix / folder view).
        let mut entries: Vec<DirectoryEntry> = Vec::new();
        let mut seen_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();
        {
            let files = self.files.read().unwrap();
            for (file_path, file) in files.iter() {
                let Some(rest) = file_path.strip_prefix(&prefix) else {
                    continue;
                };
                if rest.is_empty() {
                    continue;
                }
                if opts.recursive {
                    entries.push(DirectoryEntry {
                        path: file_path.clone(),
                        is_directory: false,
                        file_size: file.bytes.len() as i64,
                        last_modified: file.last_modified,
                        ..Default::default()
                    });
                } else if let Some((dir, _)) = rest.split_once('/') {
                    // Deeper key → its immediate subfolder is a common prefix.
                    if seen_dirs.insert(dir.to_string()) {
                        entries.push(DirectoryEntry {
                            path: format!("{prefix}{dir}"),
                            is_directory: true,
                            ..Default::default()
                        });
                    }
                } else {
                    // Direct-child file.
                    entries.push(DirectoryEntry {
                        path: file_path.clone(),
                        is_directory: false,
                        file_size: file.bytes.len() as i64,
                        last_modified: file.last_modified,
                        ..Default::default()
                    });
                }
            }
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));

        // `start_after` resumes strictly after an absolute path (exclusive).
        if let Some(after) = opts.start_after.as_deref() {
            entries.retain(|e| e.path.as_str() > after);
        }
        if let Some(n) = opts.max_results {
            entries.truncate(n);
        }
        Ok(futures::stream::iter(entries.into_iter().map(Ok)).boxed())
    }
}

fn file_metadata(path: &str, file: &StoredFile) -> FileMetadata {
    FileMetadata {
        path: path.to_string(),
        file_size: file.bytes.len() as i64,
        last_modified: file.last_modified,
        content_type: file.content_type.clone(),
        // An etag based on size+mtime is enough for the in-memory store.
        etag: format!("{}-{}", file.bytes.len(), file.last_modified),
        ..Default::default()
    }
}

/// Whether `candidate` is a direct child of `prefix` (no further `/` segments).
fn is_direct_child(prefix: &str, candidate: &str) -> bool {
    match candidate.strip_prefix(prefix) {
        Some(rest) if !rest.is_empty() => !rest.trim_end_matches('/').contains('/'),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::tags::v1::{EntityTagAssignment, TagPolicy, Value};

    #[tokio::test]
    async fn tag_policy_round_trip() {
        let store = MemoryStore::new();
        let created = store
            .create_policy(TagPolicy {
                tag_key: "cost_center".into(),
                values: vec![Value {
                    name: "eng".into(),
                    ..Default::default()
                }],
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(created.id.is_some());
        assert!(created.created_at.is_some());

        // duplicate create rejected
        let dup = store
            .create_policy(TagPolicy {
                tag_key: "cost_center".into(),
                ..Default::default()
            })
            .await;
        assert!(matches!(dup, Err(StoreError::AlreadyExists(_))));

        let got = store.get_policy("cost_center").await.unwrap();
        assert_eq!(got.tag_key, "cost_center");

        let updated = store
            .update_policy(
                "cost_center",
                TagPolicy {
                    description: Some("updated".into()),
                    ..Default::default()
                },
                &["description".to_string()],
            )
            .await
            .unwrap();
        assert_eq!(updated.description.as_deref(), Some("updated"));
        // values not in the mask are preserved
        assert_eq!(updated.values.len(), 1);

        let (list, _) = store.list_policies(Page::default()).await.unwrap();
        assert_eq!(list.len(), 1);

        store.delete_policy("cost_center").await.unwrap();
        assert!(matches!(
            store.get_policy("cost_center").await,
            Err(StoreError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn assignment_round_trip() {
        let store = MemoryStore::new();
        store
            .create_assignment(EntityTagAssignment {
                entity_type: "tables".into(),
                entity_name: "main.sales.orders".into(),
                tag_key: "pii".into(),
                tag_value: Some("true".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        let (list, _) = store
            .list_assignments("tables", "main.sales.orders", Page::default())
            .await
            .unwrap();
        assert_eq!(list.len(), 1);

        let got = store
            .get_assignment("tables", "main.sales.orders", "pii")
            .await
            .unwrap();
        assert_eq!(got.tag_value.as_deref(), Some("true"));

        store
            .delete_assignment("tables", "main.sales.orders", "pii")
            .await
            .unwrap();
        assert!(matches!(
            store
                .get_assignment("tables", "main.sales.orders", "pii")
                .await,
            Err(StoreError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn file_round_trip() {
        let store = MemoryStore::new();
        let body = b"hello portal".to_vec();
        let meta = store
            .put_file(
                "/data/greeting.txt",
                Some("text/plain".into()),
                body.clone(),
            )
            .await
            .unwrap();
        assert_eq!(meta.file_size, body.len() as i64);

        let stat = store.stat_file("/data/greeting.txt").await.unwrap();
        assert_eq!(stat.content_type, "text/plain");

        let full = store
            .read_file("/data/greeting.txt", None, None)
            .await
            .unwrap();
        assert_eq!(full, body);

        // partial range
        let partial = store
            .read_file("/data/greeting.txt", Some(6), Some(6))
            .await
            .unwrap();
        assert_eq!(partial, b"portal");

        store.delete_file("/data/greeting.txt").await.unwrap();
        assert!(matches!(
            store.stat_file("/data/greeting.txt").await,
            Err(StoreError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn directory_listing() {
        let store = MemoryStore::new();
        store.create_directory("/data").await.unwrap();
        store.create_directory("/data/sub").await.unwrap();
        store
            .put_file("/data/a.txt", None, b"a".to_vec())
            .await
            .unwrap();
        store
            .put_file("/data/sub/b.txt", None, b"b".to_vec())
            .await
            .unwrap();

        let (entries, _) = store
            .list_directory("/data", Page::default())
            .await
            .unwrap();
        // direct children only: /data/a.txt (file) and /data/sub (dir)
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"/data/a.txt"));
        assert!(paths.contains(&"/data/sub"));
        assert!(!paths.contains(&"/data/sub/b.txt"));
    }

    #[tokio::test]
    async fn delete_directory_rejects_non_empty() {
        let store = MemoryStore::new();
        store.create_directory("/data").await.unwrap();
        store
            .put_file("/data/a.txt", None, b"a".to_vec())
            .await
            .unwrap();

        // A directory with a child file cannot be deleted.
        assert!(matches!(
            store.delete_directory("/data").await,
            Err(StoreError::FailedPrecondition(_))
        ));

        // A directory with a child directory cannot be deleted either.
        store.create_directory("/empty").await.unwrap();
        store.create_directory("/empty/child").await.unwrap();
        assert!(matches!(
            store.delete_directory("/empty").await,
            Err(StoreError::FailedPrecondition(_))
        ));

        // Once emptied, the delete succeeds.
        store.delete_directory("/empty/child").await.unwrap();
        store.delete_directory("/empty").await.unwrap();
        assert!(matches!(
            store.stat_directory("/empty").await,
            Err(StoreError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn read_file_stream_returns_the_range() {
        let store = MemoryStore::new();
        store
            .put_file("/data/a.txt", None, b"hello world".to_vec())
            .await
            .unwrap();

        // Whole file.
        let mut s = store
            .read_file_stream("/data/a.txt", None, None)
            .await
            .unwrap();
        let mut buf = Vec::new();
        while let Some(chunk) = s.next().await {
            buf.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(buf, b"hello world");

        // Offset + length.
        let mut s = store
            .read_file_stream("/data/a.txt", Some(6), Some(5))
            .await
            .unwrap();
        let mut buf = Vec::new();
        while let Some(chunk) = s.next().await {
            buf.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(buf, b"world");
    }

    #[tokio::test]
    async fn list_files_stream_is_recursive() {
        let store = MemoryStore::new();
        for p in ["/d/a.txt", "/d/sub/b.txt", "/d/sub/deep/c.txt"] {
            store.put_file(p, None, b"x".to_vec()).await.unwrap();
        }

        let mut s = store.list_files_stream("/d").await.unwrap();
        let mut paths = Vec::new();
        while let Some(e) = s.next().await {
            paths.push(e.unwrap().path);
        }
        // Recursive: every file under the prefix, no subdirectory rollup.
        assert_eq!(paths, vec!["/d/a.txt", "/d/sub/b.txt", "/d/sub/deep/c.txt"]);
    }

    #[tokio::test]
    async fn list_files_opts_non_recursive_offset_and_cap() {
        let store = MemoryStore::new();
        for p in ["/d/a.txt", "/d/b.txt", "/d/c.txt", "/d/sub/deep.txt"] {
            store.put_file(p, None, b"x".to_vec()).await.unwrap();
        }

        // Non-recursive: direct-child files plus the immediate subfolder rolled
        // up as a single `is_directory` entry (the common-prefix / folder view).
        let mut s = store
            .list_files_opts("/d", ListOpts::default())
            .await
            .unwrap();
        let mut entries = Vec::new();
        while let Some(e) = s.next().await {
            entries.push(e.unwrap());
        }
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["/d/a.txt", "/d/b.txt", "/d/c.txt", "/d/sub"]);
        // The subfolder is flagged as a directory; the files are not.
        let sub = entries.iter().find(|e| e.path == "/d/sub").unwrap();
        assert!(sub.is_directory, "subfolder should be a directory entry");
        assert!(
            entries
                .iter()
                .filter(|e| e.path != "/d/sub")
                .all(|e| !e.is_directory),
            "files must not be directories"
        );

        // `start_after` resumes strictly after a path; `max_results` caps.
        let mut s = store
            .list_files_opts(
                "/d",
                ListOpts {
                    recursive: false,
                    start_after: Some("/d/a.txt".into()),
                    max_results: Some(1),
                },
            )
            .await
            .unwrap();
        let mut paths = Vec::new();
        while let Some(e) = s.next().await {
            paths.push(e.unwrap().path);
        }
        assert_eq!(paths, vec!["/d/b.txt"]);
    }

    #[test]
    fn pagination_windows() {
        let items: Vec<i32> = (0..10).collect();
        let page = Page {
            max_results: Some(3),
            page_token: None,
        };
        let (first, token) = paginate(items.clone(), &page);
        assert_eq!(first, vec![0, 1, 2]);
        assert_eq!(token.as_deref(), Some("3"));

        let page2 = Page {
            max_results: Some(3),
            page_token: token,
        };
        let (second, _) = paginate(items, &page2);
        assert_eq!(second, vec![3, 4, 5]);
    }
}
