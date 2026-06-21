//! `FilesService` handlers, including streaming upload/download.

use connectrpc::{
    ConnectError, RequestContext, Response, ServiceRequest, ServiceResult, ServiceStream,
    StreamMessage,
};
use futures::StreamExt;

use crate::proto::files::v1::{
    CreateDirectoryRequest, DeleteDirectoryRequest, DeleteFileRequest, DirectoryMetadata,
    DownloadFileRequest, DownloadFileResponse, FileMetadata, GetDirectoryMetadataRequest,
    GetFileMetadataRequest, ListDirectoryContentsRequest, ListDirectoryContentsResponse,
    UploadFileRequest, UploadFileResponse,
};
use crate::service::AppState;
use crate::services::files::v1::FilesService;
use crate::store::Page;

/// Size of each chunk streamed back from `DownloadFile`.
const DOWNLOAD_CHUNK_SIZE: usize = 64 * 1024;

impl FilesService for AppState {
    async fn upload_file(
        &self,
        _ctx: RequestContext,
        mut requests: ServiceStream<StreamMessage<UploadFileRequest>>,
    ) -> ServiceResult<UploadFileResponse> {
        let mut path: Option<String> = None;
        let mut content_type: Option<String> = None;
        let mut contents: Vec<u8> = Vec::new();

        // `None` = clean end of stream; `Some(Err)` = abnormal termination.
        while let Some(item) = requests.next().await {
            let msg = item?.to_owned_message();
            if path.is_none() && !msg.path.is_empty() {
                path = Some(msg.path);
                content_type = msg.content_type;
            }
            contents.extend_from_slice(&msg.chunk);
        }

        let path =
            path.ok_or_else(|| ConnectError::invalid_argument("first message must set `path`"))?;
        let meta = self.files.put_file(&path, content_type, contents).await?;
        Response::ok(UploadFileResponse {
            path: meta.path,
            file_size: meta.file_size,
            etag: meta.etag,
            ..Default::default()
        })
    }

    async fn download_file(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, DownloadFileRequest>,
    ) -> ServiceResult<ServiceStream<DownloadFileResponse>> {
        let bytes = self
            .files
            .read_file(request.path, request.offset, request.length)
            .await?;

        // Pre-chunk into owned response messages; the returned stream items must
        // be `'static` and cannot borrow from `request` or `&self`.
        let chunks: Vec<Result<DownloadFileResponse, ConnectError>> = bytes
            .chunks(DOWNLOAD_CHUNK_SIZE)
            .map(|chunk| {
                Ok(DownloadFileResponse {
                    chunk: chunk.to_vec(),
                    ..Default::default()
                })
            })
            .collect();

        Ok(Response::stream(futures::stream::iter(chunks)))
    }

    async fn delete_file(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, DeleteFileRequest>,
    ) -> ServiceResult<buffa_types::google::protobuf::Empty> {
        self.files.delete_file(request.path).await?;
        Response::ok(buffa_types::google::protobuf::Empty::default())
    }

    async fn get_file_metadata(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, GetFileMetadataRequest>,
    ) -> ServiceResult<FileMetadata> {
        Response::ok(self.files.stat_file(request.path).await?)
    }

    async fn create_directory(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, CreateDirectoryRequest>,
    ) -> ServiceResult<DirectoryMetadata> {
        Response::ok(self.files.create_directory(request.path).await?)
    }

    async fn delete_directory(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, DeleteDirectoryRequest>,
    ) -> ServiceResult<buffa_types::google::protobuf::Empty> {
        self.files.delete_directory(request.path).await?;
        Response::ok(buffa_types::google::protobuf::Empty::default())
    }

    async fn list_directory_contents(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListDirectoryContentsRequest>,
    ) -> ServiceResult<ListDirectoryContentsResponse> {
        let page = Page {
            max_results: request.max_results.map(|n| n.max(0) as usize),
            page_token: request.page_token.map(str::to_owned),
        };
        let (contents, next_page_token) = self.files.list_directory(request.path, page).await?;
        Response::ok(ListDirectoryContentsResponse {
            contents,
            next_page_token,
            ..Default::default()
        })
    }

    async fn get_directory_metadata(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, GetDirectoryMetadataRequest>,
    ) -> ServiceResult<DirectoryMetadata> {
        Response::ok(self.files.stat_directory(request.path).await?)
    }
}
