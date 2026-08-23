//! Operation-scoped command scratch and explicit-root promotion.
//!
//! Values in this module are coordinates, never owners. One [`AttemptArena`]
//! owns all backing storage and can be truncated to a fixed-size mark or moved
//! intact into an in-process resource continuation.

use core::marker::PhantomData;
use core::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

use tex_state::glue::GlueSpec;
use tex_state::provenance::OriginRecord;
use tex_state::token::{TokenWord, TracedTokenWord};
use tex_state::{
    DefinitionId, DefinitionPromotion, GenerationOwner, GlueId, PromotionError, ProvenanceId,
    TokenListId, TokenListPromotion, Universe,
};

#[cfg(test)]
#[path = "attempt/tests.rs"]
mod tests;

static NEXT_ATTEMPT_KEY: AtomicU64 = AtomicU64::new(1);
static NEXT_ATTEMPT_SERIAL: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct AttemptScopeSerial(NonZeroU64);

impl AttemptScopeSerial {
    const ROOT: Self = Self(NonZeroU64::MIN);

    fn checked_successor(self) -> Result<Self, AttemptError> {
        self.0
            .get()
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .map(Self)
            .ok_or(AttemptError::CapacityOverflow)
    }
}

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

/// Direct coordinate of one owned scope record inside its attempt arena.
///
/// This is a non-owning command-state cursor. The arena's private
/// [`OwnedAttemptScope`] row is the sole linear owner and is consumed exactly
/// once by LIFO close. A copied stale cursor therefore cannot double-close;
/// direct slot/serial validation rejects it before mutation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct AttemptScopeCoordinate {
    key: NonZeroU64,
    slot: u32,
    serial: AttemptScopeSerial,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AttemptScopeRetirement {
    /// The semantic owner is exhausted and may close as soon as it reaches
    /// the physical LIFO tail.
    Immediate { operation: AttemptScopeCoordinate },
    /// A scanner returned attempt-local output to its parent. The scope is
    /// exhausted, but its suffix remains live until the owning operation
    /// commits, even if a younger immediate child retires first.
    AtOperationCommit { operation: AttemptScopeCoordinate },
}

impl AttemptScopeRetirement {
    const fn operation(self) -> AttemptScopeCoordinate {
        match self {
            Self::Immediate { operation } | Self::AtOperationCommit { operation } => operation,
        }
    }
}

/// Linear capability owning one dynamically suspended attempt scope.
///
/// A Rust lifetime cannot brand a scanner or macro frame which unwinds across
/// a resource barrier and resumes later. Those frames use this smallest
/// private runtime boundary instead: one checked serial and two fixed marks.
/// Construction is arena-private, the value is neither `Copy` nor `Clone`,
/// and exact close consumes it.
#[derive(Debug, Eq, PartialEq)]
#[must_use = "an owned attempt scope must be closed or moved into a continuation"]
pub(crate) struct OwnedAttemptScope {
    key: NonZeroU64,
    serial: AttemptScopeSerial,
    parent: AttemptScopeSerial,
    opening: AttemptMark,
    close_through: AttemptMark,
    /// The direct operation which observed this semantic owner retire.
    /// Rollback clears only matching ancestor flags; commit retires the
    /// operation scope and closes the now-contiguous LIFO tail.
    retirement: Option<AttemptScopeRetirement>,
}

/// One attempt-local id whose child-scope brand cannot escape an HRTB
/// callback or be used through a sibling scope.
#[derive(Debug, Eq, PartialEq)]
pub struct ScopedAttemptTokenListId<'scope> {
    id: AttemptTokenListId,
    scope: AttemptScopeSerial,
    _scope: PhantomData<fn(&'scope mut ()) -> &'scope mut ()>,
}

/// Borrowed synchronous child scope.
///
/// The dynamic engine uses [`OwnedAttemptScope`] only where suspension makes
/// a lexical Rust lifetime impossible. Purely synchronous helpers use this
/// facade so a child id is statically unable to escape the callback.
pub struct AttemptScope<'arena, 'scope, G> {
    arena: &'arena mut AttemptArena<G>,
    coordinate: Option<AttemptScopeCoordinate>,
    _scope: PhantomData<fn(&'scope mut ()) -> &'scope mut ()>,
}

impl<'scope, G> AttemptScope<'_, 'scope, G> {
    pub fn allocate_token_list(
        &mut self,
        words: impl IntoIterator<Item = TracedTokenWord>,
    ) -> Result<ScopedAttemptTokenListId<'scope>, AttemptError> {
        let id = self.arena.allocate_token_list(words)?;
        Ok(ScopedAttemptTokenListId {
            id,
            scope: self
                .coordinate
                .expect("a lexical attempt scope remains open")
                .serial,
            _scope: PhantomData,
        })
    }

    pub fn token_words(
        &self,
        id: &ScopedAttemptTokenListId<'scope>,
    ) -> Result<&[TracedTokenWord], AttemptError> {
        if self
            .coordinate
            .is_none_or(|coordinate| coordinate.serial != id.scope)
        {
            return Err(AttemptError::InvalidCoordinate);
        }
        self.arena.token_words(id.id)
    }
}

impl<G> Drop for AttemptScope<'_, '_, G> {
    fn drop(&mut self) {
        let Some(coordinate) = self.coordinate.take() else {
            return;
        };
        self.arena
            .close_owned_scope(coordinate)
            .expect("a lexical attempt scope closes in exact LIFO order");
    }
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
attempt_id!(AttemptArgumentRecordId);
attempt_id!(AttemptTokenBufferId);
attempt_id!(AttemptProvenanceId);

attempt_id_constructor!(AttemptTokenListId);
attempt_id_constructor!(AttemptDefinitionId);
attempt_id_constructor!(AttemptArgumentRecordId);
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

/// Attempt-local roots selected for one atomic durable promotion.
///
/// The request is branded by the destination generation even though its
/// coordinates belong to the current command attempt. This prevents a batch
/// assembled for one [`Universe`] from being reused with another generation.
/// Every slice is validated before any durable row is reserved or published.
#[derive(Clone, Copy, Debug)]
pub struct AttemptPromotionRoots<'a, G> {
    pub(crate) token_lists: &'a [AttemptTokenListId],
    pub(crate) glue: &'a [AttemptGlueId],
    pub(crate) definitions: &'a [AttemptDefinitionId],
    pub(crate) provenance: &'a [AttemptProvenanceId],
    _generation: PhantomData<fn(&G) -> &G>,
}

impl<'a, G> AttemptPromotionRoots<'a, G> {
    /// Declares every root that may escape the current command attempt.
    #[must_use]
    pub const fn new(
        token_lists: &'a [AttemptTokenListId],
        glue: &'a [AttemptGlueId],
        definitions: &'a [AttemptDefinitionId],
        provenance: &'a [AttemptProvenanceId],
    ) -> Self {
        Self {
            token_lists,
            glue,
            definitions,
            provenance,
            _generation: PhantomData,
        }
    }
}

/// Durable coordinates produced by one atomic attempt-root promotion.
///
/// Each vector follows the exact order of its corresponding request slice,
/// including repeated roots.
#[derive(Debug)]
pub struct AttemptPromotionReceipt<G> {
    pub token_lists: Vec<TokenListId<G>>,
    pub glue: Vec<GlueId<G>>,
    pub definitions: Vec<DefinitionId<G>>,
    pub provenance: Vec<ProvenanceId<G>>,
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
    scope_depth: u32,
    traced_words: u32,
    traced_origins: u32,
    token_scratch: u32,
    origin_scratch: u32,
    token_builders: u32,
    token_lists: u32,
    glue_values: u32,
    definitions: u32,
    argument_words: u32,
    argument_records: u32,
    token_buffers: u32,
    #[cfg(test)]
    name_bytes: u32,
    #[cfg(test)]
    names: u32,
    provenance: u32,
}

impl AttemptMark {
    pub(crate) const fn is_empty(self) -> bool {
        self.scope_depth == 0
            && self.traced_words == 0
            && self.traced_origins == 0
            && self.token_scratch == 0
            && self.origin_scratch == 0
            && self.token_builders == 0
            && self.token_lists == 0
            && self.glue_values == 0
            && self.definitions == 0
            && self.argument_words == 0
            && self.argument_records == 0
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

    pub(crate) const fn checked_row_count(self) -> Option<u32> {
        let mut count = self.traced_words;
        let values = [
            self.traced_origins,
            self.token_scratch,
            self.origin_scratch,
            self.token_builders,
            self.token_lists,
            self.glue_values,
            self.definitions,
            self.argument_words,
            self.argument_records,
            self.token_buffers,
            self.provenance,
        ];
        let mut index = 0;
        while index < values.len() {
            let Some(next) = count.checked_add(values[index]) else {
                return None;
            };
            count = next;
            index += 1;
        }
        #[cfg(test)]
        {
            let Some(next) = count.checked_add(self.name_bytes) else {
                return None;
            };
            count = next;
            let Some(next) = count.checked_add(self.names) else {
                return None;
            };
            count = next;
        }
        Some(count)
    }
}

/// Invalid foreign coordinates or bounded-capacity failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptError {
    ForeignAttempt,
    InvalidCoordinate,
    CapacityOverflow,
    AllocationFailed,
    Promotion(PromotionError),
}

/// Failure to move one live attempt across an in-process resource boundary.
///
/// A stale opening coordinate is distinguished from failure to retain the
/// current generation. Both are detected before the live arena is moved.
#[derive(Debug)]
pub enum AttemptSuspendError {
    StaleMark(AttemptError),
    Generation(tex_state::UniverseError),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AttemptDefinition {
    parameter_text: AttemptTokenListId,
    replacement_text: AttemptTokenListId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AttemptTokenStorage {
    Range(AttemptRange),
    /// A scanner result row is reserved with its parent-owned mutable sink.
    /// Finalization points at that nonmoving sink without copying its words or
    /// adding another heap owner.
    Buffer(AttemptTokenBufferId),
    PendingBuffer,
}

#[derive(Debug, Eq, PartialEq)]
struct AttemptTokenBuffer {
    words: Vec<TracedTokenWord>,
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
    next_scope: AttemptScopeSerial,
    committed_through: AttemptScopeSerial,
    scopes: Vec<OwnedAttemptScope>,
    traced_words: Vec<TracedTokenWord>,
    traced_origins: Vec<Option<AttemptProvenanceId>>,
    token_scratch: Vec<TracedTokenWord>,
    origin_scratch: Vec<Option<AttemptProvenanceId>>,
    token_builders: Vec<AttemptTokenBuilder>,
    token_lists: Vec<AttemptRow<AttemptTokenStorage>>,
    glue_values: Vec<AttemptRow<GlueSpec>>,
    definitions: Vec<AttemptRow<AttemptDefinition>>,
    argument_words: Vec<AttemptTokenListId>,
    argument_records: Vec<AttemptRow<AttemptRange>>,
    token_buffers: Vec<AttemptRow<AttemptTokenBuffer>>,
    recycled_token_buffers: Vec<Vec<TracedTokenWord>>,
    #[cfg(test)]
    name_bytes: Vec<u8>,
    #[cfg(test)]
    names: Vec<AttemptRow<AttemptRange>>,
    provenance: Vec<AttemptRow<OriginRecord>>,
    _generation: PhantomData<fn(&G) -> &G>,
}

impl<G> Default for AttemptArena<G> {
    fn default() -> Self {
        Self {
            key: AttemptKey::fresh(),
            next_scope: AttemptScopeSerial::ROOT
                .checked_successor()
                .expect("the root attempt scope has a successor"),
            committed_through: AttemptScopeSerial::ROOT,
            scopes: Vec::new(),
            traced_words: Vec::new(),
            traced_origins: Vec::new(),
            token_scratch: Vec::new(),
            origin_scratch: Vec::new(),
            token_builders: Vec::new(),
            token_lists: Vec::new(),
            glue_values: Vec::new(),
            definitions: Vec::new(),
            argument_words: Vec::new(),
            argument_records: Vec::new(),
            token_buffers: Vec::new(),
            recycled_token_buffers: Vec::new(),
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
    /// Opens one dynamic child of the exact currently active scope.
    ///
    /// The returned linear capability may move into an in-process
    /// continuation. No row, side table, or heap owner is allocated.
    pub(crate) fn begin_owned_scope(&mut self) -> Result<AttemptScopeCoordinate, AttemptError> {
        let serial = self.next_scope;
        let next_scope = serial.checked_successor()?;
        let opening = self.mark();
        let owned = OwnedAttemptScope {
            key: self.key.0,
            serial,
            parent: self.current_scope(),
            opening,
            close_through: opening,
            retirement: None,
        };
        self.scopes
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        let slot = u32::try_from(self.scopes.len()).map_err(|_| AttemptError::CapacityOverflow)?;
        self.next_scope = next_scope;
        self.scopes.push(owned);
        Ok(AttemptScopeCoordinate {
            key: self.key.0,
            slot,
            serial,
        })
    }

    /// Closes exactly the active LIFO suffix and restores its surviving
    /// parent. Validation completes before any table or scope scalar mutates.
    pub(crate) fn close_owned_scope(
        &mut self,
        coordinate: AttemptScopeCoordinate,
    ) -> Result<(), AttemptError> {
        let index = self.validate_scope_coordinate(coordinate)?;
        if index + 1 != self.scopes.len() {
            return Err(AttemptError::InvalidCoordinate);
        }
        let close_through = self
            .scopes
            .last()
            .expect("validated scope coordinate has a row")
            .close_through;
        self.validate_mark(close_through)?;
        self.truncate(close_through)
            .expect("scope marks were preflighted before mutation");
        Ok(())
    }

    /// Marks a semantic scope retired by the active direct operation.
    ///
    /// The scope row remains owned until every younger child closes. This is
    /// the exact LIFO analogue of TeX retiring an outer macro while a scanner
    /// or newly matched macro still owns a nested suffix.
    pub(crate) fn retire_owned_scope(
        &mut self,
        coordinate: AttemptScopeCoordinate,
        operation: AttemptScopeCoordinate,
    ) -> Result<(), AttemptError> {
        let index = self.validate_retirement(coordinate, operation)?;
        self.scopes[index].retirement = Some(AttemptScopeRetirement::Immediate { operation });
        self.close_retired_scope_tail()
    }

    /// Defers reclamation until the direct operation has consumed or promoted
    /// every value returned by this scope.
    pub(crate) fn defer_owned_scope_retirement(
        &mut self,
        coordinate: AttemptScopeCoordinate,
        operation: AttemptScopeCoordinate,
    ) -> Result<(), AttemptError> {
        let index = self.validate_retirement(coordinate, operation)?;
        self.scopes[index].retirement =
            Some(AttemptScopeRetirement::AtOperationCommit { operation });
        Ok(())
    }

    /// Commits one operation and closes its contiguous retired scope tail.
    pub(crate) fn commit_owned_operation(
        &mut self,
        operation: AttemptScopeCoordinate,
    ) -> Result<(), AttemptError> {
        let index = self.validate_scope_coordinate(operation)?;
        if self.scopes[index].retirement.is_some() {
            return Err(AttemptError::InvalidCoordinate);
        }
        self.scopes[index].retirement = Some(AttemptScopeRetirement::Immediate { operation });
        self.committed_through = operation.serial;
        self.close_retired_scope_tail()
    }

    /// Rejects one operation and all children opened beneath it, restoring
    /// retirement flags on older scopes before truncating the owned suffix.
    pub(crate) fn rollback_owned_operation(
        &mut self,
        operation: AttemptScopeCoordinate,
    ) -> Result<(), AttemptError> {
        let index = self.validate_scope_coordinate(operation)?;
        let opening = self.scopes[index].opening;
        for scope in &mut self.scopes[..index] {
            if scope
                .retirement
                .is_some_and(|retirement| retirement.operation() == operation)
            {
                scope.retirement = None;
            }
        }
        self.truncate(opening)
    }

    /// Abandons one scanner scope and every unfinished child below it.
    pub(crate) fn discard_owned_scope_suffix(
        &mut self,
        coordinate: AttemptScopeCoordinate,
    ) -> Result<(), AttemptError> {
        let index = self.validate_scope_coordinate(coordinate)?;
        let opening = self.scopes[index].opening;
        self.truncate(opening)
    }

    fn close_retired_scope_tail(&mut self) -> Result<(), AttemptError> {
        while self.tail_scope_is_reclaimable() {
            let close_through = self
                .scopes
                .last()
                .expect("retired scope tail is nonempty")
                .close_through;
            self.validate_mark(close_through)?;
            self.truncate(close_through)?;
        }
        Ok(())
    }

    fn validate_retirement(
        &self,
        coordinate: AttemptScopeCoordinate,
        operation: AttemptScopeCoordinate,
    ) -> Result<usize, AttemptError> {
        let index = self.validate_scope_coordinate(coordinate)?;
        let operation_index = self.validate_scope_coordinate(operation)?;
        if self.scopes[operation_index].retirement.is_some()
            || self.scopes[index].retirement.is_some()
        {
            return Err(AttemptError::InvalidCoordinate);
        }
        Ok(index)
    }

    fn tail_scope_is_reclaimable(&self) -> bool {
        let Some(retirement) = self.scopes.last().and_then(|scope| scope.retirement) else {
            return false;
        };
        match retirement {
            AttemptScopeRetirement::Immediate { .. } => true,
            AttemptScopeRetirement::AtOperationCommit { operation } => {
                operation.key == self.key.0 && operation.serial <= self.committed_through
            }
        }
    }

    pub(crate) fn validate_scope_coordinate(
        &self,
        coordinate: AttemptScopeCoordinate,
    ) -> Result<usize, AttemptError> {
        if coordinate.key != self.key.0 {
            return Err(AttemptError::ForeignAttempt);
        }
        let index = coordinate.slot as usize;
        let owned = self
            .scopes
            .get(index)
            .filter(|owned| owned.key == self.key.0 && owned.serial == coordinate.serial)
            .ok_or(AttemptError::InvalidCoordinate)?;
        let expected_parent = index
            .checked_sub(1)
            .map_or(AttemptScopeSerial::ROOT, |parent| {
                self.scopes[parent].serial
            });
        if owned.parent != expected_parent {
            return Err(AttemptError::InvalidCoordinate);
        }
        self.validate_mark(owned.opening)?;
        self.validate_mark(owned.close_through)?;
        Ok(index)
    }

    fn current_scope(&self) -> AttemptScopeSerial {
        self.scopes
            .last()
            .map_or(AttemptScopeSerial::ROOT, |scope| scope.serial)
    }

    fn with_child_scope<R>(
        &mut self,
        operation: impl for<'scope> FnOnce(&mut AttemptScope<'_, 'scope, G>) -> R,
    ) -> Result<R, AttemptError> {
        let coordinate = self.begin_owned_scope()?;
        let mut scope = AttemptScope {
            arena: self,
            coordinate: Some(coordinate),
            _scope: PhantomData,
        };
        let result = operation(&mut scope);
        drop(scope);
        Ok(result)
    }

    #[must_use]
    pub(crate) fn mark(&self) -> AttemptMark {
        AttemptMark {
            key: self.key.0,
            scope_depth: u32::try_from(self.scopes.len()).expect("attempt scope depth is bounded"),
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
            argument_words: u32::try_from(self.argument_words.len())
                .expect("attempt argument-word length is bounded"),
            argument_records: u32::try_from(self.argument_records.len())
                .expect("attempt argument-record length is bounded"),
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
            (mark.scope_depth as usize, self.scopes.len()),
            (mark.traced_words as usize, self.traced_words.len()),
            (mark.traced_origins as usize, self.traced_origins.len()),
            (mark.token_scratch as usize, self.token_scratch.len()),
            (mark.origin_scratch as usize, self.origin_scratch.len()),
            (mark.token_builders as usize, self.token_builders.len()),
            (mark.token_lists as usize, self.token_lists.len()),
            (mark.glue_values as usize, self.glue_values.len()),
            (mark.definitions as usize, self.definitions.len()),
            (mark.argument_words as usize, self.argument_words.len()),
            (mark.argument_records as usize, self.argument_records.len()),
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

    /// Rejects a suffix in constant time per table. No value is inspected.
    pub(crate) fn truncate(&mut self, mark: AttemptMark) -> Result<(), AttemptError> {
        self.validate_mark(mark)?;
        self.scopes.truncate(mark.scope_depth as usize);
        // Child coordinates disappear before their backing suffixes.
        #[cfg(test)]
        self.names.truncate(mark.names as usize);
        self.provenance.truncate(mark.provenance as usize);
        self.argument_records
            .truncate(mark.argument_records as usize);
        self.argument_words.truncate(mark.argument_words as usize);
        while self.token_buffers.len() > mark.token_buffers as usize {
            let mut buffer = self
                .token_buffers
                .pop()
                .expect("attempt token-buffer suffix is nonempty")
                .value
                .words;
            buffer.clear();
            if buffer.capacity() != 0 {
                self.recycled_token_buffers.push(buffer);
            }
        }
        self.definitions.truncate(mark.definitions as usize);
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
    ) -> Result<&[TracedTokenWord], AttemptError> {
        self.validate_key(id.key)?;
        let row = self
            .token_lists
            .get(id.index())
            .filter(|row| row.serial == id.serial)
            .ok_or(AttemptError::InvalidCoordinate)?;
        match &row.value {
            AttemptTokenStorage::Range(range) => range.resolve(&self.traced_words),
            AttemptTokenStorage::Buffer(buffer) => self.token_buffer(*buffer),
            AttemptTokenStorage::PendingBuffer => Err(AttemptError::InvalidCoordinate),
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
            AttemptTokenStorage::PendingBuffer => Err(AttemptError::InvalidCoordinate),
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

    pub(crate) fn allocate_definition(
        &mut self,
        parameter_text: AttemptTokenListId,
        replacement_text: AttemptTokenListId,
    ) -> Result<AttemptDefinitionId, AttemptError> {
        #[cfg(feature = "profiling")]
        let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::AttemptScratch,
        );
        self.token_words(parameter_text)?;
        self.token_words(replacement_text)?;
        let id = AttemptDefinitionId::new(self.key, self.definitions.len())?;
        self.definitions
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.definitions.push(AttemptRow {
            serial: id.serial,
            value: AttemptDefinition {
                parameter_text,
                replacement_text,
            },
        });
        Ok(id)
    }

    /// Stores one macro activation's argument ranges without cloning tokens.
    pub(crate) fn allocate_arguments(
        &mut self,
        arguments: &[AttemptTokenListId],
    ) -> Result<AttemptArgumentRecordId, AttemptError> {
        #[cfg(feature = "profiling")]
        let _allocation_scope = tex_state::measurement::hot_core_allocation_scope(
            tex_state::measurement::HotCoreAllocationOwner::AttemptScratch,
        );
        if arguments.len() > 9 {
            return Err(AttemptError::InvalidCoordinate);
        }
        for &argument in arguments {
            self.token_words(argument)?;
        }
        let range = AttemptRange::checked(self.argument_words.len(), arguments.len())?;
        let id = AttemptArgumentRecordId::new(self.key, self.argument_records.len())?;
        self.argument_words
            .try_reserve(arguments.len())
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.argument_records
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.argument_words.extend_from_slice(arguments);
        self.argument_records.push(AttemptRow {
            serial: id.serial,
            value: range,
        });
        Ok(id)
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
                words: self.recycled_token_buffers.pop().unwrap_or_default(),
                result,
            },
        });
        Ok(id)
    }

    pub(crate) fn token_buffer(
        &self,
        id: AttemptTokenBufferId,
    ) -> Result<&[TracedTokenWord], AttemptError> {
        self.validate_key(id.key)?;
        self.token_buffers
            .get(id.index())
            .filter(|row| row.serial == id.serial)
            .map(|row| row.value.words.as_slice())
            .ok_or(AttemptError::InvalidCoordinate)
    }

    fn token_buffer_mut(
        &mut self,
        id: AttemptTokenBufferId,
    ) -> Result<&mut Vec<TracedTokenWord>, AttemptError> {
        self.validate_key(id.key)?;
        self.token_buffers
            .get_mut(id.index())
            .filter(|row| row.serial == id.serial)
            .map(|row| &mut row.value.words)
            .ok_or(AttemptError::InvalidCoordinate)
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
        self.token_buffer_mut(id)?
            .try_reserve(1)
            .map_err(|_| AttemptError::AllocationFailed)?;
        self.token_buffer_mut(id)?.push(word);
        Ok(())
    }

    pub(crate) fn drain_buffer_prefix(
        &mut self,
        id: AttemptTokenBufferId,
        len: usize,
    ) -> Result<Vec<TracedTokenWord>, AttemptError> {
        let buffer = self.token_buffer_mut(id)?;
        if len > buffer.len() {
            return Err(AttemptError::InvalidCoordinate);
        }
        Ok(buffer.drain(..len).collect())
    }

    pub(crate) fn strip_buffer_outer_group(
        &mut self,
        id: AttemptTokenBufferId,
    ) -> Result<(), AttemptError> {
        let buffer = self.token_buffer_mut(id)?;
        if buffer.len() >= 2 {
            buffer.pop();
            buffer.remove(0);
        }
        Ok(())
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

    pub(crate) fn arguments(
        &self,
        id: AttemptArgumentRecordId,
    ) -> Result<&[AttemptTokenListId], AttemptError> {
        self.validate_key(id.key)?;
        let row = self
            .argument_records
            .get(id.index())
            .copied()
            .filter(|row| row.serial == id.serial)
            .ok_or(AttemptError::InvalidCoordinate)?;
        row.value.resolve(&self.argument_words)
    }

    pub(crate) fn argument(
        &self,
        id: AttemptArgumentRecordId,
        slot: u8,
    ) -> Result<Option<AttemptTokenListId>, AttemptError> {
        if !(1..=9).contains(&slot) {
            return Err(AttemptError::InvalidCoordinate);
        }
        Ok(self.arguments(id)?.get(usize::from(slot - 1)).copied())
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

    /// Copies only declared roots and schema-declared definition children.
    ///
    /// This is the multi-root cold preparation path. Its inline root staging
    /// spills only when a declared batch exceeds four unique roots; work and
    /// storage are proportional to that declaration, never to unrelated live
    /// attempt rows. Ordinary one-definition promotion uses
    /// [`Self::promote_definition`] and allocates no staging or receipt.
    pub(crate) fn promote(
        &self,
        universe: &mut Universe<G>,
        roots: AttemptEscapeRoots<'_>,
    ) -> Result<AttemptPromotion<G>, AttemptError> {
        let mut token_sources =
            smallvec::SmallVec::<[(AttemptTokenListId, Vec<TokenWord>); 4]>::new();
        let mut glue_sources = smallvec::SmallVec::<[(AttemptGlueId, GlueSpec); 4]>::new();
        let mut definition_sources =
            smallvec::SmallVec::<[(AttemptDefinitionId, Vec<TokenWord>, Vec<TokenWord>); 4]>::new();
        let mut provenance_sources =
            smallvec::SmallVec::<[(AttemptProvenanceId, OriginRecord); 4]>::new();

        for &id in roots.token_lists {
            self.validate_key(id.key)?;
            self.token_words(id)?;
            if !token_sources.iter().any(|(source, _)| *source == id) {
                token_sources.push((id, self.semantic_words(id)?));
            }
        }
        for &id in roots.glue {
            self.validate_key(id.key)?;
            let glue = self.glue(id)?;
            if !glue_sources.iter().any(|(source, _)| *source == id) {
                glue_sources.push((id, glue));
            }
        }
        for &id in roots.definitions {
            self.validate_key(id.key)?;
            let row = self
                .definitions
                .get(id.index())
                .copied()
                .filter(|row| row.serial == id.serial)
                .ok_or(AttemptError::InvalidCoordinate)?;
            if definition_sources
                .iter()
                .any(|(source, _, _)| *source == id)
            {
                continue;
            }
            let definition = row.value;
            // The two token ranges are schema-declared children. Definition
            // text is copied into DefinitionArena directly, not published as
            // independent durable token-list rows unless separately rooted.
            let parameter_text = self.semantic_words(definition.parameter_text)?;
            let replacement_text = self.semantic_words(definition.replacement_text)?;
            definition_sources.push((id, parameter_text, replacement_text));
        }
        for &id in roots.provenance {
            self.validate_key(id.key)?;
            let record = self.provenance(id)?;
            if !provenance_sources.iter().any(|(source, _)| *source == id) {
                provenance_sources.push((id, record));
            }
        }

        let definitions = definition_sources
            .iter()
            .map(
                |(_, parameter_text, replacement_text)| DefinitionPromotion {
                    parameter_text,
                    replacement_text,
                },
            )
            .collect::<smallvec::SmallVec<[_; 4]>>();
        let token_lists = token_sources
            .iter()
            .map(|(_, words)| TokenListPromotion { words })
            .collect::<smallvec::SmallVec<[_; 4]>>();
        let glue_values = glue_sources
            .iter()
            .map(|(_, glue)| *glue)
            .collect::<smallvec::SmallVec<[_; 4]>>();
        let provenance = provenance_sources
            .iter()
            .map(|(_, record)| *record)
            .collect::<smallvec::SmallVec<[_; 4]>>();
        let receipt =
            universe.promote_values(&definitions, &token_lists, &glue_values, &provenance)?;

        Ok(AttemptPromotion {
            token_lists: roots
                .token_lists
                .iter()
                .map(|id| {
                    let index = token_sources
                        .iter()
                        .position(|(source, _)| source == id)
                        .expect("declared token root was promoted");
                    receipt.token_lists[index]
                })
                .collect(),
            glue: roots
                .glue
                .iter()
                .map(|id| {
                    let index = glue_sources
                        .iter()
                        .position(|(source, _)| source == id)
                        .expect("declared glue root was promoted");
                    receipt.glue[index]
                })
                .collect(),
            definitions: roots
                .definitions
                .iter()
                .map(|id| {
                    let index = definition_sources
                        .iter()
                        .position(|(source, _, _)| source == id)
                        .expect("declared definition root was promoted");
                    receipt.definitions[index]
                })
                .collect(),
            provenance: roots
                .provenance
                .iter()
                .map(|id| {
                    let index = provenance_sources
                        .iter()
                        .position(|(source, _)| source == id)
                        .expect("declared provenance root was promoted");
                    receipt.provenance[index]
                })
                .collect(),
        })
    }

    /// Promotes one macro definition without arena-sized relocation tables,
    /// temporary semantic-word vectors, or a heap-allocated receipt.
    pub(crate) fn promote_definition(
        &self,
        universe: &mut Universe<G>,
        id: AttemptDefinitionId,
    ) -> Result<DefinitionId<G>, AttemptError> {
        self.validate_key(id.key)?;
        let definition = self
            .definitions
            .get(id.index())
            .copied()
            .filter(|row| row.serial == id.serial)
            .ok_or(AttemptError::InvalidCoordinate)?
            .value;
        let parameter_text = self.token_words(definition.parameter_text)?;
        let replacement_text = self.token_words(definition.replacement_text)?;
        universe
            .promote_definition_from_words(
                parameter_text
                    .iter()
                    .copied()
                    .map(TracedTokenWord::token_word),
                replacement_text
                    .iter()
                    .copied()
                    .map(TracedTokenWord::token_word),
            )
            .map_err(AttemptError::from)
    }

    fn semantic_words(&self, id: AttemptTokenListId) -> Result<Vec<TokenWord>, AttemptError> {
        Ok(self
            .token_words(id)?
            .iter()
            .map(|word| word.token_word())
            .collect())
    }

    fn validate_key(&self, key: NonZeroU64) -> Result<(), AttemptError> {
        if key == self.key.0 {
            Ok(())
        } else {
            Err(AttemptError::ForeignAttempt)
        }
    }
}

/// Explicit roots permitted to escape one command operation.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AttemptEscapeRoots<'a> {
    pub(crate) token_lists: &'a [AttemptTokenListId],
    pub(crate) glue: &'a [AttemptGlueId],
    pub(crate) definitions: &'a [AttemptDefinitionId],
    pub(crate) provenance: &'a [AttemptProvenanceId],
}

/// Durable coordinates returned in the caller's declared root order.
#[derive(Debug)]
pub(crate) struct AttemptPromotion<G> {
    pub(crate) token_lists: Vec<TokenListId<G>>,
    pub(crate) glue: Vec<GlueId<G>>,
    pub(crate) definitions: Vec<DefinitionId<G>>,
    pub(crate) provenance: Vec<ProvenanceId<G>>,
}

/// Opaque owner transferred between consecutive command operations.
///
/// Macro activations and other command continuations may intentionally keep
/// attempt coordinates live across more than one delivered command. The
/// owner moves; individual ids never retain it.
pub struct CommandAttempt<G> {
    arena: AttemptArena<G>,
}

/// Fixed-size rollback coordinate for one command operation.
///
/// Construction is restricted to [`crate::CommandState`]. The coordinate
/// carries no storage owner and is valid only while the matching live attempt
/// remains installed in that state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandAttemptMark {
    opening: AttemptMark,
    operation: AttemptScopeCoordinate,
}

impl CommandAttemptMark {
    pub(crate) const fn new(opening: AttemptMark, operation: AttemptScopeCoordinate) -> Self {
        Self { opening, operation }
    }

    pub(crate) const fn attempt_mark(self) -> AttemptMark {
        self.opening
    }

    pub(crate) const fn operation_scope(self) -> AttemptScopeCoordinate {
        self.operation
    }
}

impl<G> core::fmt::Debug for CommandAttempt<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("CommandAttempt(..)")
    }
}

impl<G> Default for CommandAttempt<G> {
    fn default() -> Self {
        Self {
            arena: AttemptArena::default(),
        }
    }
}

impl<G> CommandAttempt<G> {
    /// Runs one synchronous child allocation episode under a fresh invariant
    /// brand and closes it in exact LIFO order before returning.
    ///
    /// The higher-ranked callback prevents both its capability and every id
    /// allocated through it from escaping. Dynamic scanner/macro frames use
    /// the crate-private owned-scope boundary instead because they may suspend.
    pub fn with_scope<R>(
        &mut self,
        operation: impl for<'scope> FnOnce(&mut AttemptScope<'_, 'scope, G>) -> R,
    ) -> Result<R, AttemptError> {
        self.arena.with_child_scope(operation)
    }

    pub(crate) const fn arena(&self) -> &AttemptArena<G> {
        &self.arena
    }

    pub(crate) const fn arena_mut(&mut self) -> &mut AttemptArena<G> {
        &mut self.arena
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.arena.mark().is_empty()
    }
}

/// Integer-only state-machine position retained at a resource barrier.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AttemptResumePoint {
    pub command: u32,
    pub scanner: u32,
    pub expansion: u32,
    pub subordinate: u32,
}

/// Complete in-process command suspension package.
///
/// `R` owns the typed request and resume variant. No field borrows either the
/// attempt or generation. The opening mark and state-machine resume point are
/// fixed-size integer coordinates into this owned attempt. Resumption consumes
/// the package, validates both the coarse generation and opening mark, drops
/// that extra owner, and only then re-borrows live storage through `Universe`.
pub struct PendingCommandAttempt<G, R> {
    attempt: Box<CommandAttempt<G>>,
    generation: GenerationOwner<G>,
    opening: CommandAttemptMark,
    resume: AttemptResumePoint,
    pending: R,
}

impl<G, R> PendingCommandAttempt<G, R> {
    pub(crate) const fn opening(&self) -> CommandAttemptMark {
        self.opening
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) fn new(
        mut attempt: CommandAttempt<G>,
        generation: GenerationOwner<G>,
        resume: AttemptResumePoint,
        pending: R,
    ) -> Self {
        let opening_mark = attempt.arena().mark();
        let operation = attempt
            .arena_mut()
            .begin_owned_scope()
            .expect("test pending attempt opens an operation scope");
        let opening = CommandAttemptMark::new(opening_mark, operation);
        Self {
            attempt: Box::new(attempt),
            generation,
            opening,
            resume,
            pending,
        }
    }

    pub(crate) fn new_at_validated_mark(
        attempt: CommandAttempt<G>,
        generation: GenerationOwner<G>,
        opening: CommandAttemptMark,
        resume: AttemptResumePoint,
        pending: R,
    ) -> Self {
        debug_assert!(
            attempt
                .arena()
                .validate_mark(opening.attempt_mark())
                .is_ok()
                && attempt
                    .arena()
                    .validate_scope_coordinate(opening.operation_scope())
                    .is_ok(),
            "a pending attempt may retain only its own validated opening cursor"
        );
        Self {
            attempt: Box::new(attempt),
            generation,
            opening,
            resume,
            pending,
        }
    }

    #[allow(
        clippy::result_large_err,
        reason = "stale admission must return the complete move-only continuation without a lifecycle allocation"
    )]
    pub(crate) fn resume(
        self,
        universe: &Universe<G>,
    ) -> Result<(CommandAttempt<G>, CommandAttemptMark, AttemptResumePoint, R), Self> {
        if !universe.owns_generation(&self.generation)
            || self
                .attempt
                .arena()
                .validate_mark(self.opening.attempt_mark())
                .is_err()
            || self
                .attempt
                .arena()
                .validate_scope_coordinate(self.opening.operation_scope())
                .is_err()
        {
            return Err(self);
        }
        let Self {
            attempt,
            generation,
            opening,
            resume,
            pending,
        } = self;
        drop(generation);
        Ok((*attempt, opening, resume, pending))
    }
}
