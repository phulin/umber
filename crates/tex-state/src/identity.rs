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
    /// owned payload coordinates and detached format DTO references. They never enter
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

/// A bounded failure that never permits identity wrap or history revival.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IdentityError {
    SlotCapacityExhausted,
    GenerationExhausted,
    InvalidatedMark,
}

/// Generation-safe identity allocation for weak payload slots that may be
/// recycled independently.
///
/// Unlike [`IdentityAllocator`], this allocator does not model an append-only
/// rollback suffix. A caller releases one exact dead slot, and the next use of
/// that slot receives a fresh generation. Fixed prefix slots are retained for
/// built-in or loaded-format values and cannot be released.
#[derive(Debug)]
pub(crate) struct ReusableIdentityAllocator {
    active_namespace: NonZeroU64,
    next_generation: NonZeroU32,
    slots: Vec<Option<AllocationTag>>,
    free: Vec<u32>,
    fixed_slots: u32,
}

impl ReusableIdentityAllocator {
    /// Creates an allocator with `fixed_slots` already-live immutable slots.
    pub(crate) fn new(fixed_slots: u32) -> Self {
        Self::from_fixed_len(0, fixed_slots)
    }

    /// Creates an allocator whose validated immutable prefix is already live.
    ///
    /// The first `builtin_slots` use universal built-in identities. Remaining
    /// fixed slots receive timeline-local identities and cannot be released.
    pub(crate) fn from_fixed_len(builtin_slots: u32, fixed_slots: u32) -> Self {
        assert!(
            builtin_slots <= fixed_slots,
            "fixed value prefix omits built-in slots"
        );
        let active_namespace = fresh_namespace();
        let active = AllocationTag {
            namespace: active_namespace,
            generation: FIRST_GENERATION,
        };
        let mut slots = Vec::with_capacity(fixed_slots as usize);
        for slot in 0..fixed_slots {
            slots.push(Some(if slot < builtin_slots {
                HandleIdentity::builtin(slot).tag()
            } else {
                active
            }));
        }
        Self {
            active_namespace,
            next_generation: FIRST_GENERATION,
            slots,
            free: Vec::new(),
            fixed_slots,
        }
    }

    /// Shares inherited identities while reserving a disjoint namespace for
    /// every allocation made after the fork.
    pub(crate) fn fork(&self) -> Self {
        let active_namespace = loop {
            let candidate = fresh_namespace();
            if candidate != self.active_namespace
                && self
                    .slots
                    .iter()
                    .flatten()
                    .all(|tag| tag.namespace != candidate)
            {
                break candidate;
            }
        };
        Self {
            active_namespace,
            next_generation: FIRST_GENERATION,
            slots: self.slots.clone(),
            free: self.free.clone(),
            fixed_slots: self.fixed_slots,
        }
    }

    /// Allocates either one exact released slot or a new dense suffix slot.
    pub(crate) fn allocate(&mut self) -> Result<HandleIdentity, IdentityError> {
        let next_generation = self
            .next_generation
            .get()
            .checked_add(1)
            .and_then(NonZeroU32::new)
            .ok_or(IdentityError::GenerationExhausted)?;
        let slot = if let Some(slot) = self.free.pop() {
            debug_assert!(self.slots[slot as usize].is_none());
            slot
        } else {
            let slot = u32::try_from(self.slots.len())
                .map_err(|_| IdentityError::SlotCapacityExhausted)?;
            self.slots.push(None);
            slot
        };
        let generation = self.next_generation;
        self.next_generation = next_generation;
        let tag = AllocationTag {
            namespace: self.active_namespace,
            generation,
        };
        self.slots[slot as usize] = Some(tag);
        Ok(HandleIdentity {
            namespace: tag.namespace,
            generation: tag.generation,
            slot,
        })
    }

    /// Releases one exact dynamic identity for later slot reuse.
    pub(crate) fn release(&mut self, identity: HandleIdentity) -> Result<(), IdentityError> {
        if identity.slot < self.fixed_slots
            || self.slots.get(identity.slot as usize).copied().flatten() != Some(identity.tag())
        {
            return Err(IdentityError::InvalidatedMark);
        }
        self.slots[identity.slot as usize] = None;
        self.free.push(identity.slot);
        Ok(())
    }

    /// Returns whether `identity` names the currently live use of its slot.
    #[allow(dead_code)]
    pub(crate) fn contains(&self, identity: HandleIdentity) -> bool {
        self.slots.get(identity.slot as usize).copied().flatten() == Some(identity.tag())
    }

    /// Returns the live identity occupying one physical slot.
    #[allow(dead_code)]
    pub(crate) fn identity_at(&self, slot: u32) -> Option<HandleIdentity> {
        let tag = self.slots.get(slot as usize).copied().flatten()?;
        Some(HandleIdentity {
            namespace: tag.namespace,
            generation: tag.generation,
            slot,
        })
    }

    /// Returns physical slot and reusable-entry counts for ownership tests.
    #[cfg(any(test, feature = "testing"))]
    #[allow(dead_code)]
    pub(crate) fn testing_shape(&self) -> (usize, usize, usize) {
        (self.slots.len(), self.slots.capacity(), self.free.len())
    }
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

    /// Creates one fresh timeline whose validated immutable prefix already
    /// contains `total_slots` dense entries.
    ///
    /// Frozen-format decoders use this after validating every record. It
    /// avoids replaying ordinary allocation while leaving subsequent job-local
    /// allocations and rollback on the same generation-safe path.
    pub(crate) fn from_frozen_len(builtin_slots: u32, total_slots: u32) -> Self {
        assert!(
            total_slots >= builtin_slots,
            "frozen identity prefix omits builtin slots"
        );
        let mut allocator = Self::with_namespace(builtin_slots, fresh_namespace());
        allocator
            .slots
            .resize(total_slots as usize, allocator.active);
        allocator
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
