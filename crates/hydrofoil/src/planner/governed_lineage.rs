//! The single session `QueryPlanner` that composes policy enforcement and
//! OpenLineage over one shared physical planner.
//!
//! DataFusion exposes exactly one `QueryPlanner` slot, and two of our concerns
//! need it: the Cedar policy gate (async, whole-plan, can *reject*) and
//! OpenLineage's START/marker step (async, whole-plan, emits before lowering).
//! Neither fits an `ExtensionPlanner` (sync, per-node, cannot reject). So this
//! one hydrofoil-owned planner sequences both crates' *reusable steps* —
//! `datafusion_policy_cedar::authorize_and_govern` and `datafusion_openlineage::begin_lineage`
//! — then delegates lowering to one `DefaultPhysicalPlanner` carrying every
//! `ExtensionPlanner` (`UnityCatalogPlanner` for UC DDL nodes,
//! `LineageExtensionPlanner` for the lineage terminal node). That is the
//! idiomatic composition seam; neither crate owns the slot.
//!
//! Order matters: **policy runs first and can reject**, so a denied query never
//! reaches the lineage step — it emits no OpenLineage run. Because governance
//! (row filters + column masks) is injected before the lineage step, the emitted
//! lineage describes the *governed* plan (what actually executes).

use std::sync::Arc;

use datafusion::catalog::Session;
use datafusion::common::Result;
use datafusion::execution::context::QueryPlanner;
use datafusion::logical_expr::LogicalPlan;
use datafusion::physical_plan::ExecutionPlan;
use datafusion::physical_planner::{
    DefaultPhysicalPlanner, ExtensionPlanner, PhysicalPlanner as _,
};

use datafusion_openlineage::context::LineageContextProvider;
use datafusion_openlineage::{
    LineageExtensionPlanner, OpenLineageClient, OpenLineageConfig, begin_lineage,
};
use datafusion_policy_cedar::{
    EvalContextProvider, PolicyEngine as Policy, PrincipalProvider, authorize_and_govern,
};

/// The OpenLineage pieces the planner needs, present only when lineage is wired.
struct LineageParts {
    client: OpenLineageClient,
    context: Arc<dyn LineageContextProvider>,
    config: OpenLineageConfig,
}

/// The single session `QueryPlanner`: policy gate → lineage START → shared
/// physical lowering. See the module docs.
pub struct GovernedLineagePlanner {
    policy: Arc<dyn Policy>,
    principal: Arc<dyn PrincipalProvider>,
    eval: Arc<dyn EvalContextProvider>,
    lineage: Option<LineageParts>,
    /// The one physical planner, carrying every extension planner. Both
    /// `UnityCatalogPlanner` and `LineageExtensionPlanner` return `Ok(None)` for
    /// nodes that aren't theirs, so registration order does not matter.
    physical: Arc<DefaultPhysicalPlanner>,
}

impl std::fmt::Debug for GovernedLineagePlanner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GovernedLineagePlanner")
            .field("policy", &self.policy)
            .field("lineage", &self.lineage.is_some())
            .finish_non_exhaustive()
    }
}

impl GovernedLineagePlanner {
    /// Build the composing planner.
    ///
    /// `extension_planners` are the host's own node lowerers (e.g.
    /// `UnityCatalogPlanner`); `LineageExtensionPlanner` is always registered
    /// alongside them so the lineage terminal node lowers. Pass `lineage` pieces
    /// to enable OpenLineage; `None` disables it (policy still runs).
    pub fn new(
        policy: Arc<dyn Policy>,
        principal: Arc<dyn PrincipalProvider>,
        eval: Arc<dyn EvalContextProvider>,
        extension_planners: Vec<Arc<dyn ExtensionPlanner + Send + Sync>>,
        lineage: Option<(
            OpenLineageClient,
            Arc<dyn LineageContextProvider>,
            OpenLineageConfig,
        )>,
    ) -> Self {
        let mut planners: Vec<Arc<dyn ExtensionPlanner + Send + Sync>> =
            vec![Arc::new(LineageExtensionPlanner)];
        planners.extend(extension_planners);
        Self {
            policy,
            principal,
            eval,
            lineage: lineage.map(|(client, context, config)| LineageParts {
                client,
                context,
                config,
            }),
            physical: Arc::new(DefaultPhysicalPlanner::with_extension_planners(planners)),
        }
    }
}

#[async_trait::async_trait]
impl QueryPlanner for GovernedLineagePlanner {
    async fn create_physical_plan(
        &self,
        logical_plan: &LogicalPlan,
        session_state: &dyn Session,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        // (a) Policy first — govern (row filters + column masks) then gate. A Deny
        // returns Err here, *before* any lineage step, so a denied query emits no
        // OpenLineage run. The principal + per-query facts are read back from the
        // session's `SessionConfig` extensions via the crate's default providers.
        let eval = self.eval.eval_context(session_state).await;
        let Some(principal) = self.principal.principal(session_state).await else {
            return datafusion::common::exec_err!(
                "no principal bound to session; cannot authorize query"
            );
        };
        let governed = authorize_and_govern(
            session_state,
            logical_plan,
            self.policy.as_ref(),
            &principal,
            &eval,
        )
        .await?;
        // Optimize the governed plan so lineage extraction and physical planning
        // both see projections/filters pushed to the scan (matches the pre-refactor
        // gate, which optimized before planning).
        let optimized = session_state.optimize(&governed)?;

        // (b) Lineage begin (only if wired): emit START and wrap the governed,
        // optimized plan in a `LineageMarker`. Nothing to emit for a no-dataset
        // query — `begin_lineage` returns `None` and we plan the bare plan.
        let Some(lineage) = &self.lineage else {
            return self
                .physical
                .create_physical_plan(&optimized, session_state)
                .await;
        };
        let Some(handle) = begin_lineage(
            &lineage.client,
            lineage.context.as_ref(),
            &lineage.config,
            &optimized,
            session_state,
        )
        .start()
        .await
        else {
            return self
                .physical
                .create_physical_plan(&optimized, session_state)
                .await;
        };

        // (c) Delegate to the one physical planner. The handle wraps the plan in
        // its `LineageMarker` and, on a planning error after START, emits FAIL
        // under the run id (the terminal `OpenLineageExec` only exists once
        // planning succeeds).
        handle
            .create_physical_plan(self.physical.as_ref(), session_state)
            .await
    }
}
