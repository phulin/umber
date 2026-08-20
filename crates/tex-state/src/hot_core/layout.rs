//! Packed semantic values carried by the canonical hot core.
//!
//! These are runtime-only values. They deliberately derive no serialization,
//! own no `Arc` or `Weak`, and remain outside format and continuation DTOs.

use crate::token::{OriginId, TokenWord};

use super::arena::{ChunkOwner, RegionCoordinate, RegionSpan};

/// A compact direct-source or provenance-run coordinate.
///
/// Its encoding is the existing exact `OriginId` domain: zero is unknown,
/// positive low-half values are direct logical source positions, and high-half
/// values address compact provenance records. Ownership remains at chunk or
/// accepted-generation granularity.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct SourceCoordinate(OriginId);

impl SourceCoordinate {
    pub(crate) const UNKNOWN: Self = Self(OriginId::UNKNOWN);

    pub(crate) const fn from_origin(origin: OriginId) -> Self {
        Self(origin)
    }

    pub(crate) const fn origin(self) -> OriginId {
        self.0
    }
}

/// A typed token-only half-open span within one arena chunk.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TokenSpan(RegionSpan<TokenWord>);

impl TokenSpan {
    pub(in crate::hot_core) const fn from_region(span: RegionSpan<TokenWord>) -> Self {
        Self(span)
    }

    pub(in crate::hot_core) const fn region(self) -> RegionSpan<TokenWord> {
        self.0
    }

    pub(crate) const fn owner(self) -> ChunkOwner {
        self.0.owner()
    }

    pub(crate) const fn start(self) -> u32 {
        self.0.start()
    }

    pub(crate) const fn len(self) -> u32 {
        self.0.len()
    }

    pub(crate) const fn is_empty(self) -> bool {
        self.0.is_empty()
    }
}

/// Canonical classification of one compact input frame.
///
/// Token-list values through `Write` retain tex.web §307's exact
/// `token_type` codes. e-TeX's `EveryEof`, source input, and Umber-owned replay
/// use values outside that closed TeX82 range.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum InputFrameKind {
    Parameter = 0,
    AlignmentUTemplate = 1,
    AlignmentVTemplate = 2,
    BackedUp = 3,
    Inserted = 4,
    Macro = 5,
    OutputRoutine = 6,
    EveryPar = 7,
    EveryMath = 8,
    EveryDisplay = 9,
    EveryHBox = 10,
    EveryVBox = 11,
    EveryJob = 12,
    EveryCr = 13,
    Mark = 14,
    Write = 15,
    EveryEof = 16,
    Source = 17,
    UmberReplay = 18,
}

/// Orthogonal delivery and retirement flags for a compact input frame.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct InputFrameFlags(u8);

impl InputFrameFlags {
    pub const EXPAND: Self = Self(1 << 0);
    pub const SUPPRESS_EXPANDABLE_CONTROL_SEQUENCE: Self = Self(1 << 1);
    pub const STOP_AT_END: Self = Self(1 << 2);
    pub const RETAIN_AT_END: Self = Self(1 << 3);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// One copy-only input cursor over a chunk-owned token span.
///
/// `start`, `current`, and `limit` are absolute offsets within `owner`.
/// `auxiliary` is interpreted by `kind` (for example a source id, macro
/// activation, or argument slot). The command-input migration owns those
/// interpretations; this layout value cannot itself deliver a token.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct InputFrame {
    owner: ChunkOwner,
    start: u32,
    current: u32,
    limit: u32,
    auxiliary: u32,
    trace: SourceCoordinate,
    kind: InputFrameKind,
    flags: InputFrameFlags,
    reserved: u16,
}

impl InputFrame {
    pub(crate) fn runtime(
        identity: u64,
        len: u32,
        kind: InputFrameKind,
        flags: InputFrameFlags,
        auxiliary: u32,
    ) -> Self {
        Self {
            owner: ChunkOwner::runtime_input(identity),
            start: 0,
            current: 0,
            limit: len,
            auxiliary,
            trace: SourceCoordinate::UNKNOWN,
            kind,
            flags,
            reserved: 0,
        }
    }

    pub(crate) fn new(
        span: TokenSpan,
        kind: InputFrameKind,
        flags: InputFrameFlags,
        auxiliary: u32,
        trace: SourceCoordinate,
    ) -> Self {
        let limit = span
            .start()
            .checked_add(span.len())
            .expect("validated arena span end fits u32");
        Self {
            owner: span.owner(),
            start: span.start(),
            current: span.start(),
            limit,
            auxiliary,
            trace,
            kind,
            flags,
            reserved: 0,
        }
    }

    pub(crate) const fn kind(self) -> InputFrameKind {
        self.kind
    }

    pub(crate) const fn flags(self) -> InputFrameFlags {
        self.flags
    }

    pub(crate) const fn auxiliary(self) -> u32 {
        self.auxiliary
    }

    pub(crate) const fn trace(self) -> SourceCoordinate {
        self.trace
    }

    pub(crate) const fn position(self) -> u32 {
        self.current - self.start
    }

    pub(crate) const fn len(self) -> u32 {
        self.limit - self.start
    }

    pub(crate) const fn is_exhausted(self) -> bool {
        self.current == self.limit
    }

    pub(crate) const fn complete_span(self) -> TokenSpan {
        TokenSpan::from_region(self.owner.span(self.start, self.limit - self.start))
    }

    pub(crate) const fn remaining_span(self) -> TokenSpan {
        TokenSpan::from_region(self.owner.span(self.current, self.limit - self.current))
    }

    /// Advances one offset without validating the owner again.
    ///
    /// A caller admits `complete_span` or `remaining_span` once through the
    /// arena and then uses this cursor while that borrow remains live.
    pub(crate) fn next_coordinate(&mut self) -> Option<RegionCoordinate<TokenWord>> {
        if self.is_exhausted() {
            return None;
        }
        let coordinate = self.owner.coordinate(self.current);
        self.current += 1;
        Some(coordinate)
    }

    pub(crate) const fn runtime_identity(self) -> u64 {
        self.owner.runtime_input_identity()
    }

    pub(crate) fn add_flags(&mut self, flags: InputFrameFlags) {
        self.flags = self.flags.union(flags);
    }

    pub(crate) fn extend_limit(&mut self, additional: u32) -> Option<()> {
        self.limit = self.limit.checked_add(additional)?;
        Some(())
    }
}

const _: () = assert!(core::mem::size_of::<SourceCoordinate>() == 4);
const _: () = assert!(core::mem::size_of::<TokenSpan>() == 24);
const _: () = assert!(core::mem::size_of::<InputFrame>() == 40);
