//! Packed handles into one generation's immutable primitive registry.

use core::marker::PhantomData;

/// A direct handle into a completely constructed primitive registry.
///
/// The low half names the primitive row and the high half records the exact
/// registry length at issuance.  The generation brand prevents a handle from
/// crossing typed engine generations, while the length rejects use after a
/// driver extends its profile.  Primitive rows are append-only and their
/// ordering is deterministic for both INITEX construction and format restore.
pub struct PrimitiveHandle<G> {
    session_epoch: u64,
    packed: u32,
    generation: PhantomData<fn(G) -> G>,
}

impl<G> PrimitiveHandle<G> {
    pub(crate) fn new(session_epoch: u64, index: u16, registry_len: u16) -> Self {
        Self {
            session_epoch,
            packed: u32::from(index) | (u32::from(registry_len) << 16),
            generation: PhantomData,
        }
    }

    pub(crate) const fn session_epoch(self) -> u64 {
        self.session_epoch
    }

    pub(crate) const fn index(self) -> usize {
        (self.packed as u16) as usize
    }

    pub(crate) const fn registry_len(self) -> usize {
        (self.packed >> 16) as usize
    }
}

impl<G> Clone for PrimitiveHandle<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for PrimitiveHandle<G> {}

impl<G> core::fmt::Debug for PrimitiveHandle<G> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PrimitiveHandle")
            .field("session_epoch", &self.session_epoch)
            .field("packed", &self.packed)
            .finish()
    }
}

impl<G> PartialEq for PrimitiveHandle<G> {
    fn eq(&self, other: &Self) -> bool {
        self.session_epoch == other.session_epoch && self.packed == other.packed
    }
}

impl<G> Eq for PrimitiveHandle<G> {}
