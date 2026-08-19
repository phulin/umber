//! Reachability-owned immutable glue-spec storage.

use crate::frozen_lookup::FrozenLookup;
use crate::ids::GlueId;
use crate::patch_domain::{
    PatchAllocationDomain, PatchHandle, PatchRoot, PatchRootAnchor, PatchRootLease,
};
use crate::reachable_value::{ReachableValuePool, ReachableValueRef};
use crate::scaled::Scaled;
use ahash::AHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
#[cfg(any(test, feature = "testing"))]
use std::sync::OnceLock;

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

/// One strong exact-content glue owner paired with its compact coordinate.
#[derive(Clone, Debug)]
pub struct GlueSpecRef {
    value: ReachableValueRef<GlueSpec>,
    patch_root: Option<PatchRootLease>,
}

/// A borrowed view of either a compact glue coordinate or its strong owner.
///
/// APIs use this view without consuming the owner, so a sole strong reference
/// remains live until the destination has cloned it.
pub trait GlueHandle {
    fn glue_id(&self) -> GlueId;
}

impl GlueHandle for GlueId {
    fn glue_id(&self) -> GlueId {
        *self
    }
}

impl GlueHandle for GlueSpecRef {
    fn glue_id(&self) -> GlueId {
        self.id()
    }
}

impl<T: GlueHandle + ?Sized> GlueHandle for &T {
    fn glue_id(&self) -> GlueId {
        (*self).glue_id()
    }
}

impl GlueSpecRef {
    #[must_use]
    pub fn id(&self) -> GlueId {
        GlueId::from_identity(self.value.identity())
    }

    #[must_use]
    pub fn raw(&self) -> u32 {
        self.id().raw()
    }

    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn testing_new(id: GlueId) -> Self {
        Self {
            value: crate::reachable_value::testing_value_ref(id.identity(), GlueSpec::ZERO),
            patch_root: None,
        }
    }

    #[must_use]
    pub fn spec(&self) -> GlueSpec {
        *self.value.value()
    }

    fn shared(&self) -> Arc<GlueSpec> {
        self.value.shared()
    }

    #[cfg(test)]
    pub(crate) fn strong_count(&self) -> usize {
        self.value.strong_count()
    }

    #[cfg(test)]
    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared(), &other.shared())
    }
}

impl PartialEq for GlueSpecRef {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl Eq for GlueSpecRef {}

impl Hash for GlueSpecRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id().hash(state);
    }
}

impl From<GlueSpecRef> for GlueId {
    fn from(root: GlueSpecRef) -> Self {
        root.id()
    }
}

impl From<&GlueSpecRef> for GlueId {
    fn from(root: &GlueSpecRef) -> Self {
        root.id()
    }
}

/// A rollback watermark over private glue-allocation metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GlueStoreMark {
    pub(crate) specs: u32,
    allocations: u32,
    patch_allocations: u32,
    #[cfg(any(test, feature = "testing"))]
    testing_detached_roots: u32,
}

/// Reachability-owned immutable glue values.
#[derive(Debug)]
pub struct GlueStore {
    pool: ReachableValuePool<u64, GlueSpec>,
    frozen_roots: Arc<[GlueSpecRef]>,
    frozen_lookup: FrozenLookup,
    frozen_len: u32,
    patch_handles: HashMap<GlueId, PatchHandle<GlueSpec>>,
    patch_root_leases: HashMap<GlueId, PatchRootAnchor>,
    patch_order: Vec<GlueId>,
    /// Explicit detached owners used only by legacy test construction APIs.
    /// Production interning returns `GlueSpecRef` and never enters this row.
    #[cfg(any(test, feature = "testing"))]
    testing_detached_roots: Vec<GlueSpecRef>,
}

impl Clone for GlueStore {
    fn clone(&self) -> Self {
        debug_assert!(
            self.patch_handles.is_empty(),
            "private glue allocations cannot cross a generation fork"
        );
        Self {
            pool: self.pool.clone(),
            frozen_roots: Arc::clone(&self.frozen_roots),
            frozen_lookup: self.frozen_lookup.clone(),
            frozen_len: self.frozen_len,
            patch_handles: HashMap::new(),
            patch_root_leases: HashMap::new(),
            patch_order: Vec::new(),
            #[cfg(any(test, feature = "testing"))]
            testing_detached_roots: self.testing_detached_roots.clone(),
        }
    }
}

impl GlueStore {
    /// Creates a store owning the canonical zero glue.
    #[must_use]
    pub(crate) fn new() -> Self {
        let (pool, roots) = ReachableValuePool::from_fixed_values(vec![GlueSpec::ZERO], 1);
        Self {
            pool,
            frozen_roots: roots
                .into_iter()
                .map(|value| GlueSpecRef {
                    value,
                    patch_root: None,
                })
                .collect::<Vec<_>>()
                .into(),
            frozen_lookup: FrozenLookup::empty(),
            frozen_len: 0,
            patch_handles: HashMap::new(),
            patch_root_leases: HashMap::new(),
            patch_order: Vec::new(),
            #[cfg(any(test, feature = "testing"))]
            testing_detached_roots: Vec::new(),
        }
    }

    /// Installs a validated frozen dense prefix as an explicit immutable base.
    pub(crate) fn from_frozen(
        specs: Vec<GlueSpec>,
        frozen_lookup: FrozenLookup,
    ) -> Result<Self, &'static str> {
        if specs.first().copied() != Some(GlueSpec::ZERO) {
            return Err("missing frozen canonical zero glue");
        }
        let count = u32::try_from(specs.len()).map_err(|_| "frozen glue capacity")?;
        let (pool, roots) = ReachableValuePool::from_fixed_values(specs, 1);
        Ok(Self {
            pool,
            frozen_roots: roots
                .into_iter()
                .map(|value| GlueSpecRef {
                    value,
                    patch_root: None,
                })
                .collect::<Vec<_>>()
                .into(),
            frozen_lookup,
            frozen_len: count,
            patch_handles: HashMap::new(),
            patch_root_leases: HashMap::new(),
            patch_order: Vec::new(),
            #[cfg(any(test, feature = "testing"))]
            testing_detached_roots: Vec::new(),
        })
    }

    /// Interns exact content and returns its strong owner.
    pub(crate) fn intern_owned(
        &mut self,
        spec: GlueSpec,
        domain: Option<&mut PatchAllocationDomain>,
    ) -> GlueSpecRef {
        if spec == GlueSpec::ZERO {
            return self.frozen_roots[0].clone();
        }
        if let Some(raw) = self.frozen_lookup.get(&lookup_key(&spec))
            && let Some(root) = self
                .frozen_roots
                .get(raw as usize)
                .filter(|root| root.spec() == spec)
        {
            return root.clone();
        }
        let key = content_hash(&spec);
        if let Some(value) = self.pool.find_exact(&key, |candidate| *candidate == spec) {
            let id = GlueId::from_identity(value.identity());
            return GlueSpecRef {
                value,
                patch_root: self
                    .patch_root_leases
                    .get(&id)
                    .map(PatchRootAnchor::lease),
            };
        }
        let value = self.pool.insert_new(key, spec);
        let mut root = GlueSpecRef {
            value,
            patch_root: None,
        };
        self.attach_patch_allocation(&mut root, domain);
        root
    }

    fn attach_patch_allocation(
        &mut self,
        root: &mut GlueSpecRef,
        domain: Option<&mut PatchAllocationDomain>,
    ) {
        let Some(domain) = domain else {
            return;
        };
        let handle = domain
            .allocate_shared(root.shared(), core::mem::size_of::<GlueSpec>())
            .expect("private glue allocation belongs to the active operation");
        assert!(
            self.patch_handles.insert(root.id(), handle).is_none(),
            "new glue value already has patch allocation metadata"
        );
        let lease = domain
            .install_root_lease(&self.patch_handles[&root.id()])
            .expect("new private glue root belongs to the active domain");
        assert!(
            self.patch_root_leases
                .insert(root.id(), lease.anchor())
                .is_none()
        );
        root.patch_root = Some(lease);
        self.patch_order.push(root.id());
    }

    #[must_use]
    pub(crate) fn owner(&self, id: GlueId) -> Option<GlueSpecRef> {
        self.frozen_roots
            .get(id.raw() as usize)
            .filter(|root| root.id() == id)
            .cloned()
            .or_else(|| {
                self.pool.resolve(id.identity()).map(|value| GlueSpecRef {
                    value,
                    patch_root: self
                        .patch_root_leases
                        .get(&id)
                        .map(PatchRootAnchor::lease),
                })
            })
    }

    #[must_use]
    pub(crate) fn stored_slot(&self, raw: u32) -> GlueSpecRef {
        self.frozen_roots
            .get(raw as usize)
            .cloned()
            .unwrap_or_else(|| {
                self.pool.resolve_slot(raw).map_or_else(
                    || self.frozen_roots[0].clone(),
                    |value| GlueSpecRef {
                        value,
                        patch_root: None,
                    },
                )
            })
    }

    #[must_use]
    pub(crate) fn contains(&self, id: GlueId) -> bool {
        self.owner(id).is_some()
    }

    #[must_use]
    pub(crate) fn resolve_stored(&self, id: GlueId) -> Option<GlueId> {
        if self.contains(id) {
            return Some(id);
        }
        id.is_stored().then(|| self.id_at(id.raw())).flatten()
    }

    #[must_use]
    pub(crate) fn resolve_get(&self, id: GlueId) -> Option<GlueSpec> {
        self.resolve_stored(id)
            .and_then(|id| self.owner(id))
            .map(|root| root.spec())
    }

    #[must_use]
    pub(crate) fn watermark(&self) -> GlueStoreMark {
        GlueStoreMark {
            specs: self.slot_len(),
            allocations: u32::try_from(self.pool.allocation_mark())
                .expect("glue allocation events exceed u32"),
            patch_allocations: u32::try_from(self.patch_order.len())
                .expect("glue patch allocations exceed u32"),
            #[cfg(any(test, feature = "testing"))]
            testing_detached_roots: u32::try_from(self.testing_detached_roots.len())
                .expect("testing detached glue roots exceed u32"),
        }
    }

    pub(crate) fn truncate_to(&mut self, mark: GlueStoreMark) {
        #[cfg(any(test, feature = "testing"))]
        self.testing_detached_roots
            .truncate(mark.testing_detached_roots as usize);
        while self.patch_order.len() > mark.patch_allocations as usize {
            let id = self.patch_order.pop().expect("patch order is nonempty");
            assert!(self.patch_handles.remove(&id).is_some());
            assert!(self.patch_root_leases.remove(&id).is_some());
        }
        self.pool
            .rollback_to_allocation_mark(mark.allocations as usize);
    }

    #[cfg(any(test, feature = "testing"))]
    #[allow(dead_code)]
    pub(crate) fn testing_intern(&mut self, spec: GlueSpec) -> GlueId {
        let root = self.intern_owned(spec, None);
        self.testing_detached_roots.push(root.clone());
        root.id()
    }

    pub(crate) fn selected_patch_roots(&self, domain: &PatchAllocationDomain) -> Vec<PatchRoot> {
        self.patch_order
            .iter()
            .filter_map(|id| self.patch_handles.get(id))
            .filter_map(|handle| {
                domain
                    .root_if_typed(handle)
                    .expect("typed glue root belongs to the private domain")
            })
            .collect()
    }

    pub(crate) fn patch_allocation_count(&self) -> usize {
        self.patch_handles.len()
    }

    pub(crate) fn clear_patch_allocations(&mut self) {
        self.patch_handles.clear();
        self.patch_root_leases.clear();
        self.patch_order.clear();
        self.pool.prioritize_reclamation_from(0);
    }

    pub(crate) fn retire_unrooted_region_values(&mut self) {
        self.pool.prioritize_reclamation_from(0);
    }

    #[must_use]
    pub(crate) fn slot_len(&self) -> u32 {
        u32::try_from(self.pool.slot_len()).expect("glue slots exceed u32")
    }

    fn id_at(&self, raw: u32) -> Option<GlueId> {
        self.frozen_roots
            .get(raw as usize)
            .map(GlueSpecRef::id)
            .or_else(|| {
                self.pool
                    .resolve_slot(raw)
                    .map(|value| GlueId::from_identity(value.identity()))
            })
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_pool_shape(&self) -> (usize, usize, usize, usize, usize, usize) {
        self.pool.testing_shape()
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing_live_totals(&self) -> (usize, usize) {
        self.pool
            .testing_live_totals(|_| core::mem::size_of::<GlueSpec>())
    }

    #[cfg(test)]
    pub(crate) fn testing_intern_with_key(&mut self, spec: GlueSpec, key: u64) -> GlueSpecRef {
        if let Some(value) = self.pool.find_exact(&key, |candidate| *candidate == spec) {
            return GlueSpecRef {
                value,
                patch_root: None,
            };
        }
        GlueSpecRef {
            value: self.pool.insert_new(key, spec),
            patch_root: None,
        }
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
    #[cfg(feature = "profiling")]
    crate::measurement::record_hot_core_content_hash();
    let mut hasher = AHasher::default();
    spec.hash(&mut hasher);
    hasher.finish()
}

#[cfg(any(test, feature = "testing"))]
pub fn testing_zero_glue_ref() -> GlueSpecRef {
    static STORE: OnceLock<GlueStore> = OnceLock::new();
    STORE
        .get_or_init(GlueStore::new)
        .owner(GlueId::ZERO)
        .expect("test glue store owns zero glue")
}

#[cfg(test)]
mod tests;
