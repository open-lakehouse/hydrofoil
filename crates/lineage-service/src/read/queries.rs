//! Reconstruct Marquez's materialized model (namespaces, jobs, datasets, lineage
//! graph) from the append-only OpenLineage events table.
//!
//! Strategy: a single scan of the events table builds an in-memory [`Model`] —
//! every distinct job with its most-recent input/output dataset references and
//! first/last seen timestamps, plus the set of datasets (both standalone dataset
//! events and those implied by job edges). All endpoints are then derived from
//! that model, including the lineage graph (built by a BFS over job↔dataset
//! edges). The query volume for a lineage UI is low, so doing the aggregation in
//! Rust over one scan is simpler and clearer than many bespoke SQL queries.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use chrono::{DateTime, Utc};
use deltalake::arrow::array::{Array, ArrayRef, StringArray, TimestampMicrosecondArray};
use deltalake::arrow::compute::cast;
use deltalake::arrow::datatypes::DataType;
use deltalake::arrow::record_batch::RecordBatch;
use serde_json::json;

use super::columns as col;
use super::model::*;
use super::{LineageStore, ReadError};

/// Default and maximum graph traversal depth (hops). Marquez's UI defaults to a
/// depth of 20; we cap to keep BFS bounded on dense graphs.
const MAX_DEPTH: usize = 20;

/// A dataset reference parsed from a job's `inputs_json` / `outputs_json`.
#[derive(serde::Deserialize)]
struct DatasetRef {
    namespace: String,
    name: String,
}

/// Aggregated, in-memory view of one job derived from its events.
#[derive(Default)]
struct JobAgg {
    inputs: Vec<EntityId>,
    outputs: Vec<EntityId>,
    /// Microseconds since epoch of the earliest / latest event for this job.
    first_seen: i64,
    last_seen: i64,
}

/// The reconstructed model: jobs keyed by id, the dataset id set, and the
/// per-dataset first/last seen times.
#[derive(Default)]
struct Model {
    jobs: BTreeMap<EntityId, JobAgg>,
    datasets: BTreeMap<EntityId, (i64, i64)>,
}

impl LineageStore {
    /// Scan the events table and fold it into a [`Model`]. An absent table
    /// (nothing ingested yet) yields an empty model.
    async fn model(&self) -> Result<Model, ReadError> {
        let Some(ctx) = self.session().await? else {
            return Ok(Model::default());
        };
        let batches = ctx
            .sql(&format!(
                "SELECT {kind}, {etime}, {jns}, {jname}, {dns}, {dname}, {inputs}, {outputs} \
                 FROM events",
                kind = col::EVENT_KIND,
                etime = col::EVENT_TIME,
                jns = col::JOB_NAMESPACE,
                jname = col::JOB_NAME,
                dns = col::DATASET_NAMESPACE,
                dname = col::DATASET_NAME,
                inputs = col::INPUTS_JSON,
                outputs = col::OUTPUTS_JSON,
            ))
            .await?
            .collect()
            .await?;

        let mut model = Model::default();
        for batch in &batches {
            fold_batch(&mut model, batch)?;
        }
        Ok(model)
    }

    /// `GET /api/v1/namespaces`
    pub async fn namespaces(&self) -> Result<Namespaces, ReadError> {
        let model = self.model().await?;
        let mut names: HashMap<String, (i64, i64)> = HashMap::new();
        let mut note = |ns: &str, lo: i64, hi: i64| {
            if ns.is_empty() {
                return;
            }
            let e = names.entry(ns.to_string()).or_insert((lo, hi));
            e.0 = e.0.min(lo);
            e.1 = e.1.max(hi);
        };
        for (id, agg) in &model.jobs {
            note(&id.namespace, agg.first_seen, agg.last_seen);
        }
        for (id, (lo, hi)) in &model.datasets {
            note(&id.namespace, *lo, *hi);
        }
        let mut namespaces: Vec<Namespace> = names
            .into_iter()
            .map(|(name, (lo, hi))| Namespace {
                name,
                created_at: micros_to_rfc3339(lo),
                updated_at: micros_to_rfc3339(hi),
                owner_name: None,
                description: None,
                is_hidden: false,
            })
            .collect();
        namespaces.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Namespaces { namespaces })
    }

    /// `GET /api/v1/namespaces/{ns}/jobs` (when `namespace` is `Some`) and the
    /// global `GET /api/v1/jobs` (when `None`, returning every namespace's jobs).
    pub async fn jobs(
        &self,
        namespace: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Jobs, ReadError> {
        let model = self.model().await?;
        let mut all: Vec<Job> = model
            .jobs
            .iter()
            .filter(|(id, _)| namespace.is_none_or(|ns| id.namespace == ns))
            .map(|(id, agg)| build_job(id, agg))
            .collect();
        all.sort_by(|a, b| a.name.cmp(&b.name));
        let total_count = all.len();
        let jobs = all.into_iter().skip(offset).take(limit).collect();
        Ok(Jobs { jobs, total_count })
    }

    /// `GET /api/v1/namespaces/{ns}/jobs/{job}`
    pub async fn job(&self, namespace: &str, name: &str) -> Result<Job, ReadError> {
        let model = self.model().await?;
        let id = EntityId {
            namespace: namespace.to_string(),
            name: name.to_string(),
        };
        model
            .jobs
            .get(&id)
            .map(|agg| build_job(&id, agg))
            .ok_or_else(|| ReadError::NotFound(format!("job {namespace}/{name}")))
    }

    /// `GET /api/v1/namespaces/{ns}/jobs/{job}/runs` — the job detail drawer's
    /// run history. We don't track real runs, so we return the same single
    /// synthetic run the job carries in `latestRuns`. 404 if the job is unknown.
    pub async fn job_runs(&self, namespace: &str, name: &str) -> Result<Runs, ReadError> {
        let job = self.job(namespace, name).await?;
        let total_count = job.latest_runs.len();
        Ok(Runs {
            runs: job.latest_runs,
            total_count,
        })
    }

    /// `GET /api/v1/namespaces/{ns}/datasets` (when `namespace` is `Some`) and
    /// the global `GET /api/v1/datasets` (when `None`).
    pub async fn datasets(
        &self,
        namespace: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Datasets, ReadError> {
        let model = self.model().await?;
        let mut all: Vec<Dataset> = model
            .datasets
            .iter()
            .filter(|(id, _)| namespace.is_none_or(|ns| id.namespace == ns))
            .map(|(id, times)| build_dataset(id, *times))
            .collect();
        all.sort_by(|a, b| a.name.cmp(&b.name));
        let total_count = all.len();
        let datasets = all.into_iter().skip(offset).take(limit).collect();
        Ok(Datasets {
            datasets,
            total_count,
        })
    }

    /// `GET /api/v1/namespaces/{ns}/datasets/{name}`
    pub async fn dataset(&self, namespace: &str, name: &str) -> Result<Dataset, ReadError> {
        let model = self.model().await?;
        let id = EntityId {
            namespace: namespace.to_string(),
            name: name.to_string(),
        };
        model
            .datasets
            .get(&id)
            .map(|times| build_dataset(&id, *times))
            .ok_or_else(|| ReadError::NotFound(format!("dataset {namespace}/{name}")))
    }

    /// `GET /api/v1/search?q=`
    pub async fn search(&self, q: &str, limit: usize) -> Result<Search, ReadError> {
        let model = self.model().await?;
        let needle = q.to_lowercase();
        let mut results: Vec<SearchResult> = Vec::new();
        for (id, agg) in &model.jobs {
            if id.name.to_lowercase().contains(&needle) {
                results.push(SearchResult {
                    name: id.name.clone(),
                    namespace: id.namespace.clone(),
                    node_id: job_node_id(&id.namespace, &id.name),
                    result_type: "JOB".into(),
                    updated_at: micros_to_rfc3339(agg.last_seen),
                });
            }
        }
        for (id, (_, hi)) in &model.datasets {
            if id.name.to_lowercase().contains(&needle) {
                results.push(SearchResult {
                    name: id.name.clone(),
                    namespace: id.namespace.clone(),
                    node_id: dataset_node_id(&id.namespace, &id.name),
                    result_type: "DATASET".into(),
                    updated_at: micros_to_rfc3339(*hi),
                });
            }
        }
        results.sort_by(|a, b| a.name.cmp(&b.name));
        results.truncate(limit);
        Ok(Search {
            total_count: results.len(),
            results,
        })
    }

    /// `GET /api/v1/lineage?nodeId=&depth=`
    ///
    /// Builds the full edge set from the model (input→job, job→output), then
    /// BFS-traverses up to `depth` hops in both directions from the seed node,
    /// emitting each reached node with its incident edges.
    pub async fn lineage(&self, node_id: &str, depth: usize) -> Result<LineageGraph, ReadError> {
        let (_, seed_ns, seed_name) = parse_node_id(node_id)
            .ok_or_else(|| ReadError::NotFound(format!("malformed nodeId {node_id}")))?;
        let _ = (&seed_ns, &seed_name);
        let model = self.model().await?;
        let depth = depth.min(MAX_DEPTH);

        // Build directed edges and an adjacency map (undirected, for traversal).
        let mut edges: HashSet<LineageEdge> = HashSet::new();
        let mut neighbors: HashMap<String, HashSet<String>> = HashMap::new();
        let connect = |from: String,
                       to: String,
                       edges: &mut HashSet<LineageEdge>,
                       neighbors: &mut HashMap<String, HashSet<String>>| {
            neighbors
                .entry(from.clone())
                .or_default()
                .insert(to.clone());
            neighbors
                .entry(to.clone())
                .or_default()
                .insert(from.clone());
            edges.insert(LineageEdge {
                origin: from,
                destination: to,
            });
        };
        for (id, agg) in &model.jobs {
            let job_id = job_node_id(&id.namespace, &id.name);
            for input in &agg.inputs {
                let ds = dataset_node_id(&input.namespace, &input.name);
                connect(ds, job_id.clone(), &mut edges, &mut neighbors);
            }
            for output in &agg.outputs {
                let ds = dataset_node_id(&output.namespace, &output.name);
                connect(job_id.clone(), ds, &mut edges, &mut neighbors);
            }
        }

        // BFS out to `depth` hops from the seed.
        let mut reached: HashSet<String> = HashSet::new();
        reached.insert(node_id.to_string());
        let mut frontier: VecDeque<(String, usize)> = VecDeque::new();
        frontier.push_back((node_id.to_string(), 0));
        while let Some((cur, d)) = frontier.pop_front() {
            if d >= depth {
                continue;
            }
            if let Some(adj) = neighbors.get(&cur) {
                for next in adj {
                    if reached.insert(next.clone()) {
                        frontier.push_back((next.clone(), d + 1));
                    }
                }
            }
        }

        // Emit each reached node with its incident in/out edges.
        let graph = reached
            .iter()
            .filter_map(|id| build_node(id, &model, &edges))
            .collect();
        Ok(LineageGraph { graph })
    }
}

/// Fold one record batch of the projected events query into the model.
fn fold_batch(model: &mut Model, batch: &RecordBatch) -> Result<(), ReadError> {
    let kind = str_col(batch, 0)?;
    let etime = batch
        .column(1)
        .as_any()
        .downcast_ref::<TimestampMicrosecondArray>()
        .ok_or_else(|| ReadError::Query("event_time column type mismatch".into()))?;
    let job_ns = str_col(batch, 2)?;
    let job_name = str_col(batch, 3)?;
    let ds_ns = str_col(batch, 4)?;
    let ds_name = str_col(batch, 5)?;
    let inputs = str_col(batch, 6)?;
    let outputs = str_col(batch, 7)?;

    for row in 0..batch.num_rows() {
        let ts = if etime.is_null(row) {
            0
        } else {
            etime.value(row)
        };
        match value(&kind, row) {
            Some("run") | Some("job") => {
                let (Some(ns), Some(name)) = (value(&job_ns, row), value(&job_name, row)) else {
                    continue;
                };
                let id = EntityId {
                    namespace: ns.to_string(),
                    name: name.to_string(),
                };
                let in_refs = parse_dataset_refs(value(&inputs, row));
                let out_refs = parse_dataset_refs(value(&outputs, row));
                // Latest event wins for the input/output sets; track time bounds.
                let entry = model.jobs.entry(id).or_insert_with(|| JobAgg {
                    first_seen: ts,
                    last_seen: ts,
                    ..Default::default()
                });
                if ts >= entry.last_seen || (entry.inputs.is_empty() && entry.outputs.is_empty()) {
                    entry.inputs = in_refs.clone();
                    entry.outputs = out_refs.clone();
                }
                entry.first_seen = entry.first_seen.min(ts);
                entry.last_seen = entry.last_seen.max(ts);

                // Datasets implied by the job's edges.
                for r in in_refs.into_iter().chain(out_refs) {
                    note_dataset(model, r.namespace, r.name, ts);
                }
            }
            Some("dataset") => {
                if let (Some(ns), Some(name)) = (value(&ds_ns, row), value(&ds_name, row)) {
                    note_dataset(model, ns.to_string(), name.to_string(), ts);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn note_dataset(model: &mut Model, namespace: String, name: String, ts: i64) {
    let id = EntityId { namespace, name };
    let e = model.datasets.entry(id).or_insert((ts, ts));
    e.0 = e.0.min(ts);
    e.1 = e.1.max(ts);
}

fn parse_dataset_refs(raw: Option<&str>) -> Vec<EntityId> {
    let Some(raw) = raw else { return Vec::new() };
    serde_json::from_str::<Vec<DatasetRef>>(raw)
        .unwrap_or_default()
        .into_iter()
        .map(|r| EntityId {
            namespace: r.namespace,
            name: r.name,
        })
        .collect()
}

fn build_job(id: &EntityId, agg: &JobAgg) -> Job {
    let updated_at = micros_to_rfc3339(agg.last_seen);
    let node_id = job_node_id(&id.namespace, &id.name);
    Job {
        id: id.clone(),
        job_type: "BATCH".into(),
        name: id.name.clone(),
        simple_name: id.name.clone(),
        namespace: id.namespace.clone(),
        created_at: micros_to_rfc3339(agg.first_seen),
        updated_at: updated_at.clone(),
        inputs: agg.inputs.clone(),
        outputs: agg.outputs.clone(),
        location: None,
        description: None,
        latest_run: None,
        // Exactly one synthetic run — the dashboard reduces over this array
        // without an initial value and crashes if it's empty.
        latest_runs: vec![LatestRun::synthetic(&node_id, &updated_at)],
        tags: Vec::new(),
        parent_job_name: None,
        parent_job_uuid: None,
    }
}

fn build_dataset(id: &EntityId, times: (i64, i64)) -> Dataset {
    Dataset {
        id: id.clone(),
        dataset_type: "DB_TABLE".into(),
        name: id.name.clone(),
        physical_name: id.name.clone(),
        namespace: id.namespace.clone(),
        source_name: id.namespace.clone(),
        created_at: micros_to_rfc3339(times.0),
        updated_at: micros_to_rfc3339(times.1),
        description: None,
        fields: Vec::new(),
        facets: json!({}),
        tags: Vec::new(),
        deleted: false,
    }
}

/// Build a graph node for `node_id` from the model, attaching its incident
/// edges. Returns `None` if the id is malformed.
fn build_node(node_id: &str, model: &Model, edges: &HashSet<LineageEdge>) -> Option<LineageNode> {
    let (kind, namespace, name) = parse_node_id(node_id)?;
    let id = EntityId {
        namespace: namespace.clone(),
        name: name.clone(),
    };
    let (node_type, data) = match kind {
        NodeKind::Job => {
            let job = model
                .jobs
                .get(&id)
                .map(|agg| build_job(&id, agg))
                .unwrap_or_else(|| build_job(&id, &JobAgg::default()));
            ("JOB", serde_json::to_value(job).ok()?)
        }
        NodeKind::Dataset => {
            let times = model.datasets.get(&id).copied().unwrap_or((0, 0));
            (
                "DATASET",
                serde_json::to_value(build_dataset(&id, times)).ok()?,
            )
        }
    };
    let in_edges = edges
        .iter()
        .filter(|e| e.destination == node_id)
        .cloned()
        .collect();
    let out_edges = edges
        .iter()
        .filter(|e| e.origin == node_id)
        .cloned()
        .collect();
    Some(LineageNode {
        id: node_id.to_string(),
        node_type: node_type.into(),
        data,
        in_edges,
        out_edges,
    })
}

/// Read column `idx` as a `Utf8` [`StringArray`], casting from whatever string
/// representation DataFusion produced (delta-rs reads strings as `Utf8View`).
fn str_col(batch: &RecordBatch, idx: usize) -> Result<StringArray, ReadError> {
    let col: &ArrayRef = batch.column(idx);
    let utf8 = if col.data_type() == &DataType::Utf8 {
        col.clone()
    } else {
        cast(col, &DataType::Utf8).map_err(|e| ReadError::Query(e.to_string()))?
    };
    utf8.as_any()
        .downcast_ref::<StringArray>()
        .cloned()
        .ok_or_else(|| ReadError::Query(format!("column {idx} is not castable to Utf8")))
}

fn value(col: &StringArray, row: usize) -> Option<&str> {
    if col.is_null(row) {
        None
    } else {
        Some(col.value(row))
    }
}

/// Format epoch-microseconds as an RFC3339 timestamp (Marquez serializes all
/// times as ISO-8601 strings). A zero (unknown) timestamp maps to the epoch.
fn micros_to_rfc3339(micros: i64) -> String {
    DateTime::<Utc>::from_timestamp_micros(micros)
        .unwrap_or_else(|| DateTime::<Utc>::from_timestamp_nanos(0))
        .to_rfc3339()
}
