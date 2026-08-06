//! Internal execution receipts and temporary differential migration support.
//!
//! This vocabulary is deliberately below every public or portable wire
//! boundary.  It describes what one aggregate main-control operation
//! committed; it does not replace `tex-command` observations and must not be
//! translated into a new `tex-oracle` schema.

use tex_command::{
    CommandObservation, CommandObserver, DiagnosticRecord, EffectRecord, FatalError, MutationRecord,
};
use tex_state::{ContentHash, EffectRecord as WorldEffectRecord};

use crate::ResourceNeed;

/// The legacy entry shape exercised by one aggregate operation.
///
/// The variants exist only while the old entry points are different.  The
/// final unified operation path can delete this selector and the shadow
/// harness together.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OperationKind {
    Ordinary,
    Observed,
    Nested,
    Alignment,
}

/// The terminal control-flow fact committed by an operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OperationTermination {
    Continue,
    End,
    EndOfInput,
    Suspended,
    Failed,
    Fatal(FatalError),
}

/// Exact externally visible effects in both their semantic and live forms.
///
/// `semantic` retains command-observer ordering and naming. `world` retains
/// the actual virtual output records. Keeping both prevents a shadow match in
/// one projection from hiding a divergence in the other.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ExecutionEffects {
    pub(crate) semantic: Vec<EffectRecord>,
    pub(crate) world: Vec<WorldEffectRecord>,
}

/// Complete typed result of one aggregate execution operation.
///
/// Full ordered command evidence remains in an optional evidence sink. These
/// categorized fields are the application/commit facts later migration
/// children consume directly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExecutionReceipt {
    pub(crate) operation: OperationKind,
    pub(crate) mutations: Vec<MutationRecord>,
    pub(crate) resources: Vec<ResourceNeed>,
    pub(crate) effects: ExecutionEffects,
    pub(crate) artifacts: Vec<ContentHash>,
    pub(crate) diagnostics: Vec<DiagnosticRecord>,
    pub(crate) termination: OperationTermination,
}

impl ExecutionReceipt {
    pub(crate) fn new(operation: OperationKind, termination: OperationTermination) -> Self {
        Self {
            operation,
            mutations: Vec::new(),
            resources: Vec::new(),
            effects: ExecutionEffects::default(),
            artifacts: Vec::new(),
            diagnostics: Vec::new(),
            termination,
        }
    }

    /// Captures the receipt-owned projection of one already committed record.
    pub(crate) fn capture_observation(&mut self, observation: &CommandObservation) {
        match observation {
            CommandObservation::Mutation(record) => self.mutations.push(record.clone()),
            CommandObservation::Diagnostic(record) => self.diagnostics.push(record.clone()),
            CommandObservation::Effect(record) => self.effects.semantic.push(record.clone()),
            _ => {}
        }
    }

    fn record_count(&self) -> usize {
        self.mutations
            .len()
            .saturating_add(self.resources.len())
            .saturating_add(self.effects.semantic.len())
            .saturating_add(self.effects.world.len())
            .saturating_add(self.artifacts.len())
            .saturating_add(self.diagnostics.len())
            .saturating_add(1)
    }
}

/// Operation-local optional evidence destination.
///
/// The disabled form allocates nothing. The buffered form has a hard record
/// ceiling and marks overflow instead of retaining an unbounded prefix.
#[derive(Debug)]
pub(crate) enum EvidenceSink {
    Disabled,
    Buffered(BoundedEvidence),
}

impl EvidenceSink {
    pub(crate) const fn disabled() -> Self {
        Self::Disabled
    }

    pub(crate) fn bounded(max_records: usize, observes_geometry: bool) -> Self {
        Self::Buffered(BoundedEvidence::new(max_records, observes_geometry))
    }

    pub(crate) fn as_observer(&mut self) -> Option<&mut dyn CommandObserver> {
        match self {
            Self::Disabled => None,
            Self::Buffered(buffer) => Some(buffer),
        }
    }

    fn into_evidence(self) -> Result<Vec<CommandObservation>, ShadowLimitExceeded> {
        match self {
            Self::Disabled => Ok(Vec::new()),
            Self::Buffered(buffer) => buffer.into_evidence(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct BoundedEvidence {
    max_records: usize,
    observes_geometry: bool,
    overflowed: bool,
    records: Vec<CommandObservation>,
}

impl BoundedEvidence {
    fn new(max_records: usize, observes_geometry: bool) -> Self {
        Self {
            max_records,
            observes_geometry,
            overflowed: false,
            records: Vec::with_capacity(max_records.min(64)),
        }
    }

    fn into_evidence(self) -> Result<Vec<CommandObservation>, ShadowLimitExceeded> {
        if self.overflowed {
            Err(ShadowLimitExceeded::EvidenceRecords {
                limit: self.max_records,
            })
        } else {
            Ok(self.records)
        }
    }
}

impl CommandObserver for BoundedEvidence {
    fn observes_geometry(&self) -> bool {
        self.observes_geometry
    }

    fn committed(&mut self, observation: CommandObservation) {
        if self.records.len() < self.max_records {
            self.records.push(observation);
        } else {
            self.overflowed = true;
        }
    }
}

/// Hard ceilings for a temporary old-versus-new comparison.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ShadowLimits {
    pub(crate) max_steps: usize,
    pub(crate) max_state_bytes: usize,
    pub(crate) max_evidence_records: usize,
    pub(crate) max_receipt_records: usize,
}

/// Exact detached state preimage supplied by each shadow implementation.
///
/// The harness compares bytes, not a hash, so collisions cannot conceal a
/// state mismatch. Construction enforces the retained-memory ceiling before
/// the evidence can enter a comparison result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExactStateEvidence(Vec<u8>);

impl ExactStateEvidence {
    pub(crate) fn new(bytes: Vec<u8>, limit: usize) -> Result<Self, ShadowLimitExceeded> {
        if bytes.len() > limit {
            return Err(ShadowLimitExceeded::StateBytes {
                limit,
                actual: bytes.len(),
            });
        }
        Ok(Self(bytes))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ShadowOutcome {
    pub(crate) state: ExactStateEvidence,
    pub(crate) receipt: ExecutionReceipt,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ShadowLimitExceeded {
    Steps { limit: usize },
    StateBytes { limit: usize, actual: usize },
    EvidenceRecords { limit: usize },
    ReceiptRecords { limit: usize, actual: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShadowComponent {
    State,
    Receipt,
    Evidence,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ShadowError {
    Limit {
        operation: OperationKind,
        side: ShadowSide,
        error: ShadowLimitExceeded,
    },
    Divergence {
        operation: OperationKind,
        component: ShadowComponent,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShadowSide {
    Legacy,
    Replacement,
}

/// Temporary exact differential runner used while operation paths coexist.
pub(crate) struct ShadowDifferentialHarness {
    limits: ShadowLimits,
    steps: usize,
}

impl ShadowDifferentialHarness {
    pub(crate) const fn new(limits: ShadowLimits) -> Self {
        Self { limits, steps: 0 }
    }

    pub(crate) fn compare<Legacy, Replacement>(
        &mut self,
        operation: OperationKind,
        legacy: Legacy,
        replacement: Replacement,
    ) -> Result<ShadowOutcome, ShadowError>
    where
        Legacy: FnOnce(&mut EvidenceSink) -> Result<ShadowOutcome, ShadowLimitExceeded>,
        Replacement: FnOnce(&mut EvidenceSink) -> Result<ShadowOutcome, ShadowLimitExceeded>,
    {
        if self.steps >= self.limits.max_steps {
            return Err(ShadowError::Limit {
                operation,
                side: ShadowSide::Legacy,
                error: ShadowLimitExceeded::Steps {
                    limit: self.limits.max_steps,
                },
            });
        }
        self.steps += 1;

        let (legacy_outcome, legacy_evidence) = self.run(operation, ShadowSide::Legacy, legacy)?;
        let (replacement_outcome, replacement_evidence) =
            self.run(operation, ShadowSide::Replacement, replacement)?;

        if legacy_outcome.state != replacement_outcome.state {
            return Err(ShadowError::Divergence {
                operation,
                component: ShadowComponent::State,
            });
        }
        if legacy_outcome.receipt != replacement_outcome.receipt {
            return Err(ShadowError::Divergence {
                operation,
                component: ShadowComponent::Receipt,
            });
        }
        if legacy_evidence != replacement_evidence {
            return Err(ShadowError::Divergence {
                operation,
                component: ShadowComponent::Evidence,
            });
        }
        Ok(replacement_outcome)
    }

    fn run<Run>(
        &self,
        operation: OperationKind,
        side: ShadowSide,
        run: Run,
    ) -> Result<(ShadowOutcome, Vec<CommandObservation>), ShadowError>
    where
        Run: FnOnce(&mut EvidenceSink) -> Result<ShadowOutcome, ShadowLimitExceeded>,
    {
        let mut sink = EvidenceSink::bounded(self.limits.max_evidence_records, true);
        let outcome = run(&mut sink).map_err(|error| ShadowError::Limit {
            operation,
            side,
            error,
        })?;
        let receipt_records = outcome.receipt.record_count();
        if receipt_records > self.limits.max_receipt_records {
            return Err(ShadowError::Limit {
                operation,
                side,
                error: ShadowLimitExceeded::ReceiptRecords {
                    limit: self.limits.max_receipt_records,
                    actual: receipt_records,
                },
            });
        }
        let evidence = sink.into_evidence().map_err(|error| ShadowError::Limit {
            operation,
            side,
            error,
        })?;
        Ok((outcome, evidence))
    }
}

#[cfg(test)]
#[path = "execution_receipt/tests.rs"]
mod tests;
