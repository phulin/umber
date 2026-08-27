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
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

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

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct AllocationTag {
    namespace: NonZeroU64,
    generation: NonZeroU32,
}

#[derive(Clone, Copy, Debug)]
struct AllocationRun {
    end: u32,
    tag: AllocationTag,
}

#[derive(Debug)]
struct AcceptedIdentityRuns {
    parent: Option<Arc<Self>>,
    runs: Arc<Vec<AllocationRun>>,
    total_len: u32,
}

impl AcceptedIdentityRuns {
    fn tag_at(&self, slot: u32) -> Option<AllocationTag> {
        if slot >= self.total_len {
            return None;
        }
        let parent_len = self.parent.as_ref().map_or(0, |parent| parent.total_len);
        if slot < parent_len {
            return self.parent.as_ref()?.tag_at(slot);
        }
        let index = self.runs.partition_point(|run| run.end <= slot);
        self.runs.get(index).map(|run| run.tag)
    }
}

/// Generation table shared by rollback-truncated store implementations.
///
/// This owns only identity/liveness metadata; semantic store data remains in
/// the owning store and mutation remains behind `Stores`/`Universe`.
#[derive(Debug)]
pub(crate) struct IdentityAllocator {
    active: AllocationTag,
    accepted: Option<Arc<AcceptedIdentityRuns>>,
    runs: Arc<Vec<AllocationRun>>,
    len: u32,
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
        allocator.extend_with_active(total_slots - builtin_slots);
        allocator
    }

    fn with_namespace(builtin_slots: u32, namespace: NonZeroU64) -> Self {
        assert_ne!(
            namespace, BUILTIN_NAMESPACE,
            "the builtin identity namespace is reserved"
        );
        Self {
            active: AllocationTag {
                namespace,
                generation: FIRST_GENERATION,
            },
            accepted: None,
            runs: Arc::new(
                (builtin_slots != 0)
                    .then_some(AllocationRun {
                        end: builtin_slots,
                        tag: HandleIdentity::builtin(0).tag(),
                    })
                    .into_iter()
                    .collect(),
            ),
            len: builtin_slots,
            builtin_slots,
        }
    }

    /// Copies inherited liveness while giving post-fork allocations a fresh
    /// namespace. Handles inherited from the parent remain valid in both
    /// timelines; handles subsequently minted by either side are foreign to
    /// the other.
    pub(crate) fn fork(&self) -> Self {
        self.fork_at(self.watermark())
            .expect("current identity watermark is forkable")
    }

    /// Forks exactly the retained prefix named by `mark`; later source slots
    /// remain owned only by the source lineage.
    pub(crate) fn fork_at(&self, mark: IdentityMark) -> Result<Self, IdentityError> {
        self.validate_rollback(mark)?;
        let namespace = fresh_namespace();
        let accepted_len = self.accepted.as_ref().map_or(0, |runs| runs.total_len);
        let mark_len = u32::try_from(mark.len).map_err(|_| IdentityError::InvalidatedMark)?;
        let accepted = if mark_len == accepted_len {
            self.accepted.clone()
        } else {
            Some(Arc::new(AcceptedIdentityRuns {
                parent: self.accepted.clone(),
                runs: Arc::clone(&self.runs),
                total_len: mark_len,
            }))
        };
        Ok(Self {
            active: AllocationTag {
                namespace,
                generation: FIRST_GENERATION,
            },
            accepted,
            runs: Arc::new(Vec::new()),
            len: mark_len,
            builtin_slots: self.builtin_slots,
        })
    }

    /// Allocates the next dense slot without exposing raw construction.
    pub(crate) fn allocate(&mut self) -> Result<HandleIdentity, IdentityError> {
        let slot = self.len;
        self.len = self
            .len
            .checked_add(1)
            .ok_or(IdentityError::SlotCapacityExhausted)?;
        let id = HandleIdentity {
            namespace: self.active.namespace,
            generation: self.active.generation,
            slot,
        };
        let runs = Arc::make_mut(&mut self.runs);
        if let Some(run) = runs.last_mut().filter(|run| run.tag == self.active) {
            run.end = self.len;
        } else {
            runs.push(AllocationRun {
                end: self.len,
                tag: self.active,
            });
        }
        Ok(id)
    }

    /// Returns whether `id` names the currently live allocation at its slot.
    #[must_use]
    pub(crate) fn contains(&self, id: HandleIdentity) -> bool {
        self.tag_at(id.slot) == Some(id.tag())
    }

    /// Returns the live identity at a dense slot for aggregate decoding of a
    /// compact stored reference.
    #[must_use]
    pub(crate) fn identity_at(&self, slot: u32) -> Option<HandleIdentity> {
        let tag = self.tag_at(slot)?;
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
            len: self.len as usize,
            frontier: self.len.checked_sub(1).and_then(|slot| self.tag_at(slot)),
        }
    }

    /// Preflights an ancestor rollback without changing liveness or generation.
    ///
    /// The active generation is intentionally absent from `IdentityMark` and
    /// exhaustion is rejected here so an aggregate can validate every family
    /// before mutating any of them.
    pub(crate) fn validate_rollback(&self, mark: IdentityMark) -> Result<(), IdentityError> {
        let len = mark.len;
        let accepted_len = self.accepted.as_ref().map_or(0, |runs| runs.total_len) as usize;
        if len < self.builtin_slots as usize
            || len < accepted_len
            || len > self.len as usize
            || (len != 0 && self.tag_at((len - 1) as u32) != mark.frontier)
            || (len == 0 && mark.frontier.is_some())
        {
            return Err(IdentityError::InvalidatedMark);
        }
        if len != self.len as usize && self.active.generation.get() == u32::MAX {
            return Err(IdentityError::GenerationExhausted);
        }
        Ok(())
    }

    /// Truncates to an ancestor mark and advances the generation before reuse.
    ///
    /// The active generation is never restored. Exhaustion leaves the
    /// allocator unchanged; callers must start a fresh timeline rather than
    /// wrap.
    pub(crate) fn rollback(&mut self, mark: IdentityMark) -> Result<(), IdentityError> {
        self.validate_rollback(mark)?;
        let len = mark.len;
        if len == self.len as usize {
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
        self.len = u32::try_from(len).expect("validated identity length fits u32");
        let accepted_len = self.accepted.as_ref().map_or(0, |runs| runs.total_len);
        let runs = Arc::make_mut(&mut self.runs);
        while runs
            .last()
            .is_some_and(|_| runs.len() > 1 && runs[runs.len() - 2].end >= self.len)
        {
            runs.pop();
        }
        if self.len == accepted_len {
            runs.clear();
        } else if let Some(run) = runs.last_mut() {
            run.end = self.len;
        }
        Ok(())
    }

    fn extend_with_active(&mut self, count: u32) {
        if count == 0 {
            return;
        }
        self.len = self
            .len
            .checked_add(count)
            .expect("frozen identity extent fits u32");
        let runs = Arc::make_mut(&mut self.runs);
        if let Some(run) = runs.last_mut().filter(|run| run.tag == self.active) {
            run.end = self.len;
        } else {
            runs.push(AllocationRun {
                end: self.len,
                tag: self.active,
            });
        }
    }

    fn tag_at(&self, slot: u32) -> Option<AllocationTag> {
        if slot >= self.len {
            return None;
        }
        let accepted_len = self.accepted.as_ref().map_or(0, |runs| runs.total_len);
        if slot < accepted_len {
            return self.accepted.as_ref()?.tag_at(slot);
        }
        let index = self.runs.partition_point(|run| run.end <= slot);
        self.runs.get(index).map(|run| run.tag)
    }
}

fn fresh_namespace() -> NonZeroU64 {
    static NEXT_NAMESPACE: AtomicU64 = AtomicU64::new(RESERVED_NAMESPACE_MAX + 1);
    let raw = NEXT_NAMESPACE.fetch_add(1, Ordering::Relaxed);
    NonZeroU64::new(raw).expect("identity namespace space exhausted")
}
