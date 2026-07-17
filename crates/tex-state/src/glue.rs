//! Immutable hash-consed glue-spec storage.
//!
//! Glue watermarks are crate-private so rollback stays coupled to the
//! aggregate `Universe` boundary.

use crate::identity::{IdentityAllocator, IdentityMark};
use crate::ids::GlueId;
use crate::scaled::Scaled;
use ahash::{AHashMap, AHasher};
use std::hash::{Hash, Hasher};

/// The infinity order attached to stretch or shrink components.
#[derive(
    Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Deserialize, serde::Serialize,
)]
#[repr(u8)]
pub enum Order {
    Normal = 0,
    Fil = 1,
    Fill = 2,
    Filll = 3,
}

/// An immutable TeX glue specification.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GlueSpec {
    pub width: Scaled,
    pub stretch: Scaled,
    pub stretch_order: Order,
    pub shrink: Scaled,
    pub shrink_order: Order,
}

impl GlueSpec {
    /// The canonical zero glue specification.
    pub const ZERO: Self = Self {
        width: Scaled::from_raw(0),
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
    };
}

/// A rollback watermark for the glue store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GlueStoreMark {
    pub(crate) specs: u32,
    identities: IdentityMark,
}

/// Hash-consed immutable glue-spec arena.
#[derive(Debug)]
pub struct GlueStore {
    specs: Vec<GlueSpec>,
    frozen_lookup: crate::frozen_lookup::FrozenLookup,
    frozen_len: u32,
    index: AHashMap<u64, Vec<GlueId>>,
    index_dirty: bool,
    identities: IdentityAllocator,
}

impl Clone for GlueStore {
    fn clone(&self) -> Self {
        Self {
            specs: self.specs.clone(),
            frozen_lookup: self.frozen_lookup.clone(),
            frozen_len: self.frozen_len,
            index: self.index.clone(),
            index_dirty: self.index_dirty,
            identities: self.identities.fork(),
        }
    }
}

impl GlueStore {
    pub(crate) fn retains_mark(&self, mark: GlueStoreMark) -> bool {
        self.identities.retains(mark.identities) && mark.specs as usize <= self.specs.len()
    }

    /// Creates a glue store containing the canonical zero spec.
    #[must_use]
    pub(crate) fn new() -> Self {
        let mut store = Self {
            specs: vec![GlueSpec::ZERO],
            frozen_lookup: crate::frozen_lookup::FrozenLookup::empty(),
            frozen_len: 0,
            index: AHashMap::new(),
            index_dirty: false,
            identities: IdentityAllocator::new(1),
        };
        store
            .index
            .entry(content_hash(&GlueSpec::ZERO))
            .or_default()
            .push(GlueId::ZERO);
        store
    }

    /// Installs a validated frozen dense prefix and builds its lookup index
    /// directly, without replaying semantic interning.
    pub(crate) fn from_frozen(
        specs: Vec<GlueSpec>,
        frozen_lookup: crate::frozen_lookup::FrozenLookup,
    ) -> Result<Self, &'static str> {
        if specs.first().copied() != Some(GlueSpec::ZERO) {
            return Err("missing frozen canonical zero glue");
        }
        let count = u32::try_from(specs.len()).map_err(|_| "frozen glue capacity")?;
        let identities = IdentityAllocator::from_frozen_len(1, count);
        let index = AHashMap::new();
        Ok(Self {
            specs,
            frozen_lookup,
            frozen_len: count,
            index,
            index_dirty: false,
            identities,
        })
    }

    /// Interns `spec`, returning a dense id for the live glue-spec content.
    pub(crate) fn intern(&mut self, spec: GlueSpec) -> GlueId {
        if spec == GlueSpec::ZERO {
            return GlueId::ZERO;
        }

        if self.index_dirty {
            self.rebuild_index();
        }

        if let Some(raw) = self.frozen_lookup.get(&lookup_key(&spec)) {
            return GlueId::from_identity(
                self.identities
                    .identity_at(raw)
                    .expect("frozen glue id is live"),
            );
        }

        let hash = content_hash(&spec);
        if let Some(candidates) = self.index.get(&hash) {
            for &id in candidates {
                if self.get(id) == spec {
                    return id;
                }
            }
        }

        let id = GlueId::from_identity(
            self.identities
                .allocate()
                .expect("glue specs exceed u32 entries"),
        );
        self.specs.push(spec);
        self.index.entry(hash).or_default().push(id);
        id
    }

    /// Reads a live frozen glue specification.
    #[must_use]
    pub(crate) fn get(&self, id: GlueId) -> GlueSpec {
        assert!(self.contains(id), "glue id is not live");
        let index = id.raw() as usize;
        assert!(index < self.specs.len(), "glue id is not live");
        self.specs[index]
    }

    /// Returns whether `id` names a currently-live glue-spec slot.
    #[must_use]
    pub(crate) fn contains(&self, id: GlueId) -> bool {
        self.identities.contains(id.identity())
    }

    #[must_use]
    pub(crate) fn resolve_stored(&self, id: GlueId) -> Option<GlueId> {
        if self.contains(id) {
            return Some(id);
        }
        if !id.is_stored() {
            return None;
        }
        self.identities
            .identity_at(id.raw())
            .map(GlueId::from_identity)
    }

    /// Resolves a live or stored glue handle and reads its immutable value
    /// with a single identity lookup.
    #[must_use]
    pub(crate) fn resolve_get(&self, id: GlueId) -> Option<GlueSpec> {
        let id = self.resolve_stored(id)?;
        self.specs.get(id.raw() as usize).copied()
    }

    /// Checks whether a retained mounted root's resource closure can be
    /// restored without changing any already-live glue slot.
    pub(crate) fn can_restore_retained(&self, retained: &[(GlueId, GlueSpec)]) -> bool {
        let mut next_raw = self.specs.len();
        for &(id, spec) in retained {
            match self.resolve_get(id) {
                Some(current) if current == spec => {}
                Some(_) => return false,
                None if id.is_stored() && id.raw() as usize == next_raw => next_raw += 1,
                None => return false,
            }
        }
        true
    }

    /// Restores a prevalidated retained resource closure at its original raw
    /// slots. Generation tags remain local; stored node words resolve through
    /// the ordinary raw-slot compatibility boundary.
    pub(crate) fn restore_retained(&mut self, retained: &[(GlueId, GlueSpec)]) -> bool {
        if !self.can_restore_retained(retained) {
            return false;
        }
        for &(id, spec) in retained {
            if self.resolve_get(id).is_none() {
                let restored = self.intern(spec);
                assert_eq!(restored.raw(), id.raw(), "retained glue slot changed");
            }
        }
        true
    }

    /// Takes a rollback watermark for aggregate snapshots.
    #[must_use]
    pub(crate) fn watermark(&self) -> GlueStoreMark {
        GlueStoreMark {
            specs: u32_len(self.specs.len(), "glue specs exceed u32 entries"),
            identities: self.identities.watermark(),
        }
    }

    /// Truncates to a previously-taken aggregate snapshot watermark.
    pub(crate) fn truncate_to(&mut self, mark: GlueStoreMark) {
        let specs = mark.specs as usize;
        assert!(specs >= 1, "glue-store mark removes zero glue");
        assert!(
            specs <= self.specs.len(),
            "glue-store mark has too many specs"
        );

        self.identities
            .rollback(mark.identities)
            .expect("glue-store mark is not an ancestor");
        self.specs.truncate(specs);
        self.index_dirty = true;
    }

    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    #[must_use]
    pub(crate) fn testing_state_hash(&self) -> u64 {
        let mut hasher = AHasher::default();
        self.specs.hash(&mut hasher);
        hasher.finish()
    }

    fn rebuild_index(&mut self) {
        self.index.clear();
        for raw in self.frozen_len as usize..self.specs.len() {
            let id = GlueId::from_identity(
                self.identities
                    .identity_at(u32_len(raw, "glue specs exceed u32 entries"))
                    .expect("glue identity table matches specs"),
            );
            let hash = content_hash(&self.get(id));
            self.index.entry(hash).or_default().push(id);
        }
        self.index_dirty = false;
    }
}

fn lookup_key(spec: &GlueSpec) -> [u8; 24] {
    let mut key = [0; 24];
    key[0..4].copy_from_slice(&spec.width.raw().to_le_bytes());
    key[4..8].copy_from_slice(&spec.stretch.raw().to_le_bytes());
    key[8..12].copy_from_slice(&spec.shrink.raw().to_le_bytes());
    key[12] = spec.stretch_order as u8;
    key[13] = spec.shrink_order as u8;
    key
}

fn content_hash(spec: &GlueSpec) -> u64 {
    let mut hasher = AHasher::default();
    spec.hash(&mut hasher);
    hasher.finish()
}

fn u32_len(value: usize, message: &str) -> u32 {
    match u32::try_from(value) {
        Ok(value) => value,
        Err(_) => panic!("{message}"),
    }
}

#[cfg(test)]
mod tests;
