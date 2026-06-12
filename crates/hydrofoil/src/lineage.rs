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
//! whitespace-normalized SQL (`query-<12-hex>`). The job *namespace* defaults to
//! the engine's configured `OpenLineageConfig::job_namespace` and may be
//! overridden per request with [`headers::JOB_NAMESPACE`].
//!
//! ## Client-forwarded job metadata
//!
//! Mirroring the OpenLineage Spark integration's `spark.openlineage.job.*`
//! properties, a client can attach business metadata to the emitted job
//! (parsed by [`job_facets_from_metadata`]; malformed entries are skipped, never
//! failing the query). See `docs/adr/0012-client-forwarded-lineage-metadata.md`.
//!
//! | header | facet | grammar |
//! |---|---|---|
//! | `x-openlineage-job-namespace` | job namespace | plain string |
//! | `x-openlineage-job-description` | `documentation` | free text |
//! | `x-openlineage-job-tags` | `tags` | `key[:value[:source]]` entries, `;`-separated |
//! | `x-openlineage-job-owners` | `ownership` | `type:name` entries, `;`-separated |
//!
//! In addition, the per-query governance context — the resolved principal and
//! any `x-hydrofoil-agent-*` headers (see [`crate::agent`]) — is folded into a
//! custom `hydrofoil` *run* facet by [`hydrofoil_run_facet`], so lineage carries
//! who ran the query and on whose behalf/why.

use datafusion::execution::context::SessionState;
use datafusion_cedar::PrincipalIdentity;
use datafusion_open_lineage::config::OpenLineageConfig;
use datafusion_open_lineage::context::{LineageContext, LineageContextProvider};
use datafusion_open_lineage::facets::{
    BaseFacet, DocumentationJobFacet, Owner, OwnershipJobFacet, ParentJob, ParentRun,
    ParentRunFacet, RootParent, TagsJobFacet, TagsJobFacetFields,
};
use serde_json::{Map, Value, json};
use tonic::metadata::MetadataMap;

use crate::agent::AgentContext;

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

    /// Optional job *namespace* override for this statement (mirrors
    /// `spark.openlineage.namespace`). Absent, the engine's configured
    /// `OpenLineageConfig::job_namespace` applies.
    pub const JOB_NAMESPACE: &str = "x-openlineage-job-namespace";

    /// Optional free-text job description -> the `documentation` job facet.
    pub const JOB_DESCRIPTION: &str = "x-openlineage-job-description";

    /// Optional job tags -> the `tags` job facet. Semicolon-separated entries,
    /// each `key`, `key:value`, or `key:value:source` (mirrors
    /// `spark.openlineage.job.tags`).
    pub const JOB_TAGS: &str = "x-openlineage-job-tags";

    /// Optional job owners -> the `ownership` job facet. Semicolon-separated
    /// `type:name` entries (mirrors `spark.openlineage.job.owners.<type>`).
    pub const JOB_OWNERS: &str = "x-openlineage-job-owners";
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
        job_namespace: get(headers::JOB_NAMESPACE).filter(|ns| !ns.trim().is_empty()),
        job_facets: job_facets_from_metadata(meta, config),
        ..Default::default()
    }
}

/// Parse client-supplied job metadata headers into OpenLineage *job facets*
/// (see the module docs for the header grammar). Returns the facet map merged
/// into the emitted event's job facets; malformed entries are skipped — bad
/// metadata must never fail the query.
pub fn job_facets_from_metadata(meta: &MetadataMap, config: &OpenLineageConfig) -> Map<String, Value> {
    let get = |key: &str| {
        meta.get(key)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };

    let mut facets = Map::new();

    if let Some(description) = get(headers::JOB_DESCRIPTION) {
        let facet = DocumentationJobFacet {
            base: BaseFacet::new(&config.producer, "1-1-0/DocumentationJobFacet.json"),
            description: description.to_string(),
        };
        if let Ok(value) = serde_json::to_value(facet) {
            facets.insert("documentation".to_string(), value);
        }
    }

    if let Some(raw) = get(headers::JOB_TAGS) {
        // `key`, `key:value`, or `key:value:source`, semicolon-separated.
        let tags: Vec<TagsJobFacetFields> = raw
            .split(';')
            .filter_map(|entry| {
                let mut parts = entry.splitn(3, ':').map(str::trim);
                let key = parts.next().filter(|k| !k.is_empty())?;
                Some(TagsJobFacetFields {
                    key: key.to_string(),
                    value: parts.next().filter(|v| !v.is_empty()).map(str::to_string),
                    source: parts.next().filter(|s| !s.is_empty()).map(str::to_string),
                })
            })
            .collect();
        if !tags.is_empty() {
            let facet = TagsJobFacet {
                base: BaseFacet::new(&config.producer, "1-0-0/TagsJobFacet.json"),
                tags,
            };
            if let Ok(value) = serde_json::to_value(facet) {
                facets.insert("tags".to_string(), value);
            }
        }
    }

    if let Some(raw) = get(headers::JOB_OWNERS) {
        // `type:name` entries, semicolon-separated (a bare `name` gets no type).
        let owners: Vec<Owner> = raw
            .split(';')
            .filter_map(|entry| {
                let mut parts = entry.splitn(2, ':').map(str::trim);
                let first = parts.next().filter(|p| !p.is_empty())?;
                Some(match parts.next().filter(|n| !n.is_empty()) {
                    Some(name) => Owner {
                        name: name.to_string(),
                        type_: Some(first.to_string()),
                    },
                    None => Owner {
                        name: first.to_string(),
                        type_: None,
                    },
                })
            })
            .collect();
        if !owners.is_empty() {
            let facet = OwnershipJobFacet {
                base: BaseFacet::new(&config.producer, "1-0-1/OwnershipJobFacet.json"),
                owners,
            };
            if let Ok(value) = serde_json::to_value(facet) {
                facets.insert("ownership".to_string(), value);
            }
        }
    }

    facets
}

/// Build the custom `hydrofoil` *run* facet carrying the per-query governance
/// context: the resolved principal and any agent context
/// (`x-hydrofoil-agent-*`, see [`crate::agent`]). Returns `None` for the
/// anonymous principal with no agent context — the facet should mark *known*
/// provenance, not stamp noise on every event.
pub fn hydrofoil_run_facet(
    principal: Option<&PrincipalIdentity>,
    agent: Option<&AgentContext>,
    config: &OpenLineageConfig,
) -> Option<Value> {
    let principal = principal
        .map(|p| p.uid.to_string())
        .filter(|uid| uid != crate::identity::DEFAULT_PRINCIPAL);
    if principal.is_none() && agent.is_none() {
        return None;
    }

    // A custom facet, so the schema lives under our producer, not openlineage.io.
    let base = BaseFacet {
        producer: config.producer.clone(),
        schema_url: format!("{}/spec/facets/1-0-0/HydrofoilRunFacet.json", config.producer),
    };

    let mut facet = serde_json::to_value(base).ok()?;
    let obj = facet.as_object_mut()?;
    if let Some(principal) = principal {
        obj.insert("principal".to_string(), json!(principal));
    }
    if let Some(agent) = agent {
        obj.insert(
            "agent".to_string(),
            json!({
                "id": agent.agent_id,
                "session": agent.agent_session,
                "task": agent.task,
                "purpose": agent.purpose,
            }),
        );
    }
    Some(facet)
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
    fn parses_namespace_override_and_job_facets() {
        let mut meta = MetadataMap::new();
        meta.insert(headers::JOB_NAMESPACE, "demo-pipeline".parse().unwrap());
        meta.insert(
            headers::JOB_DESCRIPTION,
            "Daily rollup.".parse().unwrap(),
        );
        meta.insert(
            headers::JOB_TAGS,
            "tier:bronze;adhoc;domain:ops:catalog".parse().unwrap(),
        );
        meta.insert(
            headers::JOB_OWNERS,
            "team:data-platform;user:robert.pack".parse().unwrap(),
        );

        let cx = context_from_metadata(&meta, &config());
        assert_eq!(cx.job_namespace.as_deref(), Some("demo-pipeline"));

        let doc = &cx.job_facets["documentation"];
        assert_eq!(doc["description"], "Daily rollup.");
        assert!(doc["_schemaURL"].as_str().unwrap().contains("Documentation"));

        let tags = cx.job_facets["tags"]["tags"].as_array().unwrap();
        assert_eq!(tags.len(), 3);
        assert_eq!(tags[0]["key"], "tier");
        assert_eq!(tags[0]["value"], "bronze");
        assert_eq!(tags[1]["key"], "adhoc");
        assert!(tags[1].get("value").is_none());
        assert_eq!(tags[2]["source"], "catalog");

        let owners = cx.job_facets["ownership"]["owners"].as_array().unwrap();
        assert_eq!(owners[0]["type"], "team");
        assert_eq!(owners[0]["name"], "data-platform");
        assert_eq!(owners[1]["name"], "robert.pack");
    }

    #[test]
    fn malformed_metadata_entries_are_skipped_not_fatal() {
        let mut meta = MetadataMap::new();
        // Empty namespace -> ignored; tags of separators only -> no facet.
        meta.insert(headers::JOB_NAMESPACE, "  ".parse().unwrap());
        meta.insert(headers::JOB_TAGS, ";;:;".parse().unwrap());
        meta.insert(headers::JOB_OWNERS, " ; ".parse().unwrap());

        let cx = context_from_metadata(&meta, &config());
        assert!(cx.job_namespace.is_none());
        assert!(!cx.job_facets.contains_key("tags"));
        assert!(!cx.job_facets.contains_key("ownership"));
        assert!(!cx.job_facets.contains_key("documentation"));
    }

    #[test]
    fn hydrofoil_run_facet_carries_principal_and_agent() {
        use std::str::FromStr as _;
        let principal = PrincipalIdentity::new(
            datafusion_cedar::EntityUid::from_str("User::\"alice\"").unwrap(),
        );
        let agent = AgentContext {
            agent_id: Some("assistant-7".into()),
            task: Some("verify-volume".into()),
            ..Default::default()
        };

        let facet = hydrofoil_run_facet(Some(&principal), Some(&agent), &config())
            .expect("facet present");
        assert_eq!(facet["principal"], "User::\"alice\"");
        assert_eq!(facet["agent"]["id"], "assistant-7");
        assert_eq!(facet["agent"]["task"], "verify-volume");
        assert!(facet["_producer"].is_string());
    }

    #[test]
    fn hydrofoil_run_facet_absent_for_anonymous_without_agent() {
        use std::str::FromStr as _;
        let anonymous = PrincipalIdentity::new(
            datafusion_cedar::EntityUid::from_str(crate::identity::DEFAULT_PRINCIPAL).unwrap(),
        );
        assert!(hydrofoil_run_facet(Some(&anonymous), None, &config()).is_none());
        assert!(hydrofoil_run_facet(None, None, &config()).is_none());

        // ...but an anonymous principal WITH agent context still yields a facet
        // (the agent is the provenance worth recording).
        let agent = AgentContext {
            agent_id: Some("assistant-7".into()),
            ..Default::default()
        };
        let facet = hydrofoil_run_facet(Some(&anonymous), Some(&agent), &config()).unwrap();
        assert!(facet.get("principal").is_none());
        assert_eq!(facet["agent"]["id"], "assistant-7");
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
