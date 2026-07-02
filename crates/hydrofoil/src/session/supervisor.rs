//! Composes independent session extensions (policy, lineage) onto one
//! DataFusion session.
//!
//! Both `datafusion-cedar` and `datafusion-openlineage` attach to a session the
//! same way — by wrapping its `QueryPlanner` and returning a new `SessionState`
//! (`instrument(SessionState) -> SessionState`). Neither knows about the other;
//! each just wraps whatever planner it finds. The supervisor is the one place
//! that knows both exist and folds them on in a deliberate order, so the nesting
//! of their query planners is a property of *this* file, not of either crate.
//!
//! Applied last = outermost planner. To get `lineage → policy → unity` (so a
//! query denied by the policy gate still emits an OpenLineage START+FAIL), the
//! policy augmentor is added before the lineage augmentor. See
//! `create_session_for`.

use datafusion::execution::SessionStateBuilder;
use datafusion::execution::context::SessionState;

/// A single session-extension step: takes the current `SessionState` and returns
/// it wrapped with one more extension. Both `datafusion_cedar` and
/// `datafusion_openlineage` expose `instrument`-style functions of exactly this
/// shape, so an augmentor is usually a thin closure over one of them.
pub type SessionAugmentor = Box<dyn FnOnce(SessionState) -> SessionState>;

/// Builds a base `SessionState`, then folds an ordered list of augmentors onto
/// it — each wrapping the planner the previous one left in place.
pub struct SessionSupervisor {
    base: SessionStateBuilder,
    augmentors: Vec<SessionAugmentor>,
}

impl SessionSupervisor {
    /// Start from a fully-configured base builder (config, catalogs, the innermost
    /// query planner, table factories, optimizer rules).
    pub fn new(base: SessionStateBuilder) -> Self {
        Self {
            base,
            augmentors: Vec::new(),
        }
    }

    /// Add an augmentor. Augmentors are applied in the order added, so the last
    /// one added ends up as the outermost query-planner wrapper.
    pub fn with(mut self, augmentor: SessionAugmentor) -> Self {
        self.augmentors.push(augmentor);
        self
    }

    /// Build the base state and fold every augmentor onto it in order.
    pub fn build(self) -> SessionState {
        let mut state = self.base.build();
        for augmentor in self.augmentors {
            state = augmentor(state);
        }
        state
    }
}
