use std::{
    fmt::{Display, Formatter},
    pin::Pin,
    str::FromStr,
    task::{Context, Poll},
    time::Duration,
};

use bytes::Bytes;
use datafusion::common::HashMap;
use http::{HeaderMap, HeaderValue, Response};
use http_body_util::BodyExt as _;
use humantime::{format_duration, parse_duration};
use hyper::body::{Body, Frame};
#[cfg(not(target_arch = "wasm32"))]
use object_store::Certificate;
use object_store::{
    ClientConfigKey, ClientOptions, Error, Result,
    client::{
        HttpClient, HttpConnector, HttpError, HttpErrorKind, HttpRequest, HttpResponse,
        HttpResponseBody, HttpService,
    },
};
use reqwest::{NoProxy, Proxy, dns};
use thiserror::Error;
use tokio::{runtime::Handle, task::JoinHandle};
use tracing::dispatcher;

/// Spawn error
#[derive(Debug, Error)]
#[error("SpawnError")]
struct SpawnError {}

impl From<SpawnError> for HttpError {
    fn from(value: SpawnError) -> Self {
        Self::new(HttpErrorKind::Interrupted, value)
    }
}

#[derive(Debug)]
#[allow(missing_copy_implementations)]
pub struct SpawnedTracedReqwestConnector {
    runtime: Handle,
}

impl SpawnedTracedReqwestConnector {
    /// Create a new [`SpawnedReqwestConnector`] with the provided [`Handle`] to
    /// a tokio [`Runtime`]
    ///
    /// [`Runtime`]: tokio::runtime::Runtime
    pub fn new(runtime: Handle) -> Self {
        Self { runtime }
    }
}

impl HttpConnector for SpawnedTracedReqwestConnector {
    fn connect(&self, options: &ClientOptions) -> Result<HttpClient> {
        let spawn_service = SpawnService::new(
            ClientOptionsInner::new(options).client()?,
            self.runtime.clone(),
        );
        Ok(HttpClient::new(spawn_service))
    }
}

/// Wraps a provided [`HttpService`] and runs it on a separate tokio runtime
///
/// See example on [`SpawnedReqwestConnector`]
///
/// [`SpawnedReqwestConnector`]: crate::client::http::SpawnedReqwestConnector
#[derive(Debug)]
pub struct SpawnService<T: HttpService + Clone> {
    inner: T,
    runtime: Handle,
}

impl<T: HttpService + Clone> SpawnService<T> {
    /// Creates a new [`SpawnService`] from the provided
    pub fn new(inner: T, runtime: Handle) -> Self {
        Self { inner, runtime }
    }
}

#[async_trait::async_trait]
impl<T: HttpService + Clone> HttpService for SpawnService<T> {
    async fn call(&self, req: HttpRequest) -> Result<HttpResponse, HttpError> {
        let inner = self.inner.clone();
        let (send, recv) = tokio::sync::oneshot::channel();

        // We use an unbounded channel to prevent backpressure across the runtime boundary
        // which could in turn starve the underlying IO operations
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();

        let dispatch = dispatcher::get_default(|d| d.clone());
        let span = tracing::Span::current();

        let handle = SpawnHandle(self.runtime.spawn(async move {
            dispatcher::with_default(&dispatch, || async {
                let _guard = span.enter();

                let r = match HttpService::call(&inner, req).await {
                    Ok(resp) => resp,
                    Err(e) => {
                        let _ = send.send(Err(e));
                        return;
                    }
                };

                let (parts, mut body) = r.into_parts();
                if send.send(Ok(parts)).is_err() {
                    return;
                }

                while let Some(x) = body.frame().await {
                    if sender.send(x).is_err() {
                        return;
                    }
                }
            })
            .await;
        }));

        let parts = recv.await.map_err(|_| SpawnError {})??;

        Ok(Response::from_parts(
            parts,
            HttpResponseBody::new(SpawnBody {
                stream: receiver,
                _worker: handle,
            }),
        ))
    }
}

type StreamItem = Result<Frame<Bytes>, HttpError>;
struct SpawnBody {
    stream: tokio::sync::mpsc::UnboundedReceiver<StreamItem>,
    _worker: SpawnHandle,
}

impl Body for SpawnBody {
    type Data = Bytes;
    type Error = HttpError;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<StreamItem>> {
        self.stream.poll_recv(cx)
    }
}

/// A wrapper around a [`JoinHandle`] that aborts on drop
struct SpawnHandle(JoinHandle<()>);
impl Drop for SpawnHandle {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// HTTP client configuration for remote object stores
#[derive(Debug, Clone)]
pub struct ClientOptionsInner {
    user_agent: Option<ConfigValue<HeaderValue>>,
    #[cfg(not(target_arch = "wasm32"))]
    root_certificates: Vec<Certificate>,
    content_type_map: HashMap<String, String>,
    default_content_type: Option<String>,
    default_headers: Option<HeaderMap>,
    proxy_url: Option<String>,
    proxy_ca_certificate: Option<String>,
    proxy_excludes: Option<String>,
    allow_http: ConfigValue<bool>,
    allow_insecure: ConfigValue<bool>,
    timeout: Option<ConfigValue<Duration>>,
    connect_timeout: Option<ConfigValue<Duration>>,
    pool_idle_timeout: Option<ConfigValue<Duration>>,
    pool_max_idle_per_host: Option<ConfigValue<usize>>,
    http2_keep_alive_interval: Option<ConfigValue<Duration>>,
    http2_keep_alive_timeout: Option<ConfigValue<Duration>>,
    http2_keep_alive_while_idle: ConfigValue<bool>,
    http2_max_frame_size: Option<ConfigValue<u32>>,
    http1_only: ConfigValue<bool>,
    http2_only: ConfigValue<bool>,
    randomize_addresses: ConfigValue<bool>,
}

impl Default for ClientOptionsInner {
    fn default() -> Self {
        // Defaults based on
        // <https://docs.aws.amazon.com/sdkref/latest/guide/feature-smart-config-defaults.html>
        // <https://docs.aws.amazon.com/whitepapers/latest/s3-optimizing-performance-best-practices/timeouts-and-retries-for-latency-sensitive-applications.html>
        // Which recommend a connection timeout of 3.1s and a request timeout of 2s
        //
        // As object store requests may involve the transfer of non-trivial volumes of data
        // we opt for a slightly higher default timeout of 30 seconds
        Self {
            user_agent: None,
            #[cfg(not(target_arch = "wasm32"))]
            root_certificates: Default::default(),
            content_type_map: Default::default(),
            default_content_type: None,
            default_headers: None,
            proxy_url: None,
            proxy_ca_certificate: None,
            proxy_excludes: None,
            allow_http: Default::default(),
            allow_insecure: Default::default(),
            timeout: Some(Duration::from_secs(30).into()),
            connect_timeout: Some(Duration::from_secs(5).into()),
            pool_idle_timeout: None,
            pool_max_idle_per_host: None,
            http2_keep_alive_interval: None,
            http2_keep_alive_timeout: None,
            http2_keep_alive_while_idle: Default::default(),
            http2_max_frame_size: None,
            // HTTP2 is known to be significantly slower than HTTP1, so we default
            // to HTTP1 for now.
            // https://github.com/apache/arrow-rs/issues/5194
            http1_only: true.into(),
            http2_only: Default::default(),
            randomize_addresses: true.into(),
        }
    }
}

impl ClientOptionsInner {
    /// Create a new [`ClientOptions`] with default values
    pub fn new(options: &ClientOptions) -> Self {
        let all_config_keys = &[
            ClientConfigKey::AllowHttp,
            ClientConfigKey::AllowInvalidCertificates,
            ClientConfigKey::ConnectTimeout,
            ClientConfigKey::DefaultContentType,
            ClientConfigKey::Http1Only,
            ClientConfigKey::Http2Only,
            ClientConfigKey::Http2KeepAliveInterval,
            ClientConfigKey::Http2KeepAliveTimeout,
            ClientConfigKey::Http2KeepAliveWhileIdle,
            ClientConfigKey::Http2MaxFrameSize,
            ClientConfigKey::PoolIdleTimeout,
            ClientConfigKey::PoolMaxIdlePerHost,
            ClientConfigKey::ProxyUrl,
            ClientConfigKey::ProxyCaCertificate,
            ClientConfigKey::ProxyExcludes,
            ClientConfigKey::RandomizeAddresses,
            ClientConfigKey::Timeout,
            ClientConfigKey::UserAgent,
        ];
        let mut config = Self::default();
        for key in all_config_keys {
            if let Some(value) = options.get_config_value(key) {
                config = config.with_config(key.clone(), value);
            }
        }
        config
    }

    /// Set an option by key
    pub fn with_config(mut self, key: ClientConfigKey, value: impl Into<String>) -> Self {
        match key {
            ClientConfigKey::AllowHttp => self.allow_http.parse(value),
            ClientConfigKey::AllowInvalidCertificates => self.allow_insecure.parse(value),
            ClientConfigKey::ConnectTimeout => {
                self.connect_timeout = Some(ConfigValue::Deferred(value.into()))
            }
            ClientConfigKey::DefaultContentType => self.default_content_type = Some(value.into()),
            ClientConfigKey::Http1Only => self.http1_only.parse(value),
            ClientConfigKey::Http2Only => self.http2_only.parse(value),
            ClientConfigKey::Http2KeepAliveInterval => {
                self.http2_keep_alive_interval = Some(ConfigValue::Deferred(value.into()))
            }
            ClientConfigKey::Http2KeepAliveTimeout => {
                self.http2_keep_alive_timeout = Some(ConfigValue::Deferred(value.into()))
            }
            ClientConfigKey::Http2KeepAliveWhileIdle => {
                self.http2_keep_alive_while_idle.parse(value)
            }
            ClientConfigKey::Http2MaxFrameSize => {
                self.http2_max_frame_size = Some(ConfigValue::Deferred(value.into()))
            }
            ClientConfigKey::PoolIdleTimeout => {
                self.pool_idle_timeout = Some(ConfigValue::Deferred(value.into()))
            }
            ClientConfigKey::PoolMaxIdlePerHost => {
                self.pool_max_idle_per_host = Some(ConfigValue::Deferred(value.into()))
            }
            ClientConfigKey::ProxyUrl => self.proxy_url = Some(value.into()),
            ClientConfigKey::ProxyCaCertificate => self.proxy_ca_certificate = Some(value.into()),
            ClientConfigKey::ProxyExcludes => self.proxy_excludes = Some(value.into()),
            ClientConfigKey::RandomizeAddresses => {
                self.randomize_addresses.parse(value);
            }
            ClientConfigKey::Timeout => self.timeout = Some(ConfigValue::Deferred(value.into())),
            ClientConfigKey::UserAgent => {
                self.user_agent = Some(ConfigValue::Deferred(value.into()))
            }
            _ => todo!(),
        }
        self
    }

    /// Get an option by key
    pub fn get_config_value(&self, key: &ClientConfigKey) -> Option<String> {
        match key {
            ClientConfigKey::AllowHttp => Some(self.allow_http.to_string()),
            ClientConfigKey::AllowInvalidCertificates => Some(self.allow_insecure.to_string()),
            ClientConfigKey::ConnectTimeout => self.connect_timeout.as_ref().map(fmt_duration),
            ClientConfigKey::DefaultContentType => self.default_content_type.clone(),
            ClientConfigKey::Http1Only => Some(self.http1_only.to_string()),
            ClientConfigKey::Http2KeepAliveInterval => {
                self.http2_keep_alive_interval.as_ref().map(fmt_duration)
            }
            ClientConfigKey::Http2KeepAliveTimeout => {
                self.http2_keep_alive_timeout.as_ref().map(fmt_duration)
            }
            ClientConfigKey::Http2KeepAliveWhileIdle => {
                Some(self.http2_keep_alive_while_idle.to_string())
            }
            ClientConfigKey::Http2MaxFrameSize => {
                self.http2_max_frame_size.as_ref().map(|v| v.to_string())
            }
            ClientConfigKey::Http2Only => Some(self.http2_only.to_string()),
            ClientConfigKey::PoolIdleTimeout => self.pool_idle_timeout.as_ref().map(fmt_duration),
            ClientConfigKey::PoolMaxIdlePerHost => {
                self.pool_max_idle_per_host.as_ref().map(|v| v.to_string())
            }
            ClientConfigKey::ProxyUrl => self.proxy_url.clone(),
            ClientConfigKey::ProxyCaCertificate => self.proxy_ca_certificate.clone(),
            ClientConfigKey::ProxyExcludes => self.proxy_excludes.clone(),
            ClientConfigKey::RandomizeAddresses => Some(self.randomize_addresses.to_string()),
            ClientConfigKey::Timeout => self.timeout.as_ref().map(fmt_duration),
            ClientConfigKey::UserAgent => self
                .user_agent
                .as_ref()
                .and_then(|v| v.get().ok())
                .and_then(|v| v.to_str().ok().map(|s| s.to_string())),
            _ => todo!(),
        }
    }

    fn client(&self) -> Result<reqwest::Client> {
        let mut builder = reqwest::ClientBuilder::new();

        match &self.user_agent {
            Some(user_agent) => builder = builder.user_agent(user_agent.get()?),
            None => builder = builder.user_agent("hydrofoil-object-store/0.1"),
        }

        if let Some(headers) = &self.default_headers {
            builder = builder.default_headers(headers.clone())
        }

        if let Some(proxy) = &self.proxy_url {
            let mut proxy = Proxy::all(proxy).map_err(map_client_error)?;

            if let Some(certificate) = &self.proxy_ca_certificate {
                let certificate = reqwest::tls::Certificate::from_pem(certificate.as_bytes())
                    .map_err(map_client_error)?;

                builder = builder.add_root_certificate(certificate);
            }

            if let Some(proxy_excludes) = &self.proxy_excludes {
                let no_proxy = NoProxy::from_string(proxy_excludes);

                proxy = proxy.no_proxy(no_proxy);
            }

            builder = builder.proxy(proxy);
        }

        // for certificate in &self.root_certificates {
        //     builder = builder.add_root_certificate(certificate.0.clone());
        // }

        if let Some(timeout) = &self.timeout {
            builder = builder.timeout(timeout.get()?)
        }

        if let Some(timeout) = &self.connect_timeout {
            builder = builder.connect_timeout(timeout.get()?)
        }

        if let Some(timeout) = &self.pool_idle_timeout {
            builder = builder.pool_idle_timeout(timeout.get()?)
        }

        if let Some(max) = &self.pool_max_idle_per_host {
            builder = builder.pool_max_idle_per_host(max.get()?)
        }

        if let Some(interval) = &self.http2_keep_alive_interval {
            builder = builder.http2_keep_alive_interval(interval.get()?)
        }

        if let Some(interval) = &self.http2_keep_alive_timeout {
            builder = builder.http2_keep_alive_timeout(interval.get()?)
        }

        if self.http2_keep_alive_while_idle.get()? {
            builder = builder.http2_keep_alive_while_idle(true)
        }

        if let Some(sz) = &self.http2_max_frame_size {
            builder = builder.http2_max_frame_size(Some(sz.get()?))
        }

        if self.http1_only.get()? {
            builder = builder.http1_only()
        }

        if self.http2_only.get()? {
            builder = builder.http2_prior_knowledge()
        }

        if self.allow_insecure.get()? {
            builder = builder.danger_accept_invalid_certs(true)
        }

        // Explicitly disable compression, since it may be automatically enabled
        // when certain reqwest features are enabled. Compression interferes
        // with the `Content-Length` header, which is used to determine the
        // size of objects.
        builder = builder.no_gzip().no_brotli().no_zstd().no_deflate();

        // if self.randomize_addresses.get()? {
        //     builder = builder.dns_resolver(Arc::new(dns::ShuffleResolver));
        // }

        builder
            .https_only(!self.allow_http.get()?)
            .build()
            .map_err(map_client_error)
    }
}

/// Provides deferred parsing of a value
///
/// This allows builders to defer fallibility to build
#[derive(Debug, Clone)]
pub(crate) enum ConfigValue<T> {
    Parsed(T),
    Deferred(String),
}

impl<T: Display> Display for ConfigValue<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parsed(v) => write!(f, "{v}"),
            Self::Deferred(v) => write!(f, "{v}"),
        }
    }
}

impl<T> From<T> for ConfigValue<T> {
    fn from(value: T) -> Self {
        Self::Parsed(value)
    }
}

impl<T: Parse + Clone> ConfigValue<T> {
    pub(crate) fn parse(&mut self, v: impl Into<String>) {
        *self = Self::Deferred(v.into())
    }

    pub(crate) fn get(&self) -> Result<T> {
        match self {
            Self::Parsed(v) => Ok(v.clone()),
            Self::Deferred(v) => T::parse(v),
        }
    }
}

impl<T: Default> Default for ConfigValue<T> {
    fn default() -> Self {
        Self::Parsed(T::default())
    }
}

/// A value that can be stored in [`ConfigValue`]
pub(crate) trait Parse: Sized {
    fn parse(v: &str) -> Result<Self>;
}

impl Parse for bool {
    fn parse(v: &str) -> Result<Self> {
        let lower = v.to_ascii_lowercase();
        match lower.as_str() {
            "1" | "true" | "on" | "yes" | "y" => Ok(true),
            "0" | "false" | "off" | "no" | "n" => Ok(false),
            _ => Err(Error::Generic {
                store: "Config",
                source: format!("failed to parse \"{v}\" as boolean").into(),
            }),
        }
    }
}

impl Parse for Duration {
    fn parse(v: &str) -> Result<Self> {
        parse_duration(v).map_err(|_| Error::Generic {
            store: "Config",
            source: format!("failed to parse \"{v}\" as Duration").into(),
        })
    }
}

impl Parse for usize {
    fn parse(v: &str) -> Result<Self> {
        Self::from_str(v).map_err(|_| Error::Generic {
            store: "Config",
            source: format!("failed to parse \"{v}\" as usize").into(),
        })
    }
}

impl Parse for u32 {
    fn parse(v: &str) -> Result<Self> {
        Self::from_str(v).map_err(|_| Error::Generic {
            store: "Config",
            source: format!("failed to parse \"{v}\" as u32").into(),
        })
    }
}

impl Parse for HeaderValue {
    fn parse(v: &str) -> Result<Self> {
        Self::from_str(v).map_err(|_| Error::Generic {
            store: "Config",
            source: format!("failed to parse \"{v}\" as HeaderValue").into(),
        })
    }
}

pub(crate) fn fmt_duration(duration: &ConfigValue<Duration>) -> String {
    match duration {
        ConfigValue::Parsed(v) => format_duration(*v).to_string(),
        ConfigValue::Deferred(v) => v.clone(),
    }
}

fn map_client_error(e: reqwest::Error) -> Error {
    Error::Generic {
        store: "HTTP client",
        source: Box::new(e),
    }
}
