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
    DefinitionBuildError, DefinitionBuilder, DefinitionId, DefinitionIdentityPolicy,
    GenerationOwner, GlueId, PromotionError, ProvenanceId, TokenListId, Universe,
};

mod token_lane;

use token_lane::{TokenLane, TokenLaneView, TokenSink};

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

/// Checked non-owning identity paired with one logical scope owner.
///
/// The private [`OwnedAttemptScope`] is the sole linear owner. This identity
/// may align cloneable command semantics with that move-only owner, but it
/// cannot close a scope or address storage by itself.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct AttemptScopeCoordinate {
    key: NonZeroU64,
    serial: AttemptScopeSerial,
}

/// Linear capability owning one dynamically suspended attempt scope.
///
/// A Rust lifetime cannot brand a scanner or macro frame which unwinds across
/// a resource barrier and resumes later. Those frames use this smallest
/// private runtime boundary instead: one checked serial and two fixed marks.
/// Construction is arena-private, the value is neither `Copy` nor `Clone`,
/// and exact close consumes it.
#[derive(Debug, Eq, Hash, PartialEq)]
#[must_use = "an owned attempt scope must be closed or moved into a continuation"]
pub(crate) struct OwnedAttemptScope {
    key: NonZeroU64,
    serial: AttemptScopeSerial,
    parent: AttemptScopeSerial,
    opening: AttemptMark,
    close_through_serial: AttemptScopeSerial,
    close_through: AttemptMark,
}

impl OwnedAttemptScope {
    pub(crate) const fn coordinate(&self) -> AttemptScopeCoordinate {
        AttemptScopeCoordinate {
            key: self.key,
            serial: self.serial,
        }
    }

    pub(crate) fn is_direct_child_of(&self, parent: &Self) -> bool {
        self.key == parent.key && self.parent == parent.serial
    }
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
    owner: Option<OwnedAttemptScope>,
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
                .owner
                .as_ref()
                .expect("a lexical attempt scope remains open")
                .serial,
            _scope: PhantomData,
        })
    }

    pub fn token_words(
        &self,
        id: &ScopedAttemptTokenListId<'scope>,
    ) -> Result<AttemptTokenListView<'_>, AttemptError> {
        if self
            .owner
            .as_ref()
            .is_none_or(|owner| owner.serial != id.scope)
        {
            return Err(AttemptError::InvalidCoordinate);
        }
        self.arena.token_words(id.id)
    }
}

impl<G> Drop for AttemptScope<'_, '_, G> {
    fn drop(&mut self) {
        let Some(owner) = self.owner.take() else {
            return;
        };
        self.arena
            .close_owned_scope(owner)
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

/// Failure to move one live attempt across an in-process resource boundary.
///
/// A stale opening coordinate is distinguished from failure to retain the
/// current generation. Both are detected before the live arena is moved.
#[derive(Debug)]
pub enum AttemptSuspendError {
    StaleMark(AttemptError),
    Generation(tex_state::UniverseError),
}

/// Rejected suspension together with the still-live operation capability.
///
/// Suspension validates every coordinate before moving the attempt arena, so
/// failure leaves command state unchanged. Returning the capability prevents
/// that unchanged operation from losing its only caller owner.
#[derive(Debug)]
pub struct AttemptSuspendFailure {
    operation: CommandAttemptOperation,
    error: AttemptSuspendError,
}

impl AttemptSuspendFailure {
    pub(crate) const fn new(
        operation: CommandAttemptOperation,
        error: AttemptSuspendError,
    ) -> Self {
        Self { operation, error }
    }

    #[must_use]
    pub const fn error(&self) -> &AttemptSuspendError {
        &self.error
    }

    pub fn into_parts(self) -> (CommandAttemptOperation, AttemptSuspendError) {
        (self.operation, self.error)
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
    top_scope: AttemptScopeSerial,
    next_scope: AttemptScopeSerial,
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

impl<G> Default for AttemptArena<G> {
    fn default() -> Self {
        Self {
            key: AttemptKey::fresh(),
            top_scope: AttemptScopeSerial::ROOT,
            next_scope: AttemptScopeSerial::ROOT
                .checked_successor()
                .expect("the root attempt scope has a successor"),
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
    /// Opens one dynamic child of the exact currently active scope.
    ///
    /// The returned linear capability may move into an in-process
    /// continuation. No row, side table, or heap owner is allocated.
    pub(crate) fn begin_owned_scope(&mut self) -> Result<OwnedAttemptScope, AttemptError> {
        let serial = self.next_scope;
        let next_scope = serial.checked_successor()?;
        let opening = self.mark();
        let owned = OwnedAttemptScope {
            key: self.key.0,
            serial,
            parent: self.top_scope,
            opening,
            close_through_serial: serial,
            close_through: opening,
        };
        self.next_scope = next_scope;
        self.top_scope = serial;
        Ok(owned)
    }

    /// Closes exactly the active LIFO suffix and restores its surviving
    /// parent. Validation completes before any table or scope scalar mutates.
    pub(crate) fn close_owned_scope(
        &mut self,
        owner: OwnedAttemptScope,
    ) -> Result<(), AttemptError> {
        self.validate_top_owner(&owner)?;
        self.validate_mark(owner.close_through)?;
        self.truncate(owner.close_through)?;
        self.top_scope = owner.parent;
        Ok(())
    }

    /// Transfers a retired logical parent into its still-live direct child.
    ///
    /// Both capabilities remain inline. No arena row is allocated or searched,
    /// and the child will eventually close through the parent's opening mark.
    pub(crate) fn handoff_owned_parent(
        &self,
        parent: OwnedAttemptScope,
        child: &mut OwnedAttemptScope,
    ) -> Result<(), AttemptError> {
        self.validate_owner(&parent)?;
        self.validate_owner(child)?;
        if child.parent != parent.serial {
            return Err(AttemptError::InvalidCoordinate);
        }
        child.parent = parent.parent;
        child.close_through_serial = parent.close_through_serial;
        child.close_through = parent.close_through;
        Ok(())
    }

    fn validate_owner(&self, owner: &OwnedAttemptScope) -> Result<(), AttemptError> {
        if owner.key != self.key.0 {
            return Err(AttemptError::ForeignAttempt);
        }
        if owner.serial >= self.next_scope || owner.parent >= owner.serial {
            return Err(AttemptError::InvalidCoordinate);
        }
        Ok(())
    }

    fn validate_top_owner(&self, owner: &OwnedAttemptScope) -> Result<(), AttemptError> {
        self.validate_owner(owner)?;
        if self.top_scope != owner.serial {
            return Err(AttemptError::InvalidCoordinate);
        }
        Ok(())
    }

    fn with_child_scope<R>(
        &mut self,
        operation: impl for<'scope> FnOnce(&mut AttemptScope<'_, 'scope, G>) -> R,
    ) -> Result<R, AttemptError> {
        let owner = self.begin_owned_scope()?;
        let mut scope = AttemptScope {
            arena: self,
            owner: Some(owner),
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

    pub(crate) fn allocate_definition_builder(
        &mut self,
        policy: DefinitionIdentityPolicy,
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
        let mut builder = self
            .recycled_definition_builders
            .pop()
            .unwrap_or_else(|| DefinitionBuilder::new(policy));
        builder.reset(policy);
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

    /// Publishes declared roots directly from their attempt-owned storage.
    ///
    /// Complete source validation and destination reservation happen while the
    /// attempt rows remain in place. Token words then stream once into their
    /// final publisher, and successful definition publication recycles the
    /// original builders. Failure leaves every attempt row untouched. Duplicate
    /// declarations share the one published owner without an id relocation
    /// table or a payload-bearing batch carrier.
    pub(crate) fn promote(
        &mut self,
        universe: &mut Universe<G>,
        roots: AttemptEscapeRoots<'_>,
    ) -> Result<AttemptPromotion<G>, AttemptError> {
        for &id in roots.token_lists {
            self.validate_key(id.key)?;
            self.token_words(id)?;
        }
        for &id in roots.glue {
            self.validate_key(id.key)?;
            self.glue(id)?;
        }
        for &id in roots.definitions {
            self.validate_key(id.key)?;
            self.definition_builder(id)?;
        }
        for &id in roots.provenance {
            self.validate_key(id.key)?;
            self.provenance(id)?;
        }

        let definition_count = unique_count(roots.definitions);
        self.recycled_definition_builders
            .try_reserve(definition_count)
            .map_err(|_| AttemptError::AllocationFailed)?;

        let definitions = roots
            .definitions
            .iter()
            .enumerate()
            .filter(|(index, _)| is_first_occurrence(roots.definitions, *index))
            .map(|(_, id)| {
                self.definition_builder(*id)
                    .expect("the complete definition source set was validated")
            });
        let token_lists = roots
            .token_lists
            .iter()
            .enumerate()
            .filter(|(index, _)| is_first_occurrence(roots.token_lists, *index))
            .map(|(_, id)| {
                self.token_words(*id)
                    .expect("the complete token source set was validated")
                    .iter()
                    .map(|word| word.token_word())
            });
        let glue_values = roots
            .glue
            .iter()
            .enumerate()
            .filter(|(index, _)| is_first_occurrence(roots.glue, *index))
            .map(|(_, id)| {
                self.glue(*id)
                    .expect("the complete glue source set was validated")
            });
        let provenance = roots
            .provenance
            .iter()
            .enumerate()
            .filter(|(index, _)| is_first_occurrence(roots.provenance, *index))
            .map(|(_, id)| {
                self.provenance(*id)
                    .expect("the complete provenance source set was validated")
            });
        let receipt = universe
            .promote_value_streams(definitions, token_lists, glue_values, provenance)
            .map_err(AttemptError::from)?;

        for (index, &id) in roots.definitions.iter().enumerate() {
            if !is_first_occurrence(roots.definitions, index) {
                continue;
            }
            let builder = self
                .definitions
                .get_mut(id.index())
                .filter(|row| row.serial == id.serial)
                .and_then(|row| row.value.builder.take())
                .expect("successful publication retains the original attempt builder");
            self.recycled_definition_builders.push(builder);
        }

        Ok(AttemptPromotion {
            token_lists: roots
                .token_lists
                .iter()
                .map(|id| {
                    let index = unique_index(roots.token_lists, id);
                    receipt.token_lists[index].clone()
                })
                .collect(),
            glue: roots
                .glue
                .iter()
                .map(|id| {
                    let index = unique_index(roots.glue, id);
                    receipt.glue[index]
                })
                .collect(),
            definitions: roots
                .definitions
                .iter()
                .map(|id| {
                    let index = unique_index(roots.definitions, id);
                    receipt.definitions[index].clone()
                })
                .collect(),
            provenance: roots
                .provenance
                .iter()
                .map(|id| {
                    let index = unique_index(roots.provenance, id);
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
            .filter(|row| row.serial == id.serial)
            .and_then(|row| row.value.builder.as_ref())
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

/// Explicit roots permitted to escape one command operation.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AttemptEscapeRoots<'a> {
    pub(crate) token_lists: &'a [AttemptTokenListId],
    pub(crate) glue: &'a [AttemptGlueId],
    pub(crate) definitions: &'a [AttemptDefinitionId],
    pub(crate) provenance: &'a [AttemptProvenanceId],
}

fn is_first_occurrence<T: PartialEq>(values: &[T], index: usize) -> bool {
    !values[..index].contains(&values[index])
}

fn unique_count<T: PartialEq>(values: &[T]) -> usize {
    values
        .iter()
        .enumerate()
        .filter(|(index, _)| is_first_occurrence(values, *index))
        .count()
}

fn unique_index<T: PartialEq>(values: &[T], value: &T) -> usize {
    let first = values
        .iter()
        .position(|candidate| candidate == value)
        .expect("a declared promotion root is present in its source slice");
    unique_count(&values[..first])
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
/// Scanner continuations may intentionally keep attempt coordinates live
/// across more than one delivered command. The owner moves; individual ids
/// never retain it. Macro activations use disjoint generation-owned scratch.
pub struct CommandAttempt<G> {
    arena: AttemptArena<G>,
    active_operation: Option<OwnedAttemptScope>,
    active_operation_origin: Option<AttemptScopeCoordinate>,
}

/// Fixed-size rollback coordinate for one command operation.
///
/// Construction is restricted to [`crate::CommandState`]. The coordinate
/// carries no storage owner and is valid only while the matching live attempt
/// remains installed in that state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CommandAttemptMark {
    opening: AttemptMark,
    operation: AttemptScopeCoordinate,
    parent: AttemptScopeSerial,
    macro_depth: u32,
}

/// Move-only caller capability for one active command operation.
///
/// Command state retains the matching non-owning coordinate for validation,
/// while the executor must move this value into the exact continuation that
/// can resume, commit, or roll the operation back.  It cannot reconstruct an
/// owner from command state after discarding the caller edge.
#[derive(Debug, Eq, PartialEq)]
#[must_use = "an active command operation must be finished or moved into its continuation"]
pub struct CommandAttemptOperation {
    mark: CommandAttemptMark,
}

impl CommandAttemptOperation {
    pub(crate) const fn new(mark: CommandAttemptMark) -> Self {
        Self { mark }
    }

    pub(crate) const fn coordinate(&self) -> CommandAttemptMark {
        self.mark
    }
}

impl CommandAttemptMark {
    fn new(
        opening: AttemptMark,
        operation: &OwnedAttemptScope,
        macro_depth: usize,
    ) -> Result<Self, AttemptError> {
        Ok(Self {
            opening,
            operation: operation.coordinate(),
            parent: operation.parent,
            macro_depth: u32::try_from(macro_depth).map_err(|_| AttemptError::CapacityOverflow)?,
        })
    }

    pub(crate) const fn attempt_mark(self) -> AttemptMark {
        self.opening
    }

    pub(crate) const fn macro_depth(self) -> usize {
        self.macro_depth as usize
    }
}

/// Move-only capability for one synchronous child of the active command
/// operation.
///
/// Construction and consumption are restricted to [`crate::CommandState`].
/// The child owns only its exact attempt-arena suffix; semantic command roots
/// remain parent-owned and are not restored when the child closes.
#[derive(Debug)]
pub struct CommandAttemptChildScope {
    owner: OwnedAttemptScope,
}

impl CommandAttemptChildScope {
    pub(crate) const fn new(owner: OwnedAttemptScope) -> Self {
        Self { owner }
    }

    pub(crate) fn into_owner(self) -> OwnedAttemptScope {
        self.owner
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
            active_operation: None,
            active_operation_origin: None,
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

    pub(crate) fn begin_operation(
        &mut self,
        macro_depth: usize,
    ) -> Result<CommandAttemptMark, AttemptError> {
        if self.active_operation.is_some() || self.active_operation_origin.is_some() {
            return Err(AttemptError::InvalidCoordinate);
        }
        let opening = self.arena.mark();
        let owner = self.arena.begin_owned_scope()?;
        let mark = CommandAttemptMark::new(opening, &owner, macro_depth)?;
        self.active_operation_origin = Some(owner.coordinate());
        self.active_operation = Some(owner);
        Ok(mark)
    }

    pub(crate) fn validate_operation(&self, mark: CommandAttemptMark) -> Result<(), AttemptError> {
        self.arena.validate_mark(mark.opening)?;
        if mark.operation.key != self.arena.key.0 {
            return Err(AttemptError::InvalidCoordinate);
        }
        let owns_operation =
            self.active_operation.is_some() && self.active_operation_origin == Some(mark.operation);
        owns_operation
            .then_some(())
            .ok_or(AttemptError::InvalidCoordinate)
    }

    pub(crate) fn commit_operation(
        &mut self,
        mark: CommandAttemptMark,
    ) -> Result<(), AttemptError> {
        self.validate_operation(mark)?;
        let operation = self
            .active_operation
            .take()
            .ok_or(AttemptError::InvalidCoordinate)?;
        self.active_operation_origin = None;
        self.arena.close_owned_scope(operation)
    }

    pub(crate) fn rollback_operation(
        &mut self,
        mark: CommandAttemptMark,
    ) -> Result<(), AttemptError> {
        self.validate_operation(mark)?;
        let _operation = self
            .active_operation
            .take()
            .ok_or(AttemptError::InvalidCoordinate)?;
        self.active_operation_origin = None;
        self.arena.validate_mark(mark.opening)?;
        self.arena.truncate(mark.opening)?;
        self.arena.top_scope = mark.parent;
        Ok(())
    }

    pub(crate) fn begin_child_scope(&mut self) -> Result<OwnedAttemptScope, AttemptError> {
        self.arena.begin_owned_scope()
    }

    pub(crate) fn close_child_scope(
        &mut self,
        owner: OwnedAttemptScope,
    ) -> Result<(), AttemptError> {
        self.arena.close_owned_scope(owner)
    }

    pub(crate) fn child_scope_is_direct_operation_child(&self, child: &OwnedAttemptScope) -> bool {
        self.active_operation
            .as_ref()
            .is_some_and(|operation| child.is_direct_child_of(operation))
    }

    pub(crate) fn validate_child_retirement(
        &self,
        child: &OwnedAttemptScope,
    ) -> Result<(), AttemptError> {
        self.arena.validate_top_owner(child)?;
        if self.child_scope_is_direct_operation_child(child) {
            let parent = self
                .active_operation
                .as_ref()
                .ok_or(AttemptError::InvalidCoordinate)?;
            self.arena.validate_owner(parent)?;
            if !child.is_direct_child_of(parent) {
                return Err(AttemptError::InvalidCoordinate);
            }
        } else {
            self.arena.validate_mark(child.close_through)?;
        }
        Ok(())
    }

    pub(crate) fn defer_child_to_operation(
        &mut self,
        mut child: OwnedAttemptScope,
    ) -> Result<(), AttemptError> {
        self.arena.validate_top_owner(&child)?;
        let parent = self
            .active_operation
            .as_ref()
            .ok_or(AttemptError::InvalidCoordinate)?;
        self.arena.validate_owner(parent)?;
        if !child.is_direct_child_of(parent) {
            return Err(AttemptError::InvalidCoordinate);
        }
        let parent = self
            .active_operation
            .take()
            .ok_or(AttemptError::InvalidCoordinate)?;
        self.arena.handoff_owned_parent(parent, &mut child)?;
        self.active_operation = Some(child);
        Ok(())
    }

    pub(crate) const fn arena(&self) -> &AttemptArena<G> {
        &self.arena
    }

    pub(crate) const fn arena_mut(&mut self) -> &mut AttemptArena<G> {
        &mut self.arena
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.arena.mark().is_empty()
            && self.arena.top_scope == AttemptScopeSerial::ROOT
            && self.active_operation.is_none()
            && self.active_operation_origin.is_none()
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
    operation: CommandAttemptOperation,
    resume: AttemptResumePoint,
    pending: R,
}

impl<G, R> PendingCommandAttempt<G, R> {
    pub(crate) const fn operation_coordinate(&self) -> CommandAttemptMark {
        self.operation.coordinate()
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
        let opening = attempt
            .begin_operation(0)
            .expect("test pending attempt opens an operation scope");
        debug_assert_eq!(opening.attempt_mark(), opening_mark);
        Self {
            attempt: Box::new(attempt),
            generation,
            operation: CommandAttemptOperation::new(opening),
            resume,
            pending,
        }
    }

    pub(crate) fn new_at_validated_mark(
        attempt: CommandAttempt<G>,
        generation: GenerationOwner<G>,
        operation: CommandAttemptOperation,
        resume: AttemptResumePoint,
        pending: R,
    ) -> Self {
        let opening = operation.coordinate();
        debug_assert!(
            attempt
                .arena()
                .validate_mark(opening.attempt_mark())
                .is_ok()
                && attempt.validate_operation(opening).is_ok(),
            "a pending attempt may retain only its own validated opening cursor"
        );
        Self {
            attempt: Box::new(attempt),
            generation,
            operation,
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
    ) -> Result<
        (
            CommandAttempt<G>,
            CommandAttemptOperation,
            AttemptResumePoint,
            R,
        ),
        Self,
    > {
        let opening = self.operation.coordinate();
        if !universe.owns_generation(&self.generation)
            || self
                .attempt
                .arena()
                .validate_mark(opening.attempt_mark())
                .is_err()
            || self.attempt.validate_operation(opening).is_err()
        {
            return Err(self);
        }
        let Self {
            attempt,
            generation,
            operation,
            resume,
            pending,
        } = self;
        drop(generation);
        Ok((*attempt, operation, resume, pending))
    }
}
