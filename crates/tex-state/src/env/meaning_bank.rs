//! Structure-of-arrays storage for Symbol-indexed control-sequence meanings.

use super::banks::{BankError, LEVEL_ZERO};

/// Structure-of-arrays storage for the control-sequence meaning namespace.
///
/// Meaning resolution reads only `values`, which is the one dense hot array
/// indexed by a compact [`Symbol`](crate::interner::Symbol) slot. Assignment,
/// grouping, and named-checkpoint machinery use the equally indexed cold
/// metadata arrays only on their mutation/rollback paths. All three arrays
/// reserve their complete session capacity before the first symbol is
/// admitted, so appending an admitted row never reallocates in steady state.
#[derive(Clone)]
pub(crate) struct MeaningBank<T: Clone> {
    values: Vec<T>,
    levels: Vec<u32>,
    save_serials: Vec<u64>,
    default: T,
}

impl<T: Clone> MeaningBank<T> {
    /// Allocates empty, once-reserved storage for a session meaning domain.
    pub(crate) fn new(capacity: usize, default: T) -> Result<Self, BankError> {
        let mut values = Vec::new();
        values
            .try_reserve_exact(capacity)
            .map_err(|_| BankError::AllocationFailed)?;
        let mut levels = Vec::new();
        levels
            .try_reserve_exact(capacity)
            .map_err(|_| BankError::AllocationFailed)?;
        let mut save_serials = Vec::new();
        save_serials
            .try_reserve_exact(capacity)
            .map_err(|_| BankError::AllocationFailed)?;
        Ok(Self {
            values,
            levels,
            save_serials,
            default,
        })
    }

    /// Allocates a format prefix while retaining the session's full future
    /// Symbol capacity for names admitted after the format is loaded.
    pub(crate) fn format_prefix(
        len: usize,
        capacity: usize,
        default: T,
    ) -> Result<Self, BankError> {
        if len > capacity {
            return Err(BankError::IndexOutOfBounds);
        }
        let mut bank = Self::new(capacity, default.clone())?;
        bank.values.resize(len, default);
        bank.levels.resize(len, LEVEL_ZERO);
        bank.save_serials.resize(len, 0);
        Ok(bank)
    }

    /// Admits every row through `index`, preserving direct Symbol indexing.
    /// Interner slots are append-only but can include retained spellings, so
    /// a control symbol may require filling undefined rows for earlier slots.
    pub(crate) fn admit_through(&mut self, index: u32) -> Result<(), BankError> {
        let required = usize::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(1))
            .ok_or(BankError::IndexOutOfBounds)?;
        if required > self.values.capacity()
            || required > self.levels.capacity()
            || required > self.save_serials.capacity()
        {
            return Err(BankError::IndexOutOfBounds);
        }
        if required <= self.values.len() {
            return Ok(());
        }
        debug_assert_eq!(self.values.len(), self.levels.len());
        debug_assert_eq!(self.values.len(), self.save_serials.len());
        self.values.resize(required, self.default.clone());
        self.levels.resize(required, LEVEL_ZERO);
        self.save_serials.resize(required, 0);
        Ok(())
    }

    /// Borrows the hot meaning word with one direct bounds-checked lookup.
    #[inline(always)]
    pub(crate) fn get_ref(&self, index: u32) -> Result<&T, BankError> {
        self.values
            .get(usize::try_from(index).map_err(|_| BankError::IndexOutOfBounds)?)
            .ok_or(BankError::IndexOutOfBounds)
    }

    /// Reads a complete row for cold mutation/journal coordination.
    #[inline]
    pub(crate) fn row(&self, index: u32) -> Result<(T, u32, u64), BankError> {
        let index = usize::try_from(index).map_err(|_| BankError::IndexOutOfBounds)?;
        Ok((
            self.values
                .get(index)
                .cloned()
                .ok_or(BankError::IndexOutOfBounds)?,
            *self.levels.get(index).ok_or(BankError::IndexOutOfBounds)?,
            *self
                .save_serials
                .get(index)
                .ok_or(BankError::IndexOutOfBounds)?,
        ))
    }

    /// Writes one coordinated row after journaling has captured its prior
    /// value and metadata.
    #[inline]
    pub(crate) fn write(
        &mut self,
        index: u32,
        value: T,
        level: u32,
        save_serial: u64,
    ) -> Result<(), BankError> {
        let index = usize::try_from(index).map_err(|_| BankError::IndexOutOfBounds)?;
        *self
            .values
            .get_mut(index)
            .ok_or(BankError::IndexOutOfBounds)? = value;
        *self
            .levels
            .get_mut(index)
            .ok_or(BankError::IndexOutOfBounds)? = level;
        *self
            .save_serials
            .get_mut(index)
            .ok_or(BankError::IndexOutOfBounds)? = save_serial;
        Ok(())
    }

    /// Swaps a checkpoint alternate with one live row, including its cold
    /// metadata, without constructing an interleaved temporary cell.
    #[inline]
    pub(crate) fn swap(
        &mut self,
        index: u32,
        value: &mut T,
        level: &mut u32,
        save_serial: &mut u64,
    ) -> Result<(), BankError> {
        let index = usize::try_from(index).map_err(|_| BankError::IndexOutOfBounds)?;
        std::mem::swap(
            self.values
                .get_mut(index)
                .ok_or(BankError::IndexOutOfBounds)?,
            value,
        );
        std::mem::swap(
            self.levels
                .get_mut(index)
                .ok_or(BankError::IndexOutOfBounds)?,
            level,
        );
        std::mem::swap(
            self.save_serials
                .get_mut(index)
                .ok_or(BankError::IndexOutOfBounds)?,
            save_serial,
        );
        Ok(())
    }

    pub(crate) fn values(&self) -> impl Iterator<Item = T> + '_ {
        self.values.iter().cloned()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.values.len()
    }

    #[cfg(test)]
    pub(crate) fn capacity(&self) -> usize {
        self.values.capacity()
    }
}

#[cfg(test)]
#[path = "meaning_bank/tests.rs"]
mod tests;
