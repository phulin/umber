//! Typed receipt for one observed aggregate main-control operation.
//!
//! The receipt is operation-local and shares the evidence sink's hard record
//! ceiling. It is not a portable wire type and introduces no oracle schema.

use tex_command::{CommandObservation, DiagnosticRecord, EffectRecord, FatalError, MutationRecord};
use tex_state::{ContentHash, EffectRecord as WorldEffectRecord};

use crate::ResourceNeed;

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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ExecutionReceipt {
    pub(crate) mutations: Vec<MutationRecord>,
    pub(crate) resources: Vec<ResourceNeed>,
    pub(crate) effects: ExecutionEffects,
    pub(crate) artifacts: Vec<ContentHash>,
    pub(crate) diagnostics: Vec<DiagnosticRecord>,
    pub(crate) termination: OperationTermination,
}

impl ExecutionReceipt {
    pub(crate) fn capture_observation(&mut self, observation: &CommandObservation) {
        match observation {
            CommandObservation::Mutation(record) => self.mutations.push(record.clone()),
            CommandObservation::Diagnostic(record) => self.diagnostics.push(record.clone()),
            CommandObservation::Effect(record) => self.effects.semantic.push(record.clone()),
            _ => {}
        }
    }

    pub(crate) fn record_resource(&mut self, resource: ResourceNeed) {
        self.resources.push(resource);
    }

    pub(crate) fn record_world_effect(&mut self, effect: WorldEffectRecord) {
        self.effects.world.push(effect);
    }

    pub(crate) fn record_artifact(&mut self, artifact: ContentHash) {
        self.artifacts.push(artifact);
    }

    pub(crate) fn set_termination(&mut self, termination: OperationTermination) {
        self.termination = termination;
    }
}
