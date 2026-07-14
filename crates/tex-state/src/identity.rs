//! Rollback-safe identity allocation for timeline-owned stores.
//!
//! Live handles use a slot plus the allocation tag recorded for that slot.
//! Rollback truncates slots but advances the active generation before those
//! slots can be reused. Forks retain inherited tags and mint a fresh namespace
//! for later allocations. Consequently validation is one bounds check and one
//! tag comparison, independent of rollback history length.
//!
//! These runtime capabilities deliberately have no serde implementation.
//! Durable formats serialize semantic DTO references and reconstruct fresh
//! live identities through the aggregate store facade.

use core::num::{NonZeroU32, NonZeroU64};

const BUILTIN_NAMESPACE: NonZeroU64 = NonZeroU64::MIN;
const FIRST_GENERATION: NonZeroU32 = NonZeroU32::MIN;
const RESERVED_NAMESPACE_MAX: u64 = 255;

/// A compact runtime identity embedded by a typed live-store handle.
///
/// Store-specific handle newtypes should wrap this value rather than expose it
/// directly. The two-word representation is intentionally separate from any
/// serialized dense index.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct HandleIdentity {
    namespace: NonZeroU64,
    generation: NonZeroU32,
    slot: u32,
}

impl HandleIdentity {
    /// Returns the universal identity of an immutable canonical store entry.
    ///
    /// Only entries with identical semantics in every store (for example an
    /// empty token list) may use this namespace.
    pub(crate) const fn builtin(slot: u32) -> Self {
        Self {
            namespace: BUILTIN_NAMESPACE,
            generation: FIRST_GENERATION,
            slot,
        }
    }

    pub(crate) const fn slot(self) -> u32 {
        self.slot
    }

    /// Creates an internal tagged payload in a reserved namespace.
    ///
    /// Reserved identities are for non-timeline representations such as
    /// survivor handles and detached format DTO references. They never enter
    /// an `IdentityAllocator` tag table.
    pub(crate) const fn reserved(namespace: u64, upper: NonZeroU32, lower: u32) -> Self {
        assert!(
            namespace > BUILTIN_NAMESPACE.get() && namespace <= RESERVED_NAMESPACE_MAX,
            "reserved identity namespace is out of range"
        );
        Self {
            namespace: match NonZeroU64::new(namespace) {
                Some(value) => value,
                None => panic!("reserved identity namespace must be nonzero"),
            },
            generation: upper,
            slot: lower,
        }
    }

    pub(crate) const fn namespace(self) -> u64 {
        self.namespace.get()
    }

    pub(crate) const fn upper(self) -> u32 {
        self.generation.get()
    }

    pub(crate) const fn lower(self) -> u32 {
        self.slot
    }

    const fn tag(self) -> AllocationTag {
        AllocationTag {
            namespace: self.namespace,
            generation: self.generation,
        }
    }
}

/// An O(1) aggregate-snapshot mark for an identity table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IdentityMark {
    len: usize,
    frontier: Option<AllocationTag>,
}

impl IdentityMark {
    pub(crate) const fn len(self) -> usize {
        self.len
    }
}

/// A bounded failure that never permits identity wrap or history revival.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IdentityError {
    SlotCapacityExhausted,
    GenerationExhausted,
    InvalidatedMark,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct AllocationTag {
    namespace: NonZeroU64,
    generation: NonZeroU32,
}

/// Generation table shared by rollback-truncated store implementations.
///
/// This owns only identity/liveness metadata; semantic store data remains in
/// the owning store and mutation remains behind `Stores`/`Universe`.
#[derive(Debug)]
pub(crate) struct IdentityAllocator {
    active: AllocationTag,
    slots: Vec<AllocationTag>,
    builtin_slots: u32,
}

impl IdentityAllocator {
    /// Creates a fresh identity timeline with `builtin_slots` universal,
    /// immutable prefix entries.
    pub(crate) fn new(builtin_slots: u32) -> Self {
        Self::with_namespace(builtin_slots, fresh_namespace())
    }

    fn with_namespace(builtin_slots: u32, namespace: NonZeroU64) -> Self {
        assert_ne!(
            namespace, BUILTIN_NAMESPACE,
            "the builtin identity namespace is reserved"
        );
        let builtin_len = usize::try_from(builtin_slots).expect("u32 fits usize");
        Self {
            active: AllocationTag {
                namespace,
                generation: FIRST_GENERATION,
            },
            slots: vec![HandleIdentity::builtin(0).tag(); builtin_len],
            builtin_slots,
        }
    }

    /// Copies inherited liveness while giving post-fork allocations a fresh
    /// namespace. Handles inherited from the parent remain valid in both
    /// timelines; handles subsequently minted by either side are foreign to
    /// the other.
    pub(crate) fn fork(&self) -> Self {
        let namespace = loop {
            let candidate = fresh_namespace();
            if candidate != self.active.namespace
                && self.slots.iter().all(|tag| tag.namespace != candidate)
            {
                break candidate;
            }
        };
        Self {
            active: AllocationTag {
                namespace,
                generation: FIRST_GENERATION,
            },
            slots: self.slots.clone(),
            builtin_slots: self.builtin_slots,
        }
    }

    /// Allocates the next dense slot without exposing raw construction.
    pub(crate) fn allocate(&mut self) -> Result<HandleIdentity, IdentityError> {
        let slot =
            u32::try_from(self.slots.len()).map_err(|_| IdentityError::SlotCapacityExhausted)?;
        let id = HandleIdentity {
            namespace: self.active.namespace,
            generation: self.active.generation,
            slot,
        };
        self.slots.push(self.active);
        Ok(id)
    }

    /// Returns whether `id` names the currently live allocation at its slot.
    #[must_use]
    pub(crate) fn contains(&self, id: HandleIdentity) -> bool {
        self.slots.get(id.slot as usize).copied() == Some(id.tag())
    }

    /// Returns the live identity at a dense slot for aggregate decoding of a
    /// compact stored reference.
    #[must_use]
    pub(crate) fn identity_at(&self, slot: u32) -> Option<HandleIdentity> {
        let tag = self.slots.get(slot as usize).copied()?;
        Some(HandleIdentity {
            namespace: tag.namespace,
            generation: tag.generation,
            slot,
        })
    }

    /// Captures the identity component of an aggregate store snapshot in O(1).
    #[must_use]
    pub(crate) fn watermark(&self) -> IdentityMark {
        IdentityMark {
            len: self.slots.len(),
            frontier: self.slots.last().copied(),
        }
    }

    #[cfg(feature = "node-stats")]
    pub(crate) fn measurement_shape(&self) -> (usize, usize, usize) {
        (
            self.slots.len(),
            self.slots.capacity(),
            core::mem::size_of::<AllocationTag>(),
        )
    }

    /// Truncates to an ancestor mark and advances the generation before reuse.
    ///
    /// The active generation is intentionally absent from `IdentityMark` and
    /// is never restored. Exhaustion leaves the allocator unchanged; callers
    /// must start a fresh aggregate timeline rather than wrap.
    pub(crate) fn rollback(&mut self, mark: IdentityMark) -> Result<(), IdentityError> {
        let len = mark.len;
        if len < self.builtin_slots as usize
            || len > self.slots.len()
            || (len != 0 && self.slots.get(len - 1).copied() != mark.frontier)
            || (len == 0 && mark.frontier.is_some())
        {
            return Err(IdentityError::InvalidatedMark);
        }
        if len == self.slots.len() {
            return Ok(());
        }
        let generation = self
            .active
            .generation
            .get()
            .checked_add(1)
            .and_then(NonZeroU32::new)
            .ok_or(IdentityError::GenerationExhausted)?;
        self.active.generation = generation;
        self.slots.truncate(len);
        Ok(())
    }
}

fn fresh_namespace() -> NonZeroU64 {
    loop {
        let state = ahash::RandomState::new();
        let raw = state.hash_one(0x6964_656e_7469_7479_u64);
        if let Some(namespace) = NonZeroU64::new(raw)
            && namespace.get() > RESERVED_NAMESPACE_MAX
        {
            return namespace;
        }
    }
}

#[cfg(test)]
mod tests;
