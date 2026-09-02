//! Fixed-size live input-frame coordinates shared with `tex-command`.
//!
//! The frame stores only scalar cursors and classification bits. Token and
//! source owners remain in the command input stack, so this value can be
//! copied into bounded operation marks without retaining any payload.

/// Semantic class of one input frame.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum InputFrameKind {
    Source,
    Parameter,
    AlignmentUTemplate,
    AlignmentVTemplate,
    BackedUp,
    Inserted,
    Macro,
    OutputRoutine,
    EveryPar,
    EveryMath,
    EveryDisplay,
    EveryHBox,
    EveryVBox,
    EveryJob,
    EveryCr,
    EveryEof,
    Mark,
    Write,
    UmberReplay,
}

/// Orthogonal delivery/retirement flags for a compact input frame.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct InputFrameFlags(u8);

impl InputFrameFlags {
    pub const SUPPRESS_EXPANDABLE_CONTROL_SEQUENCE: Self = Self(1 << 0);
    pub const STOP_AT_END: Self = Self(1 << 1);
    pub const RETAIN_AT_END: Self = Self(1 << 2);

    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// Host-classified semantic role of one external input source.
///
/// This is immutable descriptive metadata. The command and execution layers
/// decide how a role affects their policies; the state layer only carries the
/// role beside the source identity through input replay and rollback.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum SourceRole {
    RootDocument,
    UserDocumentInclude,
    ProjectPackageClass,
    DistributionPackageClass,
    GeneratedInput,
    FormatInitialization,
}

/// Compact external-source identity and its resolved semantic role.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceContext {
    source: crate::SourceId,
    role: SourceRole,
}

impl SourceContext {
    #[must_use]
    pub const fn new(source: crate::SourceId, role: SourceRole) -> Self {
        Self { source, role }
    }

    #[must_use]
    pub const fn source(self) -> crate::SourceId {
        self.source
    }

    #[must_use]
    pub const fn role(self) -> SourceRole {
        self.role
    }
}

/// Copy-only input frame coordinate.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InputFrame {
    identity: u64,
    position: u32,
    limit: u32,
    source: Option<SourceContext>,
    kind: InputFrameKind,
    flags: InputFrameFlags,
}

impl InputFrame {
    #[must_use]
    pub const fn source(identity: u64, source: SourceContext) -> Self {
        Self {
            identity,
            position: 0,
            limit: u32::MAX,
            source: Some(source),
            kind: InputFrameKind::Source,
            flags: InputFrameFlags::empty(),
        }
    }

    #[must_use]
    pub fn tokens(identity: u64, len: usize, kind: InputFrameKind, flags: InputFrameFlags) -> Self {
        Self {
            identity,
            position: 0,
            limit: u32::try_from(len).expect("input frame length exceeds u32"),
            source: None,
            kind,
            flags,
        }
    }

    #[must_use]
    pub const fn identity(self) -> u64 {
        self.identity
    }

    #[must_use]
    pub const fn position(self) -> u32 {
        self.position
    }

    #[must_use]
    pub const fn limit(self) -> u32 {
        self.limit
    }

    /// Swaps only the hot delivery position with one ordered undo value.
    ///
    /// Input identity, source, limit, kind, and flags are immutable for a
    /// stable source row and therefore do not belong in its lexical history.
    pub const fn swap_position(&mut self, position: &mut u32) {
        core::mem::swap(&mut self.position, position);
    }

    #[must_use]
    pub const fn kind(self) -> InputFrameKind {
        self.kind
    }

    #[must_use]
    pub const fn flags(self) -> InputFrameFlags {
        self.flags
    }

    #[must_use]
    pub const fn source_id(self) -> Option<crate::SourceId> {
        match self.source {
            Some(source) => Some(source.source()),
            None => None,
        }
    }

    #[must_use]
    pub const fn source_context(self) -> Option<SourceContext> {
        self.source
    }

    /// Installs the external source context active at this frame.
    ///
    /// For file and `\scantokens` source frames this is their own source;
    /// terminal/read frames and token frames inherit the enclosing external
    /// source. The scalar is immutable after admission and is not a backing
    /// owner, cache, or provenance graph.
    pub const fn set_source_context(&mut self, source: Option<SourceContext>) {
        self.source = source;
    }

    /// Advances once and returns the position that was consumed.
    pub const fn advance(&mut self) -> Option<u32> {
        if self.position >= self.limit {
            return None;
        }
        let consumed = self.position;
        self.position += 1;
        Some(consumed)
    }

    /// Advances a position whose resident storage read has already proved it
    /// is below this frame's admitted limit.
    ///
    /// Token owners validate the frame length when the row is admitted. A
    /// successful safe storage read at the current position is therefore the
    /// delivery-time proof which makes a second bounds branch redundant.
    pub const fn advance_resident(&mut self) -> u32 {
        debug_assert!(self.position < self.limit);
        let consumed = self.position;
        self.position += 1;
        consumed
    }

    /// Extends a token frame after tokens are prepended before first delivery.
    pub const fn extend_limit(&mut self, additional: u32) -> Option<()> {
        let Some(limit) = self.limit.checked_add(additional) else {
            return None;
        };
        self.limit = limit;
        Some(())
    }

    pub const fn add_flags(&mut self, flags: InputFrameFlags) {
        self.flags = self.flags.union(flags);
    }
}
