use std::sync::Arc;

use datafusion::{catalog::Session, common::HashMap, error::Result};
use object_store::aws::AmazonS3Builder;
use object_store::client::SpawnedReqwestConnector;
use tokio::runtime::Handle;

pub(crate) fn update_session(session: &dyn Session) -> Result<()> {
    deltalake_aws::register_handlers(None);
    add_seaweedfs(session)?;
    Ok(())
}

fn add_seaweedfs(session: &dyn Session) -> Result<()> {
    let object_store = Arc::new(
        AmazonS3Builder::new()
            .with_http_connector(SpawnedReqwestConnector::new(Handle::current()))
            .with_access_key_id("seaweed-key-id")
            .with_secret_access_key("seaweed-access-key")
            .with_endpoint("http://localhost:8333/")
            .with_bucket_name("open-lakehouse")
            .with_allow_http(true)
            .build()?,
    );
    let url = url::Url::parse("s3://open-lakehouse/").unwrap();
    session
        .runtime_env()
        .register_object_store(&url, object_store);
    Ok(())
}
