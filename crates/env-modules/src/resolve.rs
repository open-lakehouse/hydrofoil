//! Pure dependency resolution: selected modules → a resolved, ordered graph.
//!
//! `resolve` is side-effect-free. It expands a user's module selection over the
//! `requires` edges in the registry, detects cycles and unknown ids, and produces
//! a [`ResolvedGraph`] whose `nodes` are topologically ordered (dependencies
//! before dependents — i.e. a valid Compose `depends_on` / startup order). The
//! graph is the resolver's natural working representation; returning it (rather
//! than a flat list) leaves the data shaped for the artifact generator and a
//! future node-diagram visualization at near-zero extra cost.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::model::{Module, ModuleId, ModuleKind};

/// What can go wrong resolving a selection.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResolveError {
    /// A selected (or transitively required) id is not in the registry.
    #[error("unknown module: {0}")]
    UnknownModule(ModuleId),
    /// A dependency cycle was detected involving these modules (sorted).
    #[error("dependency cycle among modules: {0:?}")]
    Cycle(Vec<ModuleId>),
}

/// A directed dependency edge: `from` requires `to` (so `to` starts first).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: ModuleId,
    pub to: ModuleId,
}

/// The resolved environment graph: the full set of modules to run (transitive
/// closure of the selection), topologically ordered, plus the dependency edges.
/// Reusable for artifact generation and visualization.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedGraph {
    /// All modules to run, ordered dependencies-first (valid startup order).
    pub nodes: Vec<Module>,
    /// Dependency edges among `nodes`.
    pub edges: Vec<Edge>,
}

impl ResolvedGraph {
    /// The Docker-backed modules in startup order (those needing the daemon).
    pub fn docker_modules(&self) -> Vec<&Module> {
        self.nodes
            .iter()
            .filter(|m| m.kind.needs_docker())
            .collect()
    }

    /// The uvx sidecar modules in startup order.
    pub fn uvx_modules(&self) -> Vec<&Module> {
        self.nodes
            .iter()
            .filter(|m| matches!(m.kind, ModuleKind::UvxSidecar { .. }))
            .collect()
    }

    /// Whether running this graph requires a Docker daemon.
    pub fn needs_docker(&self) -> bool {
        self.nodes.iter().any(|m| m.kind.needs_docker())
    }
}

/// Resolve a module selection into an ordered graph, using the default registry.
pub fn resolve(selected: &[ModuleId]) -> Result<ResolvedGraph, ResolveError> {
    resolve_with(selected, &crate::model::registry())
}

/// Resolve against an explicit registry (used in tests).
pub fn resolve_with(
    selected: &[ModuleId],
    registry: &[Module],
) -> Result<ResolvedGraph, ResolveError> {
    let by_id: BTreeMap<&str, &Module> =
        registry.iter().map(|m| (m.id.as_str(), m)).collect();

    // Transitive closure of the selection over `requires` (BFS), erroring on any
    // unknown id encountered along the way.
    let mut included: BTreeSet<ModuleId> = BTreeSet::new();
    let mut queue: Vec<ModuleId> = selected.to_vec();
    while let Some(id) = queue.pop() {
        let module = by_id
            .get(id.as_str())
            .ok_or_else(|| ResolveError::UnknownModule(id.clone()))?;
        if included.insert(id.clone()) {
            for dep in &module.requires {
                queue.push(dep.clone());
            }
        }
    }

    // Topological sort (Kahn's algorithm) over the included sub-graph. Working
    // over sorted sets/maps keeps the output deterministic regardless of
    // selection order.
    let edges = collect_edges(&included, &by_id);
    let nodes = topo_sort(&included, &edges)?
        .into_iter()
        .map(|id| (*by_id.get(id.as_str()).unwrap()).clone())
        .collect();

    Ok(ResolvedGraph { nodes, edges })
}

/// Collect the dependency edges (from → to: `from` requires `to`) within the
/// included set, in a deterministic order.
fn collect_edges(
    included: &BTreeSet<ModuleId>,
    by_id: &BTreeMap<&str, &Module>,
) -> Vec<Edge> {
    let mut edges = Vec::new();
    for id in included {
        let module = by_id.get(id.as_str()).unwrap();
        for dep in &module.requires {
            if included.contains(dep) {
                edges.push(Edge {
                    from: id.clone(),
                    to: dep.clone(),
                });
            }
        }
    }
    edges.sort_by(|a, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));
    edges
}

/// Kahn topological sort: dependencies (edge `to`) ordered before dependents
/// (edge `from`). Returns the modules left in a cycle as a [`ResolveError::Cycle`].
fn topo_sort(
    included: &BTreeSet<ModuleId>,
    edges: &[Edge],
) -> Result<Vec<ModuleId>, ResolveError> {
    // in_degree counts unresolved dependencies of each node (its `requires`).
    let mut in_degree: BTreeMap<&ModuleId, usize> =
        included.iter().map(|id| (id, 0usize)).collect();
    // dependents[dep] = nodes that require `dep`, so we can decrement them once
    // `dep` is emitted.
    let mut dependents: BTreeMap<&ModuleId, Vec<&ModuleId>> = BTreeMap::new();
    for edge in edges {
        *in_degree.get_mut(&edge.from).unwrap() += 1;
        dependents.entry(&edge.to).or_default().push(&edge.from);
    }

    // Seed with dependency-free nodes (BTreeSet iteration → deterministic order).
    let mut ready: Vec<&ModuleId> = in_degree
        .iter()
        .filter(|&(_, &d)| d == 0)
        .map(|(&id, _)| id)
        .collect();
    let mut ordered: Vec<ModuleId> = Vec::with_capacity(included.len());
    while let Some(id) = ready.pop() {
        ordered.push(id.clone());
        if let Some(deps) = dependents.get(id) {
            for &dependent in deps {
                let d = in_degree.get_mut(dependent).unwrap();
                *d -= 1;
                if *d == 0 {
                    ready.push(dependent);
                }
            }
        }
    }

    if ordered.len() != included.len() {
        // Whatever wasn't emitted is part of (or downstream of) a cycle.
        let emitted: BTreeSet<&ModuleId> = ordered.iter().collect();
        let mut stuck: Vec<ModuleId> = included
            .iter()
            .filter(|id| !emitted.contains(id))
            .cloned()
            .collect();
        stuck.sort();
        return Err(ResolveError::Cycle(stuck));
    }
    Ok(ordered)
}
