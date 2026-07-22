use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixedPointLimits {
    pub attempts: u32,
    pub passes: u32,
}

impl Default for FixedPointLimits {
    fn default() -> Self {
        Self {
            attempts: 32,
            passes: 8,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FixedPointFailure {
    InvalidLimit { name: &'static str, value: u32 },
    AttemptLimit { limit: u32 },
    NoProgress,
    PassLimit { limit: u32 },
    Oscillation { first_pass: u32, repeated_pass: u32 },
}

pub(crate) struct FixedPointCoordinator {
    limits: FixedPointLimits,
    attempts: u32,
}

impl FixedPointCoordinator {
    pub(crate) fn new(limits: FixedPointLimits) -> Result<Self, FixedPointFailure> {
        for (name, value, hard) in [
            ("attempt", limits.attempts, 128),
            ("pass", limits.passes, 64),
        ] {
            if value == 0 || value > hard {
                return Err(FixedPointFailure::InvalidLimit { name, value });
            }
        }
        Ok(Self {
            limits,
            attempts: 0,
        })
    }

    pub(crate) fn begin<K: Ord>(&self, initial: K) -> FixedPointCandidate<K> {
        FixedPointCandidate {
            pass: 1,
            seen: BTreeMap::from([(initial, 0)]),
        }
    }

    pub(crate) fn start_attempt(&mut self, made_progress: bool) -> Result<(), FixedPointFailure> {
        if self.attempts >= self.limits.attempts {
            return Err(FixedPointFailure::AttemptLimit {
                limit: self.limits.attempts,
            });
        }
        if !made_progress {
            return Err(FixedPointFailure::NoProgress);
        }
        self.attempts += 1;
        Ok(())
    }

    pub(crate) const fn attempts(&self) -> u32 {
        self.attempts
    }

    pub(crate) fn reset_attempts(&mut self) {
        self.attempts = 0;
    }
}

pub(crate) struct FixedPointCandidate<K> {
    pass: u32,
    seen: BTreeMap<K, u32>,
}

impl<K: Ord> FixedPointCandidate<K> {
    pub(crate) const fn pass(&self) -> u32 {
        self.pass
    }

    pub(crate) fn observe_changed(
        &mut self,
        key: K,
        limits: FixedPointLimits,
    ) -> Result<(), FixedPointFailure> {
        if let Some(first_pass) = self.seen.insert(key, self.pass) {
            return Err(FixedPointFailure::Oscillation {
                first_pass,
                repeated_pass: self.pass,
            });
        }
        self.pass += 1;
        if self.pass > limits.passes {
            return Err(FixedPointFailure::PassLimit {
                limit: limits.passes,
            });
        }
        Ok(())
    }
}
