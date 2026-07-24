//! Borrow-scoped host capabilities.

use std::collections::BTreeMap;

use crate::SourceRegistration;

/// The executor facts observed by TeX's mode predicates.
///
/// This is a copy-only query result, never persistent command state.  The
/// executor refreshes it for each bounded command operation from its mode
/// nest; command processing merely consumes the answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConditionalState {
    mode: ConditionalMode,
    inner: bool,
}

impl ConditionalState {
    #[must_use]
    pub const fn new(mode: ConditionalMode, inner: bool) -> Self {
        Self { mode, inner }
    }

    #[must_use]
    pub const fn mode(self) -> ConditionalMode {
        self.mode
    }

    #[must_use]
    pub const fn is_inner(self) -> bool {
        self.inner
    }
}

/// TeX's three mode families, projected by the executor-owned mode nest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConditionalMode {
    Vertical,
    Horizontal,
    Math,
}

/// Opaque capability set installed by the executor for one bounded operation.
///
/// The fields remain private so host access can only be introduced as typed
/// command-core operations. This value is intentionally neither serializable
/// nor cloneable.
#[derive(Debug)]
pub struct CommandHostCapabilities {
    input: BTreeMap<String, SourceRegistration>,
    job_name: String,
    conditional_state: ConditionalState,
}

impl Default for CommandHostCapabilities {
    fn default() -> Self {
        Self {
            input: BTreeMap::new(),
            job_name: String::new(),
            // A processor outside main control observes TeX's initial outer
            // vertical mode. Real execution replaces this per operation.
            conditional_state: ConditionalState::new(ConditionalMode::Vertical, false),
        }
    }
}

impl CommandHostCapabilities {
    /// Installs immutable backing for one logical `\input` request.
    ///
    /// Acquisition is complete before this capability is constructed.  The
    /// command machine can therefore request only retained bytes and never
    /// opens files itself.
    pub fn register_input(&mut self, name: impl Into<String>, source: SourceRegistration) {
        self.input.insert(name.into(), source);
    }

    /// Sets the immutable job name presented by `\jobname` for this command
    /// operation.
    pub fn set_job_name(&mut self, name: impl Into<String>) {
        self.job_name = name.into();
    }

    /// Installs the current executor-owned mode query result for this command
    /// operation. It is deliberately capability state rather than a field of
    /// `CommandState`, so snapshots never duplicate the mode nest.
    pub fn set_conditional_state(&mut self, state: ConditionalState) {
        self.conditional_state = state;
    }
}

/// A non-owning host-capability boundary for one command-processor operation.
///
/// The mutable borrow makes the capability scope explicit and prevents the
/// context from entering owned command state, snapshots, or formats.
#[derive(Debug)]
pub struct CommandHostContext<'a> {
    _capabilities: &'a mut CommandHostCapabilities,
}

impl<'a> CommandHostContext<'a> {
    /// Borrows the capabilities installed for one bounded operation.
    #[must_use]
    pub fn new(capabilities: &'a mut CommandHostCapabilities) -> Self {
        Self {
            _capabilities: capabilities,
        }
    }

    pub(crate) fn input(&self, name: &str) -> Option<SourceRegistration> {
        self._capabilities.input.get(name).cloned()
    }

    pub(crate) fn job_name(&self) -> &str {
        &self._capabilities.job_name
    }

    pub(crate) const fn conditional_state(&self) -> ConditionalState {
        self._capabilities.conditional_state
    }
}
