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
//! **Identity enrichment.** The header-derived `role`/`region` are *advisory*
//! once an [`IdentityProvider`] is configured: the provider keys off the
//! authenticated uid, and its facts are authoritative (they override
//! self-asserted attributes). **Group membership comes only from the provider,
//! never from a client header** — a client-claimed group would directly
//! over-authorize. The v1 [`ConfigIdentityProvider`] sources facts from a static
//! config map (moving what was hardcoded in the OCI entity bundle behind the
//! seam); a real IdP/directory backend is the same trait. See
//! `docs/adr/0008-principal-identity-resolution.md`.
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

use std::collections::HashMap;
use std::sync::Arc;

use datafusion::execution::context::SessionState;
use datafusion::prelude::SessionConfig;
use datafusion_cedar::{
    Entity, EntityUid, IdentityError, IdentityProvider, PrincipalClaims, PrincipalEnrichment,
    PrincipalIdentity, RestrictedExpression,
};
use tonic::metadata::MetadataMap;

/// gRPC metadata keys carrying the principal and its attributes.
pub mod headers {
    /// The principal's Cedar entity uid, e.g. `User::"alice"`.
    pub const PRINCIPAL: &str = "x-hydrofoil-principal";
    /// The principal's role (folded into `principal.role`).
    pub const ROLE: &str = "x-hydrofoil-role";
    /// The principal's region (folded into `principal.region`).
    pub const REGION: &str = "x-hydrofoil-region";
    /// The caller's Unity Catalog bearer token, forwarded verbatim to UC so it
    /// resolves tables and vends credentials as *that* user (demo per-user
    /// permissions). A distinct key from `authorization`, which the Flight path
    /// already uses as the session-id channel (see `crate::server`). On the HTTP
    /// surface the token rides the standard `Authorization: Bearer` header
    /// instead (see [`uc_token_from_http_headers`]).
    pub const UC_TOKEN: &str = "x-hydrofoil-uc-token";
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
    let get = |key: &str| {
        meta.get(key)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    principal_from_headers(get).map_err(|e| tonic::Status::invalid_argument(e.message))
}

/// The axum-`HeaderMap` analog of [`principal_from_metadata`], for the HTTP
/// query surface (`crate::http`).
///
/// Same trust boundary as the gRPC path (see module docs): until a real
/// authenticating interceptor lands, the principal is parsed straight from
/// headers for local/dev use. The UC user token (`Authorization: Bearer <token>`)
/// is *not* yet exchanged for a verified identity — that is the deferred
/// interceptor work; for now an explicit `x-hydrofoil-principal` selects the
/// principal and a bare bearer token falls back to [`DEFAULT_PRINCIPAL`].
pub fn principal_from_http_headers(
    headers: &axum::http::HeaderMap,
) -> Result<PrincipalIdentity, PrincipalParseError> {
    let get = |key: &str| {
        headers
            .get(key)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    principal_from_headers(get)
}

/// Extract the caller's Unity Catalog bearer token from gRPC metadata.
///
/// Read from the dedicated [`headers::UC_TOKEN`] (`x-hydrofoil-uc-token`) key —
/// *not* `authorization`, which the Flight path overloads as the session-id
/// channel ([`crate::server::session_id_from_metadata`]). `None` when absent, in
/// which case the session falls back to the server-wide UC token. Never logged
/// (it's a bearer JWT).
pub fn uc_token_from_metadata(meta: &MetadataMap) -> Option<String> {
    meta.get(headers::UC_TOKEN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .filter(|t| !t.is_empty())
}

/// Extract the caller's Unity Catalog bearer token from HTTP headers.
///
/// Reads the standard `Authorization: Bearer <uc-jwt>` header (the HTTP `/query`
/// surface has no session-id overload, unlike the Flight path). `None` when
/// absent or not a bearer token, in which case the session falls back to the
/// server-wide UC token. Never logged (it's a bearer JWT).
pub fn uc_token_from_http_headers(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|auth| auth.strip_prefix("Bearer "))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// A failure to parse a principal from request headers — the supplied uid was
/// not a valid Cedar `EntityUid`.
#[derive(Debug)]
pub struct PrincipalParseError {
    pub message: String,
}

/// Shared core of [`principal_from_metadata`] /
/// [`principal_from_http_headers`]: build the [`PrincipalIdentity`] from a
/// transport-agnostic header getter, folding in the advisory `role`/`region`
/// attributes. Falls back to [`DEFAULT_PRINCIPAL`] when no principal header is
/// present.
fn principal_from_headers(
    get: impl Fn(&str) -> Option<String>,
) -> Result<PrincipalIdentity, PrincipalParseError> {
    let uid_str = get(headers::PRINCIPAL).unwrap_or_else(|| DEFAULT_PRINCIPAL.to_string());
    let uid: EntityUid = uid_str.parse().map_err(|e| PrincipalParseError {
        message: format!("invalid principal '{uid_str}': {e}"),
    })?;

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

/// A user's facts in the static identity config: string attributes and the
/// uids of the groups it directly belongs to.
#[derive(Debug, Clone, Default)]
pub struct UserFacts {
    pub attributes: Vec<(String, String)>,
    pub groups: Vec<EntityUid>,
}

/// A group's facts: the uids of its parent groups (for the transitive closure).
#[derive(Debug, Clone, Default)]
pub struct GroupFacts {
    pub parents: Vec<EntityUid>,
}

/// A v1 [`IdentityProvider`] that resolves enrichment from a static config map.
///
/// This moves what was hardcoded in the OCI entity bundle
/// (`config/policies/lakhouse.entities.json`) behind the identity-PIP seam, so
/// the enrichment path is exercised end to end. A real `OidcIdentityProvider` /
/// `DirectoryIdentityProvider` (OIDC userinfo / SCIM / LDAP) is the same trait
/// with the same closure-walking shape; this is the local/dev backend.
///
/// `enrich` walks *up* from the principal — its direct groups, then those
/// groups' parents — collecting only that principal's closure (never the whole
/// directory). An unknown uid yields empty enrichment (a success), so the
/// anonymous path still works.
#[derive(Debug, Default)]
pub struct ConfigIdentityProvider {
    users: HashMap<String, UserFacts>,
    groups: HashMap<String, GroupFacts>,
}

impl ConfigIdentityProvider {
    pub fn new(users: HashMap<String, UserFacts>, groups: HashMap<String, GroupFacts>) -> Self {
        Self { users, groups }
    }

    /// Build the entity for a group uid, carrying its parent groups as Cedar
    /// parents so the hierarchy resolves.
    fn group_entity(&self, uid: &EntityUid) -> Entity {
        let parents: std::collections::HashSet<EntityUid> = self
            .groups
            .get(&uid.to_string())
            .map(|g| g.parents.iter().cloned().collect())
            .unwrap_or_default();
        Entity::new(uid.clone(), HashMap::new(), parents)
            .unwrap_or_else(|_| Entity::new_no_attrs(uid.clone(), Default::default()))
    }
}

#[async_trait::async_trait]
impl IdentityProvider for ConfigIdentityProvider {
    async fn enrich(
        &self,
        uid: &EntityUid,
        _claims: &PrincipalClaims,
    ) -> Result<PrincipalEnrichment, IdentityError> {
        let Some(user) = self.users.get(&uid.to_string()) else {
            // Unknown principal: no extra facts (a success, not an error).
            return Ok(PrincipalEnrichment::default());
        };

        let attributes = user
            .attributes
            .iter()
            .map(|(k, v)| (k.clone(), RestrictedExpression::new_string(v.clone())))
            .collect();

        // Walk up the group hierarchy from the principal's direct groups,
        // collecting the transitive closure of group entities (deduped).
        let mut group_entities = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut frontier: Vec<EntityUid> = user.groups.clone();
        while let Some(g) = frontier.pop() {
            if !seen.insert(g.to_string()) {
                continue;
            }
            if let Some(facts) = self.groups.get(&g.to_string()) {
                frontier.extend(facts.parents.iter().cloned());
            }
            group_entities.push(self.group_entity(&g));
        }

        Ok(PrincipalEnrichment {
            attributes,
            groups: user.groups.clone(),
            group_entities,
        })
    }
}

/// The default identity provider: a [`ConfigIdentityProvider`] seeded with the
/// demo principal/group facts that the OCI entity bundle
/// (`config/policies/lakhouse.entities.json`) hardcodes — `alice ∈
/// privileged_readers ⊂ readers`, `bob ∈ readers`, `Agent::"r2d2" ∈ readers`.
///
/// This makes dynamic, provider-sourced membership the live default (the
/// static-bundle facts now flow through the identity PIP). A deployment swaps in
/// a real IdP/directory provider via
/// [`FlightSqlServiceImpl::with_identity_provider`](crate::FlightSqlServiceImpl::with_identity_provider).
pub fn default_identity_provider() -> Arc<dyn IdentityProvider> {
    use std::str::FromStr as _;
    let g = |s: &str| EntityUid::from_str(s).unwrap();

    let users = HashMap::from([
        (
            "User::\"alice\"".to_string(),
            UserFacts {
                attributes: vec![("region".to_string(), "eu".to_string())],
                groups: vec![g("UserGroup::\"privileged_readers\"")],
            },
        ),
        (
            "User::\"bob\"".to_string(),
            UserFacts {
                attributes: vec![("region".to_string(), "us".to_string())],
                groups: vec![g("UserGroup::\"readers\"")],
            },
        ),
        (
            "Agent::\"r2d2\"".to_string(),
            UserFacts {
                attributes: vec![],
                groups: vec![g("UserGroup::\"readers\"")],
            },
        ),
    ]);
    let groups = HashMap::from([
        (
            "UserGroup::\"privileged_readers\"".to_string(),
            GroupFacts {
                parents: vec![g("UserGroup::\"readers\"")],
            },
        ),
        ("UserGroup::\"readers\"".to_string(), GroupFacts::default()),
    ]);
    Arc::new(ConfigIdentityProvider::new(users, groups))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid(s: &str) -> EntityUid {
        use std::str::FromStr as _;
        EntityUid::from_str(s).unwrap()
    }

    #[tokio::test]
    async fn config_provider_resolves_group_closure() {
        // alice ∈ privileged_readers ⊂ readers.
        let users = HashMap::from([(
            "User::\"alice\"".to_string(),
            UserFacts {
                attributes: vec![("region".to_string(), "eu".to_string())],
                groups: vec![uid("UserGroup::\"privileged_readers\"")],
            },
        )]);
        let groups = HashMap::from([
            (
                "UserGroup::\"privileged_readers\"".to_string(),
                GroupFacts {
                    parents: vec![uid("UserGroup::\"readers\"")],
                },
            ),
            ("UserGroup::\"readers\"".to_string(), GroupFacts::default()),
        ]);
        let provider = ConfigIdentityProvider::new(users, groups);

        let e = provider
            .enrich(&uid("User::\"alice\""), &PrincipalClaims::default())
            .await
            .unwrap();
        assert_eq!(e.groups, vec![uid("UserGroup::\"privileged_readers\"")]);
        // Closure includes both privileged_readers and readers.
        assert_eq!(e.group_entities.len(), 2);
        assert!(e.attributes.contains_key("region"));
    }

    #[tokio::test]
    async fn config_provider_unknown_uid_is_empty_success() {
        let provider = ConfigIdentityProvider::default();
        let e = provider
            .enrich(&uid("User::\"nobody\""), &PrincipalClaims::default())
            .await
            .unwrap();
        assert!(e.groups.is_empty());
        assert!(e.group_entities.is_empty());
        assert!(e.attributes.is_empty());
    }

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

    /// The email-encoded principal UID parses unchanged (Cedar quoted EIDs allow
    /// `@`/`.`), so the client can carry the UC identity directly in the UID.
    #[test]
    fn parses_email_encoded_principal_uid() {
        let mut meta = MetadataMap::new();
        meta.insert(
            headers::PRINCIPAL,
            "User::\"alice@example.com\"".parse().unwrap(),
        );
        let id = principal_from_metadata(&meta).unwrap();
        assert_eq!(id.uid.to_string(), "User::\"alice@example.com\"");
    }

    #[test]
    fn uc_token_from_metadata_reads_dedicated_key() {
        let mut meta = MetadataMap::new();
        meta.insert(headers::UC_TOKEN, "jwt-abc".parse().unwrap());
        assert_eq!(uc_token_from_metadata(&meta).as_deref(), Some("jwt-abc"));
        // Falls back to None — not the `authorization` session-id channel.
        meta.insert("authorization", "Bearer session-id".parse().unwrap());
        let mut only_auth = MetadataMap::new();
        only_auth.insert("authorization", "Bearer session-id".parse().unwrap());
        assert!(uc_token_from_metadata(&only_auth).is_none());
    }

    #[test]
    fn uc_token_from_metadata_absent_is_none() {
        assert!(uc_token_from_metadata(&MetadataMap::new()).is_none());
    }

    #[test]
    fn uc_token_from_http_headers_strips_bearer() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer jwt-xyz".parse().unwrap(),
        );
        assert_eq!(
            uc_token_from_http_headers(&headers).as_deref(),
            Some("jwt-xyz")
        );
        assert!(uc_token_from_http_headers(&axum::http::HeaderMap::new()).is_none());
    }
}
