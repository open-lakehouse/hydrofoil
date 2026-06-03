//! The authenticated principal a query runs as.

use std::collections::HashMap;

use cedar_policy::{Entity, RestrictedExpression};

use cedar_oci::EntityUid;

/// The principal on whose behalf a query executes, plus the attributes policies
/// may condition on (e.g. `role`, `region`, `name`).
///
/// Carrying attributes alongside the `EntityUid` is why the [`Policy`](crate::Policy)
/// trait threads a `&PrincipalIdentity` rather than a bare `EntityUid`: Cedar
/// policies written against `principal.role` need the principal to exist as an
/// entity with those attributes at authorization time. The host (hydrofoil)
/// builds this from authenticated request metadata.
#[derive(Debug, Clone)]
pub struct PrincipalIdentity {
    /// The principal's Cedar entity uid, e.g. `User::"alice"`.
    pub uid: EntityUid,
    /// Principal attributes, as Cedar restricted expressions.
    pub attributes: HashMap<String, RestrictedExpression>,
}

impl PrincipalIdentity {
    /// A principal with a uid and no attributes.
    pub fn new(uid: EntityUid) -> Self {
        Self {
            uid,
            attributes: HashMap::new(),
        }
    }

    /// Set a string-valued attribute (e.g. `role`, `region`).
    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes
            .insert(key.into(), RestrictedExpression::new_string(value.into()));
        self
    }

    /// Build the Cedar [`Entity`] for this principal so an authorizer can
    /// resolve `principal.<attr>` references. Returns the bare uid entity (no
    /// attributes) if attribute evaluation fails, so authorization stays
    /// fail-closed rather than erroring open.
    pub fn to_entity(&self) -> Entity {
        Entity::new(self.uid.clone(), self.attributes.clone(), Default::default())
            .unwrap_or_else(|_| Entity::new_no_attrs(self.uid.clone(), Default::default()))
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use super::*;

    fn uid(s: &str) -> EntityUid {
        EntityUid::from_str(s).unwrap()
    }

    #[test]
    fn carries_attributes_and_builds_entity() {
        let id = PrincipalIdentity::new(uid("User::\"alice\""))
            .with_attribute("role", "analyst")
            .with_attribute("region", "eu");
        assert_eq!(id.attributes.len(), 2);
        // Entity construction succeeds and preserves the uid.
        let entity = id.to_entity();
        assert_eq!(entity.uid(), id.uid);
    }

    #[test]
    fn entity_with_no_attributes() {
        let id = PrincipalIdentity::new(uid("User::\"bob\""));
        assert!(id.attributes.is_empty());
        assert_eq!(id.to_entity().uid().to_string(), "User::\"bob\"");
    }
}
