//! Allocation-free views of the authoritative command state.

use tex_state::CommandContext;

use super::CommandState;
use crate::processor::AlignmentDeliveryState;

impl<G> CommandState<G> {
    /// Publishes the command-owned dependency roots read by one processor
    /// episode. Complex continuations without a complete canonical projection
    /// poison the outer region before the processor can inspect them.
    pub(crate) fn observe_tracked_dependencies(&self, state: &mut CommandContext<'_, G>) {
        if !state.tracked_region_is_active() {
            return;
        }
        let Some((line, mut stack)) = crate::input::tracked_input_projection(&self.input, state)
        else {
            state.unsupported_command_state();
            return;
        };
        let supported_continuation = self.scratch.is_quiescent()
            && !self.input.levels.arguments.is_matching()
            && self.scanner.is_quiescent()
            && self.alignment == AlignmentDeliveryState::<G>::default()
            && self.transient == super::TransientState::default()
            && self.replay_completions.is_empty()
            && self.semantic_diagnostics.is_empty()
            && !self.name_in_progress
            && self.named_token_list_pushes.is_empty();
        if !supported_continuation {
            state.unsupported_command_state();
            return;
        }
        stack ^= self.profile().fingerprint().get();
        state.observe_command_projection(
            tex_state::DependencyKey::InputLine,
            tex_state::DependencyValue::Projection {
                schema: 1,
                fingerprint: line,
            },
        );
        state.observe_command_projection(
            tex_state::DependencyKey::InputStack,
            tex_state::DependencyValue::Projection {
                schema: 1,
                fingerprint: stack,
            },
        );

        let (level, ty, branch) = self.conditions.current_etex_values();
        for (field, value) in [
            (tex_state::DependencyEngineField::ConditionLevel, level),
            (tex_state::DependencyEngineField::ConditionType, ty),
            (tex_state::DependencyEngineField::ConditionBranch, branch),
        ] {
            state.observe_command_projection(
                tex_state::DependencyKey::Engine(field),
                tex_state::DependencyValue::Integer(i64::from(value)),
            );
        }
        state.observe_command_projection(
            tex_state::DependencyKey::Engine(tex_state::DependencyEngineField::ConditionStack),
            tex_state::DependencyValue::Projection {
                schema: 1,
                fingerprint: self.conditions.tracked_stack_projection(),
            },
        );
    }
}
