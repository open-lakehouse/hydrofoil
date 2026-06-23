///Shorthand for `OwnedView<PreviewFileRequestView<'static>>`.
pub type OwnedPreviewFileRequestView = ::buffa::view::OwnedView<
    crate::generated::buffa::hydrofoil::ingest::v1::__buffa::view::PreviewFileRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<PreviewFileResponseView<'static>>`.
pub type OwnedPreviewFileResponseView = ::buffa::view::OwnedView<
    crate::generated::buffa::hydrofoil::ingest::v1::__buffa::view::PreviewFileResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<IngestTableRequestView<'static>>`.
pub type OwnedIngestTableRequestView = ::buffa::view::OwnedView<
    crate::generated::buffa::hydrofoil::ingest::v1::__buffa::view::IngestTableRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<IngestTableResponseView<'static>>`.
pub type OwnedIngestTableResponseView = ::buffa::view::OwnedView<
    crate::generated::buffa::hydrofoil::ingest::v1::__buffa::view::IngestTableResponseView<
        'static,
    >,
>;
impl ::connectrpc::Encodable<
    crate::generated::buffa::hydrofoil::ingest::v1::PreviewFileResponse,
>
for crate::generated::buffa::hydrofoil::ingest::v1::__buffa::view::PreviewFileResponseView<
    '_,
> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<
    crate::generated::buffa::hydrofoil::ingest::v1::PreviewFileResponse,
>
for ::buffa::view::OwnedView<
    crate::generated::buffa::hydrofoil::ingest::v1::__buffa::view::PreviewFileResponseView<
        'static,
    >,
> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self.reborrow(), codec)
    }
}
impl ::connectrpc::Encodable<
    crate::generated::buffa::hydrofoil::ingest::v1::IngestTableResponse,
>
for crate::generated::buffa::hydrofoil::ingest::v1::__buffa::view::IngestTableResponseView<
    '_,
> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<
    crate::generated::buffa::hydrofoil::ingest::v1::IngestTableResponse,
>
for ::buffa::view::OwnedView<
    crate::generated::buffa::hydrofoil::ingest::v1::__buffa::view::IngestTableResponseView<
        'static,
    >,
> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self.reborrow(), codec)
    }
}
/// Full service name for this service.
pub const INGEST_SERVICE_SERVICE_NAME: &str = "hydrofoil.ingest.v1.IngestService";
/// Static [`Spec`](::connectrpc::Spec) for the server-side `PreviewFile` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const INGEST_SERVICE_PREVIEW_FILE_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hydrofoil.ingest.v1.IngestService/PreviewFile",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `IngestTable` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const INGEST_SERVICE_INGEST_TABLE_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hydrofoil.ingest.v1.IngestService/IngestTable",
        ::connectrpc::StreamType::ClientStream,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// File-to-managed-table ingest for web/UI clients.
///
/// # Implementing handlers
///
/// Implement methods with plain `async fn`; the returned future satisfies
/// the `Send` bound automatically.
///
/// **Unary and server-streaming requests** arrive as
/// [`ServiceRequest<'_, Req>`](::connectrpc::ServiceRequest): a zero-copy
/// view of the request plus its body, valid for the duration of the call.
/// Fields are read directly (`request.name` is a `&str` into the decoded
/// buffer) and the borrow may be held across `.await` points. Anything
/// that must outlive the call — `tokio::spawn`, channels, server state,
/// or data captured by a returned response stream — takes owned data:
/// call `request.to_owned_message()` (or copy the specific fields)
/// first.
///
/// **Client-streaming and bidi requests** arrive as
/// `ServiceStream<`[`StreamMessage<Req>`](::connectrpc::StreamMessage)`>`.
/// Each item owns its decoded buffer and is `Send + 'static`, so items
/// can be buffered or moved into spawned tasks; read fields zero-copy
/// through the generated accessor methods (`item.name()`) or `.view()`,
/// convert with `.to_owned_message()`, or yield an item back unchanged —
/// `StreamMessage<M>` implements `Encodable<M>`.
///
/// Request types resolved through `extern_path` (e.g. well-known types
/// from another crate) use the same wrappers; the crate that owns the
/// type must be generated with buffa ≥ 0.7.0 and views enabled so the
/// backing `HasMessageView` impl exists.
///
/// The `impl Encodable<Out>` return bound accepts the owned `Out`, the
/// generated `OutView<'_>` / `OwnedOutView`,
/// [`MaybeBorrowed`](::connectrpc::MaybeBorrowed), or
/// [`PreEncoded`](::connectrpc::PreEncoded) for handlers that encode a
/// non-`'static` view internally and pass the bytes across the handler
/// boundary. View bodies are not emitted for output types mapped via
/// `extern_path` (the impl would be an orphan); return owned for
/// WKT/extern outputs.
///
/// Server-streaming and bidi-streaming methods return
/// `ServiceStream<impl Encodable<Out> + Send + use<Self>>`. The
/// `use<Self>` precise-capturing clause excludes `&self`'s lifetime and
/// the request's lifetime (unary methods use `use<'a, Self>` and may
/// borrow from `&self`), so stream items must be `'static` and cannot
/// borrow from the request. To stream view-encoded data, encode each
/// item inside the stream body and yield
/// [`PreEncoded`](::connectrpc::PreEncoded) — see its `# Streaming
/// example` doc.
#[allow(clippy::type_complexity)]
pub trait IngestService: Send + Sync + 'static {
    /// Parse a local file and return its inferred schema + a sample for preview.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn preview_file<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::generated::buffa::hydrofoil::ingest::v1::PreviewFileRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::generated::buffa::hydrofoil::ingest::v1::PreviewFileResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Create a managed Delta table (if needed) and append the supplied data.
    /// Client-streaming: target + schema on the first frame, then data chunks.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// Each `requests` item is a [`StreamMessage`](::connectrpc::StreamMessage):
    /// it owns its buffer, is `Send + 'static`, and exposes zero-copy
    /// accessor methods (`item.name()`), `.view()`, and
    /// `.to_owned_message()`.
    fn ingest_table<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        requests: ::connectrpc::ServiceStream<
            ::connectrpc::StreamMessage<
                crate::generated::buffa::hydrofoil::ingest::v1::IngestTableRequest,
            >,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::generated::buffa::hydrofoil::ingest::v1::IngestTableResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
}
/// Extension trait for registering a service implementation with a Router.
///
/// This trait is automatically implemented for all types that implement the service trait.
///
/// # Example
///
/// ```rust,ignore
/// use std::sync::Arc;
///
/// let service = Arc::new(MyServiceImpl);
/// let router = service.register(Router::new());
/// ```
pub trait IngestServiceExt: IngestService {
    /// Register this service implementation with a Router.
    ///
    /// Takes ownership of the `Arc<Self>` and returns a new Router with
    /// this service's methods registered.
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router;
}
impl<S: IngestService> IngestServiceExt for S {
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router {
        router
            .route_view(
                INGEST_SERVICE_SERVICE_NAME,
                "PreviewFile",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::generated::buffa::hydrofoil::ingest::v1::__buffa::view::PreviewFileRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::generated::buffa::hydrofoil::ingest::v1::PreviewFileRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.preview_file(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::generated::buffa::hydrofoil::ingest::v1::PreviewFileResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(INGEST_SERVICE_PREVIEW_FILE_SPEC)
            .route_view_client_stream(
                INGEST_SERVICE_SERVICE_NAME,
                "IngestTable",
                ::connectrpc::view_client_streaming_handler_fn({
                    let svc = ::std::sync::Arc::clone(&self);
                    move |ctx, req, format| {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let req = ::connectrpc::dispatcher::codegen::into_stream_messages::<
                                crate::generated::buffa::hydrofoil::ingest::v1::IngestTableRequest,
                            >(req);
                            svc.ingest_table(ctx, req)
                                .await?
                                .encode::<
                                    crate::generated::buffa::hydrofoil::ingest::v1::IngestTableResponse,
                                >(format)
                        }
                    }
                }),
            )
            .with_spec(INGEST_SERVICE_INGEST_TABLE_SPEC)
    }
}
/// Monomorphic dispatcher for `IngestService`.
///
/// Unlike `.register(Router)` which type-erases each method into an `Arc<dyn ErasedHandler>` stored in a `HashMap`, this struct dispatches via a compile-time `match` on method name: no vtable, no hash lookup.
///
/// # Example
///
/// ```rust,ignore
/// use connectrpc::ConnectRpcService;
///
/// let server = IngestServiceServer::new(MyImpl);
/// let service = ConnectRpcService::new(server);
/// // hand `service` to axum/hyper as a fallback_service
/// ```
pub struct IngestServiceServer<T> {
    inner: ::std::sync::Arc<T>,
}
impl<T: IngestService> IngestServiceServer<T> {
    /// Wrap a service implementation in a monomorphic dispatcher.
    pub fn new(service: T) -> Self {
        Self {
            inner: ::std::sync::Arc::new(service),
        }
    }
    /// Wrap an already-`Arc`'d service implementation.
    pub fn from_arc(inner: ::std::sync::Arc<T>) -> Self {
        Self { inner }
    }
}
impl<T> Clone for IngestServiceServer<T> {
    fn clone(&self) -> Self {
        Self {
            inner: ::std::sync::Arc::clone(&self.inner),
        }
    }
}
impl<T: IngestService> ::connectrpc::Dispatcher for IngestServiceServer<T> {
    #[inline]
    fn lookup(
        &self,
        path: &str,
    ) -> Option<::connectrpc::dispatcher::codegen::MethodDescriptor> {
        let method = path.strip_prefix("hydrofoil.ingest.v1.IngestService/")?;
        match method {
            "PreviewFile" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(INGEST_SERVICE_PREVIEW_FILE_SPEC),
                )
            }
            "IngestTable" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::client_streaming()
                        .with_spec(INGEST_SERVICE_INGEST_TABLE_SPEC),
                )
            }
            _ => None,
        }
    }
    fn call_unary(
        &self,
        path: &str,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::Payload,
        format: ::connectrpc::CodecFormat,
    ) -> ::connectrpc::dispatcher::codegen::UnaryResult {
        let Some(method) = path.strip_prefix("hydrofoil.ingest.v1.IngestService/") else {
            return ::connectrpc::dispatcher::codegen::unimplemented_unary(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            "PreviewFile" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::generated::buffa::hydrofoil::ingest::v1::PreviewFileRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::generated::buffa::hydrofoil::ingest::v1::__buffa::view::PreviewFileRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::generated::buffa::hydrofoil::ingest::v1::PreviewFileRequest,
                    >::from_parts(&req, &body);
                    svc.preview_file(ctx, req)
                        .await?
                        .encode::<
                            crate::generated::buffa::hydrofoil::ingest::v1::PreviewFileResponse,
                        >(format)
                })
            }
            _ => ::connectrpc::dispatcher::codegen::unimplemented_unary(path),
        }
    }
    fn call_server_streaming(
        &self,
        path: &str,
        ctx: ::connectrpc::RequestContext,
        request: ::buffa::bytes::Bytes,
        format: ::connectrpc::CodecFormat,
    ) -> ::connectrpc::dispatcher::codegen::StreamingResult {
        let Some(method) = path.strip_prefix("hydrofoil.ingest.v1.IngestService/") else {
            return ::connectrpc::dispatcher::codegen::unimplemented_streaming(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            _ => ::connectrpc::dispatcher::codegen::unimplemented_streaming(path),
        }
    }
    fn call_client_streaming(
        &self,
        path: &str,
        ctx: ::connectrpc::RequestContext,
        requests: ::connectrpc::dispatcher::codegen::RequestStream,
        format: ::connectrpc::CodecFormat,
    ) -> ::connectrpc::dispatcher::codegen::UnaryResult {
        let Some(method) = path.strip_prefix("hydrofoil.ingest.v1.IngestService/") else {
            return ::connectrpc::dispatcher::codegen::unimplemented_unary(path);
        };
        let _ = (&ctx, &requests, &format);
        match method {
            "IngestTable" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let req_stream = ::connectrpc::dispatcher::codegen::decode_message_request_stream::<
                        crate::generated::buffa::hydrofoil::ingest::v1::IngestTableRequest,
                    >(requests, format);
                    svc.ingest_table(ctx, req_stream)
                        .await?
                        .encode::<
                            crate::generated::buffa::hydrofoil::ingest::v1::IngestTableResponse,
                        >(format)
                })
            }
            _ => ::connectrpc::dispatcher::codegen::unimplemented_unary(path),
        }
    }
    fn call_bidi_streaming(
        &self,
        path: &str,
        ctx: ::connectrpc::RequestContext,
        requests: ::connectrpc::dispatcher::codegen::RequestStream,
        format: ::connectrpc::CodecFormat,
    ) -> ::connectrpc::dispatcher::codegen::StreamingResult {
        let Some(method) = path.strip_prefix("hydrofoil.ingest.v1.IngestService/") else {
            return ::connectrpc::dispatcher::codegen::unimplemented_streaming(path);
        };
        let _ = (&ctx, &requests, &format);
        match method {
            _ => ::connectrpc::dispatcher::codegen::unimplemented_streaming(path),
        }
    }
}
/// Client for this service.
///
/// Generic over `T: ClientTransport`. For **gRPC** (HTTP/2), use
/// `Http2Connection` — it has honest `poll_ready` and composes with
/// `tower::balance` for multi-connection load balancing. For **Connect
/// over HTTP/1.1** (or unknown protocol), use `HttpClient`.
///
/// # Example (gRPC / HTTP/2)
///
/// ```rust,ignore
/// use connectrpc::client::{Http2Connection, ClientConfig};
/// use connectrpc::Protocol;
///
/// let uri: http::Uri = "http://localhost:8080".parse()?;
/// let conn = Http2Connection::connect_plaintext(uri.clone()).await?.shared(1024);
/// let config = ClientConfig::new(uri).with_protocol(Protocol::Grpc);
///
/// let client = IngestServiceClient::new(conn, config);
/// let response = client.preview_file(request).await?;
/// ```
///
/// # Example (Connect / HTTP/1.1 or ALPN)
///
/// ```rust,ignore
/// use connectrpc::client::{HttpClient, ClientConfig};
///
/// let http = HttpClient::plaintext();  // cleartext http:// only
/// let config = ClientConfig::new("http://localhost:8080".parse()?);
///
/// let client = IngestServiceClient::new(http, config);
/// let response = client.preview_file(request).await?;
/// ```
///
/// # Working with the response
///
/// Unary calls return [`UnaryResponse<OwnedView<FooView>>`](::connectrpc::client::UnaryResponse).
/// [`view()`](::connectrpc::client::UnaryResponse::view) borrows the response
/// message, so field access is zero-copy:
///
/// ```rust,ignore
/// let resp = client.preview_file(request).await?;
/// let name: &str = resp.view().name;  // borrow into the response buffer
/// ```
///
/// If you need the owned struct (e.g. to store or pass by value), use
/// [`into_owned()`](::connectrpc::client::UnaryResponse::into_owned):
///
/// ```rust,ignore
/// let owned = client.preview_file(request).await?.into_owned();
/// ```
///
/// [`into_view()`](::connectrpc::client::UnaryResponse::into_view) keeps the
/// zero-copy decoded body (an `OwnedView`) without copying; field access on it
/// goes through `.reborrow()`. Streaming responses yield one `OwnedView` per
/// received message from `.message().await` — bind `msg.reborrow()` for field
/// access, or convert with `.to_owned_message()`.
#[derive(Clone)]
pub struct IngestServiceClient<T> {
    transport: T,
    config: ::connectrpc::client::ClientConfig,
}
impl<T> IngestServiceClient<T>
where
    T: ::connectrpc::client::ClientTransport,
    <T::ResponseBody as ::http_body::Body>::Error: ::std::fmt::Display,
{
    /// Create a new client with the given transport and configuration.
    pub fn new(transport: T, config: ::connectrpc::client::ClientConfig) -> Self {
        Self { transport, config }
    }
    /// Get the client configuration.
    pub fn config(&self) -> &::connectrpc::client::ClientConfig {
        &self.config
    }
    /// Get a mutable reference to the client configuration.
    pub fn config_mut(&mut self) -> &mut ::connectrpc::client::ClientConfig {
        &mut self.config
    }
    /// Call the PreviewFile RPC. Sends a request to /hydrofoil.ingest.v1.IngestService/PreviewFile.
    pub async fn preview_file(
        &self,
        request: crate::generated::buffa::hydrofoil::ingest::v1::PreviewFileRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::generated::buffa::hydrofoil::ingest::v1::__buffa::view::PreviewFileResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.preview_file_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the PreviewFile RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn preview_file_with_options(
        &self,
        request: crate::generated::buffa::hydrofoil::ingest::v1::PreviewFileRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::generated::buffa::hydrofoil::ingest::v1::__buffa::view::PreviewFileResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                INGEST_SERVICE_SERVICE_NAME,
                "PreviewFile",
                request,
                options,
            )
            .await
    }
    /// Call the IngestTable RPC. Sends a request to /hydrofoil.ingest.v1.IngestService/IngestTable.
    pub async fn ingest_table(
        &self,
        requests: impl IntoIterator<
            Item = crate::generated::buffa::hydrofoil::ingest::v1::IngestTableRequest,
        >,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::generated::buffa::hydrofoil::ingest::v1::__buffa::view::IngestTableResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.ingest_table_with_options(
                requests,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the IngestTable RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn ingest_table_with_options(
        &self,
        requests: impl IntoIterator<
            Item = crate::generated::buffa::hydrofoil::ingest::v1::IngestTableRequest,
        >,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::generated::buffa::hydrofoil::ingest::v1::__buffa::view::IngestTableResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_client_stream(
                &self.transport,
                &self.config,
                INGEST_SERVICE_SERVICE_NAME,
                "IngestTable",
                requests,
                options,
            )
            .await
    }
}
