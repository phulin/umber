//! Monotonic resource accounting for canonical command processing.

/// Finite default for one standalone command-processing episode.
pub const DEFAULT_COMMAND_FUEL_LIMIT: u64 = 100_000_000;

/// Largest command-work budget admitted by the canonical engine.
pub const MAX_COMMAND_FUEL_LIMIT: u64 = 1_000_000_000;

/// Invalid command-work budget supplied by a host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandFuelLimitError {
    requested: u64,
}

impl CommandFuelLimitError {
    #[must_use]
    pub const fn requested(self) -> u64 {
        self.requested
    }
}

impl std::fmt::Display for CommandFuelLimitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "canonical command fuel limit {} is outside 1..={MAX_COMMAND_FUEL_LIMIT}",
            self.requested
        )
    }
}

impl std::error::Error for CommandFuelLimitError {}

/// A checked, monotonic command-work ledger.
///
/// This is deliberately separate from semantic [`crate::CommandState`].
/// Snapshots cannot therefore refund work or make resource policy part of
/// format identity.
#[derive(Debug, Eq, PartialEq)]
pub struct CommandFuel {
    limit: u64,
    burned: u64,
}

impl CommandFuel {
    const fn new(limit: u64) -> Result<Self, CommandFuelLimitError> {
        if limit == 0 || limit > MAX_COMMAND_FUEL_LIMIT {
            return Err(CommandFuelLimitError { requested: limit });
        }
        Ok(Self { limit, burned: 0 })
    }

    #[must_use]
    pub const fn limit(&self) -> u64 {
        self.limit
    }

    #[must_use]
    pub const fn burned(&self) -> u64 {
        self.burned
    }

    /// Charges one bounded command-machine transition.
    ///
    /// Execution-layer state machines use the same monotonic ledger as token
    /// delivery so rollback cannot refund work performed below the scanner.
    pub fn charge(&mut self) -> Result<(), crate::CommandError> {
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

/// Top-level owner of one canonical command session's monotonic work ledger.
///
/// Command-processing leaf APIs deliberately receive only `&mut
/// CommandFuel`. They cannot construct, replace, or reset a ledger because
/// [`CommandFuel`] has no public constructors and does not implement
/// [`Default`].
#[derive(Debug, Eq, PartialEq)]
pub struct CommandFuelLedger {
    fuel: CommandFuel,
}

impl CommandFuelLedger {
    /// Creates a session ledger with a checked finite limit.
    pub const fn new(limit: u64) -> Result<Self, CommandFuelLimitError> {
        match CommandFuel::new(limit) {
            Ok(fuel) => Ok(Self { fuel }),
            Err(error) => Err(error),
        }
    }

    /// Lends the authoritative ledger to a command-processing operation.
    pub const fn fuel_mut(&mut self) -> &mut CommandFuel {
        &mut self.fuel
    }

    #[must_use]
    pub const fn limit(&self) -> u64 {
        self.fuel.limit()
    }

    #[must_use]
    pub const fn burned(&self) -> u64 {
        self.fuel.burned()
    }
}

impl Default for CommandFuelLedger {
    fn default() -> Self {
        Self {
            fuel: CommandFuel {
                limit: DEFAULT_COMMAND_FUEL_LIMIT,
                burned: 0,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_limit_funds_exactly_that_many_actions() {
        let mut fuel = CommandFuel::new(3).expect("valid test limit");
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

    #[test]
    fn invalid_limits_are_rejected_instead_of_becoming_unlimited() {
        for requested in [0, MAX_COMMAND_FUEL_LIMIT + 1, u64::MAX] {
            assert_eq!(
                CommandFuel::new(requested),
                Err(CommandFuelLimitError { requested })
            );
        }
        assert_eq!(CommandFuel::new(1).expect("minimum").limit(), 1);
        assert_eq!(
            CommandFuel::new(MAX_COMMAND_FUEL_LIMIT)
                .expect("maximum")
                .limit(),
            MAX_COMMAND_FUEL_LIMIT
        );
    }

    #[test]
    fn default_is_positive_finite_and_within_the_hard_maximum() {
        let fuel = CommandFuelLedger::default();
        assert_eq!(fuel.limit(), DEFAULT_COMMAND_FUEL_LIMIT);
        assert!(fuel.limit() > 0);
        assert!(fuel.limit() <= MAX_COMMAND_FUEL_LIMIT);
        assert_ne!(fuel.limit(), u64::MAX);
    }
}
