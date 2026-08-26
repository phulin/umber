//! Monotonic resource accounting for canonical command processing.

/// Finite default for one standalone command-processing episode.
pub const DEFAULT_COMMAND_FUEL_LIMIT: u64 = 100_000_000;

/// Largest command-work budget admitted by the canonical engine.
///
/// Full INITEX format construction legitimately performs billions of charged
/// transitions while building Unicode and macro tables. Hosts still select a
/// smaller per-run budget and retain their independent time and memory guards.
pub const MAX_COMMAND_FUEL_LIMIT: u64 = 100_000_000_000;

/// Monotonic scalar-work accounting for one canonical command session.
///
/// These counters are operational evidence, not TeX state. They live beside
/// the fuel ledger so a semantic rollback cannot refund work already done.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CommandWorkCounters {
    /// Successful charges against the canonical command-work guard.
    pub fuel_charges: u64,
    /// Raw token frames converted into current commands.
    pub token_frame_steps: u64,
    /// Completed ordinary expanded-command deliveries.
    pub expanded_deliveries: u64,
    /// Control-sequence meanings read from the live environment.
    pub meaning_lookups: u64,
    /// Raw token frames delivered while TeX's scanner status is non-normal.
    pub scanner_tokens: u64,
    /// Expandable commands executed inside deferred-write expansion.
    pub write_expansions: u64,
}

impl CommandWorkCounters {
    const ZERO: Self = Self {
        fuel_charges: 0,
        token_frame_steps: 0,
        expanded_deliveries: 0,
        meaning_lookups: 0,
        scanner_tokens: 0,
        write_expansions: 0,
    };
}

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
    work: CommandWorkCounters,
}

impl CommandFuel {
    const fn new(limit: u64) -> Result<Self, CommandFuelLimitError> {
        if limit == 0 || limit > MAX_COMMAND_FUEL_LIMIT {
            return Err(CommandFuelLimitError { requested: limit });
        }
        Ok(Self {
            limit,
            work: CommandWorkCounters::ZERO,
        })
    }

    #[must_use]
    pub const fn limit(&self) -> u64 {
        self.limit
    }

    #[must_use]
    pub const fn burned(&self) -> u64 {
        self.work.fuel_charges
    }

    #[must_use]
    pub const fn work(&self) -> CommandWorkCounters {
        self.work
    }

    /// Charges one bounded command-machine transition.
    ///
    /// Execution-layer state machines use the same monotonic ledger as token
    /// delivery so rollback cannot refund work performed below the scanner.
    pub fn charge(&mut self) -> Result<(), crate::CommandError> {
        let attempted =
            self.work
                .fuel_charges
                .checked_add(1)
                .ok_or(crate::CommandError::FuelExhausted {
                    limit: self.limit,
                    burned: self.work.fuel_charges,
                    work: self.work,
                })?;
        if attempted > self.limit {
            return Err(crate::CommandError::FuelExhausted {
                limit: self.limit,
                burned: self.work.fuel_charges,
                work: self.work,
            });
        }
        self.work.fuel_charges = attempted;
        Ok(())
    }

    /// Charges a proven packed episode without replaying one counter update
    /// per canonical transition.
    ///
    /// The packed command processor computes the exact transition count in
    /// its admitted vocabulary. Fuel remains monotonic and a rejected charge
    /// burns the remaining budget, exactly as repeated scalar charges would
    /// before reporting exhaustion.
    pub fn charge_many(&mut self, amount: u64) -> Result<(), crate::CommandError> {
        let remaining = self.limit.saturating_sub(self.work.fuel_charges);
        if amount > remaining {
            self.work.fuel_charges = self.limit;
            return Err(crate::CommandError::FuelExhausted {
                limit: self.limit,
                burned: self.work.fuel_charges,
                work: self.work,
            });
        }
        self.work.fuel_charges = self.work.fuel_charges.saturating_add(amount);
        Ok(())
    }

    pub(crate) fn record_token_frame(&mut self, scanner: bool) {
        self.work.token_frame_steps = self.work.token_frame_steps.saturating_add(1);
        if scanner {
            self.work.scanner_tokens = self.work.scanner_tokens.saturating_add(1);
        }
    }

    pub(crate) fn record_expanded_delivery(&mut self) {
        self.work.expanded_deliveries = self.work.expanded_deliveries.saturating_add(1);
    }

    pub(crate) fn record_meaning_lookup(&mut self) {
        self.work.meaning_lookups = self.work.meaning_lookups.saturating_add(1);
    }

    pub(crate) fn record_write_expansion(&mut self) {
        self.work.write_expansions = self.work.write_expansions.saturating_add(1);
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

    #[must_use]
    pub const fn work(&self) -> CommandWorkCounters {
        self.fuel.work()
    }
}

impl Default for CommandFuelLedger {
    fn default() -> Self {
        Self {
            fuel: CommandFuel {
                limit: DEFAULT_COMMAND_FUEL_LIMIT,
                work: CommandWorkCounters::default(),
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
                burned: 3,
                work: CommandWorkCounters {
                    fuel_charges: 3,
                    ..CommandWorkCounters::default()
                },
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
