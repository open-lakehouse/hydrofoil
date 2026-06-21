//! `TagPoliciesService` handlers.

use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};

use crate::proto::tags::v1::{
    CreateTagPolicyRequest, DeleteTagPolicyRequest, GetTagPolicyRequest, ListTagPoliciesRequest,
    ListTagPoliciesResponse, TagPolicy, UpdateTagPolicyRequest,
};
use crate::service::AppState;
use crate::services::tags::v1::TagPoliciesService;
use crate::store::Page;

impl TagPoliciesService for AppState {
    async fn list_tag_policies(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListTagPoliciesRequest>,
    ) -> ServiceResult<ListTagPoliciesResponse> {
        let page = Page {
            max_results: request.max_results.map(|n| n.max(0) as usize),
            page_token: request.page_token.map(str::to_owned),
        };
        let (tag_policies, next_page_token) = self.tags.list_policies(page).await?;
        Response::ok(ListTagPoliciesResponse {
            tag_policies,
            next_page_token,
            ..Default::default()
        })
    }

    async fn create_tag_policy(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, CreateTagPolicyRequest>,
    ) -> ServiceResult<TagPolicy> {
        let policy = request
            .to_owned_message()
            .tag_policy
            .into_option()
            .ok_or_else(|| ConnectError::invalid_argument("tag_policy is required"))?;
        Response::ok(self.tags.create_policy(policy).await?)
    }

    async fn get_tag_policy(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, GetTagPolicyRequest>,
    ) -> ServiceResult<TagPolicy> {
        let policy = self.tags.get_policy(request.tag_key).await?;
        Response::ok(policy)
    }

    async fn update_tag_policy(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, UpdateTagPolicyRequest>,
    ) -> ServiceResult<TagPolicy> {
        let req = request.to_owned_message();
        let policy = req
            .tag_policy
            .into_option()
            .ok_or_else(|| ConnectError::invalid_argument("tag_policy is required"))?;
        let mask = parse_update_mask(req.update_mask.as_deref());
        let updated = self.tags.update_policy(&req.tag_key, policy, &mask).await?;
        Response::ok(updated)
    }

    async fn delete_tag_policy(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, DeleteTagPolicyRequest>,
    ) -> ServiceResult<buffa_types::google::protobuf::Empty> {
        self.tags.delete_policy(request.tag_key).await?;
        Response::ok(buffa_types::google::protobuf::Empty::default())
    }
}

/// Split a comma-separated `update_mask` into trimmed, non-empty field names.
pub(crate) fn parse_update_mask(mask: Option<&str>) -> Vec<String> {
    mask.into_iter()
        .flat_map(|m| m.split(','))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}
