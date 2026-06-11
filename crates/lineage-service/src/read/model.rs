//! Serde structs mirroring the subset of the Marquez REST API the web UI needs
//! to render the namespace/job/dataset browse views and the lineage graph.
//!
//! Field names and envelope keys are taken from the Marquez web client
//! (`web/src/store/requests/*` and `web/src/types/*`). Everything serializes as
//! camelCase to match. Fields we cannot derive from a raw OpenLineage event log
//! (runs, versions, tags, facets, column lineage, metrics) are emitted as empty
//! collections / nulls — the graph views tolerate that and we keep the contract
//! shape intact.

use serde::Serialize;

/// `{ namespace, name }` — the identity object the UI uses for both jobs and
/// datasets.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityId {
    pub namespace: String,
    pub name: String,
}

// --- /namespaces ---

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Namespace {
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub owner_name: Option<String>,
    pub description: Option<String>,
    pub is_hidden: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Namespaces {
    pub namespaces: Vec<Namespace>,
}

// --- /namespaces/{ns}/jobs ---

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: EntityId,
    /// Marquez job type. We always report `BATCH`; the event log doesn't
    /// distinguish stream/service jobs.
    #[serde(rename = "type")]
    pub job_type: String,
    pub name: String,
    pub simple_name: String,
    pub namespace: String,
    pub created_at: String,
    pub updated_at: String,
    pub inputs: Vec<EntityId>,
    pub outputs: Vec<EntityId>,
    pub location: Option<String>,
    pub description: Option<String>,
    pub latest_run: Option<LatestRun>,
    /// We don't reconstruct real run history, but the dashboard's `JobRunItem`
    /// calls `latestRuns.reduce(...)` with no initial value and crashes on an
    /// empty array — so we always emit exactly one synthetic run here. See
    /// [`build_job`](crate::read::queries) / [`LatestRun::synthetic`].
    pub latest_runs: Vec<LatestRun>,
    pub tags: Vec<String>,
    pub parent_job_name: Option<String>,
    pub parent_job_uuid: Option<String>,
}

/// Minimal Marquez `Run` shape — only the fields the web UI dereferences
/// (`id`, `state`, `durationMs`, the timestamps). We don't track real runs, so
/// these are synthesized; see [`LatestRun::synthetic`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestRun {
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    /// `NEW` | `RUNNING` | `COMPLETED` | `FAILED` | `ABORTED`.
    pub state: String,
    pub nominal_start_time: Option<String>,
    pub nominal_end_time: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub duration_ms: u64,
}

impl LatestRun {
    /// A single neutral run so the dashboard's `latestRuns.reduce(...)` has an
    /// element to fold over. `durationMs` is 0 (renders a minimal bar); state is
    /// `COMPLETED` so it isn't flagged as failed.
    pub fn synthetic(job_id: &str, updated_at: &str) -> Self {
        Self {
            id: format!("synthetic:{job_id}"),
            created_at: updated_at.to_string(),
            updated_at: updated_at.to_string(),
            state: "COMPLETED".to_string(),
            nominal_start_time: None,
            nominal_end_time: None,
            started_at: None,
            ended_at: None,
            duration_ms: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Jobs {
    pub jobs: Vec<Job>,
    pub total_count: usize,
}

/// `GET /api/v1/namespaces/{ns}/jobs/{job}/runs` envelope. The UI's runs
/// reducer reads `payload.runs` and `totalCount`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Runs {
    pub runs: Vec<LatestRun>,
    pub total_count: usize,
}

// --- /namespaces/{ns}/datasets ---

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dataset {
    pub id: EntityId,
    /// Marquez dataset type; we always report `DB_TABLE`.
    #[serde(rename = "type")]
    pub dataset_type: String,
    pub name: String,
    pub physical_name: String,
    pub namespace: String,
    pub source_name: String,
    pub created_at: String,
    pub updated_at: String,
    pub description: Option<String>,
    pub fields: Vec<serde_json::Value>,
    pub facets: serde_json::Value,
    pub tags: Vec<String>,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Datasets {
    pub datasets: Vec<Dataset>,
    pub total_count: usize,
}

// --- /search ---

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub name: String,
    pub namespace: String,
    pub node_id: String,
    /// `JOB` or `DATASET`.
    #[serde(rename = "type")]
    pub result_type: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Search {
    pub total_count: usize,
    pub results: Vec<SearchResult>,
}

// --- /lineage ---

/// A directed edge between two nodes, addressed by their `nodeId` strings.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Hash)]
pub struct LineageEdge {
    pub origin: String,
    pub destination: String,
}

/// One node in the lineage graph. `data` carries the full [`Job`] or [`Dataset`]
/// payload the UI renders in the side panel. Note the `camelCase` rename: the
/// UI's graph layout reads `node.inEdges` / `node.outEdges` and crashes
/// (`.map()` of undefined) if they arrive as snake_case.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineageNode {
    /// The `nodeId`, e.g. `job:ns:name` or `dataset:ns:name`.
    pub id: String,
    /// `JOB` or `DATASET`.
    #[serde(rename = "type")]
    pub node_type: String,
    pub data: serde_json::Value,
    pub in_edges: Vec<LineageEdge>,
    pub out_edges: Vec<LineageEdge>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LineageGraph {
    pub graph: Vec<LineageNode>,
}

/// Build the Marquez `nodeId` for a job.
pub fn job_node_id(namespace: &str, name: &str) -> String {
    format!("job:{namespace}:{name}")
}

/// Build the Marquez `nodeId` for a dataset.
pub fn dataset_node_id(namespace: &str, name: &str) -> String {
    format!("dataset:{namespace}:{name}")
}

/// The two node kinds a `nodeId` can address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Job,
    Dataset,
}

/// Parse a Marquez `nodeId` of the form `job:<namespace>:<name>` or
/// `dataset:<namespace>:<name>`. The name may itself contain `:` (dataset names
/// often do), so only the first two `:` are treated as separators.
pub fn parse_node_id(node_id: &str) -> Option<(NodeKind, String, String)> {
    let (kind, rest) = node_id.split_once(':')?;
    let (namespace, name) = rest.split_once(':')?;
    let kind = match kind {
        "job" => NodeKind::Job,
        "dataset" => NodeKind::Dataset,
        _ => return None,
    };
    Some((kind, namespace.to_string(), name.to_string()))
}
