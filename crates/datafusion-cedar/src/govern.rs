//! Layer 2: fine-grained governance — inject row filters and column masks into
//! a logical plan before optimization.
//!
//! [`govern_plan`] is a two-phase pass (the rewriter API is sync, but policy
//! resolution is async): an async phase collects the distinct tables a plan
//! reads and awaits each table's [`TablePolicy`]; a sync [`GovernRewriter`] then
//! wraps every `TableScan` in a mask `Projection` and a row `Filter`. Running
//! *before* `optimize()` lets the optimizer push the filter into the scan and
//! prune masked-away columns.

use std::collections::HashMap;

use datafusion::common::Result;
use datafusion::common::tree_node::{
    Transformed, TreeNode as _, TreeNodeRecursion, TreeNodeRewriter, TreeNodeVisitor,
};
use datafusion::logical_expr::{Expr, LogicalPlan, LogicalPlanBuilder, col};
use datafusion::sql::TableReference;

use crate::policy::Policy;
use crate::principal::PrincipalIdentity;

/// The fine-grained enforcement that applies to one table for one principal.
#[derive(Debug, Clone, Default)]
pub struct TablePolicy {
    /// Conjunctive row-filter predicates over the table's columns. Principal
    /// attributes are already folded to literals; only `resource.<col>`
    /// references remain (as `col(<col>)`).
    pub row_filters: Vec<Expr>,
    /// Column name -> replacement expression for masked columns. The expression
    /// must not be a bare column (else the optimizer may absorb the projection);
    /// e.g. a literal or a hash.
    pub column_masks: HashMap<String, Expr>,
}

impl TablePolicy {
    /// Whether there is anything to enforce.
    pub fn is_empty(&self) -> bool {
        self.row_filters.is_empty() && self.column_masks.is_empty()
    }
}

/// Collect the distinct tables a plan reads (from `TableScan` nodes).
struct TableCollector {
    tables: Vec<(TableReference, datafusion::common::DFSchemaRef)>,
}

impl TreeNodeVisitor<'_> for TableCollector {
    type Node = LogicalPlan;

    fn f_down(&mut self, node: &Self::Node) -> Result<TreeNodeRecursion> {
        if let LogicalPlan::TableScan(scan) = node {
            self.tables
                .push((scan.table_name.clone(), scan.projected_schema.clone()));
        }
        Ok(TreeNodeRecursion::Continue)
    }
}

/// Inject row filters and column masks for each governed table.
///
/// Phase 1 (async): resolve each table's [`TablePolicy`]. Phase 2 (sync):
/// rewrite the plan. Returns the plan unchanged when nothing is governed.
pub async fn govern_plan(
    plan: &LogicalPlan,
    policy: &dyn Policy,
    principal: &PrincipalIdentity,
) -> Result<LogicalPlan> {
    // Phase 1: collect tables, then await per-table policy.
    let mut collector = TableCollector { tables: vec![] };
    plan.visit(&mut collector)?;

    let mut policies: HashMap<TableReference, TablePolicy> = HashMap::new();
    for (table, schema) in collector.tables {
        if policies.contains_key(&table) {
            continue;
        }
        let tp = policy.table_policy(&table, schema.as_ref(), principal).await?;
        if !tp.is_empty() {
            policies.insert(table, tp);
        }
    }

    if policies.is_empty() {
        return Ok(plan.clone());
    }

    // Phase 2: sync rewrite.
    let mut rewriter = GovernRewriter { policies };
    Ok(plan.clone().rewrite(&mut rewriter)?.data)
}

/// The sync rewriter that wraps each governed `TableScan` in a mask projection
/// and a row filter.
struct GovernRewriter {
    policies: HashMap<TableReference, TablePolicy>,
}

impl TreeNodeRewriter for GovernRewriter {
    type Node = LogicalPlan;

    fn f_up(&mut self, node: LogicalPlan) -> Result<Transformed<LogicalPlan>> {
        let LogicalPlan::TableScan(scan) = &node else {
            return Ok(Transformed::no(node));
        };
        let Some(tp) = self.policies.get(&scan.table_name) else {
            return Ok(Transformed::no(node));
        };

        let schema = scan.projected_schema.clone();
        let mut builder = LogicalPlanBuilder::from(node.clone());

        // Column masks: rebuild the projection, replacing masked columns with
        // their mask expression (aliased to the original name) and passing the
        // rest through unchanged.
        if !tp.column_masks.is_empty() {
            let projections: Vec<Expr> = schema
                .fields()
                .iter()
                .map(|field| {
                    let name = field.name();
                    match tp.column_masks.get(name) {
                        Some(mask) => mask.clone().alias(name),
                        None => col(name),
                    }
                })
                .collect();
            builder = builder.project(projections)?;
        }

        // Row filters: AND them together into one filter above the (possibly
        // masked) scan.
        if let Some(predicate) = tp.row_filters.iter().cloned().reduce(Expr::and) {
            builder = builder.filter(predicate)?;
        }

        Ok(Transformed::yes(builder.build()?))
    }
}

#[cfg(test)]
mod tests {
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::common::DFSchema;
    use datafusion::error::Result as DFResult;
    use datafusion::logical_expr::logical_plan::builder::table_scan;
    use datafusion::logical_expr::lit;

    use cedar_oci::{Decision, EntityUid};

    use super::*;
    use crate::principal::PrincipalIdentity;

    /// A test policy returning a fixed enforcement for any table.
    #[derive(Debug)]
    struct FixedPolicy(TablePolicy);

    #[async_trait::async_trait]
    impl Policy for FixedPolicy {
        async fn is_allowed(
            &self,
            _plan: &LogicalPlan,
            _principal: &PrincipalIdentity,
        ) -> DFResult<Decision> {
            Ok(Decision::Allow)
        }
        async fn table_policy(
            &self,
            _table: &TableReference,
            _schema: &DFSchema,
            _principal: &PrincipalIdentity,
        ) -> DFResult<TablePolicy> {
            Ok(self.0.clone())
        }
    }

    fn principal() -> PrincipalIdentity {
        use std::str::FromStr as _;
        PrincipalIdentity::new(EntityUid::from_str("User::\"alice\"").unwrap())
    }

    fn scan() -> LogicalPlan {
        let schema = Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("region", DataType::Utf8, true),
            Field::new("ssn", DataType::Utf8, true),
        ]);
        table_scan(Some("t"), &schema, None).unwrap().build().unwrap()
    }

    #[tokio::test]
    async fn no_policy_leaves_plan_unchanged() {
        let policy = FixedPolicy(TablePolicy::default());
        let plan = scan();
        let governed = govern_plan(&plan, &policy, &principal()).await.unwrap();
        assert_eq!(format!("{plan:?}"), format!("{governed:?}"));
    }

    #[tokio::test]
    async fn injects_row_filter() {
        let policy = FixedPolicy(TablePolicy {
            row_filters: vec![col("region").eq(lit("eu"))],
            column_masks: Default::default(),
        });
        let governed = govern_plan(&scan(), &policy, &principal()).await.unwrap();
        // Top of the governed subtree is a Filter.
        assert!(matches!(governed, LogicalPlan::Filter(_)), "expected a Filter at the top, got: {governed:?}");
    }

    #[tokio::test]
    async fn injects_column_mask_as_non_identity_projection() {
        let mut masks = HashMap::new();
        masks.insert("ssn".to_string(), lit("***"));
        let policy = FixedPolicy(TablePolicy {
            row_filters: vec![],
            column_masks: masks,
        });
        let governed = govern_plan(&scan(), &policy, &principal()).await.unwrap();
        // A Projection wraps the scan; the masked column is a literal, not a
        // bare column reference (so the optimizer cannot absorb it).
        let LogicalPlan::Projection(proj) = &governed else {
            panic!("expected a Projection at the top, got: {governed:?}");
        };
        let masked = proj
            .expr
            .iter()
            .find(|e| e.schema_name().to_string() == "ssn")
            .expect("ssn projection present");
        assert!(
            !matches!(masked, Expr::Column(_)),
            "masked column must not be a bare Column expr"
        );
    }
}
