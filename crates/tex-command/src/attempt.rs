//! Command-lifetime scratch and explicit-root promotion.
//!
//! Values in this module are coordinates, never owners. One [`AttemptArena`]
//! owns all backing storage. It is clear at every command boundary: a
//! command's rows are released when the command settles, and a synchronous
//! scanner that fails truncates its own suffix to a fixed-size mark.

use core::marker::PhantomData;
use core::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

use tex_state::glue::GlueSpec;
use tex_state::provenance::OriginRecord;
use tex_state::token::{TokenWord, TracedTokenWord};
use tex_state::{
    DefinitionBuildError, DefinitionBuilder, GlueId, PromotionError, ProvenanceId, TokenListId,
    Universe,
};

mod token_lane;

use token_lane::{TokenLane, TokenLaneView, TokenSink};

#[cfg(test)]
#[path = "attempt/tests.rs"]
mod tests;

static NEXT_ATTEMPT_KEY: AtomicU64 = AtomicU64::new(1);
static NEXT_ATTEMPT_SERIAL: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AttemptKey(NonZeroU64);

impl AttemptKey {
    fn fresh() -> Self {
        loop {
            if let Some(key) = NonZeroU64::new(NEXT_ATTEMPT_KEY.fetch_add(1, Ordering::Relaxed)) {
                return Self(key);
            }
        }
    }
}

/// Move-only rewind point for one synchronous child of the running command.
///
/// The child owns only the attempt rows allocated after it opened. Closing it
/// truncates that suffix; semantic command roots stay with the command.
#[derive(Debug)]
#[must_use = "a child attempt scope must be closed"]
pub struct CommandAttemptChildScope {
    opening: AttemptMark,
}

macro_rules! attempt_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub struct $name {
            key: NonZeroU64,
            row: u32,
            serial: NonZeroU64,
        }

        impl $name {
            const fn index(self) -> usize {
                self.row as usize
            }
        }
    };
}

macro_rules! attempt_id_constructor {
    ($name:ident) => {
        impl $name {
            fn new(key: AttemptKey, row: usize) -> Result<Self, AttemptError> {
                Ok(Self {
                    key: key.0,
                    row: u32::try_from(row).map_err(|_| AttemptError::CapacityOverflow)?,
                    serial: NonZeroU64::new(NEXT_ATTEMPT_SERIAL.fetch_add(1, Ordering::Relaxed))
                        .ok_or(AttemptError::CapacityOverflow)?,
                })
            }
        }
    };
}

attempt_id!(AttemptTokenListId);
attempt_id!(AttemptGlueId);
attempt_id!(AttemptDefinitionId);
attempt_id!(AttemptTokenBufferId);
attempt_id!(AttemptProvenanceId);

attempt_id_constructor!(AttemptTokenListId);
attempt_id_constructor!(AttemptDefinitionId);
attempt_id_constructor!(AttemptTokenBufferId);

#[cfg(test)]
attempt_id_constructor!(AttemptGlueId);
#[cfg(test)]
attempt_id_constructor!(AttemptProvenanceId);

/// Reserved coordinate vocabulary for future attempt-local source names.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AttemptNameId {
    key: NonZeroU64,
    row: u32,
    serial: NonZeroU64,
}

#[cfg(test)]
impl AttemptNameId {
    fn new(key: AttemptKey, row: usize) -> Result<Self, AttemptError> {
        Ok(Self {
            key: key.0,
            row: u32::try_from(row).map_err(|_| AttemptError::CapacityOverflow)?,
            serial: NonZeroU64::new(NEXT_ATTEMPT_SERIAL.fetch_add(1, Ordering::Relaxed))
                .ok_or(AttemptError::CapacityOverflow)?,
        })
    }

    const fn index(self) -> usize {
        self.row as usize
    }
}

/// Resident fields whose attempt-local roots cross one atomic promotion.
///
/// The destination remains at one caller-owned address for the complete
/// transition. Indexed access exposes its current attempt roots in stable
/// field order during preflight. Each `settle_*` call replaces every field
/// which names that source with the supplied durable owner and must not fail:
/// source validation and destination reservation have already completed.
/// Repeated roots therefore create only legitimate semantic aliases, never a
/// relocation table or duplicate aggregate receipt.
pub trait AttemptPromotionDestination<G> {
    fn token_root_count(&self) -> usize;
    fn token_root(&self, index: usize) -> AttemptTokenListId;
    fn next_token_root(&self) -> AttemptTokenListId;
    fn settle_token_root(&mut self, source: AttemptTokenListId, tokens: TokenListId<G>);

    fn glue_root_count(&self) -> usize;
    fn glue_root(&self, index: usize) -> AttemptGlueId;
    fn next_glue_root(&self) -> AttemptGlueId;
    fn settle_glue_root(&mut self, source: AttemptGlueId, glue: GlueId<G>);

    fn definition_root_count(&self) -> usize;
    fn definition_root(&self, index: usize) -> AttemptDefinitionId;
    fn next_definition_root(&self) -> AttemptDefinitionId;
    fn settle_definition_root(
        &mut self,
        source: AttemptDefinitionId,
        definition: tex_state::PublishedDefinition<G>,
    );

    fn provenance_root_count(&self) -> usize;
    fn provenance_root(&self, index: usize) -> AttemptProvenanceId;
    fn next_provenance_root(&self) -> AttemptProvenanceId;
    fn settle_provenance_root(&mut self, source: AttemptProvenanceId, provenance: ProvenanceId<G>);
}

/// Provenance beside one attempt token: either an already-admitted compact
/// origin or a typed row owned by this attempt.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AttemptOrigin {
    Admitted(tex_state::token::OriginId),
    Local(AttemptProvenanceId),
}

/// A typed open token-builder cursor. It names no allocation of its own.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AttemptTokenBuilder {
    key: NonZeroU64,
    start: u32,
    depth: u32,
    serial: NonZeroU64,
}

/// Fixed-size rollback coordinates for every command-attempt table.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct AttemptMark {
    key: NonZeroU64,
    traced_words: u32,
    traced_origins: u32,
    token_scratch: u32,
    origin_scratch: u32,
    token_builders: u32,
    token_lists: u32,
    glue_values: u32,
    definitions: u32,
    token_buffers: u32,
    #[cfg(test)]
    name_bytes: u32,
    #[cfg(test)]
    names: u32,
    provenance: u32,
}

impl AttemptMark {
    pub(crate) const fn is_empty(self) -> bool {
        self.traced_words == 0
            && self.traced_origins == 0
            && self.token_scratch == 0
            && self.origin_scratch == 0
            && self.token_builders == 0
            && self.token_lists == 0
            && self.glue_values == 0
            && self.definitions == 0
            && self.token_buffers == 0
            && self.provenance == 0
            && self.test_names_are_empty()
    }

    #[cfg(test)]
    const fn test_names_are_empty(self) -> bool {
        self.name_bytes == 0 && self.names == 0
    }

    #[cfg(not(test))]
    const fn test_names_are_empty(self) -> bool {
        true
    }
}

/// Invalid foreign coordinates or bounded-capacity failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptError {
    ForeignAttempt,
    InvalidCoordinate,
    CapacityOverflow,
    AllocationFailed,
    Definition(DefinitionBuildError),
    Promotion(PromotionError),
}

impl From<DefinitionBuildError> for AttemptError {
    fn from(error: DefinitionBuildError) -> Self {
        Self::Definition(error)
    }
}

impl From<PromotionError> for AttemptError {
    fn from(error: PromotionError) -> Self {
        Self::Promotion(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AttemptRange {
    start: u32,
    len: u32,
}

impl AttemptRange {
    fn checked(start: usize, len: usize) -> Result<Self, AttemptError> {
        let start = u32::try_from(start).map_err(|_| AttemptError::CapacityOverflow)?;
        let len = u32::try_from(len).map_err(|_| AttemptError::CapacityOverflow)?;
        start
            .checked_add(len)
            .ok_or(AttemptError::CapacityOverflow)?;
        Ok(Self { start, len })
    }

    fn resolve<T>(self, values: &[T]) -> Result<&[T], AttemptError> {
        let start = self.start as usize;
        let end = start
            .checked_add(self.len as usize)
            .ok_or(AttemptError::InvalidCoordinate)?;
        values
            .get(start..end)
            .ok_or(AttemptError::InvalidCoordinate)
    }
}

struct AttemptDefinition {
    builder: Option<DefinitionBuilder>,
}

/// Borrowed words from one operation-local token list.
///
/// The representation is deliberately opaque: ordinary attempt lists are
/// contiguous, while scanner destinations are fixed-chunk branches in the
/// attempt's shared lane. Consumers iterate or index without materializing a
/// second list.
#[derive(Clone, Copy)]
pub struct AttemptTokenListView<'arena> {
    storage: AttemptTokenListViewStorage<'arena>,
}

#[derive(Clone, Copy)]
enum AttemptTokenListViewStorage<'arena> {
    Contiguous(&'arena [TracedTokenWord]),
    Scanner(TokenLaneView<'arena>),
}

impl<'arena> AttemptTokenListView<'arena> {
    #[must_use]
    pub fn len(self) -> usize {
        match self.storage {
            AttemptTokenListViewStorage::Contiguous(words) => words.len(),
            AttemptTokenListViewStorage::Scanner(words) => words.len(),
        }
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn get(self, index: usize) -> Option<&'arena TracedTokenWord> {
        match self.storage {
            AttemptTokenListViewStorage::Contiguous(words) => words.get(index),
            AttemptTokenListViewStorage::Scanner(words) => words.get(index),
        }
    }

    #[must_use]
    pub fn first(self) -> Option<&'arena TracedTokenWord> {
        self.get(0)
    }

    pub fn iter(self) -> AttemptTokenListIter<'arena> {
        AttemptTokenListIter {
            storage: match self.storage {
                AttemptTokenListViewStorage::Contiguous(words) => {
                    AttemptTokenListIterStorage::Contiguous(words.iter())
                }
                AttemptTokenListViewStorage::Scanner(words) => {
                    AttemptTokenListIterStorage::Scanner(words.iter())
                }
            },
        }
    }

    #[must_use]
    pub fn to_vec(self) -> Vec<TracedTokenWord> {
        self.iter().copied().collect()
    }
}

enum AttemptTokenListIterStorage<'arena> {
    Contiguous(core::slice::Iter<'arena, TracedTokenWord>),
    Scanner(token_lane::TokenLaneIter<'arena>),
}

pub struct AttemptTokenListIter<'arena> {
    storage: AttemptTokenListIterStorage<'arena>,
}

impl<'arena> Iterator for AttemptTokenListIter<'arena> {
    type Item = &'arena TracedTokenWord;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.storage {
            AttemptTokenListIterStorage::Contiguous(words) => words.next(),
            AttemptTokenListIterStorage::Scanner(words) => words.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.storage {
            AttemptTokenListIterStorage::Contiguous(words) => words.size_hint(),
            AttemptTokenListIterStorage::Scanner(words) => words.size_hint(),
        }
    }
}

impl ExactSizeIterator for AttemptTokenListIter<'_> {}

impl<'arena> IntoIterator for AttemptTokenListView<'arena> {
    type Item = &'arena TracedTokenWord;
    type IntoIter = AttemptTokenListIter<'arena>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl core::fmt::Debug for AttemptTokenListView<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_list().entries(self.iter()).finish()
    }
}

impl PartialEq for AttemptTokenListView<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl Eq for AttemptTokenListView<'_> {}

impl PartialEq<&[TracedTokenWord]> for AttemptTokenListView<'_> {
    fn eq(&self, other: &&[TracedTokenWord]) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl<const N: usize> PartialEq<&[TracedTokenWord; N]> for AttemptTokenListView<'_> {
    fn eq(&self, other: &&[TracedTokenWord; N]) -> bool {
        self.len() == N && self.iter().eq(other.iter())
    }
}

impl PartialEq<AttemptTokenListView<'_>> for &[TracedTokenWord] {
    fn eq(&self, other: &AttemptTokenListView<'_>) -> bool {
        other == self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AttemptTokenStorage {
    Range(AttemptRange),
    /// A scanner result row is reserved with its parent-owned mutable sink.
    /// Finalization points at that nonmoving sink without copying its words or
    /// adding another heap owner.
    Buffer(AttemptTokenBufferId),
    PendingBuffer,
    ConsumedBuffer,
}

#[derive(Debug, Eq, PartialEq)]
struct AttemptTokenBuffer {
    sink: TokenSink,
    result: AttemptTokenListId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AttemptRow<T> {
    serial: NonZeroU64,
    value: T,
}

/// All command-side storage which shares one operation lifetime.
pub(crate) struct AttemptArena<G> {
    key: AttemptKey,
    traced_words: Vec<TracedTokenWord>,
    traced_origins: Vec<Option<AttemptProvenanceId>>,
    token_scratch: Vec<TracedTokenWord>,
    origin_scratch: Vec<Option<AttemptProvenanceId>>,
    token_builders: Vec<AttemptTokenBuilder>,
    token_lists: Vec<AttemptRow<AttemptTokenStorage>>,
    glue_values: Vec<AttemptRow<GlueSpec>>,
    definitions: Vec<AttemptRow<AttemptDefinition>>,
    recycled_definition_builders: Vec<DefinitionBuilder>,
    #[cfg(test)]
    fail_next_definition_builder_allocation: bool,
    #[cfg(test)]
    fail_next_definition_finalization: bool,
    token_lane: TokenLane,
    token_buffers: Vec<AttemptRow<AttemptTokenBuffer>>,
    #[cfg(test)]
    name_bytes: Vec<u8>,
    #[cfg(test)]
    names: Vec<AttemptRow<AttemptRange>>,
    provenance: Vec<AttemptRow<OriginRecord>>,
    _generation: PhantomData<fn(&G) -> &G>,
}

impl<G> core::fmt::Debug for AttemptArena<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("AttemptArena(..)")
    }
}

impl<G> Default for AttemptArena<G> {
    fn default() -> Self {
        Self {
            key: AttemptKey::fresh(),
            traced_words: Vec::new(),
            traced_origins: Vec::new(),
            token_scratch: Vec::new(),
            origin_scratch: Vec::new(),
            token_builders: Vec::new(),
            token_lists: Vec::new(),
            glue_values: Vec::new(),
            definitions: Vec::new(),
            recycled_definition_builders: Vec::new(),
            #[cfg(test)]
            fail_next_definition_builder_allocation: false,
            #[cfg(test)]
            fail_next_definition_finalization: false,
            token_lane: TokenLane::default(),
            token_buffers: Vec::new(),
            #[cfg(test)]
            name_bytes: Vec::new(),
            #[cfg(test)]
            names: Vec::new(),
            provenance: Vec::new(),
            _generation: PhantomData,
        }
    }
}

impl<G> AttemptArena<G> {
    #[must_use]
    pub(crate) fn mark(&self) -> AttemptMark {
        AttemptMark {
            key: self.key.0,
            traced_words: u32::try_from(self.traced_words.len())
                .expect("attempt traced-word length is bounded"),
            traced_origins: u32::try_from(self.traced_origins.len())
                .expect("attempt traced-origin length is bounded"),
            token_scratch: u32::try_from(self.token_scratch.len())
                .expect("attempt token-scratch length is bounded"),
            origin_scratch: u32::try_from(self.origin_scratch.len())
                .expect("attempt origin-scratch length is bounded"),
            token_builders: u32::try_from(self.token_builders.len())
                .expect("attempt token-builder length is bounded"),
            token_lists: u32::try_from(self.token_lists.len())
                .expect("attempt token-list length is bounded"),
            glue_values: u32::try_from(self.glue_values.len())
                .expect("attempt glue length is bounded"),
            definitions: u32::try_from(self.definitions.len())
                .expect("attempt definition length is bounded"),
            token_buffers: u32::try_from(self.token_buffers.len())
                .expect("attempt token-buffer length is bounded"),
            #[cfg(test)]
            name_bytes: u32::try_from(self.name_bytes.len())
                .expect("attempt name-byte length is bounded"),
            #[cfg(test)]
            names: u32::try_from(self.names.len()).expect("attempt name length is bounded"),
            provenance: u32::try_from(self.provenance.len())
                .expect("attempt provenance length is bounded"),
        }
    }

    pub(crate) fn validate_mark(&self, mark: AttemptMark) -> Result<(), AttemptError> {
        if mark.key != self.key.0 {
            return Err(AttemptError::ForeignAttempt);
        }
        let lengths = [
            (mark.traced_words as usize, self.traced_words.len()),
            (mark.traced_origins as usize, self.traced_origins.len()),
            (mark.token_scratch as usize, self.token_scratch.len()),
            (mark.origin_scratch as usize, self.origin_scratch.len()),
            (mark.token_builders as usize, self.token_builders.len()),
            (mark.token_lists as usize, self.token_lists.len()),
            (mark.glue_values as usize, self.glue_values.len()),
            (mark.definitions as usize, self.definitions.len()),
            (mark.token_buffers as usize, self.token_buffers.len()),
            (mark.provenance as usize, self.provenance.len()),
        ];
        if lengths.iter().any(|(mark, live)| mark > live) {
            return Err(AttemptError::InvalidCoordinate);
        }
        #[cfg(test)]
        if mark.name_bytes as usize > self.name_bytes.len()
            || mark.names as usize > self.names.len()
        {
            return Err(AttemptError::InvalidCoordinate);
        }
        Ok(())
    }

    /// Returns every attempt row to the empty root mark while retaining table
    /// capacity. Checkpoint captures are taken only while the attempt is
    /// empty, so restoring one restores this exact shape.
    pub(crate) fn clear(&mut self) {
        let empty = AttemptMark {
            key: self.key.0,
            traced_words: 0,
            traced_origins: 0,
            token_scratch: 0,
            origin_scratch: 0,
            token_builders: 0,
            token_lists: 0,
            glue_values: 0,
            definitions: 0,
            token_buffers: 0,
            #[cfg(test)]
            name_bytes: 0,
            #[cfg(test)]
            names: 0,
            provenance: 0,
        };
        self.truncate(empty)
            .expect("the empty attempt mark is always a valid prefix");
    }

    /// Whether no row is live. Every row hangs from a root table: an open
    /// token builder, a finished list, a scanner buffer, a definition
    /// builder, glue, provenance, or a name. Scratch words exist only under
    /// an open builder and list words only under a finished list.
    #[inline(always)]
    pub(crate) fn is_clear(&self) -> bool {
        (self.token_builders.len()
            | self.token_lists.len()
            | self.token_buffers.len()
            | self.definitions.len()
            | self.glue_values.len()
            | self.provenance.len()
            | self.test_name_count())
            == 0
    }

    #[cfg(test)]
    fn test_name_count(&self) -> usize {
        self.names.len()
    }

    #[cfg(not(test))]
    #[inline(always)]
    const fn test_name_count(&self) -> usize {
        0
    }

    /// Releases every row of the command that just finished. Attempt rows
    /// never outlive their command, so the arena is clear at every command
    /// boundary and a command that allocated nothing pays one test.
    #[inline(always)]
    pub(crate) fn settle(&mut self) {
        if !self.is_clear() {
            self.clear();
        }
        debug_assert!(self.mark().is_empty());
    }

    /// Opens a synchronous child scope at the current suffix.
    pub(crate) fn begin_child_scope(&self) -> CommandAttemptChildScope {
        CommandAttemptChildScope {
            opening: self.mark(),
        }
    }

    /// Closes a child scope, releasing exactly the rows allocated inside it.
    pub(crate) fn close_child_scope(
        &mut self,
        scope: CommandAttemptChildScope,
    ) -> Result<(), AttemptError> {
        self.truncate(scope.opening)
    }

    /// Rejects a suffix by truncating tables and returning whole scanner chunks.
    ///
    /// Work is constant per table plus the number of retired scanner chunks;
    /// packed token words are never visited, copied, or dropped individually.
    pub(crate) fn truncate(&mut self, mark: AttemptMark) -> Result<(), AttemptError> {
        self.validate_mark(mark)?;
        #[cfg(test)]
        self.names.truncate(mark.names as usize);
        self.provenance.truncate(mark.provenance as usize);
        while self.token_buffers.len() > mark.token_buffers as usize {
            let buffer = self
                .token_buffers
                .pop()
                .expect("attempt token-buffer suffix is nonempty")
                .value;
            self.token_lane.release(buffer.sink)?;
        }
        while self.definitions.len() > mark.definitions as usize {
            let builder = self
                .definitions
                .pop()
                .expect("attempt definition suffix is nonempty")
                .value
                .builder;
            if let Some(builder) = builder {
                self.recycled_definition_builders.push(builder);
            }
        }
        self.glue_values.truncate(mark.glue_values as usize);
        self.token_lists.truncate(mark.token_lists as usize);
        self.token_builders.truncate(mark.token_builders as usize);
        self.token_scratch.truncate(mark.token_scratch as usize);
        self.origin_scratch.truncate(mark.origin_scratch as usize);
        self.traced_words.truncate(mark.traced_words as usize);
        self.traced_origins.truncate(mark.traced_origins as usize);
        #[cfg(test)]
        self.name_bytes.truncate(mark.name_bytes as usize);
        Ok(())
    }

    pub(crate) fn begin_token_list(&mut self) -> Result<AttemptTokenBuilder, AttemptError> {
        #[cfg(feature = "profiling")]
        let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::AttemptScratch,
        );
        let builder = AttemptTokenBuilder {
            key: self.key.0,
            start: u32::try_from(self.token_scratch.len())
                .map_err(|_| AttemptError::CapacityOverflow)?,
            depth: u32::try_from(self.token_builders.len())
                .map_err(|_| AttemptError::CapacityOverflow)?,
            serial: NonZeroU64::new(NEXT_ATTEMPT_SERIAL.fetch_add(1, Ordering::Relaxed))
                .ok_or(AttemptError::CapacityOverflow)?,
        };
        self.token_builders
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.token_builders.push(builder);
        Ok(builder)
    }

    pub(crate) fn push_token(
        &mut self,
        builder: AttemptTokenBuilder,
        word: TracedTokenWord,
    ) -> Result<(), AttemptError> {
        self.push_token_parts(builder, word, None)
    }

    #[cfg(test)]
    pub(crate) fn push_token_with_local_origin(
        &mut self,
        builder: AttemptTokenBuilder,
        word: TokenWord,
        origin: AttemptProvenanceId,
    ) -> Result<(), AttemptError> {
        self.provenance(origin)?;
        self.push_token_parts(
            builder,
            TracedTokenWord::from_parts(word, tex_state::token::OriginId::UNKNOWN),
            Some(origin),
        )
    }

    fn push_token_parts(
        &mut self,
        builder: AttemptTokenBuilder,
        word: TracedTokenWord,
        origin: Option<AttemptProvenanceId>,
    ) -> Result<(), AttemptError> {
        #[cfg(feature = "profiling")]
        let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::AttemptScratch,
        );
        self.validate_key(builder.key)?;
        if self.token_builders.last() != Some(&builder)
            || builder.start as usize > self.token_scratch.len()
        {
            return Err(AttemptError::InvalidCoordinate);
        }
        self.token_scratch
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.origin_scratch
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.token_scratch.push(word);
        self.origin_scratch.push(origin);
        Ok(())
    }

    pub(crate) fn finish_token_list(
        &mut self,
        builder: AttemptTokenBuilder,
    ) -> Result<AttemptTokenListId, AttemptError> {
        #[cfg(feature = "profiling")]
        let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::AttemptScratch,
        );
        self.validate_key(builder.key)?;
        if self.token_builders.last() != Some(&builder) {
            return Err(AttemptError::InvalidCoordinate);
        }
        let start = builder.start as usize;
        let len = self
            .token_scratch
            .len()
            .checked_sub(start)
            .ok_or(AttemptError::InvalidCoordinate)?;
        let range = AttemptRange::checked(self.traced_words.len(), len)?;
        let id = AttemptTokenListId::new(self.key, self.token_lists.len())?;
        self.traced_words
            .try_reserve(len)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.traced_origins
            .try_reserve(len)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.token_lists
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.traced_words
            .extend_from_slice(&self.token_scratch[start..]);
        self.traced_origins
            .extend_from_slice(&self.origin_scratch[start..]);
        self.token_scratch.truncate(start);
        self.origin_scratch.truncate(start);
        self.token_builders.pop();
        self.token_lists.push(AttemptRow {
            serial: id.serial,
            value: AttemptTokenStorage::Range(range),
        });
        Ok(id)
    }

    pub(crate) fn allocate_token_list(
        &mut self,
        words: impl IntoIterator<Item = TracedTokenWord>,
    ) -> Result<AttemptTokenListId, AttemptError> {
        let mark = self.mark();
        let result = (|| {
            let builder = self.begin_token_list()?;
            for word in words {
                self.push_token(builder, word)?;
            }
            self.finish_token_list(builder)
        })();
        if result.is_err() {
            self.truncate(mark)
                .expect("the allocation-local attempt mark is valid");
        }
        result
    }

    pub(crate) fn token_words(
        &self,
        id: AttemptTokenListId,
    ) -> Result<AttemptTokenListView<'_>, AttemptError> {
        self.validate_key(id.key)?;
        let row = self
            .token_lists
            .get(id.index())
            .filter(|row| row.serial == id.serial)
            .ok_or(AttemptError::InvalidCoordinate)?;
        match &row.value {
            AttemptTokenStorage::Range(range) => {
                range
                    .resolve(&self.traced_words)
                    .map(|words| AttemptTokenListView {
                        storage: AttemptTokenListViewStorage::Contiguous(words),
                    })
            }
            AttemptTokenStorage::Buffer(buffer) => self.token_buffer(*buffer),
            AttemptTokenStorage::PendingBuffer | AttemptTokenStorage::ConsumedBuffer => {
                Err(AttemptError::InvalidCoordinate)
            }
        }
    }

    pub(crate) fn token_word(
        &self,
        id: AttemptTokenListId,
        index: usize,
    ) -> Result<TracedTokenWord, AttemptError> {
        self.token_words(id)?
            .get(index)
            .copied()
            .ok_or(AttemptError::InvalidCoordinate)
    }

    #[cfg(test)]
    pub(crate) fn allocate_glue(&mut self, value: GlueSpec) -> Result<AttemptGlueId, AttemptError> {
        let id = AttemptGlueId::new(self.key, self.glue_values.len())?;
        self.glue_values
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.glue_values.push(AttemptRow {
            serial: id.serial,
            value,
        });
        Ok(id)
    }

    #[cfg(test)]
    pub(crate) fn allocate_provenance(
        &mut self,
        value: OriginRecord,
    ) -> Result<AttemptProvenanceId, AttemptError> {
        let id = AttemptProvenanceId::new(self.key, self.provenance.len())?;
        self.provenance
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.provenance.push(AttemptRow {
            serial: id.serial,
            value,
        });
        Ok(id)
    }

    pub(crate) fn provenance(&self, id: AttemptProvenanceId) -> Result<OriginRecord, AttemptError> {
        self.validate_key(id.key)?;
        self.provenance
            .get(id.index())
            .copied()
            .filter(|row| row.serial == id.serial)
            .map(|row| row.value)
            .ok_or(AttemptError::InvalidCoordinate)
    }

    #[cfg(test)]
    pub(crate) fn token_origin(
        &self,
        id: AttemptTokenListId,
        index: usize,
    ) -> Result<AttemptOrigin, AttemptError> {
        self.validate_key(id.key)?;
        let storage = &self
            .token_lists
            .get(id.index())
            .filter(|row| row.serial == id.serial)
            .ok_or(AttemptError::InvalidCoordinate)?
            .value;
        match storage {
            AttemptTokenStorage::Range(range) => {
                if index >= range.len as usize {
                    return Err(AttemptError::InvalidCoordinate);
                }
                let absolute = range.start as usize + index;
                match self.traced_origins[absolute] {
                    Some(origin) => Ok(AttemptOrigin::Local(origin)),
                    None => Ok(AttemptOrigin::Admitted(
                        self.traced_words[absolute].origin(),
                    )),
                }
            }
            AttemptTokenStorage::Buffer(buffer) => self
                .token_buffer(*buffer)?
                .get(index)
                .map(|word| AttemptOrigin::Admitted(word.origin()))
                .ok_or(AttemptError::InvalidCoordinate),
            AttemptTokenStorage::PendingBuffer | AttemptTokenStorage::ConsumedBuffer => {
                Err(AttemptError::InvalidCoordinate)
            }
        }
    }

    pub(crate) fn glue(&self, id: AttemptGlueId) -> Result<GlueSpec, AttemptError> {
        self.validate_key(id.key)?;
        self.glue_values
            .get(id.index())
            .copied()
            .filter(|row| row.serial == id.serial)
            .map(|row| row.value)
            .ok_or(AttemptError::InvalidCoordinate)
    }

    pub(crate) fn allocate_definition_builder(
        &mut self,
    ) -> Result<AttemptDefinitionId, AttemptError> {
        #[cfg(feature = "profiling")]
        let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::AttemptScratch,
        );
        #[cfg(test)]
        if core::mem::take(&mut self.fail_next_definition_builder_allocation) {
            return Err(AttemptError::AllocationFailed);
        }
        let id = AttemptDefinitionId::new(self.key, self.definitions.len())?;
        self.definitions
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        let mut builder = self.recycled_definition_builders.pop().unwrap_or_default();
        builder.reset();
        self.definitions.push(AttemptRow {
            serial: id.serial,
            value: AttemptDefinition {
                builder: Some(builder),
            },
        });
        Ok(id)
    }

    fn definition_builder(
        &self,
        id: AttemptDefinitionId,
    ) -> Result<&DefinitionBuilder, AttemptError> {
        self.validate_key(id.key)?;
        self.definitions
            .get(id.index())
            .filter(|row| row.serial == id.serial)
            .and_then(|row| row.value.builder.as_ref())
            .ok_or(AttemptError::InvalidCoordinate)
    }

    fn definition_builder_mut(
        &mut self,
        id: AttemptDefinitionId,
    ) -> Result<&mut DefinitionBuilder, AttemptError> {
        self.validate_key(id.key)?;
        self.definitions
            .get_mut(id.index())
            .filter(|row| row.serial == id.serial)
            .and_then(|row| row.value.builder.as_mut())
            .ok_or(AttemptError::InvalidCoordinate)
    }

    pub(crate) fn push_definition_parameter(
        &mut self,
        id: AttemptDefinitionId,
        word: TokenWord,
    ) -> Result<(), AttemptError> {
        #[cfg(feature = "profiling")]
        let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::AttemptScratch,
        );
        self.definition_builder_mut(id)?.push_parameter(word)?;
        Ok(())
    }

    pub(crate) fn finish_definition_parameters(
        &mut self,
        id: AttemptDefinitionId,
    ) -> Result<(), AttemptError> {
        self.definition_builder_mut(id)?.finish_parameters()?;
        Ok(())
    }

    pub(crate) fn push_definition_replacement(
        &mut self,
        id: AttemptDefinitionId,
        word: TokenWord,
    ) -> Result<(), AttemptError> {
        #[cfg(feature = "profiling")]
        let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::AttemptScratch,
        );
        self.definition_builder_mut(id)?.push_replacement(word)?;
        Ok(())
    }

    pub(crate) fn finish_definition(
        &mut self,
        id: AttemptDefinitionId,
    ) -> Result<(), AttemptError> {
        #[cfg(test)]
        if core::mem::take(&mut self.fail_next_definition_finalization) {
            return Err(AttemptError::AllocationFailed);
        }
        self.definition_builder_mut(id)?.seal()?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn inject_definition_builder_allocation_failure(&mut self) {
        self.fail_next_definition_builder_allocation = true;
    }

    #[cfg(test)]
    pub(crate) fn inject_definition_finalization_failure(&mut self) {
        self.fail_next_definition_finalization = true;
    }

    pub(crate) fn definition_parameter_words(
        &self,
        id: AttemptDefinitionId,
    ) -> Result<&[TokenWord], AttemptError> {
        Ok(self.definition_builder(id)?.parameter_text())
    }

    pub(crate) fn definition_replacement_words(
        &self,
        id: AttemptDefinitionId,
    ) -> Result<&[TokenWord], AttemptError> {
        Ok(self.definition_builder(id)?.replacement_text())
    }

    /// Allocates one mutable scanner buffer owned by this attempt.
    pub(crate) fn allocate_token_buffer(&mut self) -> Result<AttemptTokenBufferId, AttemptError> {
        #[cfg(feature = "profiling")]
        let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::AttemptScratch,
        );
        let id = AttemptTokenBufferId::new(self.key, self.token_buffers.len())?;
        let result = AttemptTokenListId::new(self.key, self.token_lists.len())?;
        self.token_buffers
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.token_lists
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.token_lists.push(AttemptRow {
            serial: result.serial,
            value: AttemptTokenStorage::PendingBuffer,
        });
        self.token_buffers.push(AttemptRow {
            serial: id.serial,
            value: AttemptTokenBuffer {
                sink: TokenSink::default(),
                result,
            },
        });
        Ok(id)
    }

    pub(crate) fn token_buffer(
        &self,
        id: AttemptTokenBufferId,
    ) -> Result<AttemptTokenListView<'_>, AttemptError> {
        self.validate_key(id.key)?;
        let sink = &self
            .token_buffers
            .get(id.index())
            .filter(|row| row.serial == id.serial)
            .ok_or(AttemptError::InvalidCoordinate)?
            .value
            .sink;
        Ok(AttemptTokenListView {
            storage: AttemptTokenListViewStorage::Scanner(self.token_lane.view(sink)),
        })
    }

    pub(crate) fn push_buffer_token(
        &mut self,
        id: AttemptTokenBufferId,
        word: TracedTokenWord,
    ) -> Result<(), AttemptError> {
        #[cfg(feature = "profiling")]
        let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::AttemptScratch,
        );
        self.validate_key(id.key)?;
        let Self {
            token_lane,
            token_buffers,
            ..
        } = self;
        let sink = &mut token_buffers
            .get_mut(id.index())
            .filter(|row| row.serial == id.serial)
            .ok_or(AttemptError::InvalidCoordinate)?
            .value
            .sink;
        token_lane.push(sink, word)
    }

    /// Moves a completed child scanner list into a live parent scanner sink.
    ///
    /// This is the direct-splice ownership boundary used by `\unexpanded`.
    /// The source list is consumed and its fixed-chunk chain becomes part of
    /// the destination without rereading or copying any token word.
    pub(crate) fn consume_token_list_into_buffer(
        &mut self,
        source: AttemptTokenListId,
        destination: AttemptTokenBufferId,
    ) -> Result<u32, AttemptError> {
        self.validate_key(source.key)?;
        self.validate_key(destination.key)?;
        let source_buffer = match self
            .token_lists
            .get(source.index())
            .filter(|row| row.serial == source.serial)
            .map(|row| row.value)
        {
            Some(AttemptTokenStorage::Buffer(buffer)) => buffer,
            _ => return Err(AttemptError::InvalidCoordinate),
        };
        if source_buffer == destination {
            return Err(AttemptError::InvalidCoordinate);
        }
        let destination_index = destination.index();
        let source_index = source_buffer.index();
        let destination_serial = self
            .token_buffers
            .get(destination_index)
            .filter(|row| row.serial == destination.serial)
            .map(|row| row.serial)
            .ok_or(AttemptError::InvalidCoordinate)?;
        let source_serial = self
            .token_buffers
            .get(source_index)
            .filter(|row| row.serial == source_buffer.serial)
            .map(|row| row.serial)
            .ok_or(AttemptError::InvalidCoordinate)?;
        debug_assert_eq!(destination_serial, destination.serial);
        debug_assert_eq!(source_serial, source_buffer.serial);

        let (destination_sink, source_sink) = if destination_index < source_index {
            let (before_source, source_and_after) = self.token_buffers.split_at_mut(source_index);
            (
                &mut before_source[destination_index].value.sink,
                &mut source_and_after[0].value.sink,
            )
        } else {
            let (before_destination, destination_and_after) =
                self.token_buffers.split_at_mut(destination_index);
            (
                &mut destination_and_after[0].value.sink,
                &mut before_destination[source_index].value.sink,
            )
        };
        let moved = source_sink.len();
        self.token_lane.append_sink(destination_sink, source_sink)?;
        self.token_lists[source.index()].value = AttemptTokenStorage::ConsumedBuffer;
        Ok(moved)
    }

    pub(crate) fn finish_token_buffer(
        &mut self,
        id: AttemptTokenBufferId,
    ) -> Result<AttemptTokenListId, AttemptError> {
        #[cfg(feature = "profiling")]
        let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::AttemptScratch,
        );
        self.validate_key(id.key)?;
        let result = self
            .token_buffers
            .get(id.index())
            .filter(|row| row.serial == id.serial)
            .map(|row| row.value.result)
            .ok_or(AttemptError::InvalidCoordinate)?;
        let storage = &self
            .token_lists
            .get(result.index())
            .filter(|row| row.serial == result.serial)
            .ok_or(AttemptError::InvalidCoordinate)?
            .value;
        if !matches!(storage, AttemptTokenStorage::PendingBuffer) {
            return Err(AttemptError::InvalidCoordinate);
        }
        self.token_lists
            .get_mut(result.index())
            .filter(|row| row.serial == result.serial)
            .expect("validated token-list row remains live")
            .value = AttemptTokenStorage::Buffer(id);
        Ok(result)
    }

    #[cfg(test)]
    pub(crate) fn allocate_name(&mut self, name: &str) -> Result<AttemptNameId, AttemptError> {
        let range = AttemptRange::checked(self.name_bytes.len(), name.len())?;
        let id = AttemptNameId::new(self.key, self.names.len())?;
        self.name_bytes
            .try_reserve(name.len())
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.names
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.name_bytes.extend_from_slice(name.as_bytes());
        self.names.push(AttemptRow {
            serial: id.serial,
            value: range,
        });
        Ok(id)
    }

    #[cfg(test)]
    pub(crate) fn name(&self, id: AttemptNameId) -> Result<&str, AttemptError> {
        self.validate_key(id.key)?;
        let row = self
            .names
            .get(id.index())
            .copied()
            .filter(|row| row.serial == id.serial)
            .ok_or(AttemptError::InvalidCoordinate)?;
        let bytes = row.value.resolve(&self.name_bytes)?;
        std::str::from_utf8(bytes).map_err(|_| AttemptError::InvalidCoordinate)
    }

    /// Publishes every attempt root directly into one resident destination.
    ///
    /// Complete source and destination traversal precedes reservation. On
    /// success the durable publishers settle each final field before
    /// advancing to the next source; no builder batch, relocation table, or
    /// aggregate owner receipt exists. Definition builders remain in their
    /// authoritative attempt rows through preflight and become vacant only at
    /// their exact successful publication.
    pub(crate) fn promote_into<D>(
        &mut self,
        universe: &mut Universe<G>,
        destination: &mut D,
    ) -> Result<(), AttemptError>
    where
        D: AttemptPromotionDestination<G>,
    {
        for index in 0..destination.token_root_count() {
            let id = destination.token_root(index);
            self.validate_key(id.key)?;
            self.token_words(id)?;
        }
        for index in 0..destination.glue_root_count() {
            let id = destination.glue_root(index);
            self.validate_key(id.key)?;
            self.glue(id)?;
        }
        for index in 0..destination.definition_root_count() {
            let id = destination.definition_root(index);
            self.validate_key(id.key)?;
            self.definition_builder(id)?;
        }
        for index in 0..destination.provenance_root_count() {
            let id = destination.provenance_root(index);
            self.validate_key(id.key)?;
            self.provenance(id)?;
        }

        let definition_count = unique_count_by(destination.definition_root_count(), |index| {
            destination.definition_root(index)
        });
        self.recycled_definition_builders
            .try_reserve(definition_count)
            .map_err(|_| AttemptError::AllocationFailed)?;
        let mut batch = AttemptResidentPromotion {
            arena: self,
            destination,
        };
        universe.promote_resident_batch(&mut batch)?;
        Ok(())
    }

    /// Promotes one macro definition without arena-sized relocation tables,
    /// temporary semantic-word vectors, or a heap-allocated receipt.
    pub(crate) fn promote_definition(
        &mut self,
        universe: &mut Universe<G>,
        id: AttemptDefinitionId,
    ) -> Result<tex_state::PublishedDefinition<G>, AttemptError> {
        self.validate_key(id.key)?;
        let definition = self
            .definitions
            .get_mut(id.index())
            .filter(|row| row.serial == id.serial)
            .and_then(|row| row.value.builder.as_mut())
            .ok_or(AttemptError::InvalidCoordinate)?;
        universe
            .promote_definition_builder(definition)
            .map_err(AttemptError::from)
    }

    fn validate_key(&self, key: NonZeroU64) -> Result<(), AttemptError> {
        if key == self.key.0 {
            Ok(())
        } else {
            Err(AttemptError::ForeignAttempt)
        }
    }
}

fn unique_count_by<T>(len: usize, mut at: impl FnMut(usize) -> T) -> usize
where
    T: Copy + PartialEq,
{
    (0..len)
        .filter(|&index| (0..index).all(|prior| at(prior) != at(index)))
        .count()
}

struct AttemptResidentPromotion<'a, G, D> {
    arena: &'a mut AttemptArena<G>,
    destination: &'a mut D,
}

impl<G, D> tex_state::ResidentPromotionBatch<G> for AttemptResidentPromotion<'_, G, D>
where
    D: AttemptPromotionDestination<G>,
{
    fn definition_source_count(&self) -> usize {
        self.destination.definition_root_count()
    }

    fn definition_count(&self) -> usize {
        unique_count_by(self.destination.definition_root_count(), |index| {
            self.destination.definition_root(index)
        })
    }

    fn definition(&self, index: usize) -> &DefinitionBuilder {
        let id = self.destination.definition_root(index);
        self.arena
            .definition_builder(id)
            .expect("the complete definition source set was validated")
    }

    fn next_definition_mut(&mut self) -> &mut DefinitionBuilder {
        let id = self.destination.next_definition_root();
        self.arena
            .definitions
            .get_mut(id.index())
            .filter(|row| row.serial == id.serial)
            .and_then(|row| row.value.builder.as_mut())
            .expect("the next preflighted definition source remains resident")
    }

    fn settle_next_definition(&mut self, definition: tex_state::PublishedDefinition<G>) {
        let source = self.destination.next_definition_root();
        self.destination.settle_definition_root(source, definition);
        let builder = self
            .arena
            .definitions
            .get_mut(source.index())
            .filter(|row| row.serial == source.serial)
            .and_then(|row| row.value.builder.take())
            .expect("published definition leaves one vacant resident builder");
        self.arena.recycled_definition_builders.push(builder);
    }

    fn token_list_source_count(&self) -> usize {
        self.destination.token_root_count()
    }

    fn token_list_count(&self) -> usize {
        unique_count_by(self.destination.token_root_count(), |index| {
            self.destination.token_root(index)
        })
    }

    fn token_list_len(&self, index: usize) -> usize {
        let id = self.destination.token_root(index);
        self.arena
            .token_words(id)
            .expect("the complete token source set was validated")
            .len()
    }

    fn token_list_word(&self, index: usize, word: usize) -> TokenWord {
        let id = self.destination.token_root(index);
        self.arena
            .token_words(id)
            .expect("the complete token source set was validated")
            .get(word)
            .expect("preflighted token word is inside the source list")
            .token_word()
    }

    fn next_token_list_words(&self) -> impl ExactSizeIterator<Item = TokenWord> + '_ {
        self.arena
            .token_words(self.destination.next_token_root())
            .expect("the next preflighted token source remains resident")
            .iter()
            .map(|word| word.token_word())
    }

    fn settle_next_token_list(&mut self, tokens: TokenListId<G>) {
        let source = self.destination.next_token_root();
        self.destination.settle_token_root(source, tokens);
    }

    fn glue_source_count(&self) -> usize {
        self.destination.glue_root_count()
    }

    fn glue_count(&self) -> usize {
        unique_count_by(self.destination.glue_root_count(), |index| {
            self.destination.glue_root(index)
        })
    }

    fn glue(&self, index: usize) -> GlueSpec {
        let id = self.destination.glue_root(index);
        self.arena
            .glue(id)
            .expect("the complete glue source set was validated")
    }

    fn next_glue(&self) -> GlueSpec {
        self.arena
            .glue(self.destination.next_glue_root())
            .expect("the next preflighted glue source remains resident")
    }

    fn settle_next_glue(&mut self, glue: GlueId<G>) {
        let source = self.destination.next_glue_root();
        self.destination.settle_glue_root(source, glue);
    }

    fn provenance_source_count(&self) -> usize {
        self.destination.provenance_root_count()
    }

    fn provenance_count(&self) -> usize {
        unique_count_by(self.destination.provenance_root_count(), |index| {
            self.destination.provenance_root(index)
        })
    }

    fn provenance(&self, index: usize) -> OriginRecord {
        let id = self.destination.provenance_root(index);
        self.arena
            .provenance(id)
            .expect("the complete provenance source set was validated")
    }

    fn next_provenance(&self) -> OriginRecord {
        self.arena
            .provenance(self.destination.next_provenance_root())
            .expect("the next preflighted provenance source remains resident")
    }

    fn settle_next_provenance(&mut self, provenance: ProvenanceId<G>) {
        let source = self.destination.next_provenance_root();
        self.destination.settle_provenance_root(source, provenance);
    }
}
