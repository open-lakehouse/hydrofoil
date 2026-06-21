///Shorthand for `OwnedView<ListTagPoliciesRequestView<'static>>`.
pub type OwnedListTagPoliciesRequestView = ::buffa::view::OwnedView<
    crate::generated::buffa::portal::tags::v1::__buffa::view::ListTagPoliciesRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListTagPoliciesResponseView<'static>>`.
pub type OwnedListTagPoliciesResponseView = ::buffa::view::OwnedView<
    crate::generated::buffa::portal::tags::v1::__buffa::view::ListTagPoliciesResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<CreateTagPolicyRequestView<'static>>`.
pub type OwnedCreateTagPolicyRequestView = ::buffa::view::OwnedView<
    crate::generated::buffa::portal::tags::v1::__buffa::view::CreateTagPolicyRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<TagPolicyView<'static>>`.
pub type OwnedTagPolicyView = ::buffa::view::OwnedView<
    crate::generated::buffa::portal::tags::v1::__buffa::view::TagPolicyView<'static>,
>;
///Shorthand for `OwnedView<GetTagPolicyRequestView<'static>>`.
pub type OwnedGetTagPolicyRequestView = ::buffa::view::OwnedView<
    crate::generated::buffa::portal::tags::v1::__buffa::view::GetTagPolicyRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<UpdateTagPolicyRequestView<'static>>`.
pub type OwnedUpdateTagPolicyRequestView = ::buffa::view::OwnedView<
    crate::generated::buffa::portal::tags::v1::__buffa::view::UpdateTagPolicyRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<DeleteTagPolicyRequestView<'static>>`.
pub type OwnedDeleteTagPolicyRequestView = ::buffa::view::OwnedView<
    crate::generated::buffa::portal::tags::v1::__buffa::view::DeleteTagPolicyRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<EmptyView<'static>>`.
pub type OwnedEmptyView = ::buffa::view::OwnedView<
    ::buffa_types::google::protobuf::__buffa::view::EmptyView<'static>,
>;
///Shorthand for `OwnedView<ListEntityTagAssignmentsRequestView<'static>>`.
pub type OwnedListEntityTagAssignmentsRequestView = ::buffa::view::OwnedView<
    crate::generated::buffa::portal::tags::v1::__buffa::view::ListEntityTagAssignmentsRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListEntityTagAssignmentsResponseView<'static>>`.
pub type OwnedListEntityTagAssignmentsResponseView = ::buffa::view::OwnedView<
    crate::generated::buffa::portal::tags::v1::__buffa::view::ListEntityTagAssignmentsResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<CreateEntityTagAssignmentRequestView<'static>>`.
pub type OwnedCreateEntityTagAssignmentRequestView = ::buffa::view::OwnedView<
    crate::generated::buffa::portal::tags::v1::__buffa::view::CreateEntityTagAssignmentRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<EntityTagAssignmentView<'static>>`.
pub type OwnedEntityTagAssignmentView = ::buffa::view::OwnedView<
    crate::generated::buffa::portal::tags::v1::__buffa::view::EntityTagAssignmentView<
        'static,
    >,
>;
///Shorthand for `OwnedView<GetEntityTagAssignmentRequestView<'static>>`.
pub type OwnedGetEntityTagAssignmentRequestView = ::buffa::view::OwnedView<
    crate::generated::buffa::portal::tags::v1::__buffa::view::GetEntityTagAssignmentRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<UpdateEntityTagAssignmentRequestView<'static>>`.
pub type OwnedUpdateEntityTagAssignmentRequestView = ::buffa::view::OwnedView<
    crate::generated::buffa::portal::tags::v1::__buffa::view::UpdateEntityTagAssignmentRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<DeleteEntityTagAssignmentRequestView<'static>>`.
pub type OwnedDeleteEntityTagAssignmentRequestView = ::buffa::view::OwnedView<
    crate::generated::buffa::portal::tags::v1::__buffa::view::DeleteEntityTagAssignmentRequestView<
        'static,
    >,
>;
impl ::connectrpc::Encodable<
    crate::generated::buffa::portal::tags::v1::ListTagPoliciesResponse,
>
for crate::generated::buffa::portal::tags::v1::__buffa::view::ListTagPoliciesResponseView<
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
    crate::generated::buffa::portal::tags::v1::ListTagPoliciesResponse,
>
for ::buffa::view::OwnedView<
    crate::generated::buffa::portal::tags::v1::__buffa::view::ListTagPoliciesResponseView<
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
impl ::connectrpc::Encodable<crate::generated::buffa::portal::tags::v1::TagPolicy>
for crate::generated::buffa::portal::tags::v1::__buffa::view::TagPolicyView<'_> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<crate::generated::buffa::portal::tags::v1::TagPolicy>
for ::buffa::view::OwnedView<
    crate::generated::buffa::portal::tags::v1::__buffa::view::TagPolicyView<'static>,
> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self.reborrow(), codec)
    }
}
impl ::connectrpc::Encodable<
    crate::generated::buffa::portal::tags::v1::ListEntityTagAssignmentsResponse,
>
for crate::generated::buffa::portal::tags::v1::__buffa::view::ListEntityTagAssignmentsResponseView<
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
    crate::generated::buffa::portal::tags::v1::ListEntityTagAssignmentsResponse,
>
for ::buffa::view::OwnedView<
    crate::generated::buffa::portal::tags::v1::__buffa::view::ListEntityTagAssignmentsResponseView<
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
    crate::generated::buffa::portal::tags::v1::EntityTagAssignment,
>
for crate::generated::buffa::portal::tags::v1::__buffa::view::EntityTagAssignmentView<
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
    crate::generated::buffa::portal::tags::v1::EntityTagAssignment,
>
for ::buffa::view::OwnedView<
    crate::generated::buffa::portal::tags::v1::__buffa::view::EntityTagAssignmentView<
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
pub const TAG_POLICIES_SERVICE_SERVICE_NAME: &str = "portal.tags.v1.TagPoliciesService";
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ListTagPolicies` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const TAG_POLICIES_SERVICE_LIST_TAG_POLICIES_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/portal.tags.v1.TagPoliciesService/ListTagPolicies",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `CreateTagPolicy` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const TAG_POLICIES_SERVICE_CREATE_TAG_POLICY_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/portal.tags.v1.TagPoliciesService/CreateTagPolicy",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `GetTagPolicy` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const TAG_POLICIES_SERVICE_GET_TAG_POLICY_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/portal.tags.v1.TagPoliciesService/GetTagPolicy",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `UpdateTagPolicy` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const TAG_POLICIES_SERVICE_UPDATE_TAG_POLICY_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/portal.tags.v1.TagPoliciesService/UpdateTagPolicy",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `DeleteTagPolicy` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const TAG_POLICIES_SERVICE_DELETE_TAG_POLICY_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/portal.tags.v1.TagPoliciesService/DeleteTagPolicy",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Manage governed tag definitions (tag policies).
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
pub trait TagPoliciesService: Send + Sync + 'static {
    /// List tag policies.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn list_tag_policies<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::generated::buffa::portal::tags::v1::ListTagPoliciesRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::generated::buffa::portal::tags::v1::ListTagPoliciesResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Create a new governed tag definition.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn create_tag_policy<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::generated::buffa::portal::tags::v1::CreateTagPolicyRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::generated::buffa::portal::tags::v1::TagPolicy,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Get the governed tag definition for the specified tag key.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn get_tag_policy<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::generated::buffa::portal::tags::v1::GetTagPolicyRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::generated::buffa::portal::tags::v1::TagPolicy,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Update the governed tag definition that matches the supplied tag key.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn update_tag_policy<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::generated::buffa::portal::tags::v1::UpdateTagPolicyRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::generated::buffa::portal::tags::v1::TagPolicy,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Delete the governed tag definition that matches the supplied tag key.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn delete_tag_policy<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::generated::buffa::portal::tags::v1::DeleteTagPolicyRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                ::buffa_types::google::protobuf::Empty,
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
pub trait TagPoliciesServiceExt: TagPoliciesService {
    /// Register this service implementation with a Router.
    ///
    /// Takes ownership of the `Arc<Self>` and returns a new Router with
    /// this service's methods registered.
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router;
}
impl<S: TagPoliciesService> TagPoliciesServiceExt for S {
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router {
        router
            .route_view(
                TAG_POLICIES_SERVICE_SERVICE_NAME,
                "ListTagPolicies",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::generated::buffa::portal::tags::v1::__buffa::view::ListTagPoliciesRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::generated::buffa::portal::tags::v1::ListTagPoliciesRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.list_tag_policies(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::generated::buffa::portal::tags::v1::ListTagPoliciesResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(TAG_POLICIES_SERVICE_LIST_TAG_POLICIES_SPEC)
            .route_view(
                TAG_POLICIES_SERVICE_SERVICE_NAME,
                "CreateTagPolicy",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::generated::buffa::portal::tags::v1::__buffa::view::CreateTagPolicyRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::generated::buffa::portal::tags::v1::CreateTagPolicyRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.create_tag_policy(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::generated::buffa::portal::tags::v1::TagPolicy,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(TAG_POLICIES_SERVICE_CREATE_TAG_POLICY_SPEC)
            .route_view(
                TAG_POLICIES_SERVICE_SERVICE_NAME,
                "GetTagPolicy",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::generated::buffa::portal::tags::v1::__buffa::view::GetTagPolicyRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::generated::buffa::portal::tags::v1::GetTagPolicyRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.get_tag_policy(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::generated::buffa::portal::tags::v1::TagPolicy,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(TAG_POLICIES_SERVICE_GET_TAG_POLICY_SPEC)
            .route_view(
                TAG_POLICIES_SERVICE_SERVICE_NAME,
                "UpdateTagPolicy",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::generated::buffa::portal::tags::v1::__buffa::view::UpdateTagPolicyRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::generated::buffa::portal::tags::v1::UpdateTagPolicyRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.update_tag_policy(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::generated::buffa::portal::tags::v1::TagPolicy,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(TAG_POLICIES_SERVICE_UPDATE_TAG_POLICY_SPEC)
            .route_view(
                TAG_POLICIES_SERVICE_SERVICE_NAME,
                "DeleteTagPolicy",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::generated::buffa::portal::tags::v1::__buffa::view::DeleteTagPolicyRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::generated::buffa::portal::tags::v1::DeleteTagPolicyRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.delete_tag_policy(ctx, sreq)
                                .await?
                                .encode::<::buffa_types::google::protobuf::Empty>(format)
                        }
                    })
                },
            )
            .with_spec(TAG_POLICIES_SERVICE_DELETE_TAG_POLICY_SPEC)
    }
}
/// Monomorphic dispatcher for `TagPoliciesService`.
///
/// Unlike `.register(Router)` which type-erases each method into an `Arc<dyn ErasedHandler>` stored in a `HashMap`, this struct dispatches via a compile-time `match` on method name: no vtable, no hash lookup.
///
/// # Example
///
/// ```rust,ignore
/// use connectrpc::ConnectRpcService;
///
/// let server = TagPoliciesServiceServer::new(MyImpl);
/// let service = ConnectRpcService::new(server);
/// // hand `service` to axum/hyper as a fallback_service
/// ```
pub struct TagPoliciesServiceServer<T> {
    inner: ::std::sync::Arc<T>,
}
impl<T: TagPoliciesService> TagPoliciesServiceServer<T> {
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
impl<T> Clone for TagPoliciesServiceServer<T> {
    fn clone(&self) -> Self {
        Self {
            inner: ::std::sync::Arc::clone(&self.inner),
        }
    }
}
impl<T: TagPoliciesService> ::connectrpc::Dispatcher for TagPoliciesServiceServer<T> {
    #[inline]
    fn lookup(
        &self,
        path: &str,
    ) -> Option<::connectrpc::dispatcher::codegen::MethodDescriptor> {
        let method = path.strip_prefix("portal.tags.v1.TagPoliciesService/")?;
        match method {
            "ListTagPolicies" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(TAG_POLICIES_SERVICE_LIST_TAG_POLICIES_SPEC),
                )
            }
            "CreateTagPolicy" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(TAG_POLICIES_SERVICE_CREATE_TAG_POLICY_SPEC),
                )
            }
            "GetTagPolicy" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(TAG_POLICIES_SERVICE_GET_TAG_POLICY_SPEC),
                )
            }
            "UpdateTagPolicy" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(TAG_POLICIES_SERVICE_UPDATE_TAG_POLICY_SPEC),
                )
            }
            "DeleteTagPolicy" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(TAG_POLICIES_SERVICE_DELETE_TAG_POLICY_SPEC),
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
        let Some(method) = path.strip_prefix("portal.tags.v1.TagPoliciesService/") else {
            return ::connectrpc::dispatcher::codegen::unimplemented_unary(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            "ListTagPolicies" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::generated::buffa::portal::tags::v1::ListTagPoliciesRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::generated::buffa::portal::tags::v1::__buffa::view::ListTagPoliciesRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::generated::buffa::portal::tags::v1::ListTagPoliciesRequest,
                    >::from_parts(&req, &body);
                    svc.list_tag_policies(ctx, req)
                        .await?
                        .encode::<
                            crate::generated::buffa::portal::tags::v1::ListTagPoliciesResponse,
                        >(format)
                })
            }
            "CreateTagPolicy" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::generated::buffa::portal::tags::v1::CreateTagPolicyRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::generated::buffa::portal::tags::v1::__buffa::view::CreateTagPolicyRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::generated::buffa::portal::tags::v1::CreateTagPolicyRequest,
                    >::from_parts(&req, &body);
                    svc.create_tag_policy(ctx, req)
                        .await?
                        .encode::<
                            crate::generated::buffa::portal::tags::v1::TagPolicy,
                        >(format)
                })
            }
            "GetTagPolicy" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::generated::buffa::portal::tags::v1::GetTagPolicyRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::generated::buffa::portal::tags::v1::__buffa::view::GetTagPolicyRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::generated::buffa::portal::tags::v1::GetTagPolicyRequest,
                    >::from_parts(&req, &body);
                    svc.get_tag_policy(ctx, req)
                        .await?
                        .encode::<
                            crate::generated::buffa::portal::tags::v1::TagPolicy,
                        >(format)
                })
            }
            "UpdateTagPolicy" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::generated::buffa::portal::tags::v1::UpdateTagPolicyRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::generated::buffa::portal::tags::v1::__buffa::view::UpdateTagPolicyRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::generated::buffa::portal::tags::v1::UpdateTagPolicyRequest,
                    >::from_parts(&req, &body);
                    svc.update_tag_policy(ctx, req)
                        .await?
                        .encode::<
                            crate::generated::buffa::portal::tags::v1::TagPolicy,
                        >(format)
                })
            }
            "DeleteTagPolicy" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::generated::buffa::portal::tags::v1::DeleteTagPolicyRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::generated::buffa::portal::tags::v1::__buffa::view::DeleteTagPolicyRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::generated::buffa::portal::tags::v1::DeleteTagPolicyRequest,
                    >::from_parts(&req, &body);
                    svc.delete_tag_policy(ctx, req)
                        .await?
                        .encode::<::buffa_types::google::protobuf::Empty>(format)
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
        let Some(method) = path.strip_prefix("portal.tags.v1.TagPoliciesService/") else {
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
        let Some(method) = path.strip_prefix("portal.tags.v1.TagPoliciesService/") else {
            return ::connectrpc::dispatcher::codegen::unimplemented_unary(path);
        };
        let _ = (&ctx, &requests, &format);
        match method {
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
        let Some(method) = path.strip_prefix("portal.tags.v1.TagPoliciesService/") else {
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
/// let client = TagPoliciesServiceClient::new(conn, config);
/// let response = client.list_tag_policies(request).await?;
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
/// let client = TagPoliciesServiceClient::new(http, config);
/// let response = client.list_tag_policies(request).await?;
/// ```
///
/// # Working with the response
///
/// Unary calls return [`UnaryResponse<OwnedView<FooView>>`](::connectrpc::client::UnaryResponse).
/// [`view()`](::connectrpc::client::UnaryResponse::view) borrows the response
/// message, so field access is zero-copy:
///
/// ```rust,ignore
/// let resp = client.list_tag_policies(request).await?;
/// let name: &str = resp.view().name;  // borrow into the response buffer
/// ```
///
/// If you need the owned struct (e.g. to store or pass by value), use
/// [`into_owned()`](::connectrpc::client::UnaryResponse::into_owned):
///
/// ```rust,ignore
/// let owned = client.list_tag_policies(request).await?.into_owned();
/// ```
///
/// [`into_view()`](::connectrpc::client::UnaryResponse::into_view) keeps the
/// zero-copy decoded body (an `OwnedView`) without copying; field access on it
/// goes through `.reborrow()`. Streaming responses yield one `OwnedView` per
/// received message from `.message().await` — bind `msg.reborrow()` for field
/// access, or convert with `.to_owned_message()`.
#[derive(Clone)]
pub struct TagPoliciesServiceClient<T> {
    transport: T,
    config: ::connectrpc::client::ClientConfig,
}
impl<T> TagPoliciesServiceClient<T>
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
    /// Call the ListTagPolicies RPC. Sends a request to /portal.tags.v1.TagPoliciesService/ListTagPolicies.
    pub async fn list_tag_policies(
        &self,
        request: crate::generated::buffa::portal::tags::v1::ListTagPoliciesRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::generated::buffa::portal::tags::v1::__buffa::view::ListTagPoliciesResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.list_tag_policies_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ListTagPolicies RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn list_tag_policies_with_options(
        &self,
        request: crate::generated::buffa::portal::tags::v1::ListTagPoliciesRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::generated::buffa::portal::tags::v1::__buffa::view::ListTagPoliciesResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                TAG_POLICIES_SERVICE_SERVICE_NAME,
                "ListTagPolicies",
                request,
                options,
            )
            .await
    }
    /// Call the CreateTagPolicy RPC. Sends a request to /portal.tags.v1.TagPoliciesService/CreateTagPolicy.
    pub async fn create_tag_policy(
        &self,
        request: crate::generated::buffa::portal::tags::v1::CreateTagPolicyRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::generated::buffa::portal::tags::v1::__buffa::view::TagPolicyView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.create_tag_policy_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the CreateTagPolicy RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn create_tag_policy_with_options(
        &self,
        request: crate::generated::buffa::portal::tags::v1::CreateTagPolicyRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::generated::buffa::portal::tags::v1::__buffa::view::TagPolicyView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                TAG_POLICIES_SERVICE_SERVICE_NAME,
                "CreateTagPolicy",
                request,
                options,
            )
            .await
    }
    /// Call the GetTagPolicy RPC. Sends a request to /portal.tags.v1.TagPoliciesService/GetTagPolicy.
    pub async fn get_tag_policy(
        &self,
        request: crate::generated::buffa::portal::tags::v1::GetTagPolicyRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::generated::buffa::portal::tags::v1::__buffa::view::TagPolicyView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.get_tag_policy_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the GetTagPolicy RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn get_tag_policy_with_options(
        &self,
        request: crate::generated::buffa::portal::tags::v1::GetTagPolicyRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::generated::buffa::portal::tags::v1::__buffa::view::TagPolicyView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                TAG_POLICIES_SERVICE_SERVICE_NAME,
                "GetTagPolicy",
                request,
                options,
            )
            .await
    }
    /// Call the UpdateTagPolicy RPC. Sends a request to /portal.tags.v1.TagPoliciesService/UpdateTagPolicy.
    pub async fn update_tag_policy(
        &self,
        request: crate::generated::buffa::portal::tags::v1::UpdateTagPolicyRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::generated::buffa::portal::tags::v1::__buffa::view::TagPolicyView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.update_tag_policy_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the UpdateTagPolicy RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn update_tag_policy_with_options(
        &self,
        request: crate::generated::buffa::portal::tags::v1::UpdateTagPolicyRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::generated::buffa::portal::tags::v1::__buffa::view::TagPolicyView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                TAG_POLICIES_SERVICE_SERVICE_NAME,
                "UpdateTagPolicy",
                request,
                options,
            )
            .await
    }
    /// Call the DeleteTagPolicy RPC. Sends a request to /portal.tags.v1.TagPoliciesService/DeleteTagPolicy.
    pub async fn delete_tag_policy(
        &self,
        request: crate::generated::buffa::portal::tags::v1::DeleteTagPolicyRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                ::buffa_types::google::protobuf::__buffa::view::EmptyView<'static>,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.delete_tag_policy_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the DeleteTagPolicy RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn delete_tag_policy_with_options(
        &self,
        request: crate::generated::buffa::portal::tags::v1::DeleteTagPolicyRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                ::buffa_types::google::protobuf::__buffa::view::EmptyView<'static>,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                TAG_POLICIES_SERVICE_SERVICE_NAME,
                "DeleteTagPolicy",
                request,
                options,
            )
            .await
    }
}
/// Full service name for this service.
pub const ENTITY_TAG_ASSIGNMENTS_SERVICE_SERVICE_NAME: &str = "portal.tags.v1.EntityTagAssignmentsService";
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ListEntityTagAssignments` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const ENTITY_TAG_ASSIGNMENTS_SERVICE_LIST_ENTITY_TAG_ASSIGNMENTS_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/portal.tags.v1.EntityTagAssignmentsService/ListEntityTagAssignments",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `CreateEntityTagAssignment` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const ENTITY_TAG_ASSIGNMENTS_SERVICE_CREATE_ENTITY_TAG_ASSIGNMENT_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/portal.tags.v1.EntityTagAssignmentsService/CreateEntityTagAssignment",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `GetEntityTagAssignment` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const ENTITY_TAG_ASSIGNMENTS_SERVICE_GET_ENTITY_TAG_ASSIGNMENT_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/portal.tags.v1.EntityTagAssignmentsService/GetEntityTagAssignment",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `UpdateEntityTagAssignment` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const ENTITY_TAG_ASSIGNMENTS_SERVICE_UPDATE_ENTITY_TAG_ASSIGNMENT_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/portal.tags.v1.EntityTagAssignmentsService/UpdateEntityTagAssignment",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `DeleteEntityTagAssignment` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const ENTITY_TAG_ASSIGNMENTS_SERVICE_DELETE_ENTITY_TAG_ASSIGNMENT_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/portal.tags.v1.EntityTagAssignmentsService/DeleteEntityTagAssignment",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Manage assignments of tags to lakehouse entities.
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
pub trait EntityTagAssignmentsService: Send + Sync + 'static {
    /// List the tag assignments for the specified entity.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn list_entity_tag_assignments<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::generated::buffa::portal::tags::v1::ListEntityTagAssignmentsRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::generated::buffa::portal::tags::v1::ListEntityTagAssignmentsResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Assign a tag to a lakehouse entity.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn create_entity_tag_assignment<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::generated::buffa::portal::tags::v1::CreateEntityTagAssignmentRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::generated::buffa::portal::tags::v1::EntityTagAssignment,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Get the tag assignment for the specified entity and tag key.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn get_entity_tag_assignment<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::generated::buffa::portal::tags::v1::GetEntityTagAssignmentRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::generated::buffa::portal::tags::v1::EntityTagAssignment,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Update the tag assignment for the specified entity and tag key.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn update_entity_tag_assignment<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::generated::buffa::portal::tags::v1::UpdateEntityTagAssignmentRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::generated::buffa::portal::tags::v1::EntityTagAssignment,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Delete the tag assignment for the specified entity and tag key.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn delete_entity_tag_assignment<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::generated::buffa::portal::tags::v1::DeleteEntityTagAssignmentRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                ::buffa_types::google::protobuf::Empty,
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
pub trait EntityTagAssignmentsServiceExt: EntityTagAssignmentsService {
    /// Register this service implementation with a Router.
    ///
    /// Takes ownership of the `Arc<Self>` and returns a new Router with
    /// this service's methods registered.
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router;
}
impl<S: EntityTagAssignmentsService> EntityTagAssignmentsServiceExt for S {
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router {
        router
            .route_view(
                ENTITY_TAG_ASSIGNMENTS_SERVICE_SERVICE_NAME,
                "ListEntityTagAssignments",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::generated::buffa::portal::tags::v1::__buffa::view::ListEntityTagAssignmentsRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::generated::buffa::portal::tags::v1::ListEntityTagAssignmentsRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.list_entity_tag_assignments(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::generated::buffa::portal::tags::v1::ListEntityTagAssignmentsResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(ENTITY_TAG_ASSIGNMENTS_SERVICE_LIST_ENTITY_TAG_ASSIGNMENTS_SPEC)
            .route_view(
                ENTITY_TAG_ASSIGNMENTS_SERVICE_SERVICE_NAME,
                "CreateEntityTagAssignment",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::generated::buffa::portal::tags::v1::__buffa::view::CreateEntityTagAssignmentRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::generated::buffa::portal::tags::v1::CreateEntityTagAssignmentRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.create_entity_tag_assignment(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::generated::buffa::portal::tags::v1::EntityTagAssignment,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(ENTITY_TAG_ASSIGNMENTS_SERVICE_CREATE_ENTITY_TAG_ASSIGNMENT_SPEC)
            .route_view(
                ENTITY_TAG_ASSIGNMENTS_SERVICE_SERVICE_NAME,
                "GetEntityTagAssignment",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::generated::buffa::portal::tags::v1::__buffa::view::GetEntityTagAssignmentRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::generated::buffa::portal::tags::v1::GetEntityTagAssignmentRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.get_entity_tag_assignment(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::generated::buffa::portal::tags::v1::EntityTagAssignment,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(ENTITY_TAG_ASSIGNMENTS_SERVICE_GET_ENTITY_TAG_ASSIGNMENT_SPEC)
            .route_view(
                ENTITY_TAG_ASSIGNMENTS_SERVICE_SERVICE_NAME,
                "UpdateEntityTagAssignment",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::generated::buffa::portal::tags::v1::__buffa::view::UpdateEntityTagAssignmentRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::generated::buffa::portal::tags::v1::UpdateEntityTagAssignmentRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.update_entity_tag_assignment(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::generated::buffa::portal::tags::v1::EntityTagAssignment,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(ENTITY_TAG_ASSIGNMENTS_SERVICE_UPDATE_ENTITY_TAG_ASSIGNMENT_SPEC)
            .route_view(
                ENTITY_TAG_ASSIGNMENTS_SERVICE_SERVICE_NAME,
                "DeleteEntityTagAssignment",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::generated::buffa::portal::tags::v1::__buffa::view::DeleteEntityTagAssignmentRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::generated::buffa::portal::tags::v1::DeleteEntityTagAssignmentRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.delete_entity_tag_assignment(ctx, sreq)
                                .await?
                                .encode::<::buffa_types::google::protobuf::Empty>(format)
                        }
                    })
                },
            )
            .with_spec(ENTITY_TAG_ASSIGNMENTS_SERVICE_DELETE_ENTITY_TAG_ASSIGNMENT_SPEC)
    }
}
/// Monomorphic dispatcher for `EntityTagAssignmentsService`.
///
/// Unlike `.register(Router)` which type-erases each method into an `Arc<dyn ErasedHandler>` stored in a `HashMap`, this struct dispatches via a compile-time `match` on method name: no vtable, no hash lookup.
///
/// # Example
///
/// ```rust,ignore
/// use connectrpc::ConnectRpcService;
///
/// let server = EntityTagAssignmentsServiceServer::new(MyImpl);
/// let service = ConnectRpcService::new(server);
/// // hand `service` to axum/hyper as a fallback_service
/// ```
pub struct EntityTagAssignmentsServiceServer<T> {
    inner: ::std::sync::Arc<T>,
}
impl<T: EntityTagAssignmentsService> EntityTagAssignmentsServiceServer<T> {
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
impl<T> Clone for EntityTagAssignmentsServiceServer<T> {
    fn clone(&self) -> Self {
        Self {
            inner: ::std::sync::Arc::clone(&self.inner),
        }
    }
}
impl<T: EntityTagAssignmentsService> ::connectrpc::Dispatcher
for EntityTagAssignmentsServiceServer<T> {
    #[inline]
    fn lookup(
        &self,
        path: &str,
    ) -> Option<::connectrpc::dispatcher::codegen::MethodDescriptor> {
        let method = path.strip_prefix("portal.tags.v1.EntityTagAssignmentsService/")?;
        match method {
            "ListEntityTagAssignments" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(
                            ENTITY_TAG_ASSIGNMENTS_SERVICE_LIST_ENTITY_TAG_ASSIGNMENTS_SPEC,
                        ),
                )
            }
            "CreateEntityTagAssignment" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(
                            ENTITY_TAG_ASSIGNMENTS_SERVICE_CREATE_ENTITY_TAG_ASSIGNMENT_SPEC,
                        ),
                )
            }
            "GetEntityTagAssignment" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(
                            ENTITY_TAG_ASSIGNMENTS_SERVICE_GET_ENTITY_TAG_ASSIGNMENT_SPEC,
                        ),
                )
            }
            "UpdateEntityTagAssignment" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(
                            ENTITY_TAG_ASSIGNMENTS_SERVICE_UPDATE_ENTITY_TAG_ASSIGNMENT_SPEC,
                        ),
                )
            }
            "DeleteEntityTagAssignment" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(
                            ENTITY_TAG_ASSIGNMENTS_SERVICE_DELETE_ENTITY_TAG_ASSIGNMENT_SPEC,
                        ),
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
        let Some(method) = path
            .strip_prefix("portal.tags.v1.EntityTagAssignmentsService/") else {
            return ::connectrpc::dispatcher::codegen::unimplemented_unary(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            "ListEntityTagAssignments" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::generated::buffa::portal::tags::v1::ListEntityTagAssignmentsRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::generated::buffa::portal::tags::v1::__buffa::view::ListEntityTagAssignmentsRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::generated::buffa::portal::tags::v1::ListEntityTagAssignmentsRequest,
                    >::from_parts(&req, &body);
                    svc.list_entity_tag_assignments(ctx, req)
                        .await?
                        .encode::<
                            crate::generated::buffa::portal::tags::v1::ListEntityTagAssignmentsResponse,
                        >(format)
                })
            }
            "CreateEntityTagAssignment" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::generated::buffa::portal::tags::v1::CreateEntityTagAssignmentRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::generated::buffa::portal::tags::v1::__buffa::view::CreateEntityTagAssignmentRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::generated::buffa::portal::tags::v1::CreateEntityTagAssignmentRequest,
                    >::from_parts(&req, &body);
                    svc.create_entity_tag_assignment(ctx, req)
                        .await?
                        .encode::<
                            crate::generated::buffa::portal::tags::v1::EntityTagAssignment,
                        >(format)
                })
            }
            "GetEntityTagAssignment" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::generated::buffa::portal::tags::v1::GetEntityTagAssignmentRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::generated::buffa::portal::tags::v1::__buffa::view::GetEntityTagAssignmentRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::generated::buffa::portal::tags::v1::GetEntityTagAssignmentRequest,
                    >::from_parts(&req, &body);
                    svc.get_entity_tag_assignment(ctx, req)
                        .await?
                        .encode::<
                            crate::generated::buffa::portal::tags::v1::EntityTagAssignment,
                        >(format)
                })
            }
            "UpdateEntityTagAssignment" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::generated::buffa::portal::tags::v1::UpdateEntityTagAssignmentRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::generated::buffa::portal::tags::v1::__buffa::view::UpdateEntityTagAssignmentRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::generated::buffa::portal::tags::v1::UpdateEntityTagAssignmentRequest,
                    >::from_parts(&req, &body);
                    svc.update_entity_tag_assignment(ctx, req)
                        .await?
                        .encode::<
                            crate::generated::buffa::portal::tags::v1::EntityTagAssignment,
                        >(format)
                })
            }
            "DeleteEntityTagAssignment" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::generated::buffa::portal::tags::v1::DeleteEntityTagAssignmentRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::generated::buffa::portal::tags::v1::__buffa::view::DeleteEntityTagAssignmentRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::generated::buffa::portal::tags::v1::DeleteEntityTagAssignmentRequest,
                    >::from_parts(&req, &body);
                    svc.delete_entity_tag_assignment(ctx, req)
                        .await?
                        .encode::<::buffa_types::google::protobuf::Empty>(format)
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
        let Some(method) = path
            .strip_prefix("portal.tags.v1.EntityTagAssignmentsService/") else {
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
        let Some(method) = path
            .strip_prefix("portal.tags.v1.EntityTagAssignmentsService/") else {
            return ::connectrpc::dispatcher::codegen::unimplemented_unary(path);
        };
        let _ = (&ctx, &requests, &format);
        match method {
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
        let Some(method) = path
            .strip_prefix("portal.tags.v1.EntityTagAssignmentsService/") else {
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
/// let client = EntityTagAssignmentsServiceClient::new(conn, config);
/// let response = client.list_entity_tag_assignments(request).await?;
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
/// let client = EntityTagAssignmentsServiceClient::new(http, config);
/// let response = client.list_entity_tag_assignments(request).await?;
/// ```
///
/// # Working with the response
///
/// Unary calls return [`UnaryResponse<OwnedView<FooView>>`](::connectrpc::client::UnaryResponse).
/// [`view()`](::connectrpc::client::UnaryResponse::view) borrows the response
/// message, so field access is zero-copy:
///
/// ```rust,ignore
/// let resp = client.list_entity_tag_assignments(request).await?;
/// let name: &str = resp.view().name;  // borrow into the response buffer
/// ```
///
/// If you need the owned struct (e.g. to store or pass by value), use
/// [`into_owned()`](::connectrpc::client::UnaryResponse::into_owned):
///
/// ```rust,ignore
/// let owned = client.list_entity_tag_assignments(request).await?.into_owned();
/// ```
///
/// [`into_view()`](::connectrpc::client::UnaryResponse::into_view) keeps the
/// zero-copy decoded body (an `OwnedView`) without copying; field access on it
/// goes through `.reborrow()`. Streaming responses yield one `OwnedView` per
/// received message from `.message().await` — bind `msg.reborrow()` for field
/// access, or convert with `.to_owned_message()`.
#[derive(Clone)]
pub struct EntityTagAssignmentsServiceClient<T> {
    transport: T,
    config: ::connectrpc::client::ClientConfig,
}
impl<T> EntityTagAssignmentsServiceClient<T>
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
    /// Call the ListEntityTagAssignments RPC. Sends a request to /portal.tags.v1.EntityTagAssignmentsService/ListEntityTagAssignments.
    pub async fn list_entity_tag_assignments(
        &self,
        request: crate::generated::buffa::portal::tags::v1::ListEntityTagAssignmentsRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::generated::buffa::portal::tags::v1::__buffa::view::ListEntityTagAssignmentsResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.list_entity_tag_assignments_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ListEntityTagAssignments RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn list_entity_tag_assignments_with_options(
        &self,
        request: crate::generated::buffa::portal::tags::v1::ListEntityTagAssignmentsRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::generated::buffa::portal::tags::v1::__buffa::view::ListEntityTagAssignmentsResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                ENTITY_TAG_ASSIGNMENTS_SERVICE_SERVICE_NAME,
                "ListEntityTagAssignments",
                request,
                options,
            )
            .await
    }
    /// Call the CreateEntityTagAssignment RPC. Sends a request to /portal.tags.v1.EntityTagAssignmentsService/CreateEntityTagAssignment.
    pub async fn create_entity_tag_assignment(
        &self,
        request: crate::generated::buffa::portal::tags::v1::CreateEntityTagAssignmentRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::generated::buffa::portal::tags::v1::__buffa::view::EntityTagAssignmentView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.create_entity_tag_assignment_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the CreateEntityTagAssignment RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn create_entity_tag_assignment_with_options(
        &self,
        request: crate::generated::buffa::portal::tags::v1::CreateEntityTagAssignmentRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::generated::buffa::portal::tags::v1::__buffa::view::EntityTagAssignmentView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                ENTITY_TAG_ASSIGNMENTS_SERVICE_SERVICE_NAME,
                "CreateEntityTagAssignment",
                request,
                options,
            )
            .await
    }
    /// Call the GetEntityTagAssignment RPC. Sends a request to /portal.tags.v1.EntityTagAssignmentsService/GetEntityTagAssignment.
    pub async fn get_entity_tag_assignment(
        &self,
        request: crate::generated::buffa::portal::tags::v1::GetEntityTagAssignmentRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::generated::buffa::portal::tags::v1::__buffa::view::EntityTagAssignmentView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.get_entity_tag_assignment_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the GetEntityTagAssignment RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn get_entity_tag_assignment_with_options(
        &self,
        request: crate::generated::buffa::portal::tags::v1::GetEntityTagAssignmentRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::generated::buffa::portal::tags::v1::__buffa::view::EntityTagAssignmentView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                ENTITY_TAG_ASSIGNMENTS_SERVICE_SERVICE_NAME,
                "GetEntityTagAssignment",
                request,
                options,
            )
            .await
    }
    /// Call the UpdateEntityTagAssignment RPC. Sends a request to /portal.tags.v1.EntityTagAssignmentsService/UpdateEntityTagAssignment.
    pub async fn update_entity_tag_assignment(
        &self,
        request: crate::generated::buffa::portal::tags::v1::UpdateEntityTagAssignmentRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::generated::buffa::portal::tags::v1::__buffa::view::EntityTagAssignmentView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.update_entity_tag_assignment_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the UpdateEntityTagAssignment RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn update_entity_tag_assignment_with_options(
        &self,
        request: crate::generated::buffa::portal::tags::v1::UpdateEntityTagAssignmentRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::generated::buffa::portal::tags::v1::__buffa::view::EntityTagAssignmentView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                ENTITY_TAG_ASSIGNMENTS_SERVICE_SERVICE_NAME,
                "UpdateEntityTagAssignment",
                request,
                options,
            )
            .await
    }
    /// Call the DeleteEntityTagAssignment RPC. Sends a request to /portal.tags.v1.EntityTagAssignmentsService/DeleteEntityTagAssignment.
    pub async fn delete_entity_tag_assignment(
        &self,
        request: crate::generated::buffa::portal::tags::v1::DeleteEntityTagAssignmentRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                ::buffa_types::google::protobuf::__buffa::view::EmptyView<'static>,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.delete_entity_tag_assignment_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the DeleteEntityTagAssignment RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn delete_entity_tag_assignment_with_options(
        &self,
        request: crate::generated::buffa::portal::tags::v1::DeleteEntityTagAssignmentRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                ::buffa_types::google::protobuf::__buffa::view::EmptyView<'static>,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                ENTITY_TAG_ASSIGNMENTS_SERVICE_SERVICE_NAME,
                "DeleteEntityTagAssignment",
                request,
                options,
            )
            .await
    }
}
