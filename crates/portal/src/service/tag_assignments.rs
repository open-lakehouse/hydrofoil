//! `EntityTagAssignmentsService` handlers.

use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};

use crate::proto::tags::v1::{
    CreateEntityTagAssignmentRequest, DeleteEntityTagAssignmentRequest, EntityTagAssignment,
    GetEntityTagAssignmentRequest, ListEntityTagAssignmentsRequest,
    ListEntityTagAssignmentsResponse, UpdateEntityTagAssignmentRequest,
};
use crate::service::AppState;
use crate::services::tags::v1::EntityTagAssignmentsService;
use crate::store::{Page, TagStore};

impl EntityTagAssignmentsService for AppState {
    async fn list_entity_tag_assignments(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListEntityTagAssignmentsRequest>,
    ) -> ServiceResult<ListEntityTagAssignmentsResponse> {
        let page = Page {
            max_results: request.max_results.map(|n| n.max(0) as usize),
            page_token: request.page_token.map(str::to_owned),
        };
        let (tag_assignments, next_page_token) = self
            .store
            .list_assignments(request.entity_type, request.entity_name, page)
            .await?;
        Response::ok(ListEntityTagAssignmentsResponse {
            tag_assignments,
            next_page_token,
            ..Default::default()
        })
    }

    async fn create_entity_tag_assignment(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, CreateEntityTagAssignmentRequest>,
    ) -> ServiceResult<EntityTagAssignment> {
        let assignment = request
            .to_owned_message()
            .tag_assignment
            .into_option()
            .ok_or_else(|| ConnectError::invalid_argument("tag_assignment is required"))?;
        Response::ok(self.store.create_assignment(assignment).await?)
    }

    async fn get_entity_tag_assignment(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, GetEntityTagAssignmentRequest>,
    ) -> ServiceResult<EntityTagAssignment> {
        let assignment = self
            .store
            .get_assignment(request.entity_type, request.entity_name, request.tag_key)
            .await?;
        Response::ok(assignment)
    }

    async fn update_entity_tag_assignment(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, UpdateEntityTagAssignmentRequest>,
    ) -> ServiceResult<EntityTagAssignment> {
        let req = request.to_owned_message();
        let assignment = req
            .tag_assignment
            .into_option()
            .ok_or_else(|| ConnectError::invalid_argument("tag_assignment is required"))?;
        let updated = self
            .store
            .update_assignment(&req.entity_type, &req.entity_name, &req.tag_key, assignment)
            .await?;
        Response::ok(updated)
    }

    async fn delete_entity_tag_assignment(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, DeleteEntityTagAssignmentRequest>,
    ) -> ServiceResult<buffa_types::google::protobuf::Empty> {
        self.store
            .delete_assignment(request.entity_type, request.entity_name, request.tag_key)
            .await?;
        Response::ok(buffa_types::google::protobuf::Empty::default())
    }
}
