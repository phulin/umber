//! Ordinary expanded-command delivery.

use crate::profile::CommandProfile;

/// Future-relevant expansion facts.
///
/// Per-request fuel is deliberately absent: it is call-local and recreated
/// when an executor step is retried. Caches and profiling likewise belong to
/// [`crate::CommandRuntime`].
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct ExpansionState {
    pub(crate) cumulative_expansions: u64,
    pub(crate) next_resource_resolution: u64,
    pub(crate) pending_diagnostics: Vec<u64>,
    pub(crate) observed_dependencies: Vec<u64>,
    pub(crate) semantic_barriers: Vec<u64>,
    pub(crate) profile: CommandProfile,
}
