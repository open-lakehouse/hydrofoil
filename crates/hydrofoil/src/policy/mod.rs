use std::sync::LazyLock;

use datafusion::common::tree_node::{TreeNode as _, TreeNodeRecursion};
use datafusion::logical_expr::{DdlStatement, DmlStatement, LogicalPlan, WriteOp};
use datafusion::sql::TableReference;
use datafusion::{common::tree_node::TreeNodeVisitor, error::Result};

use hydrofoil_policy::{Decision, EntityId, EntityUid};

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

#[async_trait::async_trait]
pub trait Policy: std::fmt::Debug + Send + Sync {
    async fn is_allowed(
        &self,
        logical_plan: &LogicalPlan,
        principal: &EntityUid,
    ) -> Result<Decision>;
}

#[derive(Debug, Clone)]
pub struct NoOpPolicy;

#[async_trait::async_trait]
impl Policy for NoOpPolicy {
    async fn is_allowed(
        &self,
        _logical_plan: &LogicalPlan,
        _principal: &EntityUid,
    ) -> Result<Decision> {
        Ok(Decision::Allow)
    }
}

enum PlanAction {
    ReadTable(Vec<String>),
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
                self.actions.push(PlanAction::ReadTable(fields));
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

pub(crate) fn authorize_plan(plan: &LogicalPlan) -> Result<()> {
    let mut visitor = AuthorizationVisitor { actions: vec![] };
    plan.visit(&mut visitor)?;

    for action in visitor.actions {
        match action {
            PlanAction::ReadTable(fields) => {
                tracing::info!("Authorize read table with fields: {:?}", fields);
            }
            PlanAction::WriteTable => {
                tracing::info!("Authorize write table");
            }
            PlanAction::CreateTable(table_ref) => {
                tracing::info!("Authorize create table: {:?}", table_ref);
            }
        }
    }

    Ok(())
}
