//! Walk a [`LogicalPlan`] and turn the tables/actions it references into Cedar
//! authorization [`Request`]s.
//!
//! This mirrors `datafusion-open-lineage`'s `extract()`: a [`TreeNodeVisitor`]
//! over the optimized plan that classifies each relevant node into a
//! [`PlanAction`], which is then lowered to a Cedar request.

use std::str::FromStr as _;
use std::sync::LazyLock;

use cedar_policy::{Context, EntityTypeName, Request};
use datafusion::common::plan_datafusion_err;
use datafusion::common::tree_node::{TreeNode as _, TreeNodeRecursion, TreeNodeVisitor};
use datafusion::error::Result;
use datafusion::logical_expr::{DdlStatement, LogicalPlan, WriteOp};
use datafusion::sql::TableReference;

use cedar_oci::{EntityId, EntityUid};

/// An access-relevant operation discovered in a logical plan.
pub(crate) enum PlanAction {
    ReadTable(TableReference, Vec<String>),
    WriteTable,
    CreateTable(TableReference),
}

pub(crate) struct AuthorizationVisitor {
    pub(crate) actions: Vec<PlanAction>,
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

// Write/DDL authorization (and thus these actions) is implemented in Phase 1.
#[allow(dead_code)]
static CREATE_EXTERNAL_TABLE_ACTION: LazyLock<EntityUid> = LazyLock::new(|| {
    EntityUid::from_type_name_and_id(
        "Action".parse().unwrap(),
        EntityId::new("create_external_table"),
    )
});
static READ_TABLE_ACTION: LazyLock<EntityUid> = LazyLock::new(|| {
    EntityUid::from_type_name_and_id("Action".parse().unwrap(), EntityId::new("read_table"))
});
#[allow(dead_code)]
static WRITE_TABLE_ACTION: LazyLock<EntityUid> = LazyLock::new(|| {
    EntityUid::from_type_name_and_id("Action".parse().unwrap(), EntityId::new("write_table"))
});

/// Lower a logical plan to the set of Cedar requests that must all be permitted
/// for the plan to run.
pub(crate) fn authorize_plan(plan: &LogicalPlan, principal: &EntityUid) -> Result<Vec<Request>> {
    let mut visitor = AuthorizationVisitor { actions: vec![] };
    plan.visit(&mut visitor)?;

    let mut requests = vec![];
    for action in visitor.actions {
        match action {
            PlanAction::ReadTable(table_ref, _fields) => {
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
