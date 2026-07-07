//! Monotonic state epoch stamps.

/// A monotonic state epoch.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Epoch(u32);

impl Epoch {
    /// The epoch stamp for never-written cells.
    pub(crate) const ZERO: Self = Self(0);

    /// The first epoch in a session.
    pub const START: Self = Self(1);

    /// Returns the raw epoch value.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Advances to the next epoch.
    pub fn bump(&mut self) {
        self.0 = match self.0.checked_add(1) {
            Some(value) => value,
            None => panic!("epoch overflow"),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::Epoch;

    #[test]
    fn epoch_starts_at_one_and_bumps() {
        let mut epoch = Epoch::START;

        assert_eq!(epoch.raw(), 1);
        epoch.bump();
        assert_eq!(epoch.raw(), 2);
    }
}
