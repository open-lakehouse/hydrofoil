use std::str::FromStr as _;
use std::sync::LazyLock;

use cedar_local_agent::public::simple::{Authorizer, AuthorizerConfigBuilder};
pub use cedar_local_agent::public::{SimpleEntityProvider, SimplePolicySetProvider};
use cedar_policy::{Context, Entities, EntityTypeName, Request};
use datafusion::common::plan_datafusion_err;
use datafusion::common::tree_node::{TreeNode as _, TreeNodeRecursion};
use datafusion::logical_expr::{DdlStatement, DmlStatement, LogicalPlan, WriteOp};
use datafusion::sql::TableReference;
use datafusion::{common::tree_node::TreeNodeVisitor, error::Result};

use hydrofoil_policy::{Decision, EntityId, EntityUid};

use super::Policy;

#[derive(Debug)]
pub struct CedarPolicy<P, E>
where
    P: SimplePolicySetProvider + 'static,
    E: SimpleEntityProvider + 'static,
{
    authorizer: Authorizer<P, E>,
}

impl<P, E> CedarPolicy<P, E>
where
    P: SimplePolicySetProvider + 'static,
    E: SimpleEntityProvider + 'static,
{
    fn new(authorizer: Authorizer<P, E>) -> Self {
        Self { authorizer }
    }
}

#[async_trait::async_trait]
impl<P, E> Policy for CedarPolicy<P, E>
where
    P: SimplePolicySetProvider + 'static,
    E: SimpleEntityProvider + 'static,
{
    async fn is_allowed(
        &self,
        logical_plan: &LogicalPlan,
        principal: &EntityUid,
    ) -> Result<Decision> {
        let requests = authorize_plan(logical_plan, principal)?;
        for request in requests {
            let decision = self
                .authorizer
                .is_authorized(&request, &Entities::empty())
                .await
                .map_err(|e| plan_datafusion_err!("Failed to authorize plan: {}", e))?
                .decision();
            if decision == Decision::Deny {
                return Ok(Decision::Deny);
            }
        }
        Ok(Decision::Allow)
    }
}

enum PlanAction {
    ReadTable(TableReference, Vec<String>),
    WriteTable,
    CreateTable(TableReference),
}

struct AuthorizationVisitor {
    actions: Vec<PlanAction>,
}

impl TreeNodeVisitor<'_> for AuthorizationVisitor {
    type Node = LogicalPlan;

    fn f_down(&mut self, node: &Self::Node) -> Result<TreeNodeRecursion> {
        match node {
            LogicalPlan::TableScan(scan) => {
                let fields = scan
                    .projected_schema
                    .fields()
                    .iter()
                    .map(|f| f.name().to_string())
                    .collect();
                self.actions
                    .push(PlanAction::ReadTable(scan.table_name.clone(), fields));
            }
            LogicalPlan::Ddl(ddl) => match ddl {
                DdlStatement::CreateExternalTable(cmd) => {
                    self.actions.push(PlanAction::CreateTable(cmd.name.clone()));
                }
                DdlStatement::CreateCatalogSchema(_cmd) => {
                    // Handle create schema
                }
                _ => {}
            },
            LogicalPlan::Dml(dml) => match dml.op {
                WriteOp::Insert(_) | WriteOp::Delete => {
                    self.actions.push(PlanAction::WriteTable);
                }
                _ => {}
            },
            _ => {}
        }
        Ok(TreeNodeRecursion::Continue)
    }
}

static CREATE_EXTERNAL_TABLE_ACTION: LazyLock<EntityUid> = LazyLock::new(|| {
    EntityUid::from_type_name_and_id(
        "Action".parse().unwrap(),
        EntityId::new("create_external_table"),
    )
});
static READ_TABLE_ACTION: LazyLock<EntityUid> = LazyLock::new(|| {
    EntityUid::from_type_name_and_id("Action".parse().unwrap(), EntityId::new("read_table"))
});
static WRITE_TABLE_ACTION: LazyLock<EntityUid> = LazyLock::new(|| {
    EntityUid::from_type_name_and_id("Action".parse().unwrap(), EntityId::new("write_table"))
});

fn authorize_plan(plan: &LogicalPlan, principal: &EntityUid) -> Result<Vec<Request>> {
    let mut visitor = AuthorizationVisitor { actions: vec![] };
    plan.visit(&mut visitor)?;

    let mut requests = vec![];
    for action in visitor.actions {
        match action {
            PlanAction::ReadTable(table_ref, fields) => {
                let table_type_name = EntityTypeName::from_str("Table").unwrap();
                let resource = EntityUid::from_type_name_and_id(
                    table_type_name,
                    EntityId::new(table_ref.to_string()),
                );
                requests.push(
                    Request::new(
                        principal.clone(),
                        READ_TABLE_ACTION.clone(),
                        resource,
                        Context::empty(),
                        None,
                    )
                    .map_err(|e| plan_datafusion_err!("Failed to create request: {}", e))?,
                )
            }
            PlanAction::WriteTable => {
                tracing::info!("Authorize write table");
                todo!()
            }
            PlanAction::CreateTable(table_ref) => {
                tracing::info!("Authorize create table: {:?}", table_ref);
                todo!()
            }
        }
    }

    Ok(requests)
}
