//! Local-filesystem implementation of [`FileStore`], rooted at a sandbox dir.
//!
//! Backs the editor's always-available "home" volume on the desktop: a real
//! directory under the app working dir (`.open-lakehouse/envs/<id>/home`), so the
//! user always has somewhere to work even before any Unity Catalog volume exists.
//! Routed to by [`super::routing::RoutingFileStore`] on the `/home` prefix.
//!
//! Every logical path is resolved *under* the configured root and any path that
//! would escape it (`..`, absolute components) is rejected — this sandbox is the
//! security boundary, since these paths originate from the UI.

use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use bytes::Bytes;
use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::error::{StoreError, StoreResult};
use crate::proto::files::v1::{DirectoryEntry, DirectoryMetadata, FileMetadata};
use crate::store::{ByteStream, EntryStream, FileStore, ListOpts, Page, paginate};

/// A [`FileStore`] over `tokio::fs`, confined to `root`.
pub struct LocalFileStore {
    root: PathBuf,
}

impl LocalFileStore {
    /// Create a store rooted at `root`, creating the directory if needed.
    pub fn new(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        // Canonicalize so the sandbox check compares against a real, symlink-free
        // base; the dir now exists so this can't fail for that reason.
        let root = std::fs::canonicalize(&root)?;
        Ok(Self { root })
    }

    /// Resolve a logical path (the file-API path, e.g. `/queries/a.sql`) to a real
    /// filesystem path under `root`, rejecting anything that escapes the sandbox.
    ///
    /// The check is purely lexical (no filesystem access) so it also protects
    /// not-yet-existing paths (writes/mkdir): a path is accepted only when every
    /// component is a plain name. `.` is dropped; `..`, root, and prefix
    /// (Windows drive) components are rejected.
    fn resolve(&self, logical: &str) -> StoreResult<PathBuf> {
        let rel = logical.trim_start_matches('/');
        let mut out = self.root.clone();
        for comp in Path::new(rel).components() {
            match comp {
                Component::Normal(seg) => out.push(seg),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(StoreError::InvalidArgument(format!(
                        "path escapes the home volume: {logical:?}"
                    )));
                }
            }
        }
        Ok(out)
    }
}

fn mtime_millis(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// Map a filesystem error to a store error, translating not-found.
fn io_err(path: &str, e: std::io::Error) -> StoreError {
    match e.kind() {
        std::io::ErrorKind::NotFound => StoreError::NotFound(format!("{path:?}")),
        std::io::ErrorKind::AlreadyExists => StoreError::AlreadyExists(format!("{path:?}")),
        _ => StoreError::Internal(format!("{path:?}: {e}")),
    }
}

fn guess_content_type(path: &str) -> String {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext.to_ascii_lowercase().as_str() {
        "sql" | "txt" | "csv" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn file_metadata(path: &str, meta: &std::fs::Metadata) -> FileMetadata {
    let size = meta.len() as i64;
    let mtime = mtime_millis(meta);
    FileMetadata {
        path: path.to_string(),
        file_size: size,
        last_modified: mtime,
        content_type: guess_content_type(path),
        // Size+mtime etag is enough for optimistic concurrency on a local dir.
        etag: format!("{size}-{mtime}"),
        ..Default::default()
    }
}

#[async_trait::async_trait]
impl FileStore for LocalFileStore {
    async fn put_file(
        &self,
        path: &str,
        content_type: Option<String>,
        contents: Vec<u8>,
    ) -> StoreResult<FileMetadata> {
        let chunks = futures::stream::once(async move { Ok(Bytes::from(contents)) }).boxed();
        self.put_file_stream(path, content_type, chunks).await
    }

    async fn put_file_stream(
        &self,
        path: &str,
        _content_type: Option<String>,
        mut chunks: ByteStream,
    ) -> StoreResult<FileMetadata> {
        let target = self.resolve(path)?;
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| io_err(path, e))?;
        }
        let mut file = tokio::fs::File::create(&target)
            .await
            .map_err(|e| io_err(path, e))?;
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await.map_err(|e| io_err(path, e))?;
        }
        file.flush().await.map_err(|e| io_err(path, e))?;
        let meta = file.metadata().await.map_err(|e| io_err(path, e))?;
        Ok(file_metadata(path, &meta))
    }

    async fn read_file(
        &self,
        path: &str,
        offset: Option<i64>,
        length: Option<i64>,
    ) -> StoreResult<Vec<u8>> {
        let target = self.resolve(path)?;
        let mut file = tokio::fs::File::open(&target)
            .await
            .map_err(|e| io_err(path, e))?;
        let start = offset.unwrap_or(0).max(0) as u64;
        if start > 0 {
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|e| io_err(path, e))?;
        }
        let mut buf = Vec::new();
        match length {
            Some(len) if len >= 0 => {
                let mut limited = file.take(len as u64);
                limited
                    .read_to_end(&mut buf)
                    .await
                    .map_err(|e| io_err(path, e))?;
            }
            _ => {
                file.read_to_end(&mut buf)
                    .await
                    .map_err(|e| io_err(path, e))?;
            }
        }
        Ok(buf)
    }

    async fn read_file_stream(
        &self,
        path: &str,
        offset: Option<i64>,
        length: Option<i64>,
    ) -> StoreResult<ByteStream> {
        // A local read is cheap and the editor's files are small; read the range
        // and hand it back as a single chunk. (The streaming contract holds; the
        // real streaming win lives in the object-store-backed UnityVolumeStore.)
        let bytes = self.read_file(path, offset, length).await?;
        Ok(futures::stream::once(async move { Ok(Bytes::from(bytes)) }).boxed())
    }

    async fn stat_file(&self, path: &str) -> StoreResult<FileMetadata> {
        let target = self.resolve(path)?;
        let meta = tokio::fs::metadata(&target)
            .await
            .map_err(|e| io_err(path, e))?;
        if meta.is_dir() {
            return Err(StoreError::InvalidArgument(format!(
                "{path:?} is a directory, not a file"
            )));
        }
        Ok(file_metadata(path, &meta))
    }

    async fn delete_file(&self, path: &str) -> StoreResult<()> {
        let target = self.resolve(path)?;
        tokio::fs::remove_file(&target)
            .await
            .map_err(|e| io_err(path, e))
    }

    async fn create_directory(&self, path: &str) -> StoreResult<DirectoryMetadata> {
        let target = self.resolve(path)?;
        tokio::fs::create_dir_all(&target)
            .await
            .map_err(|e| io_err(path, e))?;
        let meta = tokio::fs::metadata(&target)
            .await
            .map_err(|e| io_err(path, e))?;
        Ok(DirectoryMetadata {
            path: path.to_string(),
            last_modified: mtime_millis(&meta),
            ..Default::default()
        })
    }

    async fn delete_directory(&self, path: &str) -> StoreResult<()> {
        let target = self.resolve(path)?;
        // Databricks semantics: only an empty directory may be deleted.
        let mut entries = tokio::fs::read_dir(&target)
            .await
            .map_err(|e| io_err(path, e))?;
        if entries
            .next_entry()
            .await
            .map_err(|e| io_err(path, e))?
            .is_some()
        {
            return Err(StoreError::FailedPrecondition(format!(
                "directory {path:?} is not empty; delete its contents first"
            )));
        }
        tokio::fs::remove_dir(&target)
            .await
            .map_err(|e| io_err(path, e))
    }

    async fn stat_directory(&self, path: &str) -> StoreResult<DirectoryMetadata> {
        let target = self.resolve(path)?;
        let meta = tokio::fs::metadata(&target)
            .await
            .map_err(|e| io_err(path, e))?;
        if !meta.is_dir() {
            return Err(StoreError::InvalidArgument(format!(
                "{path:?} is a file, not a directory"
            )));
        }
        Ok(DirectoryMetadata {
            path: path.to_string(),
            last_modified: mtime_millis(&meta),
            ..Default::default()
        })
    }

    async fn list_directory(
        &self,
        path: &str,
        page: Page,
    ) -> StoreResult<(Vec<DirectoryEntry>, Option<String>)> {
        let entries = self.read_children(path).await?;
        Ok(paginate(entries, &page))
    }

    async fn list_files_opts(&self, path: &str, opts: ListOpts) -> StoreResult<EntryStream> {
        let mut entries = if opts.recursive {
            self.walk(path).await?
        } else {
            self.read_children(path).await?
        };
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        if let Some(after) = opts.start_after.as_deref() {
            entries.retain(|e| e.path.as_str() > after);
        }
        if let Some(n) = opts.max_results {
            entries.truncate(n);
        }
        Ok(futures::stream::iter(entries.into_iter().map(Ok)).boxed())
    }
}

impl LocalFileStore {
    /// The logical path for a child entry, preserving the caller's prefix style
    /// (so `/home/queries` lists children as `/home/queries/a.sql`).
    fn child_logical(parent: &str, name: &str) -> String {
        let base = parent.trim_end_matches('/');
        if base.is_empty() {
            format!("/{name}")
        } else {
            format!("{base}/{name}")
        }
    }

    /// Immediate children of `path` as hierarchical entries (dirs rolled up),
    /// sorted by path. A missing directory lists as empty (mirrors object-store
    /// prefix semantics, where an absent prefix simply has no children).
    async fn read_children(&self, path: &str) -> StoreResult<Vec<DirectoryEntry>> {
        let dir = self.resolve(path)?;
        let mut rd = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_err(path, e)),
        };
        let mut entries = Vec::new();
        while let Some(ent) = rd.next_entry().await.map_err(|e| io_err(path, e))? {
            let name = ent.file_name().to_string_lossy().into_owned();
            let meta = ent.metadata().await.map_err(|e| io_err(path, e))?;
            let logical = Self::child_logical(path, &name);
            if meta.is_dir() {
                entries.push(DirectoryEntry {
                    path: logical,
                    is_directory: true,
                    last_modified: mtime_millis(&meta),
                    ..Default::default()
                });
            } else {
                entries.push(DirectoryEntry {
                    path: logical,
                    is_directory: false,
                    file_size: meta.len() as i64,
                    last_modified: mtime_millis(&meta),
                    ..Default::default()
                });
            }
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    /// Every file under `path`, recursively (files only — a recursive listing has
    /// no folder entries, matching the object-store flat view).
    async fn walk(&self, path: &str) -> StoreResult<Vec<DirectoryEntry>> {
        let mut out = Vec::new();
        let mut stack = vec![path.to_string()];
        while let Some(current) = stack.pop() {
            for entry in self.read_children(&current).await? {
                if entry.is_directory {
                    stack.push(entry.path.clone());
                } else {
                    out.push(entry);
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, LocalFileStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalFileStore::new(dir.path()).unwrap();
        (dir, store)
    }

    #[tokio::test]
    async fn put_read_stat_round_trip() {
        let (_dir, store) = store();
        let meta = store
            .put_file("/queries/a.sql", Some("text/plain".into()), b"SELECT 1;".to_vec())
            .await
            .unwrap();
        assert_eq!(meta.path, "/queries/a.sql");
        assert_eq!(meta.file_size, 9);

        let bytes = store.read_file("/queries/a.sql", None, None).await.unwrap();
        assert_eq!(bytes, b"SELECT 1;");

        let stat = store.stat_file("/queries/a.sql").await.unwrap();
        assert_eq!(stat.file_size, 9);
        assert_eq!(stat.content_type, "text/plain");
    }

    #[tokio::test]
    async fn read_range_honors_offset_and_length() {
        let (_dir, store) = store();
        store
            .put_file("/f.txt", None, b"0123456789".to_vec())
            .await
            .unwrap();
        let bytes = store.read_file("/f.txt", Some(2), Some(3)).await.unwrap();
        assert_eq!(bytes, b"234");
    }

    #[tokio::test]
    async fn list_directory_paginates() {
        let (_dir, store) = store();
        store.create_directory("/d").await.unwrap();
        for i in 0..5 {
            store
                .put_file(&format!("/d/f{i}.txt"), None, vec![b'x'])
                .await
                .unwrap();
        }
        let (page1, token) = store
            .list_directory(
                "/d",
                Page {
                    max_results: Some(2),
                    page_token: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(page1.len(), 2);
        assert!(token.is_some());
        assert_eq!(page1[0].path, "/d/f0.txt");
    }

    #[tokio::test]
    async fn create_and_delete_directory() {
        let (_dir, store) = store();
        store.create_directory("/sub").await.unwrap();
        store.stat_directory("/sub").await.unwrap();
        // Non-empty delete is rejected.
        store.put_file("/sub/f", None, vec![1]).await.unwrap();
        assert!(matches!(
            store.delete_directory("/sub").await,
            Err(StoreError::FailedPrecondition(_))
        ));
        store.delete_file("/sub/f").await.unwrap();
        store.delete_directory("/sub").await.unwrap();
        assert!(matches!(
            store.stat_directory("/sub").await,
            Err(StoreError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn rejects_path_escape() {
        let (_dir, store) = store();
        for evil in ["/../secret", "/a/../../secret", "/queries/../../etc/passwd"] {
            assert!(
                matches!(
                    store.read_file(evil, None, None).await,
                    Err(StoreError::InvalidArgument(_))
                ),
                "expected {evil:?} to be rejected"
            );
            assert!(matches!(
                store.put_file(evil, None, vec![1]).await,
                Err(StoreError::InvalidArgument(_))
            ));
        }
    }

    #[tokio::test]
    async fn missing_directory_lists_empty() {
        let (_dir, store) = store();
        let (entries, _) = store.list_directory("/nope", Page::default()).await.unwrap();
        assert!(entries.is_empty());
    }
}
