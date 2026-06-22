//! In-process service wiring for embedders (the Tauri desktop backend).
//!
//! The desktop app runs the portal (Tags + Files) and hydrofoil (QueryService)
//! executors *inside* its own process rather than over HTTP/gRPC. This crate
//! builds exactly those executors — the same `connectrpc::Router` the HTTP
//! binaries serve, plus the file store — so the backend can drive them directly
//! (via the `connectrpc::Dispatcher` the router implements for Tags + Query, and
//! via the `FileStore` trait directly for Files).
//!
//! Only Unity Catalog must be a real running server; [`HostConfig::unity_endpoint`]
//! is its REST base URL (a Tauri sidecar, or a dev UC). Lineage and Cedar policy
//! are left at their no-op defaults for local desktop use.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use connectrpc::Router;
use portal::service::AppState;
use portal::store::{FileStore, MemoryStore, TagStore, UnityVolumeStore};
use unitycatalog_object_store::UnityObjectStoreFactory;

use hydrofoil::{FlightSqlServiceImpl, QueryAppState};

/// Configuration for the in-process executors.
///
/// Carries only what the embedded services need; there are no HTTP/gRPC ports —
/// nothing here is served over the network. Unity Catalog is reached over HTTP at
/// [`unity_endpoint`](Self::unity_endpoint).
#[derive(Debug, Clone)]
pub struct HostConfig {
    /// Unity Catalog REST base URL (e.g.
    /// `https://<host>/api/2.1/unity-catalog/`). When `None`, Files falls back to
    /// an in-memory store and the query engine runs without Unity Catalog table
    /// resolution — useful for a standalone smoke test.
    ///
    /// Use `https://` when a token is set: an `http://` endpoint 301-redirects and
    /// drops the bearer token.
    pub unity_endpoint: Option<String>,
    /// Optional UC bearer token. Omit for a local unauthenticated OSS server.
    pub unity_token: Option<String>,
    /// AWS region override for vended credentials.
    pub unity_region: Option<String>,
    /// Idle session TTL for the query engine's session store.
    pub session_ttl_secs: u64,
    /// Default row limit applied when a query omits one.
    pub query_default_limit: u32,
    /// Hard cap on a query's row limit.
    pub query_max_limit: u32,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            unity_endpoint: None,
            unity_token: None,
            unity_region: None,
            // Mirrors hydrofoil's config defaults.
            session_ttl_secs: 1800,
            query_default_limit: 1_000,
            query_max_limit: 10_000,
        }
    }
}

/// The in-process executors, ready to drive directly.
pub struct Hosted {
    /// Portal Tags services (TagPolicies + EntityTagAssignments). Implements
    /// [`connectrpc::Dispatcher`]; call `call_unary` with a method path.
    pub tags: Router,
    /// Hydrofoil QueryService (server-streaming SQL). Implements
    /// [`connectrpc::Dispatcher`]; call `call_server_streaming` for `RunQuery`.
    pub query: Router,
    /// The file store, called directly with native types (no proto framing) — the
    /// store already *is* the sanitized handler the Connect Files adapter wraps.
    pub files: Arc<dyn FileStore>,
}

/// Build the in-process executors from [`HostConfig`].
///
/// Must run on a Tokio runtime: the Unity Catalog object-store factory captures
/// the current runtime handle for its background credential refresh, and the
/// query engine spawns a session sweeper.
pub async fn build(cfg: HostConfig) -> anyhow::Result<Hosted> {
    // --- Files + Tags (portal) ---
    let tags_store: Arc<dyn TagStore> = Arc::new(MemoryStore::new());
    let files = files_store(&cfg).await?;

    // Register only the Tags services; Files is served directly via `files`.
    let tags = AppState::new(Arc::clone(&files), tags_store).register_tags(Router::new());

    // --- QueryService (hydrofoil) ---
    let query = build_query_router(&cfg).await?;

    Ok(Hosted { tags, query, files })
}

/// Build the [`FileStore`]: Unity Catalog volumes when an endpoint is configured,
/// otherwise an in-memory store so the host still runs with no external deps.
async fn files_store(cfg: &HostConfig) -> anyhow::Result<Arc<dyn FileStore>> {
    let Some(endpoint) = cfg.unity_endpoint.as_deref().filter(|e| !e.is_empty()) else {
        tracing::info!("files backed by in-memory store (set unity_endpoint to use UC volumes)");
        return Ok(Arc::new(MemoryStore::new()));
    };
    tracing::info!("files backed by Unity Catalog volumes ({endpoint})");
    let factory = unity_factory(cfg).await?;
    Ok(Arc::new(UnityVolumeStore::new(Arc::new(factory))))
}

/// Build the hydrofoil QueryService router, wiring Unity Catalog when configured.
/// Lineage and Cedar policy are left at their no-op defaults for local desktop.
async fn build_query_router(cfg: &HostConfig) -> anyhow::Result<Router> {
    let mut service =
        FlightSqlServiceImpl::try_new().context("failed to initialize query engine")?;

    if cfg.unity_endpoint.as_deref().is_some_and(|e| !e.is_empty()) {
        let factory = unity_factory(cfg).await?;
        service = service.with_unity(
            Arc::new(factory),
            cfg.unity_endpoint.clone(),
            cfg.unity_region.clone().filter(|r| !r.is_empty()),
        );
        tracing::info!("query engine: Unity Catalog integration enabled");
    } else {
        tracing::info!("query engine: Unity Catalog disabled (set unity_endpoint to enable)");
    }

    let service = Arc::new(service.build(Duration::from_secs(cfg.session_ttl_secs)));
    let state = QueryAppState {
        service,
        query_default_limit: cfg.query_default_limit,
        query_max_limit: cfg.query_max_limit,
    };
    Ok(state.register(Router::new()))
}

/// Build a Unity Catalog object-store factory from the config. Shared by the
/// Files volume store and the query engine so both resolve against the same UC.
async fn unity_factory(cfg: &HostConfig) -> anyhow::Result<UnityObjectStoreFactory> {
    let endpoint = cfg
        .unity_endpoint
        .clone()
        .filter(|e| !e.is_empty())
        .context("unity_endpoint is required to build a Unity Catalog factory")?;
    let mut builder = UnityObjectStoreFactory::builder()
        .with_uri(endpoint)
        .with_io_runtime(tokio::runtime::Handle::current());
    match cfg.unity_token.as_deref().filter(|t| !t.is_empty()) {
        Some(token) => builder = builder.with_token(token.to_string()),
        None => builder = builder.with_allow_unauthenticated(true),
    }
    if let Some(region) = cfg.unity_region.as_deref().filter(|r| !r.is_empty()) {
        builder = builder.with_aws_region(region.to_string());
    }
    builder
        .build()
        .await
        .context("failed to build Unity Catalog object-store factory")
}

#[cfg(test)]
mod tests {
    use super::*;

    use bytes::Bytes;
    use connectrpc::{CodecFormat, Dispatcher, Payload, RequestContext};
    use futures::stream;
    use http::HeaderMap;
    use portal::store::ByteStream;

    /// The Tags router dispatches a JSON request directly — no HTTP server — and
    /// returns a well-formed JSON response. This is the path the Tauri
    /// `connect_unary` command takes.
    #[tokio::test]
    async fn tags_router_dispatches_json_unary() {
        // No UC endpoint: Files/Tags fall back to in-memory stores.
        let hosted = build(HostConfig::default()).await.expect("build hosted");

        let path = "portal.tags.v1.TagPoliciesService/ListTagPolicies";
        let ctx = RequestContext::new(HeaderMap::new()).with_path(path);
        let resp = hosted
            .tags
            .call_unary(
                path,
                ctx,
                Payload::new(Bytes::from_static(b"{}"), CodecFormat::Json),
                CodecFormat::Json,
            )
            .await
            .expect("dispatch ListTagPolicies");

        // The response body is JSON (Connect/JSON codec) and parses as an object.
        let json: serde_json::Value =
            serde_json::from_slice(&resp.body).expect("response body is JSON");
        assert!(json.is_object(), "expected a JSON object, got {json}");
    }

    /// The Files path is the `FileStore` trait itself — native types, no proto
    /// framing. This is what the Tauri `files_*` commands call directly. Exercise
    /// the streaming round-trip over the in-memory backend.
    #[tokio::test]
    async fn files_store_direct_roundtrip() {
        let hosted = build(HostConfig::default()).await.expect("build hosted");
        let files = hosted.files;

        let expected = b"hello desktop-host";
        let chunks: ByteStream = Box::pin(stream::once(async move {
            Ok(Bytes::from_static(b"hello desktop-host"))
        }));

        let meta = files
            .put_file_stream("a/b/file.txt", Some("text/plain".into()), chunks)
            .await
            .expect("put_file_stream");
        assert_eq!(meta.file_size, expected.len() as i64);

        let stat = files.stat_file("a/b/file.txt").await.expect("stat_file");
        assert_eq!(stat.file_size, expected.len() as i64);

        let mut read = files
            .read_file_stream("a/b/file.txt", None, None)
            .await
            .expect("read_file_stream");
        let mut got = Vec::new();
        while let Some(chunk) = futures::StreamExt::next(&mut read).await {
            got.extend_from_slice(&chunk.expect("read chunk"));
        }
        assert_eq!(got, expected);
    }
}
