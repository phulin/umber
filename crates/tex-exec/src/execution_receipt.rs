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
    fn new(operation: OperationKind, termination: OperationTermination) -> Self {
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
    fn capture_observation(&mut self, observation: &CommandObservation) {
        match observation {
            CommandObservation::Mutation(record) => self.mutations.push(record.clone()),
            CommandObservation::Diagnostic(record) => self.diagnostics.push(record.clone()),
            CommandObservation::Effect(record) => self.effects.semantic.push(record.clone()),
            _ => {}
        }
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

    pub(crate) fn into_evidence(self) -> Result<Vec<CommandObservation>, ShadowLimitExceeded> {
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
/// state mismatch. Only [`ShadowCapture`] can construct this value; its
/// append-time ceiling prevents an implementation from first handing the
/// harness an already-unbounded detached `Vec`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExactStateEvidence(Vec<u8>);

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

/// Bounded capture owned by one side of a shadow comparison.
///
/// State bytes and receipt records enter only through append-time checked
/// methods. Once a ceiling is crossed, the capture retains no more records and
/// reports the exact attempted size when the runner returns. Command evidence
/// uses the independently bounded [`EvidenceSink`].
pub(crate) struct ShadowCapture {
    limits: ShadowLimits,
    state: Vec<u8>,
    attempted_state_bytes: usize,
    receipt: ExecutionReceipt,
    attempted_receipt_records: usize,
    evidence: EvidenceSink,
}

impl ShadowCapture {
    pub(crate) fn new(operation: OperationKind, limits: ShadowLimits) -> Self {
        Self {
            limits,
            state: Vec::with_capacity(limits.max_state_bytes.min(4096)),
            attempted_state_bytes: 0,
            receipt: ExecutionReceipt::new(operation, OperationTermination::Continue),
            attempted_receipt_records: 1,
            evidence: EvidenceSink::bounded(limits.max_evidence_records, true),
        }
    }

    pub(crate) fn record_state_bytes(&mut self, bytes: &[u8]) {
        self.attempted_state_bytes = self.attempted_state_bytes.saturating_add(bytes.len());
        if self.attempted_state_bytes <= self.limits.max_state_bytes {
            self.state.extend_from_slice(bytes);
        }
    }

    pub(crate) fn record_observation(&mut self, observation: CommandObservation) {
        let receipt_records = usize::from(matches!(
            &observation,
            CommandObservation::Mutation(_)
                | CommandObservation::Diagnostic(_)
                | CommandObservation::Effect(_)
        ));
        if self.reserve_receipt_records(receipt_records) {
            self.receipt.capture_observation(&observation);
        }
        if let Some(observer) = self.evidence.as_observer() {
            observer.committed(observation);
        }
    }

    pub(crate) fn record_resource(&mut self, resource: ResourceNeed) {
        if self.reserve_receipt_records(1) {
            self.receipt.resources.push(resource);
        }
    }

    pub(crate) fn record_world_effect(&mut self, effect: WorldEffectRecord) {
        if self.reserve_receipt_records(1) {
            self.receipt.effects.world.push(effect);
        }
    }

    pub(crate) fn record_artifact(&mut self, artifact: ContentHash) {
        if self.reserve_receipt_records(1) {
            self.receipt.artifacts.push(artifact);
        }
    }

    pub(crate) fn set_termination(&mut self, termination: OperationTermination) {
        self.receipt.termination = termination;
    }

    fn reserve_receipt_records(&mut self, additional: usize) -> bool {
        self.attempted_receipt_records = self.attempted_receipt_records.saturating_add(additional);
        self.attempted_receipt_records <= self.limits.max_receipt_records
    }

    pub(crate) fn retained_state_bytes(&self) -> usize {
        self.state.len()
    }

    pub(crate) fn retained_receipt_records(&self) -> usize {
        self.receipt.mutations.len()
            + self.receipt.resources.len()
            + self.receipt.effects.semantic.len()
            + self.receipt.effects.world.len()
            + self.receipt.artifacts.len()
            + self.receipt.diagnostics.len()
            + 1
    }

    pub(crate) fn finish(
        self,
    ) -> Result<(ShadowOutcome, Vec<CommandObservation>), ShadowLimitExceeded> {
        if self.attempted_state_bytes > self.limits.max_state_bytes {
            return Err(ShadowLimitExceeded::StateBytes {
                limit: self.limits.max_state_bytes,
                actual: self.attempted_state_bytes,
            });
        }
        if self.attempted_receipt_records > self.limits.max_receipt_records {
            return Err(ShadowLimitExceeded::ReceiptRecords {
                limit: self.limits.max_receipt_records,
                actual: self.attempted_receipt_records,
            });
        }
        let evidence = self.evidence.into_evidence()?;
        Ok((
            ShadowOutcome {
                state: ExactStateEvidence(self.state),
                receipt: self.receipt,
            },
            evidence,
        ))
    }
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
        Legacy: FnOnce(&mut ShadowCapture) -> Result<(), ShadowLimitExceeded>,
        Replacement: FnOnce(&mut ShadowCapture) -> Result<(), ShadowLimitExceeded>,
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
        Run: FnOnce(&mut ShadowCapture) -> Result<(), ShadowLimitExceeded>,
    {
        let mut capture = ShadowCapture::new(operation, self.limits);
        run(&mut capture).map_err(|error| ShadowError::Limit {
            operation,
            side,
            error,
        })?;
        capture.finish().map_err(|error| ShadowError::Limit {
            operation,
            side,
            error,
        })
    }
}
