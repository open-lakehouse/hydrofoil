//! End-to-end test: start the portal ConnectRPC server on an ephemeral port and
//! drive it with the generated client, covering a unary tag call and the
//! streaming file upload/download round-trip.

use std::sync::Arc;

use connectrpc::client::{ClientConfig, HttpClient};
use portal::proto::files::v1::{DownloadFileRequest, UploadFileRequest};
use portal::proto::tags::v1::{CreateTagPolicyRequest, TagPolicy};
use portal::service::AppState;
use portal::services::files::v1::FilesServiceClient;
use portal::services::tags::v1::TagPoliciesServiceClient;
use portal::store::MemoryStore;

/// Bind to port 0, spawn the server, and return the base URI it listens on.
async fn spawn_server() -> String {
    let store = Arc::new(MemoryStore::new());
    let state = AppState::new(Arc::clone(&store) as _, store as _);
    let connect = state.register_all(connectrpc::Router::new());
    let app = axum::Router::new().fallback_service(connect.into_axum_service());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn tags_client(base: &str) -> TagPoliciesServiceClient<HttpClient> {
    TagPoliciesServiceClient::new(
        HttpClient::plaintext(),
        ClientConfig::new(base.parse().unwrap()),
    )
}

fn files_client(base: &str) -> FilesServiceClient<HttpClient> {
    FilesServiceClient::new(
        HttpClient::plaintext(),
        ClientConfig::new(base.parse().unwrap()),
    )
}

#[tokio::test]
async fn unary_create_tag_policy() {
    let base = spawn_server().await;
    let client = tags_client(&base);

    let resp = client
        .create_tag_policy(CreateTagPolicyRequest {
            tag_policy: TagPolicy {
                tag_key: "cost_center".into(),
                description: Some("cost tracking".into()),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_owned();

    assert_eq!(resp.tag_key, "cost_center");
    assert!(resp.id.is_some(), "server should assign an id");
    assert!(resp.created_at.is_some());
}

#[tokio::test]
async fn streaming_upload_then_download() {
    let base = spawn_server().await;
    let client = files_client(&base);

    let body = b"hello connect streaming world".to_vec();

    // Client-streaming upload: first message carries the path, then the chunks.
    let requests = vec![
        UploadFileRequest {
            path: "/data/hello.txt".into(),
            content_type: Some("text/plain".into()),
            ..Default::default()
        },
        UploadFileRequest {
            chunk: body[..10].to_vec(),
            ..Default::default()
        },
        UploadFileRequest {
            chunk: body[10..].to_vec(),
            ..Default::default()
        },
    ];
    let upload = client.upload_file(requests).await.unwrap().into_owned();
    assert_eq!(upload.path, "/data/hello.txt");
    assert_eq!(upload.file_size, body.len() as i64);

    // Server-streaming download: reassemble the chunks.
    let mut stream = client
        .download_file(DownloadFileRequest {
            path: "/data/hello.txt".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut downloaded = Vec::new();
    while let Some(msg) = stream.message().await.unwrap() {
        downloaded.extend_from_slice(&msg.to_owned_message().chunk);
    }
    assert_eq!(downloaded, body);
}
