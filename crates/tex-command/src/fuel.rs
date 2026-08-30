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
    const fn with_fuel_charges(fuel_charges: u64, detail: CommandWorkDetail) -> Self {
        Self {
            fuel_charges,
            token_frame_steps: detail.token_frame_steps,
            expanded_deliveries: detail.expanded_deliveries,
            meaning_lookups: detail.meaning_lookups,
            scanner_tokens: detail.scanner_tokens,
            write_expansions: detail.write_expansions,
        }
    }
}

/// Work telemetry that is independent of the command-fuel countdown.
///
/// Fuel consumption has no second stored representation: publication derives
/// it from the immutable admitted limit and the one mutable remaining count.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CommandWorkDetail {
    token_frame_steps: u64,
    expanded_deliveries: u64,
    meaning_lookups: u64,
    scanner_tokens: u64,
    write_expansions: u64,
}

impl CommandWorkDetail {
    const ZERO: Self = Self {
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
    remaining: u64,
    work: CommandWorkDetail,
}

impl CommandFuel {
    const fn new(limit: u64) -> Result<Self, CommandFuelLimitError> {
        if limit == 0 || limit > MAX_COMMAND_FUEL_LIMIT {
            return Err(CommandFuelLimitError { requested: limit });
        }
        Ok(Self {
            limit,
            remaining: limit,
            work: CommandWorkDetail::ZERO,
        })
    }

    #[must_use]
    pub const fn limit(&self) -> u64 {
        self.limit
    }

    #[must_use]
    pub const fn burned(&self) -> u64 {
        self.limit - self.remaining
    }

    #[must_use]
    pub const fn work(&self) -> CommandWorkCounters {
        CommandWorkCounters::with_fuel_charges(self.burned(), self.work)
    }

    /// Charges one bounded command-machine transition.
    ///
    /// Execution-layer state machines use the same monotonic ledger as token
    /// delivery so rollback cannot refund work performed below the scanner.
    pub fn charge(&mut self) -> Result<(), crate::CommandError> {
        if self.remaining == 0 {
            return Err(self.exhausted_error());
        }
        self.remaining -= 1;
        Ok(())
    }

    #[cold]
    #[inline(never)]
    fn exhausted_error(&self) -> crate::CommandError {
        let work = self.work();
        crate::CommandError::FuelExhausted {
            limit: self.limit,
            burned: work.fuel_charges,
            work,
        }
    }

    /// Commits the exact work produced by one resolved raw delivery.
    ///
    /// Resolution knows all three facts at once, so the hot pipeline updates
    /// the singular ledger once instead of repeatedly reborrowing it. The
    /// preceding fuel charge remains separate and happens before input work.
    pub(crate) fn record_raw_delivery(&mut self, scanner: bool, meaning_lookup: bool) {
        self.work.token_frame_steps = self.work.token_frame_steps.saturating_add(1);
        if scanner {
            self.work.scanner_tokens = self.work.scanner_tokens.saturating_add(1);
        }
        if meaning_lookup {
            self.work.meaning_lookups = self.work.meaning_lookups.saturating_add(1);
        }
    }

    pub(crate) fn record_expanded_delivery(&mut self) {
        self.work.expanded_deliveries = self.work.expanded_deliveries.saturating_add(1);
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
                remaining: DEFAULT_COMMAND_FUEL_LIMIT,
                work: CommandWorkDetail::ZERO,
            },
        }
    }
}

#[cfg(test)]
mod tests;
