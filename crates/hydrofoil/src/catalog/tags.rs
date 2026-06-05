//! The resource/catalog classification PIP: deriving a table's tags/taints and
//! its reader/writer ACLs from Unity Catalog metadata.
//!
//! Unity Catalog (in this fork) has **no tags API** — the `Table` model exposes
//! only `owner`, a `properties` string map, a `comment`, and `columns` (each
//! with a `comment`). So v1 derives classification *by convention* from those
//! fields, behind the [`TagProvider`] trait. A future backend can call an
//! external classification service instead — same trait, async already. See
//! `docs/adr/0007-fact-gathering-pips.md`.
//!
//! The neutral [`TableFacts`](datafusion_cedar::TableFacts) these produce cross
//! into the policy layer; this module is the only place a UC `Table` is read for
//! facts, keeping `datafusion-cedar` free of any catalog dependency.

use std::collections::BTreeSet;

use datafusion::error::DataFusionError;
use datafusion_cedar::CatalogFactSink;
use unitycatalog_common::models::tables::v1::Table;

/// gRPC/property convention keys.
mod keys {
    /// Table-level tags: `properties["tags"] = "pii,regulated"`.
    pub const TABLE_TAGS: &[&str] = &["tags", "classification"];
    /// Column-level tags: `properties["tag.<column>"] = "pii,regulated"`.
    pub const COLUMN_TAG_PREFIX: &str = "tag.";
    /// Reader/writer ACLs: `properties["readers"|"writers"] = "grp_a,grp_b"`.
    pub const READERS: &str = "readers";
    pub const WRITERS: &str = "writers";
}

/// A `SessionConfig` extension carrying the per-session [`CatalogFactSink`].
///
/// Set on the session config before the Unity resolver captures the task
/// context, so `build_delta` (deep in catalog resolution) and the policy layer
/// share one sink instance. Mirrors `PrincipalExt` / `AgentContextExt`.
#[derive(Debug, Clone, Default)]
pub struct CatalogFactSinkExt(pub CatalogFactSink);

/// The classification of one table: its table-level tags and per-column tags.
#[derive(Debug, Clone, Default)]
pub struct TableClassification {
    pub table_tags: BTreeSet<String>,
    pub column_tags: std::collections::HashMap<String, BTreeSet<String>>,
}

/// Source of a table's classification tags/taints — the seam for sourcing
/// `pii`/`regulated`/… classifications. v1 reads UC conventions; a future impl
/// can call an external classification service. Receives the already-fetched
/// `&Table` (catalog resolution has it in hand; re-fetching would be wasteful).
#[async_trait::async_trait]
pub trait TagProvider: Send + Sync + std::fmt::Debug {
    async fn classify(&self, table: &Table) -> Result<TableClassification, DataFusionError>;
}

/// v1 [`TagProvider`]: derives tags by convention from UC `Table` metadata.
///
/// - table tags from `properties["tags"]` / `properties["classification"]`,
/// - column tags from `properties["tag.<column>"]`,
/// - column tags from a `[tags: …]` marker in a column's `comment`.
///
/// Tags are comma-split, trimmed, and lower-cased.
#[derive(Debug, Default)]
pub struct ConventionTagProvider;

#[async_trait::async_trait]
impl TagProvider for ConventionTagProvider {
    async fn classify(&self, table: &Table) -> Result<TableClassification, DataFusionError> {
        let mut out = TableClassification::default();

        for key in keys::TABLE_TAGS {
            if let Some(v) = table.properties.get(*key) {
                out.table_tags.extend(split_tags(v));
            }
        }
        for (k, v) in &table.properties {
            if let Some(col) = k.strip_prefix(keys::COLUMN_TAG_PREFIX) {
                out.column_tags
                    .entry(col.to_string())
                    .or_default()
                    .extend(split_tags(v));
            }
        }
        for col in &table.columns {
            if let Some(comment) = &col.comment {
                let tags = parse_comment_tags(comment);
                if !tags.is_empty() {
                    out.column_tags
                        .entry(col.name.clone())
                        .or_default()
                        .extend(tags);
                }
            }
        }
        Ok(out)
    }
}

/// Owner + reader/writer ACLs derived from UC `Table` metadata, kept beside the
/// other `Table`-reading conventions. Owner is `Table.owner`; readers/writers
/// come from `properties["readers"|"writers"]`.
pub fn table_acl_facts(table: &Table) -> (Option<String>, BTreeSet<String>, BTreeSet<String>) {
    let readers = table
        .properties
        .get(keys::READERS)
        .map(|v| split_tags(v).collect())
        .unwrap_or_default();
    let writers = table
        .properties
        .get(keys::WRITERS)
        .map(|v| split_tags(v).collect())
        .unwrap_or_default();
    (table.owner.clone(), readers, writers)
}

/// Split a comma-separated value into trimmed, lower-cased, non-empty tokens.
fn split_tags(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(',')
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| !t.is_empty())
}

/// Parse a `[tags: pii, regulated]` marker out of a column comment; empty if
/// none.
fn parse_comment_tags(comment: &str) -> Vec<String> {
    let Some(start) = comment.find("[tags:") else {
        return Vec::new();
    };
    let inner_start = start + "[tags:".len();
    let Some(rel_end) = comment[inner_start..].find(']') else {
        return Vec::new();
    };
    split_tags(&comment[inner_start..inner_start + rel_end]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use unitycatalog_common::models::tables::v1::Column;

    fn col(name: &str, comment: Option<&str>) -> Column {
        Column {
            name: name.to_string(),
            comment: comment.map(|c| c.to_string()),
            ..Default::default()
        }
    }

    fn tags(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn classifies_table_and_column_tags_from_properties_and_comments() {
        let table = Table {
            properties: std::collections::HashMap::from([
                ("tags".to_string(), "Regulated, internal".to_string()),
                ("tag.ssn".to_string(), "pii,sensitive".to_string()),
            ]),
            columns: vec![
                col("ssn", Some("SSN [tags: pii]")),
                col("email", Some("contact [tags: pii, contact_info]")),
                col("id", None),
            ],
            ..Default::default()
        };

        let c = ConventionTagProvider.classify(&table).await.unwrap();
        // Table tags are lower-cased and split.
        assert_eq!(c.table_tags, tags(&["internal", "regulated"]));
        // ssn merges the property tags and the comment tag.
        assert_eq!(
            c.column_tags.get("ssn").unwrap(),
            &tags(&["pii", "sensitive"])
        );
        // email comes only from the comment marker.
        assert_eq!(
            c.column_tags.get("email").unwrap(),
            &tags(&["contact_info", "pii"])
        );
        // An untagged column is absent.
        assert!(!c.column_tags.contains_key("id"));
    }

    #[test]
    fn acl_facts_read_owner_and_readers_writers() {
        let table = Table {
            owner: Some("User::\"alice\"".to_string()),
            properties: std::collections::HashMap::from([
                (
                    "readers".to_string(),
                    "readers,privileged_readers".to_string(),
                ),
                ("writers".to_string(), "lakehouse_admins".to_string()),
            ]),
            ..Default::default()
        };
        let (owner, readers, writers) = table_acl_facts(&table);
        assert_eq!(owner.as_deref(), Some("User::\"alice\""));
        assert_eq!(readers, tags(&["privileged_readers", "readers"]));
        assert_eq!(writers, tags(&["lakehouse_admins"]));
    }

    #[tokio::test]
    async fn empty_metadata_yields_empty_classification() {
        let c = ConventionTagProvider
            .classify(&Table::default())
            .await
            .unwrap();
        assert!(c.table_tags.is_empty());
        assert!(c.column_tags.is_empty());
    }
}
