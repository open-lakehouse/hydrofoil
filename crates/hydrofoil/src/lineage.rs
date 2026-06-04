//! Hydrofoil's OpenLineage context wiring.
//!
//! Bridges gRPC request metadata (parent-run headers a client sends) into the
//! [`LineageContext`] the OpenLineage planner consumes. See
//! `docs/session-management.md` for the broader session design.
//!
//! The server parses parent-run context per request via [`context_from_metadata`]
//! and attaches it to a request-scoped session
//! (`LakehouseCtx::session_with_lineage`), where [`HydrofoilContextProvider`]
//! reads it back at planning time. Per-statement run-id correlation across
//! separate RPCs remains the deferred session-store work.

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
}
