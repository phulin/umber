//! Immutable hash-consed glue-spec storage.
//!
//! Glue watermarks are crate-private so rollback stays coupled to the
//! aggregate `Universe` boundary.

use crate::identity::{IdentityAllocator, IdentityMark};
use crate::ids::GlueId;
use crate::scaled::Scaled;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
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
    index: HashMap<u64, Vec<GlueId>>,
    index_dirty: bool,
    identities: IdentityAllocator,
}

impl Clone for GlueStore {
    fn clone(&self) -> Self {
        Self {
            specs: self.specs.clone(),
            index: self.index.clone(),
            index_dirty: self.index_dirty,
            identities: self.identities.fork(),
        }
    }
}

impl GlueStore {
    /// Creates a glue store containing the canonical zero spec.
    #[must_use]
    pub(crate) fn new() -> Self {
        let mut store = Self {
            specs: vec![GlueSpec::ZERO],
            index: HashMap::new(),
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

    /// Interns `spec`, returning a dense id for the live glue-spec content.
    pub(crate) fn intern(&mut self, spec: GlueSpec) -> GlueId {
        if spec == GlueSpec::ZERO {
            return GlueId::ZERO;
        }

        if self.index_dirty {
            self.rebuild_index();
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
        let mut hasher = DefaultHasher::new();
        self.specs.hash(&mut hasher);
        hasher.finish()
    }

    fn rebuild_index(&mut self) {
        self.index.clear();
        for raw in 0..self.specs.len() {
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

fn content_hash(spec: &GlueSpec) -> u64 {
    let mut hasher = DefaultHasher::new();
    // PERF: revisit hasher (fastpaths epic).
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
