//! Fetch policies from OCI registries.
use std::{fmt::Debug, str::FromStr, sync::Arc};

use cedar_local_agent::public::{
    EntityProviderError, PolicySetProviderError, SimpleEntityProvider, SimplePolicySetProvider,
};
use cedar_policy::{Entities, PolicySet, Request, Schema};
use oci_client::{
    Client, Reference,
    client::{ClientConfig, ClientProtocol},
    secrets::RegistryAuth,
};
use tokio::sync::RwLock;
use tracing::instrument;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CedarOciMediaTypes {
    PolicySet,
    Schema,
    Entities,
}

impl AsRef<str> for CedarOciMediaTypes {
    fn as_ref(&self) -> &str {
        match self {
            CedarOciMediaTypes::PolicySet => "application/vnd.cedar.policyset.v1",
            CedarOciMediaTypes::Schema => "application/vnd.cedar.schema.v1",
            CedarOciMediaTypes::Entities => "application/vnd.cedar.entities.v1",
        }
    }
}

pub fn build_client_config() -> ClientConfig {
    let protocol = ClientProtocol::Http;

    ClientConfig {
        protocol,
        ..Default::default()
    }
}

pub struct OciEntityProvider {
    client: Client,
    reference: Reference,
    entities: RwLock<Arc<Entities>>,
    policy_set: RwLock<Arc<PolicySet>>,
}

impl Debug for OciEntityProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OciPolicyProvider")
            .field("reference", &self.reference)
            .finish()
    }
}

impl OciEntityProvider {
    #[instrument(skip(client), err(Debug))]
    pub async fn try_new(
        client: Client,
        reference: Reference,
    ) -> Result<Self, EntityProviderError> {
        let accepted_media_types = vec![
            CedarOciMediaTypes::PolicySet.as_ref(),
            CedarOciMediaTypes::Schema.as_ref(),
            CedarOciMediaTypes::Entities.as_ref(),
        ];

        let image = client
            .pull(&reference, &RegistryAuth::Anonymous, accepted_media_types)
            .await
            .map_err(|e| EntityProviderError::General(e.into()))?;

        let mut schema = None;
        let mut entities_raw = vec![];
        let mut policy_sets_raw = vec![];
        for layer in image.layers {
            if layer.media_type.as_str() == CedarOciMediaTypes::Schema.as_ref() {
                if schema.is_some() {
                    return Err(EntityProviderError::General(
                        "Multiple schema layers found".to_string().into(),
                    ));
                }
                let schema_raw = String::from_utf8(layer.data.to_vec())
                    .map_err(|e| EntityProviderError::General(e.into()))?;
                schema = Some(
                    Schema::from_str(&schema_raw)
                        .map_err(|e| EntityProviderError::General(e.into()))?,
                );
            } else if layer.media_type.as_str() == CedarOciMediaTypes::Entities.as_ref() {
                entities_raw.push(
                    String::from_utf8(layer.data.to_vec())
                        .map_err(|e| EntityProviderError::General(e.into()))?,
                );
            } else if layer.media_type.as_str() == CedarOciMediaTypes::PolicySet.as_ref() {
                policy_sets_raw.push(
                    String::from_utf8(layer.data.to_vec())
                        .map_err(|e| EntityProviderError::General(e.into()))?,
                );
            }
        }

        let mut entities = Entities::empty();
        for entity_str in entities_raw {
            entities = entities
                .add_entities_from_json_str(&entity_str, schema.as_ref())
                .map_err(|e| EntityProviderError::General(e.into()))?;
        }

        let mut policy_set = PolicySet::new();
        for policy_str in policy_sets_raw {
            let new_set = PolicySet::from_str(&policy_str)
                .map_err(|e| EntityProviderError::General(e.into()))?;
            policy_set
                .merge(&new_set, false)
                .map_err(|e| EntityProviderError::General(e.into()))?;
        }

        Ok(Self {
            client,
            reference,
            entities: RwLock::new(Arc::new(entities)),
            policy_set: RwLock::new(Arc::new(policy_set)),
        })
    }
}

#[async_trait::async_trait]
impl SimpleEntityProvider for OciEntityProvider {
    #[instrument(skip_all, err(Debug))]
    async fn get_entities(&self, _: &Request) -> Result<Arc<Entities>, EntityProviderError> {
        Ok(self.entities.read().await.clone())
    }
}

#[async_trait::async_trait]
impl SimplePolicySetProvider for OciEntityProvider {
    #[instrument(skip_all, err(Debug))]
    async fn get_policy_set(&self, _: &Request) -> Result<Arc<PolicySet>, PolicySetProviderError> {
        Ok(self.policy_set.read().await.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fetch_policy() {
        let client_config = build_client_config();
        let client = Client::new(client_config);
        let reference: Reference = "localhost:10100/hydrofoil/plan-policy:latest"
            .parse()
            .unwrap();

        let provider = OciEntityProvider::try_new(client, reference).await.unwrap();
    }
}
