//! Walk a [`LogicalPlan`] and turn the tables/actions it references into Cedar
//! authorization [`Request`]s.
//!
//! This mirrors `datafusion-open-lineage`'s `extract()`: a [`TreeNodeVisitor`]
//! over the optimized plan that classifies each relevant node into a
//! [`PlanAction`], which is then lowered to a Cedar request whose resource and
//! context carry the table identity and the columns being accessed.

use std::str::FromStr as _;
use std::sync::LazyLock;

use cedar_policy::{Context, EntityTypeName, Request, RestrictedExpression};
use datafusion::common::plan_datafusion_err;
use datafusion::common::tree_node::{TreeNode as _, TreeNodeRecursion, TreeNodeVisitor};
use datafusion::error::Result;
use datafusion::logical_expr::{DdlStatement, LogicalPlan, WriteOp};
use datafusion::sql::TableReference;

use cedar_oci::{EntityId, EntityUid};

use crate::principal::PrincipalIdentity;

/// An access-relevant operation discovered in a logical plan.
pub(crate) enum PlanAction {
    /// Read `table`, accessing the listed columns.
    ReadTable(TableReference, Vec<String>),
    /// Write (insert/update/delete/truncate) into `table`.
    WriteTable(TableReference),
    /// Create `table`.
    CreateTable(TableReference),
    /// A state-changing node we do not model. We cannot prove it is safe, so it
    /// must be denied (fail-closed). Carries a short description for diagnostics.
    DenyUnsupported(String),
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
                DdlStatement::CreateMemoryTable(cmd) => {
                    self.actions.push(PlanAction::CreateTable(cmd.name.clone()));
                }
                // Any other (state-changing) DDL we do not model — schema/catalog
                // create/drop, table drop, views, indexes, functions — is denied
                // rather than silently allowed through.
                other => {
                    self.actions
                        .push(PlanAction::DenyUnsupported(format!("DDL {}", other.name())));
                }
            },
            LogicalPlan::Dml(dml) => match dml.op {
                // INSERT/UPDATE/DELETE/TRUNCATE all mutate the target table.
                WriteOp::Insert(_)
                | WriteOp::Update
                | WriteOp::Delete
                | WriteOp::Truncate => {
                    self.actions
                        .push(PlanAction::WriteTable(dml.table_name.clone()));
                }
                // CTAS produces a new table; treat as a create.
                WriteOp::Ctas => {
                    self.actions
                        .push(PlanAction::CreateTable(dml.table_name.clone()));
                }
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
/// Action used for state-changing nodes we do not model. No policy is expected
/// to permit it, so Cedar's default-deny rejects the query (fail-closed).
static DENY_UNSUPPORTED_ACTION: LazyLock<EntityUid> = LazyLock::new(|| {
    EntityUid::from_type_name_and_id(
        "Action".parse().unwrap(),
        EntityId::new("deny_unsupported"),
    )
});

/// Build the `Table` resource uid for a table reference.
fn table_resource(table_ref: &TableReference) -> EntityUid {
    let table_type_name = EntityTypeName::from_str("Table").unwrap();
    EntityUid::from_type_name_and_id(table_type_name, EntityId::new(table_ref.to_string()))
}

/// Build the request context carrying the table identity and accessed columns,
/// so policies can condition on `context.catalog/schema/table/columns`.
///
/// Shared with Layer-2 governance (`cedar::table_policy`), which builds the same
/// context (with an empty column set) for its partial-eval request so that
/// row-filter policies can also condition on the table identity.
pub(crate) fn table_context(table_ref: &TableReference, columns: &[String]) -> Result<Context> {
    let mut pairs: Vec<(String, RestrictedExpression)> = Vec::new();
    if let Some(catalog) = table_ref.catalog() {
        pairs.push((
            "catalog".to_string(),
            RestrictedExpression::new_string(catalog.to_string()),
        ));
    }
    if let Some(schema) = table_ref.schema() {
        pairs.push((
            "schema".to_string(),
            RestrictedExpression::new_string(schema.to_string()),
        ));
    }
    pairs.push((
        "table".to_string(),
        RestrictedExpression::new_string(table_ref.table().to_string()),
    ));
    if !columns.is_empty() {
        pairs.push((
            "columns".to_string(),
            RestrictedExpression::new_set(
                columns
                    .iter()
                    .map(|c| RestrictedExpression::new_string(c.clone())),
            ),
        ));
    }
    Context::from_pairs(pairs)
        .map_err(|e| plan_datafusion_err!("Failed to build request context: {}", e))
}

/// Lower a logical plan to the set of Cedar requests that must all be permitted
/// for the plan to run. Each request carries the principal, the action, the
/// `Table` resource, and a context describing the table and accessed columns.
pub(crate) fn authorize_plan(
    plan: &LogicalPlan,
    principal: &PrincipalIdentity,
) -> Result<Vec<Request>> {
    let mut visitor = AuthorizationVisitor { actions: vec![] };
    plan.visit(&mut visitor)?;

    let mut requests = vec![];
    for action in visitor.actions {
        let (action_uid, table_ref, columns) = match action {
            PlanAction::ReadTable(table_ref, columns) => {
                (READ_TABLE_ACTION.clone(), table_ref, columns)
            }
            PlanAction::WriteTable(table_ref) => (WRITE_TABLE_ACTION.clone(), table_ref, vec![]),
            PlanAction::CreateTable(table_ref) => {
                (CREATE_EXTERNAL_TABLE_ACTION.clone(), table_ref, vec![])
            }
            // Lower an unsupported node to a request no policy permits; Cedar's
            // default-deny then rejects the query (fail-closed).
            PlanAction::DenyUnsupported(what) => {
                tracing::warn!(node = %what, "unsupported state-changing plan node; denying (fail-closed)");
                (DENY_UNSUPPORTED_ACTION.clone(), TableReference::bare(what), vec![])
            }
        };
        let resource = table_resource(&table_ref);
        let context = table_context(&table_ref, &columns)?;
        requests.push(
            Request::new(
                principal.uid.clone(),
                action_uid,
                resource,
                context,
                None,
            )
            .map_err(|e| plan_datafusion_err!("Failed to create request: {}", e))?,
        );
    }

    Ok(requests)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::logical_expr::logical_plan::builder::table_scan;

    use super::*;

    fn principal() -> PrincipalIdentity {
        PrincipalIdentity::new(EntityUid::from_str("User::\"alice\"").unwrap())
    }

    fn schema() -> Schema {
        Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
        ])
    }

    #[test]
    fn read_scan_yields_one_request_with_columns() {
        let plan = table_scan(Some("t"), &schema(), None).unwrap().build().unwrap();
        let requests = authorize_plan(&plan, &principal()).unwrap();
        assert_eq!(requests.len(), 1);
        // The action is read_table; context carries the table identity.
        assert_eq!(requests[0].action().unwrap().to_string(), "Action::\"read_table\"");
    }

    #[test]
    fn projected_scan_limits_columns() {
        // Projecting a single column should still authorize the scan (and the
        // visitor records only the projected column set).
        let plan = table_scan(Some("t"), &schema(), Some(vec![0]))
            .unwrap()
            .build()
            .unwrap();
        let requests = authorize_plan(&plan, &principal()).unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[test]
    fn empty_relation_yields_no_requests() {
        use datafusion::logical_expr::LogicalPlanBuilder;
        let plan = LogicalPlanBuilder::empty(false).build().unwrap();
        let requests = authorize_plan(&plan, &principal()).unwrap();
        assert!(requests.is_empty());
    }

    fn action_of(req: &Request) -> String {
        req.action().unwrap().to_string()
    }

    // Build real plans through a SessionContext with registered tables, so the
    // DML/DDL node shapes match what the engine actually produces.
    async fn ctx_with_tables() -> datafusion::prelude::SessionContext {
        use std::sync::Arc;
        use datafusion::datasource::MemTable;
        use datafusion::prelude::SessionContext;
        let ctx = SessionContext::new();
        let s = Arc::new(schema());
        for name in ["a", "b", "dst"] {
            let table = MemTable::try_new(s.clone(), vec![vec![]]).unwrap();
            ctx.register_table(name, Arc::new(table)).unwrap();
        }
        ctx
    }

    #[tokio::test]
    async fn insert_yields_write_table_request() {
        let ctx = ctx_with_tables().await;
        let plan = ctx
            .sql("INSERT INTO dst SELECT * FROM a")
            .await
            .unwrap()
            .into_unoptimized_plan();
        let requests = authorize_plan(&plan, &principal()).unwrap();
        assert!(requests.iter().any(|r| action_of(r) == "Action::\"write_table\""));
        assert!(requests.iter().any(|r| action_of(r) == "Action::\"read_table\""));
    }

    #[tokio::test]
    async fn join_yields_one_read_request_per_table() {
        let ctx = ctx_with_tables().await;
        let plan = ctx
            .sql("SELECT a.id FROM a JOIN b ON a.id = b.id")
            .await
            .unwrap()
            .into_unoptimized_plan();
        let requests = authorize_plan(&plan, &principal()).unwrap();
        let reads = requests
            .iter()
            .filter(|r| action_of(r) == "Action::\"read_table\"")
            .count();
        assert_eq!(reads, 2, "each joined table is authorized independently");
    }

    #[test]
    fn unmodeled_ddl_yields_deny_unsupported() {
        use datafusion::logical_expr::{
            DdlStatement, DropTable, LogicalPlan,
        };
        use std::sync::Arc;
        // DROP TABLE is state-changing and not modeled -> deny sentinel action.
        let inner = table_scan(Some("t"), &schema(), None).unwrap().build().unwrap();
        let plan = LogicalPlan::Ddl(DdlStatement::DropTable(DropTable {
            name: TableReference::bare("t"),
            if_exists: false,
            schema: Arc::new(inner.schema().as_ref().clone()),
        }));
        let requests = authorize_plan(&plan, &principal()).unwrap();
        assert!(
            requests
                .iter()
                .any(|r| action_of(r) == "Action::\"deny_unsupported\""),
            "unmodeled DDL must produce a deny sentinel"
        );
    }
}
