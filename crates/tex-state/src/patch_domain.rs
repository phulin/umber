//! Disposable ownership for allocations made by one private revision.
//!
//! The domain is deliberately independent of the legacy typed stores. Those
//! stores migrate their payloads separately; this module supplies the exact
//! operation-mark and explicit-root-transfer authority they share.

use std::{
    any::Any,
    fmt,
    marker::PhantomData,
    mem,
    panic::RefUnwindSafe,
    sync::Arc,
    sync::atomic::{AtomicU64, Ordering},
};

type ErasedPayload = dyn Any + Send + Sync;

#[derive(Debug)]
pub(crate) struct DomainOwnerToken {
    identity: u64,
}

static NEXT_DOMAIN_ID: AtomicU64 = AtomicU64::new(1);

struct AllocationSlot {
    payload: Arc<ErasedPayload>,
    logical_bytes: usize,
}

impl fmt::Debug for AllocationSlot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AllocationSlot")
            .field("logical_bytes", &self.logical_bytes)
            .finish_non_exhaustive()
    }
}

// Every constructor requires the concrete immutable payload to be
// `RefUnwindSafe`; type erasure cannot retain that auto-trait on the standard
// `Arc<dyn Any>` downcast surface, so the slot records the proven property.
impl RefUnwindSafe for AllocationSlot {}

#[derive(Debug)]
struct ActiveOperation {
    serial: u64,
    slot_len: usize,
    slot_capacity: usize,
    logical_bytes: usize,
}

/// One private revision's disposable allocation owner.
#[derive(Debug)]
pub(crate) struct PatchAllocationDomain {
    owner: Arc<DomainOwnerToken>,
    slots: Vec<AllocationSlot>,
    logical_bytes: usize,
    next_operation_serial: u64,
    active_operation: Option<ActiveOperation>,
}

/// A single-use aggregate operation mark.
#[derive(Debug)]
pub(crate) struct PatchOperationMark {
    owner: u64,
    serial: u64,
}

/// A typed, non-owning coordinate into a live private revision domain.
#[allow(dead_code)] // Typed store migrations consume this generic hook in later epic children.
#[derive(Debug)]
pub(crate) struct PatchHandle<T> {
    owner: u64,
    slot: usize,
    marker: PhantomData<fn() -> T>,
}

impl<T> Clone for PatchHandle<T> {
    fn clone(&self) -> Self {
        Self {
            owner: self.owner,
            slot: self.slot,
            marker: PhantomData,
        }
    }
}

/// One type-erased root named explicitly at revision acceptance.
#[derive(Clone, Debug)]
pub(crate) struct PatchRoot {
    owner: u64,
    slot: usize,
    payload: Arc<ErasedPayload>,
    logical_bytes: usize,
}

/// Independently owned immutable objects selected by accepted roots.
#[derive(Debug)]
pub(crate) struct AcceptedPatchObjects {
    owner: u64,
    _owner_lifetime: Arc<DomainOwnerToken>,
    objects: Vec<(usize, AllocationSlot)>,
    #[allow(dead_code)] // Reported when migrated stores begin transferring roots.
    logical_bytes: usize,
}

/// Exact logical and allocation-metadata ownership for focused controls.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PatchDomainStats {
    pub(crate) allocations: usize,
    pub(crate) logical_bytes: usize,
    pub(crate) slot_capacity_bytes: usize,
    pub(crate) operation_active: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PatchDomainError {
    OperationAlreadyActive,
    NoActiveOperation,
    StaleOperation,
    OperationSerialExhausted,
    #[allow(dead_code)] // Enforced by the generic allocation hook before store migrations land.
    AllocationOutsideOperation,
    ForeignRoot,
    StaleRoot,
    #[allow(dead_code)] // Enforced by typed reads after store migrations land.
    WrongPayloadType,
}

impl PatchAllocationDomain {
    pub(crate) fn new() -> Self {
        let identity = NEXT_DOMAIN_ID.fetch_add(1, Ordering::Relaxed);
        assert_ne!(identity, 0, "private revision domain identity exhausted");
        Self {
            owner: Arc::new(DomainOwnerToken { identity }),
            slots: Vec::new(),
            logical_bytes: 0,
            next_operation_serial: 0,
            active_operation: None,
        }
    }

    pub(crate) fn begin_operation(&mut self) -> Result<PatchOperationMark, PatchDomainError> {
        if self.active_operation.is_some() {
            return Err(PatchDomainError::OperationAlreadyActive);
        }
        let serial = self.next_operation_serial;
        self.next_operation_serial = self
            .next_operation_serial
            .checked_add(1)
            .ok_or(PatchDomainError::OperationSerialExhausted)?;
        self.active_operation = Some(ActiveOperation {
            serial,
            slot_len: self.slots.len(),
            slot_capacity: self.slots.capacity(),
            logical_bytes: self.logical_bytes,
        });
        Ok(PatchOperationMark {
            owner: self.owner.identity,
            serial,
        })
    }

    pub(crate) fn commit_operation(
        &mut self,
        mark: PatchOperationMark,
    ) -> Result<(), PatchDomainError> {
        self.validate_operation(&mark)?;
        self.active_operation = None;
        Ok(())
    }

    pub(crate) fn rollback_operation(
        &mut self,
        mark: PatchOperationMark,
    ) -> Result<(), PatchDomainError> {
        self.validate_operation(&mark)?;
        let operation = self
            .active_operation
            .take()
            .expect("validated operation is active");
        self.slots.truncate(operation.slot_len);
        if self.slots.capacity() != operation.slot_capacity {
            let mut restored = Vec::with_capacity(operation.slot_capacity);
            restored.append(&mut self.slots);
            self.slots = restored;
        }
        self.logical_bytes = operation.logical_bytes;
        Ok(())
    }

    #[allow(dead_code)] // Typed store migrations consume this generic hook in later epic children.
    pub(crate) fn allocate<T>(
        &mut self,
        value: T,
        logical_bytes: usize,
    ) -> Result<PatchHandle<T>, PatchDomainError>
    where
        T: Any + Send + Sync + RefUnwindSafe,
    {
        self.allocate_shared(Arc::new(value), logical_bytes)
    }

    /// Records domain ownership of an immutable payload already shared with
    /// its typed private destination.
    pub(crate) fn allocate_shared<T>(
        &mut self,
        value: Arc<T>,
        logical_bytes: usize,
    ) -> Result<PatchHandle<T>, PatchDomainError>
    where
        T: Any + Send + Sync + RefUnwindSafe,
    {
        if self.active_operation.is_none() {
            return Err(PatchDomainError::AllocationOutsideOperation);
        }
        let slot = self.slots.len();
        self.slots.push(AllocationSlot {
            payload: value,
            logical_bytes,
        });
        self.logical_bytes = self.logical_bytes.saturating_add(logical_bytes);
        Ok(PatchHandle {
            owner: self.owner.identity,
            slot,
            marker: PhantomData,
        })
    }

    #[allow(dead_code)] // Typed store migrations consume this generic hook in later epic children.
    pub(crate) fn get<T>(&self, handle: &PatchHandle<T>) -> Result<&T, PatchDomainError>
    where
        T: Any + Send + Sync + RefUnwindSafe,
    {
        if !self.owner_matches(handle.owner) {
            return Err(PatchDomainError::ForeignRoot);
        }
        let slot = self
            .slots
            .get(handle.slot)
            .ok_or(PatchDomainError::StaleRoot)?;
        slot.payload
            .downcast_ref::<T>()
            .ok_or(PatchDomainError::WrongPayloadType)
    }

    #[allow(dead_code)] // Typed store migrations consume this generic hook in later epic children.
    pub(crate) fn root<T>(&self, handle: &PatchHandle<T>) -> Result<PatchRoot, PatchDomainError>
    where
        T: Any + Send + Sync + RefUnwindSafe,
    {
        let _ = self.get(handle)?;
        let slot = &self.slots[handle.slot];
        Ok(PatchRoot {
            owner: self.owner.identity,
            slot: handle.slot,
            payload: Arc::clone(&slot.payload),
            logical_bytes: slot.logical_bytes,
        })
    }

    pub(crate) fn accept(
        self,
        mut roots: Vec<PatchRoot>,
    ) -> Result<AcceptedPatchObjects, PatchDomainError> {
        if self.active_operation.is_some() {
            return Err(PatchDomainError::OperationAlreadyActive);
        }
        for root in &roots {
            if self.owner.identity != root.owner {
                return Err(PatchDomainError::ForeignRoot);
            }
            let Some(slot) = self.slots.get(root.slot) else {
                return Err(PatchDomainError::StaleRoot);
            };
            if !Arc::ptr_eq(&slot.payload, &root.payload)
                || slot.logical_bytes != root.logical_bytes
            {
                return Err(PatchDomainError::StaleRoot);
            }
        }
        roots.sort_unstable_by_key(|root| root.slot);
        roots.dedup_by_key(|root| root.slot);
        let mut objects = Vec::with_capacity(roots.len());
        let mut logical_bytes = 0_usize;
        for root in roots {
            logical_bytes = logical_bytes.saturating_add(root.logical_bytes);
            objects.push((
                root.slot,
                AllocationSlot {
                    payload: root.payload,
                    logical_bytes: root.logical_bytes,
                },
            ));
        }
        Ok(AcceptedPatchObjects {
            owner: self.owner.identity,
            _owner_lifetime: self.owner,
            objects,
            logical_bytes,
        })
    }

    pub(crate) fn stats(&self) -> PatchDomainStats {
        PatchDomainStats {
            allocations: self.slots.len(),
            logical_bytes: self.logical_bytes,
            slot_capacity_bytes: self
                .slots
                .capacity()
                .saturating_mul(mem::size_of::<AllocationSlot>()),
            operation_active: self.active_operation.is_some(),
        }
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        mem::size_of::<DomainOwnerToken>()
            .saturating_add(self.stats().slot_capacity_bytes)
            .saturating_add(self.logical_bytes)
    }

    fn validate_operation(&self, mark: &PatchOperationMark) -> Result<(), PatchDomainError> {
        if !self.owner_matches(mark.owner) {
            return Err(PatchDomainError::StaleOperation);
        }
        let operation = self
            .active_operation
            .as_ref()
            .ok_or(PatchDomainError::NoActiveOperation)?;
        if operation.serial != mark.serial {
            return Err(PatchDomainError::StaleOperation);
        }
        Ok(())
    }

    fn owner_matches(&self, owner: u64) -> bool {
        owner == self.owner.identity
    }
}

impl AcceptedPatchObjects {
    #[allow(dead_code)] // Typed store migrations consume transferred owners in later children.
    pub(crate) fn get<T>(&self, handle: &PatchHandle<T>) -> Result<Arc<T>, PatchDomainError>
    where
        T: Any + Send + Sync + RefUnwindSafe,
    {
        if self.owner != handle.owner {
            return Err(PatchDomainError::ForeignRoot);
        }
        let index = self
            .objects
            .binary_search_by_key(&handle.slot, |(slot, _)| *slot)
            .map_err(|_| PatchDomainError::StaleRoot)?;
        Arc::clone(&self.objects[index].1.payload)
            .downcast::<T>()
            .map_err(|_| PatchDomainError::WrongPayloadType)
    }

    pub(crate) const fn len(&self) -> usize {
        self.objects.len()
    }

    #[allow(dead_code)] // Reported when migrated stores begin transferring roots.
    pub(crate) const fn logical_bytes(&self) -> usize {
        self.logical_bytes
    }
}
