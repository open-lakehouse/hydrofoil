//! The OpenLineage client: a non-blocking emit front-end over a [`Transport`].
//!
//! Emission must never break or slow the host query. [`OpenLineageClient::emit`]
//! is non-blocking: it hands the event to a bounded channel drained by a
//! background task that calls the transport and swallows + logs any error. If
//! the channel is full the event is dropped with a warning (back-pressure must
//! not stall planning).

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::event::RunEvent;
use crate::transport::{NoopTransport, Transport};

/// Default bound on the in-flight event queue.
const DEFAULT_QUEUE_SIZE: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("invalid OpenLineage configuration: {0}")]
    Config(String),
}

/// Non-blocking front-end for emitting OpenLineage events.
#[derive(Debug, Clone)]
pub struct OpenLineageClient {
    tx: mpsc::Sender<RunEvent>,
}

impl OpenLineageClient {
    /// Start a client that drains events into `transport` on a background task.
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self::with_queue_size(transport, DEFAULT_QUEUE_SIZE)
    }

    pub fn with_queue_size(transport: Arc<dyn Transport>, queue_size: usize) -> Self {
        let (tx, mut rx) = mpsc::channel::<RunEvent>(queue_size);
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let Err(err) = transport.emit(&event).await {
                    tracing::warn!(
                        target: "openlineage",
                        error = %err,
                        "failed to emit lineage event; dropping"
                    );
                }
            }
        });
        Self { tx }
    }

    pub fn builder() -> OpenLineageClientBuilder {
        OpenLineageClientBuilder::default()
    }

    /// A client whose transport drops everything ([`NoopTransport`]).
    pub fn noop() -> Self {
        Self::new(Arc::new(NoopTransport))
    }

    /// Construct from the standard OpenLineage environment.
    ///
    /// If `OPENLINEAGE_URL` is set, builds an HTTP transport (requires the
    /// `http` feature); otherwise returns a no-op client. `OPENLINEAGE_API_KEY`,
    /// if present, is sent as a bearer token.
    pub fn from_env() -> Result<Self, ClientError> {
        match std::env::var("OPENLINEAGE_URL") {
            Ok(url) if !url.is_empty() => Self::http_from_env(&url),
            _ => Ok(Self::noop()),
        }
    }

    #[cfg(feature = "http")]
    fn http_from_env(url: &str) -> Result<Self, ClientError> {
        use crate::cloud::CloudClientTransport;

        let endpoint =
            std::env::var("OPENLINEAGE_ENDPOINT").unwrap_or_else(|_| "/api/v1/lineage".to_string());
        let full = url.trim_end_matches('/').to_string() + &endpoint;
        let endpoint_url = url::Url::parse(&full)
            .map_err(|e| ClientError::Config(format!("invalid OPENLINEAGE_URL/ENDPOINT: {e}")))?;

        let transport: Arc<dyn Transport> = match std::env::var("OPENLINEAGE_API_KEY") {
            Ok(token) if !token.is_empty() => {
                Arc::new(CloudClientTransport::with_token(endpoint_url, token))
            }
            _ => Arc::new(CloudClientTransport::unauthenticated(endpoint_url)),
        };
        Ok(Self::new(transport))
    }

    #[cfg(not(feature = "http"))]
    fn http_from_env(_url: &str) -> Result<Self, ClientError> {
        Err(ClientError::Config(
            "OPENLINEAGE_URL is set but the `http` feature is disabled".to_string(),
        ))
    }

    /// Emit an event without blocking. On a full queue the event is dropped
    /// with a warning — lineage never applies back-pressure to the query.
    pub fn emit(&self, event: RunEvent) {
        if let Err(err) = self.tx.try_send(event) {
            tracing::warn!(
                target: "openlineage",
                error = %err,
                "lineage queue full or closed; dropping event"
            );
        }
    }
}

#[derive(Default)]
pub struct OpenLineageClientBuilder {
    transport: Option<Arc<dyn Transport>>,
    queue_size: Option<usize>,
}

impl OpenLineageClientBuilder {
    pub fn transport(mut self, transport: Arc<dyn Transport>) -> Self {
        self.transport = Some(transport);
        self
    }

    pub fn queue_size(mut self, queue_size: usize) -> Self {
        self.queue_size = Some(queue_size);
        self
    }

    pub fn build(self) -> OpenLineageClient {
        let transport = self.transport.unwrap_or_else(|| Arc::new(NoopTransport));
        OpenLineageClient::with_queue_size(transport, self.queue_size.unwrap_or(DEFAULT_QUEUE_SIZE))
    }
}
