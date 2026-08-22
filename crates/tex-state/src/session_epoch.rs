//! Coarse ownership of one append-only session interning epoch.

use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex};

use crate::interner::{Interner, InternerBudget, InternerRetirement};

/// Failure to admit or retire a session interning epoch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionEpochError {
    /// Another revision generation currently holds the unique mutable lease.
    GenerationAdmitted,
    /// Another coarse epoch owner still exists at whole-session retirement.
    EpochRetained,
    /// The complete epoch was already retired.
    Retired,
}

#[derive(Debug)]
struct SessionEpochStorage {
    interner: Option<Interner>,
    retired: bool,
}

/// Cloneable coarse owner of one append-only symbol epoch.
///
/// Revision generations borrow the epoch exclusively while admitted. They do
/// not clone the interner or manufacture a second symbol domain. Dropping a
/// lease returns the exact physical interner to this owner, including during
/// unwinding.
#[derive(Clone, Debug)]
pub struct SessionInternerEpoch {
    storage: Arc<Mutex<SessionEpochStorage>>,
}

impl SessionInternerEpoch {
    #[must_use]
    pub fn new(budget: InternerBudget) -> Self {
        Self {
            storage: Arc::new(Mutex::new(SessionEpochStorage {
                interner: Some(Interner::new(budget)),
                retired: false,
            })),
        }
    }

    /// Whether two owners name the same physical session epoch.
    #[must_use]
    pub fn same_epoch(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.storage, &other.storage)
    }

    pub(crate) fn lease(&self) -> Result<InternerLease, SessionEpochError> {
        let mut storage = self
            .storage
            .lock()
            .expect("session interner epoch lock poisoned");
        if storage.retired {
            return Err(SessionEpochError::Retired);
        }
        let interner = storage
            .interner
            .take()
            .ok_or(SessionEpochError::GenerationAdmitted)?;
        Ok(InternerLease {
            storage: Arc::clone(&self.storage),
            interner: Some(interner),
        })
    }

    /// Retires the complete epoch exactly once after every generation owner
    /// and admitted lease has been released.
    pub fn retire(self) -> Result<InternerRetirement, SessionEpochError> {
        let storage =
            Arc::try_unwrap(self.storage).map_err(|_| SessionEpochError::EpochRetained)?;
        let mut storage = storage
            .into_inner()
            .expect("session interner epoch lock poisoned");
        if storage.retired {
            return Err(SessionEpochError::Retired);
        }
        let mut interner = storage
            .interner
            .take()
            .ok_or(SessionEpochError::GenerationAdmitted)?;
        let retirement = interner.retire().map_err(|_| SessionEpochError::Retired)?;
        storage.retired = true;
        Ok(retirement)
    }
}

/// Exclusive physical interner lease held by one admitted generation.
#[derive(Debug)]
pub(crate) struct InternerLease {
    storage: Arc<Mutex<SessionEpochStorage>>,
    interner: Option<Interner>,
}

impl Deref for InternerLease {
    type Target = Interner;

    fn deref(&self) -> &Self::Target {
        self.interner
            .as_ref()
            .expect("live epoch lease retains its interner")
    }
}

impl DerefMut for InternerLease {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.interner
            .as_mut()
            .expect("live epoch lease retains its interner")
    }
}

impl InternerLease {
    #[must_use]
    pub(crate) fn is_last_owner(&self) -> bool {
        Arc::strong_count(&self.storage) == 1
    }
}

impl Drop for InternerLease {
    fn drop(&mut self) {
        let Some(interner) = self.interner.take() else {
            return;
        };
        let mut storage = self
            .storage
            .lock()
            .expect("session interner epoch lock poisoned");
        debug_assert!(storage.interner.is_none());
        storage.interner = Some(interner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget() -> InternerBudget {
        InternerBudget::new(64, 128, 4096).expect("test budget")
    }

    #[test]
    fn one_epoch_lends_exactly_one_generation_and_preserves_symbols() {
        let epoch = SessionInternerEpoch::new(budget());
        let symbol = {
            let mut first = epoch.lease().expect("first generation admission");
            let symbol = first.intern("shared").expect("intern");
            assert_eq!(
                epoch.lease().unwrap_err(),
                SessionEpochError::GenerationAdmitted
            );
            symbol
        };
        let second = epoch.lease().expect("next generation admission");
        assert_eq!(
            second.resolve_id(symbol).expect("same epoch symbol"),
            "shared"
        );
    }

    #[test]
    fn foreign_epoch_rejects_a_qualified_symbol() {
        let first = SessionInternerEpoch::new(budget());
        let second = SessionInternerEpoch::new(budget());
        let symbol = first
            .lease()
            .expect("first epoch")
            .intern("foreign")
            .expect("intern");
        assert_eq!(
            second.lease().expect("second epoch").resolve_id(symbol),
            Err(crate::interner::InternerAccessError::ForeignEpoch)
        );
    }

    #[test]
    fn retirement_requires_the_last_coarse_owner() {
        let epoch = SessionInternerEpoch::new(budget());
        let retained = epoch.clone();
        assert_eq!(epoch.retire(), Err(SessionEpochError::EpochRetained));
        retained.retire().expect("last owner retires epoch");
    }
}
