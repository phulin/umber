//! Typed receipt for one observed aggregate main-control operation.
//!
//! The receipt is operation-local and shares the evidence sink's hard record
//! ceiling. It is not a portable wire type and introduces no oracle schema.

use tex_command::{CommandObservation, DiagnosticRecord, EffectRecord, FatalError, MutationRecord};
use tex_state::{ContentHash, EffectRecord as WorldEffectRecord};

use crate::ResourceNeed;

pub(crate) const MAX_EXECUTION_RECEIPT_RECORDS: usize = 1_000_000;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum OperationTermination {
    #[default]
    Continue,
    End,
    EndOfInput,
    Suspended,
    Failed,
    Fatal(FatalError),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ExecutionEffects {
    pub(crate) semantic: Vec<EffectRecord>,
    pub(crate) world: Vec<WorldEffectRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExecutionReceipt {
    pub(crate) mutations: Vec<MutationRecord>,
    pub(crate) resources: Vec<ResourceNeed>,
    pub(crate) effects: ExecutionEffects,
    pub(crate) artifacts: Vec<ContentHash>,
    pub(crate) diagnostics: Vec<DiagnosticRecord>,
    pub(crate) termination: OperationTermination,
    limit: usize,
}

impl Default for ExecutionReceipt {
    fn default() -> Self {
        Self {
            mutations: Vec::new(),
            resources: Vec::new(),
            effects: ExecutionEffects::default(),
            artifacts: Vec::new(),
            diagnostics: Vec::new(),
            termination: OperationTermination::Continue,
            limit: MAX_EXECUTION_RECEIPT_RECORDS,
        }
    }
}

impl ExecutionReceipt {
    pub(crate) fn capture_observation(&mut self, observation: &CommandObservation) -> bool {
        match observation {
            CommandObservation::Mutation(record) => self.push_mutation(record.clone()),
            CommandObservation::Diagnostic(record) => self.push_diagnostic(record.clone()),
            CommandObservation::Effect(record) => self.push_semantic_effect(record.clone()),
            _ => true,
        }
    }

    pub(crate) fn record_count(&self) -> usize {
        self.mutations
            .len()
            .saturating_add(self.resources.len())
            .saturating_add(self.effects.semantic.len())
            .saturating_add(self.effects.world.len())
            .saturating_add(self.artifacts.len())
            .saturating_add(self.diagnostics.len())
            .saturating_add(1)
    }

    fn has_capacity(&self) -> bool {
        self.record_count() < self.limit
    }

    pub(crate) const fn limit(&self) -> usize {
        self.limit
    }

    fn push_mutation(&mut self, record: MutationRecord) -> bool {
        if !self.has_capacity() {
            return false;
        }
        self.mutations.push(record);
        true
    }

    fn push_diagnostic(&mut self, record: DiagnosticRecord) -> bool {
        if !self.has_capacity() {
            return false;
        }
        self.diagnostics.push(record);
        true
    }

    fn push_semantic_effect(&mut self, record: EffectRecord) -> bool {
        if !self.has_capacity() {
            return false;
        }
        self.effects.semantic.push(record);
        true
    }

    pub(crate) fn record_resource(&mut self, resource: ResourceNeed) -> bool {
        if !self.has_capacity() {
            return false;
        }
        self.resources.push(resource);
        true
    }

    pub(crate) fn record_world_effect(&mut self, effect: WorldEffectRecord) -> bool {
        if !self.has_capacity() {
            return false;
        }
        self.effects.world.push(effect);
        true
    }

    pub(crate) fn record_artifact(&mut self, artifact: ContentHash) -> bool {
        if !self.has_capacity() {
            return false;
        }
        self.artifacts.push(artifact);
        true
    }

    pub(crate) fn set_termination(&mut self, termination: OperationTermination) {
        self.termination = termination;
    }

    fn consumed_projection(&self) -> ConsumedExecutionReceipt {
        ConsumedExecutionReceipt {
            records: self
                .mutations
                .len()
                .saturating_add(self.resources.len())
                .saturating_add(self.effects.semantic.len())
                .saturating_add(self.effects.world.len())
                .saturating_add(self.artifacts.len())
                .saturating_add(self.diagnostics.len())
                .saturating_add(1),
            termination: self.termination,
        }
    }

    /// Consumes one operation's projection while retaining its warmed storage
    /// for the next operation owned by the same observation buffer.
    pub(crate) fn reset_for_next_operation(&mut self) -> ConsumedExecutionReceipt {
        let consumed = self.consumed_projection();
        self.mutations.clear();
        self.resources.clear();
        self.effects.semantic.clear();
        self.effects.world.clear();
        self.artifacts.clear();
        self.diagnostics.clear();
        self.termination = OperationTermination::Continue;
        consumed
    }

    /// Consumes the typed projection at the observer publication seam.
    ///
    /// Reading every category here is deliberate: the receipt is an active
    /// internal commit contract, not a shadow value built beside ordered
    /// observations and then discarded. The returned projection lets the
    /// caller verify the terminal fact against the result it publishes.
    pub(crate) fn consume(self) -> ConsumedExecutionReceipt {
        self.consumed_projection()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ConsumedExecutionReceipt {
    pub(crate) records: usize,
    pub(crate) termination: OperationTermination,
}

#[cfg(test)]
mod tests;
