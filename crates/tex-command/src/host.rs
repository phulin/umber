//! Borrow-scoped host capabilities.

/// Opaque capability set installed by the executor for one bounded operation.
///
/// The fields remain private so host access can only be introduced as typed
/// command-core operations. This value is intentionally neither serializable
/// nor cloneable.
#[derive(Debug, Default)]
pub struct CommandHostCapabilities {
    _private: (),
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
}
