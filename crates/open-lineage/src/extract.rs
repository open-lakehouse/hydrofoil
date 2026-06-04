//! Extract lineage (input/output datasets + column-level provenance) from a
//! DataFusion [`LogicalPlan`].
//!
//! Follows the same `TreeNodeVisitor`-over-`LogicalPlan` shape the Cedar policy
//! integration uses. Run this on the *optimized* plan so projections/filters are
//! pushed down to the scans.

use std::collections::BTreeMap;

use datafusion::common::tree_node::{TreeNode, TreeNodeRecursion, TreeNodeVisitor};
use datafusion::error::Result;
use datafusion::logical_expr::{DdlStatement, Expr, LogicalPlan, Projection, WriteOp};
use datafusion::sql::TableReference;
use serde_json::{Map, Value};

use crate::config::OpenLineageConfig;
use crate::facets::{
    BaseFacet, ColumnLineageDatasetFacet, DatasetFacets, FieldLineage, InputField,
    SchemaDatasetFacet, SchemaField, Transformation, TransformationType,
};
use crate::naming::DatasetName;

const SCHEMA_FACET: &str = "1-1-0/SchemaDatasetFacet.json";
const COLUMN_LINEAGE_FACET: &str = "1-2-0/ColumnLineageDatasetFacet.json";

/// What a query reads, writes, and how output columns derive from inputs.
#[derive(Debug, Default)]
pub struct QueryLineage {
    pub inputs: Vec<InputTable>,
    pub outputs: Vec<OutputTable>,
    /// Per output column -> the input `(dataset, field)`s it derives from.
    pub column_lineage: BTreeMap<String, Vec<InputField>>,
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
        column_lineage: BTreeMap::new(),
    };
    // The visitor never returns an error; ignore the traversal Result.
    let _ = plan.visit(&mut visitor);

    QueryLineage {
        inputs: visitor.inputs,
        outputs: visitor.outputs,
        column_lineage: visitor.column_lineage,
        sql: None,
    }
}

struct LineageVisitor<'a> {
    config: &'a OpenLineageConfig,
    inputs: Vec<InputTable>,
    outputs: Vec<OutputTable>,
    column_lineage: BTreeMap<String, Vec<InputField>>,
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
            LogicalPlan::TableScan(scan) => {
                let dataset = self.dataset_for(&scan.table_name);
                let fields: Vec<SchemaField> = scan
                    .projected_schema
                    .fields()
                    .iter()
                    .map(|f| SchemaField {
                        name: f.name().to_string(),
                        type_: f.data_type().to_string(),
                        description: None,
                    })
                    .collect();
                // A trivial `SELECT a, b FROM t` optimizes to a bare scan with
                // no Projection node, so identity lineage lives here: each
                // scanned column maps 1:1 to itself. A Projection node above
                // (handled below) overrides this for transformed columns.
                for field in &fields {
                    self.column_lineage
                        .entry(field.name.clone())
                        .or_insert_with(|| {
                            vec![InputField {
                                namespace: dataset.namespace.clone(),
                                name: dataset.name.clone(),
                                field: Some(field.name.clone()),
                                transformations: vec![Transformation {
                                    type_: TransformationType::Direct,
                                    subtype: Some("IDENTITY".to_string()),
                                    description: String::new(),
                                    masking: false,
                                }],
                            }]
                        });
                }
                self.inputs.push(InputTable {
                    name: dataset,
                    fields,
                });
            }
            LogicalPlan::Projection(proj) => self.collect_column_lineage(proj),
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

impl LineageVisitor<'_> {
    /// Map each projection output column to the input columns its expression
    /// references. Identity projections are `DIRECT/IDENTITY`; any other
    /// expression over columns is `DIRECT/TRANSFORMATION`.
    fn collect_column_lineage(&mut self, proj: &Projection) {
        for (field, expr) in proj.schema.fields().iter().zip(proj.expr.iter()) {
            let output_col = field.name().to_string();
            let is_identity = matches!(expr, Expr::Column(_));
            let subtype = if is_identity {
                "IDENTITY"
            } else {
                "TRANSFORMATION"
            };

            let mut input_fields: Vec<InputField> = Vec::new();
            for col in expr.column_refs() {
                let Some(rel) = &col.relation else { continue };
                let ds = self.dataset_for(rel);
                input_fields.push(InputField {
                    namespace: ds.namespace,
                    name: ds.name,
                    field: Some(col.name.clone()),
                    transformations: vec![Transformation {
                        type_: TransformationType::Direct,
                        subtype: Some(subtype.to_string()),
                        description: String::new(),
                        masking: false,
                    }],
                });
            }

            if !input_fields.is_empty() {
                // A column may appear in multiple projections as the plan is
                // walked; keep the richest (last) mapping.
                self.column_lineage.insert(output_col, input_fields);
            }
        }
    }
}

/// Build the [`DatasetFacets`] for an input table: schema + column lineage
/// (the latter only for columns this dataset contributes to).
pub fn input_dataset_facets(
    input: &InputTable,
    column_lineage: &BTreeMap<String, Vec<InputField>>,
    config: &OpenLineageConfig,
) -> DatasetFacets {
    let schema = SchemaDatasetFacet {
        base: BaseFacet::new(&config.producer, SCHEMA_FACET),
        fields: input.fields.clone(),
    };

    // Column lineage facet, restricted to fields sourced from this dataset.
    let mut fields: Map<String, Value> = Map::new();
    for (out_col, inputs) in column_lineage {
        let from_here: Vec<InputField> = inputs
            .iter()
            .filter(|f| f.namespace == input.name.namespace && f.name == input.name.name)
            .cloned()
            .collect();
        if !from_here.is_empty() {
            let lineage = FieldLineage {
                input_fields: from_here,
            };
            if let Ok(v) = serde_json::to_value(lineage) {
                fields.insert(out_col.clone(), v);
            }
        }
    }

    let column_lineage = if fields.is_empty() {
        None
    } else {
        Some(ColumnLineageDatasetFacet {
            base: BaseFacet::new(&config.producer, COLUMN_LINEAGE_FACET),
            fields,
            dataset: Vec::new(),
        })
    };

    DatasetFacets {
        schema: Some(schema),
        column_lineage,
        ..Default::default()
    }
}
