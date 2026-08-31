//! Storage-independent diagnostic token-provenance values.

use crate::input::{SourceId, TokenListReplayKind};
use crate::source_map::SourceSpan;
use crate::token::{OriginId, Token};
use crate::world::InputRecordId;

const DEFAULT_PROVENANCE_RECORD_LIMIT: usize = 1_048_576;

/// Exact immutable physical source range behind one compact token origin.
///
/// This is a demand-only projection. Hot token and command values retain the
/// [`OriginId`] instead of copying this decoded geometry on every delivery.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OriginSourceRange {
    source: SourceId,
    start: u64,
    end: u64,
}

impl OriginSourceRange {
    pub(crate) const fn new(source: SourceId, start: u64, end: u64) -> Self {
        Self { source, start, end }
    }

    #[must_use]
    pub const fn source(self) -> SourceId {
        self.source
    }

    #[must_use]
    pub const fn start(self) -> u64 {
        self.start
    }

    #[must_use]
    pub const fn end(self) -> u64 {
        self.end
    }
}

/// Optional provenance surfaces selected once for an engine job.
///
/// Source registration and compact token positions are unconditional engine
/// state. This policy controls only consumers which retain additional roots at
/// an output boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProvenanceDemand {
    diagnostics: bool,
    rendered_source: bool,
}

impl ProvenanceDemand {
    /// Ordinary batch execution: diagnostics remain exact, but shipped pages
    /// do not retain rendered-source sidecars.
    pub const DIAGNOSTICS: Self = Self {
        diagnostics: true,
        rendered_source: false,
    };

    /// Editor execution with both diagnostic and rendered-source consumers.
    pub const DIAGNOSTICS_AND_RENDERED_SOURCE: Self = Self {
        diagnostics: true,
        rendered_source: true,
    };

    /// Whether an error consumer may capture diagnostic roots.
    #[must_use]
    pub const fn diagnostics(self) -> bool {
        self.diagnostics
    }

    /// Whether shipout retains node-to-source roots and recipes.
    #[must_use]
    pub const fn rendered_source(self) -> bool {
        self.rendered_source
    }

    /// Returns the same policy with rendered-source consumption enabled.
    #[must_use]
    pub const fn with_rendered_source(self) -> Self {
        Self {
            rendered_source: true,
            ..self
        }
    }
}

impl Default for ProvenanceDemand {
    fn default() -> Self {
        Self::DIAGNOSTICS
    }
}

/// Independent production admission limits for retained provenance.
///
/// Exhaustion degrades only optional provenance to unknown. It never aborts
/// TeX execution or changes artifact bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProvenanceBudgets {
    pub live_atoms: usize,
    pub detached_artifact_recipe_bytes: usize,
}

impl Default for ProvenanceBudgets {
    fn default() -> Self {
        Self {
            live_atoms: DEFAULT_PROVENANCE_RECORD_LIMIT,
            detached_artifact_recipe_bytes: 16 * 1024 * 1024,
        }
    }
}

/// Source coordinate for a token read from an input source.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceOrigin {
    byte_offset: u64,
    source: SourceId,
    input_record: Option<InputRecordId>,
    line: u32,
    column: u32,
}

impl SourceOrigin {
    /// Creates a source-origin coordinate.
    #[must_use]
    pub const fn new(source: SourceId, byte_offset: u64, line: u32, column: u32) -> Self {
        Self {
            byte_offset,
            source,
            input_record: None,
            line,
            column,
        }
    }

    /// Attaches the `World` record that owns the source's path and bytes.
    #[must_use]
    pub const fn with_input_record(mut self, input_record: InputRecordId) -> Self {
        self.input_record = Some(input_record);
        self
    }

    #[must_use]
    pub const fn source(self) -> SourceId {
        self.source
    }

    #[must_use]
    pub const fn input_record(self) -> Option<InputRecordId> {
        self.input_record
    }

    #[must_use]
    pub const fn byte_offset(self) -> u64 {
        self.byte_offset
    }

    #[must_use]
    pub const fn line(self) -> u32 {
        self.line
    }

    #[must_use]
    pub const fn column(self) -> u32 {
        self.column
    }
}

/// Provenance for one live macro invocation frame.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MacroInvocationOrigin {
    definition_operand: u64,
    invocation: OriginId,
    definition_origin: OriginId,
    parent_invocation: OriginId,
}

impl MacroInvocationOrigin {
    #[must_use]
    pub const fn definition_operand(self) -> u64 {
        self.definition_operand
    }

    #[must_use]
    pub const fn invocation(self) -> OriginId {
        self.invocation
    }

    #[must_use]
    pub const fn definition_origin(self) -> OriginId {
        self.definition_origin
    }

    #[must_use]
    pub const fn parent_invocation(self) -> OriginId {
        self.parent_invocation
    }
}

/// Provenance for a token inserted into the input stream by TeX machinery.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InsertedOrigin {
    kind: InsertedOriginKind,
    token: Token,
    parent: OriginId,
}

impl InsertedOrigin {
    /// Creates an inserted-token origin.
    #[must_use]
    pub const fn new(kind: InsertedOriginKind, token: Token, parent: OriginId) -> Self {
        Self {
            kind,
            token,
            parent,
        }
    }

    #[must_use]
    pub const fn kind(self) -> InsertedOriginKind {
        self.kind
    }

    #[must_use]
    pub const fn token(self) -> Token {
        self.token
    }

    #[must_use]
    pub const fn parent(self) -> OriginId {
        self.parent
    }
}

/// The source of an inserted token.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum InsertedOriginKind {
    EndLine,
    Paragraph,
    AfterGroup,
    AfterAssignment,
    NoExpand,
    Unexpanded,
    ExpandAfter,
    Unread,
    TokenListReplay(TokenListReplayKind),
    ErrorRecovery,
}

/// Provenance for a token synthesized from semantic state rather than copied
/// from a source or token list.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SynthesizedOrigin {
    kind: SynthesizedOriginKind,
    parent: OriginId,
}

impl SynthesizedOrigin {
    /// Creates a synthesized-token origin.
    #[must_use]
    pub const fn new(kind: SynthesizedOriginKind, parent: OriginId) -> Self {
        Self { kind, parent }
    }

    #[must_use]
    pub const fn kind(self) -> SynthesizedOriginKind {
        self.kind
    }

    #[must_use]
    pub const fn parent(self) -> OriginId {
        self.parent
    }
}

/// The operation that synthesized a token.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SynthesizedOriginKind {
    Expansion,
    Scanner,
    ValueRendering,
    NoExpand,
    ErrorRecovery,
}

/// Provenance for bootstrap or engine-owned tokens with no source coordinate.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SyntheticOrigin {
    kind: SyntheticOriginKind,
}

impl SyntheticOrigin {
    /// Creates a synthetic/bootstrap origin.
    #[must_use]
    pub const fn new(kind: SyntheticOriginKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(self) -> SyntheticOriginKind {
        self.kind
    }
}

/// The family of a synthetic/bootstrap origin.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SyntheticOriginKind {
    Bootstrap,
    Primitive,
    Format,
    Engine,
    Test,
}

/// The semantic role of a secondary diagnostic location.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RelatedLocationRole {
    Invocation,
    Definition,
    RecoveryFrontier,
    SecondarySpelling,
}

impl RelatedLocationRole {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Invocation => "invoked here",
            Self::Definition => "defined here",
            Self::RecoveryFrontier => "recovery begins here",
            Self::SecondarySpelling => "also consumed here",
        }
    }
}

/// One labeled secondary location captured when a diagnostic is created.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RelatedLocation {
    role: RelatedLocationRole,
    origin: OriginId,
}

impl RelatedLocation {
    #[must_use]
    pub const fn new(role: RelatedLocationRole, origin: OriginId) -> Self {
        Self { role, origin }
    }

    #[must_use]
    pub const fn role(self) -> RelatedLocationRole {
        self.role
    }

    #[must_use]
    pub const fn origin(self) -> OriginId {
        self.origin
    }
}

/// One lazily-resolved token-origin record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OriginRecord {
    /// Reserved record for unknown, bootstrap, or lost provenance.
    UnknownBootstrap,
    Source(SourceOrigin),
    /// A validated source-map range, used by tagged direct/fallback origins.
    SourceSpan(SourceSpan),
    MacroInvocation(MacroInvocationOrigin),
    Inserted(InsertedOrigin),
    Synthesized(SynthesizedOrigin),
    Synthetic(SyntheticOrigin),
}
