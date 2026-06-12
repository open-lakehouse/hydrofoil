//! Extract table-level lineage (input/output datasets + schema) from a
//! DataFusion [`LogicalPlan`].
//!
//! Follows the same `TreeNodeVisitor`-over-`LogicalPlan` shape the Cedar policy
//! integration uses. Run this on the *optimized* plan so projections/filters are
//! pushed down to the scans.
//!
//! Column-level lineage is intentionally **not** extracted here — see the
//! "Column lineage" section of `docs/open-lineage-design.md` for why the
//! name-based approach was unsound and what a correct implementation requires.

use datafusion::common::tree_node::{TreeNode, TreeNodeRecursion, TreeNodeVisitor};
use datafusion::error::Result;
use datafusion::logical_expr::{DdlStatement, LogicalPlan, WriteOp};
use datafusion::sql::TableReference;

use crate::config::OpenLineageConfig;
use crate::facets::{BaseFacet, DatasetFacets, SchemaDatasetFacet, SchemaField};
use crate::naming::DatasetName;

const SCHEMA_FACET: &str = "1-1-0/SchemaDatasetFacet.json";

/// What a query reads and writes.
#[derive(Debug, Default)]
pub struct QueryLineage {
    pub inputs: Vec<InputTable>,
    pub outputs: Vec<OutputTable>,
    pub sql: Option<String>,
}

#[derive(Debug)]
pub struct InputTable {
    pub name: DatasetName,
    pub fields: Vec<SchemaField>,
}

#[derive(Debug)]
pub struct OutputTable {
    pub name: DatasetName,
}

/// Extract [`QueryLineage`] from an (ideally optimized) logical plan.
pub fn extract(plan: &LogicalPlan, config: &OpenLineageConfig) -> QueryLineage {
    let mut visitor = LineageVisitor {
        config,
        inputs: Vec::new(),
        outputs: Vec::new(),
    };
    // The visitor never returns an error; ignore the traversal Result.
    let _ = plan.visit(&mut visitor);

    QueryLineage {
        inputs: visitor.inputs,
        outputs: visitor.outputs,
        sql: None,
    }
}

struct LineageVisitor<'a> {
    config: &'a OpenLineageConfig,
    inputs: Vec<InputTable>,
    outputs: Vec<OutputTable>,
}

impl LineageVisitor<'_> {
    fn dataset_for(&self, table_ref: &TableReference) -> DatasetName {
        // A bare TableScan carries only the qualified reference, not a storage
        // location. Use the qualified name under the configured namespace; the
        // host integration can enrich with a physical location + symlinks facet.
        DatasetName::from_table_ref(&self.config.job_namespace, &table_ref.to_string())
    }
}

impl TreeNodeVisitor<'_> for LineageVisitor<'_> {
    type Node = LogicalPlan;

    fn f_down(&mut self, node: &Self::Node) -> Result<TreeNodeRecursion> {
        match node {
            // Skip scans of the `information_schema` virtual catalog: it is
            // DataFusion's metadata surface, not a real dataset, so reporting it
            // as a lineage input only adds noise. Treating these scans as
            // non-inputs is also what lets the planner suppress pure-metadata
            // queries (no inputs + no outputs => no events).
            LogicalPlan::TableScan(scan)
                if scan
                    .table_name
                    .schema()
                    .is_some_and(|s| s.eq_ignore_ascii_case("information_schema")) => {}
            LogicalPlan::TableScan(scan) => {
                let dataset = self.dataset_for(&scan.table_name);
                // Report the *full* table schema, not the projected scan schema:
                // after projection pushdown `SELECT a FROM t` would otherwise
                // report `t` as having only column `a`, causing the dataset's
                // schema version to flap between queries.
                let fields: Vec<SchemaField> = scan
                    .source
                    .schema()
                    .fields()
                    .iter()
                    .map(|f| SchemaField {
                        name: f.name().to_string(),
                        type_: f.data_type().to_string(),
                        description: None,
                    })
                    .collect();
                // Dedupe by `(namespace, name)`: a self-join scans the same
                // table twice but it is a single input dataset.
                if !self
                    .inputs
                    .iter()
                    .any(|i| i.name.namespace == dataset.namespace && i.name.name == dataset.name)
                {
                    self.inputs.push(InputTable {
                        name: dataset,
                        fields,
                    });
                }
            }
            LogicalPlan::Dml(dml) => match dml.op {
                WriteOp::Insert(_) | WriteOp::Update | WriteOp::Delete | WriteOp::Ctas => {
                    self.outputs.push(OutputTable {
                        name: self.dataset_for(&dml.table_name),
                    });
                }
                WriteOp::Truncate => {}
            },
            LogicalPlan::Ddl(ddl) => match ddl {
                DdlStatement::CreateExternalTable(cmd) => {
                    self.outputs.push(OutputTable {
                        name: self.dataset_for(&cmd.name),
                    });
                }
                // `CREATE TABLE ... AS SELECT` lowers to CreateMemoryTable; the
                // new table is the output dataset (the SELECT's scans are its
                // inputs, picked up by the TableScan arm).
                DdlStatement::CreateMemoryTable(cmd) => {
                    self.outputs.push(OutputTable {
                        name: self.dataset_for(&cmd.name),
                    });
                }
                _ => {}
            },
            // Nodes we don't yet derive lineage from. Warn so coverage gaps are
            // visible rather than silently dropped.
            other => {
                tracing::trace!(
                    target: "openlineage",
                    node = other.display().to_string(),
                    "no lineage extraction for plan node"
                );
            }
        }
        Ok(TreeNodeRecursion::Continue)
    }
}

/// Build the [`DatasetFacets`] for an input table: its schema facet.
///
/// Column-level lineage is intentionally omitted (see the module docs and
/// `docs/open-lineage-design.md`); inputs carry table-level schema only.
pub fn input_dataset_facets(input: &InputTable, config: &OpenLineageConfig) -> DatasetFacets {
    let schema = SchemaDatasetFacet {
        base: BaseFacet::new(&config.producer, SCHEMA_FACET),
        fields: input.fields.clone(),
    };

    DatasetFacets {
        schema: Some(schema),
        ..Default::default()
    }
}
