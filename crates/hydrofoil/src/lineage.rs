//! Hydrofoil's OpenLineage context wiring.
//!
//! Bridges gRPC request metadata (parent-run headers a client sends) into the
//! [`LineageContext`] the OpenLineage planner consumes. See
//! `docs/session-management.md` for the broader session design.
//!
//! The server parses parent-run context per request via [`context_from_metadata`]
//! and attaches it to a request-scoped session
//! (`LakehouseCtx::session_with_lineage`), where [`HydrofoilContextProvider`]
//! reads it back at planning time.
//!
//! ## Job identity
//!
//! Each statement gets a stable per-statement *job name* (see
//! [`job_name_from_metadata`]) so distinct queries surface as distinct Marquez
//! jobs rather than collapsing onto one node. A client may pin the job name
//! explicitly with the [`headers::JOB_NAME`] (`x-openlineage-job-name`) metadata
//! key; absent that, the name is derived from a stable hash of the
//! whitespace-normalized SQL (`query-<12-hex>`). The job *namespace* stays
//! config-driven (the engine's `OpenLineageConfig`), not per request.

use datafusion::execution::context::SessionState;
use datafusion_open_lineage::config::OpenLineageConfig;
use datafusion_open_lineage::context::{LineageContext, LineageContextProvider};
use datafusion_open_lineage::facets::{
    BaseFacet, ParentJob, ParentRun, ParentRunFacet, RootParent,
};
use tonic::metadata::MetadataMap;

/// gRPC metadata keys for forwarding OpenLineage parent-run context.
///
/// OpenLineage defines no standard header for this, so we mirror Spark's
/// discrete `spark.openlineage.parent*` property names as lowercase metadata
/// keys (slash-safe). See `docs/session-management.md`.
pub mod headers {
    pub const PARENT_RUN_ID: &str = "x-openlineage-parent-run-id";
    pub const PARENT_JOB_NAMESPACE: &str = "x-openlineage-parent-job-namespace";
    pub const PARENT_JOB_NAME: &str = "x-openlineage-parent-job-name";
    pub const ROOT_PARENT_RUN_ID: &str = "x-openlineage-root-parent-run-id";
    pub const ROOT_PARENT_JOB_NAMESPACE: &str = "x-openlineage-root-parent-job-namespace";
    pub const ROOT_PARENT_JOB_NAME: &str = "x-openlineage-root-parent-job-name";

    /// Optional client-supplied OpenLineage *job name* for this statement. When
    /// present it wins over the SQL-hash fallback (see
    /// [`super::job_name_from_metadata`]), letting a caller pin a meaningful,
    /// stable job identity (e.g. a dbt model or named report).
    pub const JOB_NAME: &str = "x-openlineage-job-name";
}

/// The `SessionConfig` extension type carrying per-session lineage context.
///
/// A distinct newtype so `SessionConfig::get_extension` (which keys by
/// `TypeId`) resolves it unambiguously.
#[derive(Debug, Clone, Default)]
pub struct LineageContextExt(pub LineageContext);

/// Parse OpenLineage parent-run context from gRPC request metadata.
///
/// Returns an empty [`LineageContext`] when no parent headers are present.
/// A parent facet is only produced when all three parent fields
/// (run id, job namespace, job name) are supplied together; likewise for root.
pub fn context_from_metadata(meta: &MetadataMap, config: &OpenLineageConfig) -> LineageContext {
    let get = |key: &str| {
        meta.get(key)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };

    let parent_run = match (
        get(headers::PARENT_RUN_ID),
        get(headers::PARENT_JOB_NAMESPACE),
        get(headers::PARENT_JOB_NAME),
    ) {
        (Some(run_id), Some(namespace), Some(name)) => {
            let root = match (
                get(headers::ROOT_PARENT_RUN_ID),
                get(headers::ROOT_PARENT_JOB_NAMESPACE),
                get(headers::ROOT_PARENT_JOB_NAME),
            ) {
                (Some(r_run), Some(r_ns), Some(r_name)) => Some(RootParent {
                    run: ParentRun { run_id: r_run },
                    job: ParentJob {
                        namespace: r_ns,
                        name: r_name,
                    },
                }),
                _ => None,
            };
            Some(ParentRunFacet {
                base: BaseFacet::new(&config.producer, "1-0-0/ParentRunFacet.json"),
                run: ParentRun { run_id },
                job: ParentJob { namespace, name },
                root,
            })
        }
        _ => None,
    };

    LineageContext {
        parent_run,
        ..Default::default()
    }
}

/// Derive the per-statement OpenLineage job name.
///
/// A client-supplied [`headers::JOB_NAME`] wins; otherwise the name is the SQL
/// hash from [`job_name_from_sql`]. Trimmed; an empty header value is ignored
/// (falls back to the hash).
pub fn job_name_from_metadata(meta: &MetadataMap, sql: &str) -> String {
    meta.get(headers::JOB_NAME)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| job_name_from_sql(sql))
}

/// A stable job name derived from the SQL text: `query-<12-hex>` over a
/// 64-bit FNV-1a hash of the whitespace-normalized query.
///
/// Normalizing whitespace (collapsing runs of ASCII whitespace to single
/// spaces, trimmed) means cosmetically-different spellings of the same query
/// map to one job. FNV-1a is used (rather than `DefaultHasher`) because the
/// name must be stable across process restarts and Rust versions, so the same
/// query is always the same Marquez job.
pub fn job_name_from_sql(sql: &str) -> String {
    let normalized = normalize_whitespace(sql);
    let hash = fnv1a_64(normalized.as_bytes());
    // 16 hex chars for a u64; take the first 12 for a compact, collision-rare id.
    let hex: String = format!("{hash:016x}").chars().take(12).collect();
    format!("query-{hex}")
}

/// Collapse runs of ASCII whitespace to a single space and trim the ends.
fn normalize_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 64-bit FNV-1a. Small, dependency-free, and deterministic across releases —
/// the properties a persisted job identity needs (it is not a cryptographic
/// hash and is not used as one).
fn fnv1a_64(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Derive the lineage context for a single *execution* of a stored statement
/// from the context snapshotted at planning/creation time.
///
/// Each execution mints a **fresh** `run_id` (so re-executing a prepared
/// statement N times yields N distinct runs — exactly-once START/terminal per
/// run, no clobbering of one run's COMPLETE by a later run's FAIL). The
/// planning/creation run becomes this run's `parent` facet, preserving the
/// correlation ADR 0003 established: a run can still be traced back to the
/// statement it came from, now via the parent chain rather than a shared id.
///
/// Job identity (name + namespace) and SQL carry over unchanged, so every
/// execution of one statement stays the same Marquez job. Any parent facet
/// already present on the planning context (a client-supplied orchestrator
/// parent) is promoted to this run's `root`, keeping the orchestration lineage
/// intact above the per-statement parent.
pub fn execution_context(planning: &LineageContext, config: &OpenLineageConfig) -> LineageContext {
    let mut cx = planning.clone();
    cx.run_id = Some(uuid::Uuid::now_v7());

    if let Some(planning_run) = planning.run_id {
        let namespace = planning
            .job_namespace
            .clone()
            .unwrap_or_else(|| config.job_namespace.clone());
        let name = planning
            .job_name
            .clone()
            .unwrap_or_else(|| "datafusion_query".to_string());

        // Promote any orchestrator-supplied parent on the planning context to
        // this run's root, so the chain reads execution -> statement -> orchestrator.
        let root = planning.parent_run.as_ref().map(|p| RootParent {
            run: ParentRun {
                run_id: p.run.run_id.clone(),
            },
            job: ParentJob {
                namespace: p.job.namespace.clone(),
                name: p.job.name.clone(),
            },
        });

        cx.parent_run = Some(ParentRunFacet {
            base: BaseFacet::new(&config.producer, "1-0-0/ParentRunFacet.json"),
            run: ParentRun {
                run_id: planning_run.to_string(),
            },
            job: ParentJob { namespace, name },
            root,
        });
    }

    cx
}

/// A [`LineageContextProvider`] that reads the [`LineageContext`] attached to
/// the session's `SessionConfig` as a [`LineageContextExt`] extension.
///
/// The server attaches the extension (parsed from request metadata) when it
/// builds the session; here we read it back at planning time. When no
/// extension is present (the current default), an empty context is returned.
#[derive(Debug, Default)]
pub struct HydrofoilContextProvider;

#[async_trait::async_trait]
impl LineageContextProvider for HydrofoilContextProvider {
    async fn context(&self, session_state: &SessionState) -> LineageContext {
        session_state
            .config()
            .get_extension::<LineageContextExt>()
            .map(|ext| ext.0.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> OpenLineageConfig {
        OpenLineageConfig::default()
    }

    #[test]
    fn parses_full_parent_context() {
        let mut meta = MetadataMap::new();
        meta.insert(headers::PARENT_RUN_ID, "run-123".parse().unwrap());
        meta.insert(headers::PARENT_JOB_NAMESPACE, "airflow".parse().unwrap());
        meta.insert(headers::PARENT_JOB_NAME, "dag.task".parse().unwrap());

        let cx = context_from_metadata(&meta, &config());
        let parent = cx.parent_run.expect("parent facet present");
        assert_eq!(parent.run.run_id, "run-123");
        assert_eq!(parent.job.namespace, "airflow");
        assert_eq!(parent.job.name, "dag.task");
        assert!(parent.root.is_none());
    }

    #[test]
    fn partial_parent_context_is_ignored() {
        let mut meta = MetadataMap::new();
        // Missing job name -> incomplete -> no facet.
        meta.insert(headers::PARENT_RUN_ID, "run-123".parse().unwrap());
        meta.insert(headers::PARENT_JOB_NAMESPACE, "airflow".parse().unwrap());

        let cx = context_from_metadata(&meta, &config());
        assert!(cx.parent_run.is_none());
    }

    #[test]
    fn parses_root_parent_when_complete() {
        let mut meta = MetadataMap::new();
        meta.insert(headers::PARENT_RUN_ID, "run-1".parse().unwrap());
        meta.insert(headers::PARENT_JOB_NAMESPACE, "ns".parse().unwrap());
        meta.insert(headers::PARENT_JOB_NAME, "job".parse().unwrap());
        meta.insert(headers::ROOT_PARENT_RUN_ID, "root-1".parse().unwrap());
        meta.insert(
            headers::ROOT_PARENT_JOB_NAMESPACE,
            "root-ns".parse().unwrap(),
        );
        meta.insert(headers::ROOT_PARENT_JOB_NAME, "root-job".parse().unwrap());

        let cx = context_from_metadata(&meta, &config());
        let root = cx.parent_run.unwrap().root.expect("root present");
        assert_eq!(root.run.run_id, "root-1");
        assert_eq!(root.job.namespace, "root-ns");
    }

    #[test]
    fn empty_metadata_yields_empty_context() {
        let cx = context_from_metadata(&MetadataMap::new(), &config());
        assert!(cx.parent_run.is_none());
    }

    #[test]
    fn job_name_hash_is_stable_and_whitespace_insensitive() {
        let a = job_name_from_sql("SELECT id FROM t");
        let b = job_name_from_sql("SELECT id FROM t");
        let c = job_name_from_sql("SELECT   id\nFROM\tt");
        assert_eq!(a, b, "identical SQL -> identical job name");
        assert_eq!(a, c, "whitespace-only differences collapse to one job");
        assert!(a.starts_with("query-"), "name is the query-<hex> form: {a}");
        assert_eq!(a.len(), "query-".len() + 12, "12 hex chars: {a}");
    }

    #[test]
    fn job_name_differs_for_different_sql() {
        assert_ne!(
            job_name_from_sql("SELECT 1"),
            job_name_from_sql("SELECT 2"),
            "different queries -> different jobs"
        );
    }

    #[test]
    fn job_name_header_wins_over_hash() {
        let mut meta = MetadataMap::new();
        meta.insert(headers::JOB_NAME, "dbt.orders".parse().unwrap());
        let name = job_name_from_metadata(&meta, "SELECT id FROM t");
        assert_eq!(name, "dbt.orders", "client-supplied header is used verbatim");
    }

    #[test]
    fn job_name_falls_back_to_hash_when_header_absent_or_blank() {
        let expected = job_name_from_sql("SELECT id FROM t");
        // Absent header.
        assert_eq!(
            job_name_from_metadata(&MetadataMap::new(), "SELECT id FROM t"),
            expected
        );
        // Blank header is ignored.
        let mut meta = MetadataMap::new();
        meta.insert(headers::JOB_NAME, "   ".parse().unwrap());
        assert_eq!(job_name_from_metadata(&meta, "SELECT id FROM t"), expected);
    }

    #[test]
    fn execution_context_mints_fresh_run_parented_to_planning() {
        let planning_run = uuid::Uuid::now_v7();
        let planning = LineageContext {
            run_id: Some(planning_run),
            job_name: Some("query-abc".to_string()),
            sql: Some("SELECT 1".to_string()),
            ..Default::default()
        };

        let cfg = config();
        let exec1 = execution_context(&planning, &cfg);
        let exec2 = execution_context(&planning, &cfg);

        // Two executions -> two distinct run ids, neither equal to the planning id.
        assert_ne!(exec1.run_id, exec2.run_id, "each execution is a distinct run");
        assert_ne!(exec1.run_id, Some(planning_run));
        assert_ne!(exec2.run_id, Some(planning_run));

        // Both parent to the same planning run, under the carried-over job name.
        for exec in [&exec1, &exec2] {
            let parent = exec.parent_run.as_ref().expect("parent facet present");
            assert_eq!(parent.run.run_id, planning_run.to_string());
            assert_eq!(parent.job.name, "query-abc");
            assert_eq!(parent.job.namespace, cfg.job_namespace);
            assert!(parent.root.is_none(), "no orchestrator parent -> no root");
        }

        // Job identity + SQL carry over unchanged.
        assert_eq!(exec1.job_name, planning.job_name);
        assert_eq!(exec1.sql, planning.sql);
    }

    #[test]
    fn execution_context_promotes_orchestrator_parent_to_root() {
        let planning_run = uuid::Uuid::now_v7();
        let cfg = config();
        let orchestrator_parent = ParentRunFacet {
            base: BaseFacet::new(&cfg.producer, "1-0-0/ParentRunFacet.json"),
            run: ParentRun {
                run_id: "airflow-run-1".to_string(),
            },
            job: ParentJob {
                namespace: "airflow".to_string(),
                name: "dag.task".to_string(),
            },
            root: None,
        };
        let planning = LineageContext {
            run_id: Some(planning_run),
            job_name: Some("query-abc".to_string()),
            parent_run: Some(orchestrator_parent),
            ..Default::default()
        };

        let exec = execution_context(&planning, &cfg);
        let parent = exec.parent_run.expect("parent present");
        // Immediate parent is now the statement (planning) run...
        assert_eq!(parent.run.run_id, planning_run.to_string());
        // ...and the orchestrator parent is promoted to root.
        let root = parent.root.expect("root present");
        assert_eq!(root.run.run_id, "airflow-run-1");
        assert_eq!(root.job.name, "dag.task");
    }
}
