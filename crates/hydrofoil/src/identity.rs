//! Hydrofoil's principal-identity wiring.
//!
//! Bridges authenticated gRPC request metadata into the [`PrincipalIdentity`]
//! the Cedar policy layer authorizes against — the access-control analog of
//! [`crate::lineage`]'s OpenLineage context wiring.
//!
//! **Trust boundary:** the metadata keys below are the *transport* seam, not the
//! *trust* boundary. In production the principal must be established by real
//! authentication (a tonic interceptor validating a bearer token / mTLS
//! subject) upstream of [`principal_from_metadata`]; a client-asserted header is
//! never trusted on its own. Until that interceptor lands, this parses the
//! principal directly from metadata for local/dev use.
//!
//! [`principal_from_metadata`] is wired into the server, and [`with_principal`]
//! is now called when a session is built (`Engine::new_session` →
//! `create_session_for`), so the principal reaches planning-time policy
//! evaluation via the [`PrincipalExt`] `SessionConfig` extension. The
//! [`PrincipalProvider`] read-back trait is the provider-facing seam for
//! components that only see a `SessionState` (mirroring
//! [`datafusion_open_lineage::context::LineageContextProvider`]); it is retained
//! for that use even though the policy gate currently reads the principal from a
//! struct field.

use std::sync::Arc;

use datafusion::execution::context::SessionState;
use datafusion::prelude::SessionConfig;
use datafusion_cedar::{EntityUid, PrincipalIdentity};
use tonic::metadata::MetadataMap;

/// gRPC metadata keys carrying the principal and its attributes.
pub mod headers {
    /// The principal's Cedar entity uid, e.g. `User::"alice"`.
    pub const PRINCIPAL: &str = "x-hydrofoil-principal";
    /// The principal's role (folded into `principal.role`).
    pub const ROLE: &str = "x-hydrofoil-role";
    /// The principal's region (folded into `principal.region`).
    pub const REGION: &str = "x-hydrofoil-region";
}

/// The default principal used when a request carries no principal metadata.
///
/// An unauthenticated/local caller; policies decide what `User::"anonymous"`
/// may do.
pub const DEFAULT_PRINCIPAL: &str = "User::\"anonymous\"";

/// The `SessionConfig` extension type carrying the per-request principal.
///
/// A distinct newtype so `SessionConfig::get_extension` (which keys by
/// `TypeId`) resolves it unambiguously.
#[derive(Debug, Clone)]
pub struct PrincipalExt(pub PrincipalIdentity);

/// Parse the principal (and its attributes) from gRPC request metadata.
///
/// Falls back to [`DEFAULT_PRINCIPAL`] when no principal header is present.
/// Errors only when a supplied principal uid fails to parse as a Cedar
/// `EntityUid`.
pub fn principal_from_metadata(meta: &MetadataMap) -> Result<PrincipalIdentity, tonic::Status> {
    let get = |key: &str| meta.get(key).and_then(|v| v.to_str().ok()).map(str::to_string);

    let uid_str = get(headers::PRINCIPAL).unwrap_or_else(|| DEFAULT_PRINCIPAL.to_string());
    let uid: EntityUid = uid_str
        .parse()
        .map_err(|e| tonic::Status::invalid_argument(format!("invalid principal '{uid_str}': {e}")))?;

    let mut identity = PrincipalIdentity::new(uid);
    if let Some(role) = get(headers::ROLE) {
        identity = identity.with_attribute("role", role);
    }
    if let Some(region) = get(headers::REGION) {
        identity = identity.with_attribute("region", region);
    }
    Ok(identity)
}

/// Attach a [`PrincipalIdentity`] to a `SessionConfig` so the
/// [`SessionConfigPrincipalProvider`] can read it back during planning.
pub fn with_principal(config: SessionConfig, principal: PrincipalIdentity) -> SessionConfig {
    config.with_extension(Arc::new(PrincipalExt(principal)))
}

/// Resolves the principal a query runs as, from a [`SessionState`].
///
/// Mirrors [`datafusion_open_lineage::context::LineageContextProvider`]: the
/// provider only sees a `SessionState`, so per-request data reaches it through
/// a typed `SessionConfig` extension. Retained as the provider-facing seam; the
/// policy gate currently reads the principal from a `LakehouseSession` field.
#[allow(dead_code)]
#[async_trait::async_trait]
pub trait PrincipalProvider: std::fmt::Debug + Send + Sync {
    async fn principal(&self, session_state: &SessionState) -> Option<PrincipalIdentity>;
}

/// A [`PrincipalProvider`] that reads the [`PrincipalIdentity`] attached to the
/// session's `SessionConfig` as a [`PrincipalExt`] extension.
#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct SessionConfigPrincipalProvider;

#[async_trait::async_trait]
impl PrincipalProvider for SessionConfigPrincipalProvider {
    async fn principal(&self, session_state: &SessionState) -> Option<PrincipalIdentity> {
        session_state
            .config()
            .get_extension::<PrincipalExt>()
            .map(|ext| ext.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_anonymous_when_absent() {
        let id = principal_from_metadata(&MetadataMap::new()).unwrap();
        assert_eq!(id.uid.to_string(), DEFAULT_PRINCIPAL);
        assert!(id.attributes.is_empty());
    }

    #[test]
    fn parses_principal_and_attributes() {
        let mut meta = MetadataMap::new();
        meta.insert(headers::PRINCIPAL, "User::\"alice\"".parse().unwrap());
        meta.insert(headers::ROLE, "analyst".parse().unwrap());
        meta.insert(headers::REGION, "eu".parse().unwrap());

        let id = principal_from_metadata(&meta).unwrap();
        assert_eq!(id.uid.to_string(), "User::\"alice\"");
        assert!(id.attributes.contains_key("role"));
        assert!(id.attributes.contains_key("region"));
    }

    #[test]
    fn rejects_malformed_principal() {
        let mut meta = MetadataMap::new();
        meta.insert(headers::PRINCIPAL, "not a valid uid".parse().unwrap());
        assert!(principal_from_metadata(&meta).is_err());
    }
}
