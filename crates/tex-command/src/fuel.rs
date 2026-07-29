//! Monotonic resource accounting for canonical command processing.

/// Finite default for one standalone command-processing episode.
pub const DEFAULT_COMMAND_FUEL_LIMIT: u64 = 100_000_000;

/// A checked, monotonic command-work ledger.
///
/// This is deliberately separate from semantic [`crate::CommandState`] and
/// discardable [`crate::CommandRuntime`]. Snapshots and runtime resets cannot
/// therefore refund work or make resource policy part of format identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandFuel {
    limit: u64,
    burned: u64,
}

impl CommandFuel {
    /// Creates a finite ledger. Zero is promoted to one so every live
    /// processor can attempt at least one action and no sentinel means
    /// "unlimited".
    #[must_use]
    pub const fn new(limit: u64) -> Self {
        Self {
            limit: if limit == 0 { 1 } else { limit },
            burned: 0,
        }
    }

    #[must_use]
    pub const fn limit(self) -> u64 {
        self.limit
    }

    #[must_use]
    pub const fn burned(self) -> u64 {
        self.burned
    }

    pub(crate) fn charge(&mut self) -> Result<(), crate::CommandError> {
        let attempted = self
            .burned
            .checked_add(1)
            .ok_or(crate::CommandError::FuelExhausted {
                limit: self.limit,
                burned: self.burned,
            })?;
        if attempted > self.limit {
            return Err(crate::CommandError::FuelExhausted {
                limit: self.limit,
                burned: self.burned,
            });
        }
        self.burned = attempted;
        Ok(())
    }
}

impl Default for CommandFuel {
    fn default() -> Self {
        Self::new(DEFAULT_COMMAND_FUEL_LIMIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_limit_funds_exactly_that_many_actions() {
        let mut fuel = CommandFuel::new(3);
        assert!(fuel.charge().is_ok());
        assert!(fuel.charge().is_ok());
        assert!(fuel.charge().is_ok());
        assert_eq!(
            fuel.charge(),
            Err(crate::CommandError::FuelExhausted {
                limit: 3,
                burned: 3
            })
        );
        assert_eq!(fuel.burned(), 3);
    }
}
