//! Ordinary expanded-command delivery.

use tex_state::ids::{MacroDefinitionId, OriginListId, TokenListId};
use tex_state::token::OriginId;

use crate::input::InputLevelId;
use crate::macro_call::MacroArguments;
use crate::profile::CommandProfile;

use super::CommandProcessor;

impl CommandProcessor<'_> {
    /// Creates one invocation provenance node and atomically exposes its
    /// activation/body ownership pair to the input stack.
    ///
    /// The scalar macro matcher owns argument matching and calls this only
    /// after it has completed every range. Nested invocations use the live
    /// activation chain, not a replay trace, as their provenance parent.
    #[allow(dead_code)] // consumed by the ordered scalar macro matcher issue
    pub(crate) fn push_macro_activation(
        &mut self,
        definition: MacroDefinitionId,
        call_site: OriginId,
        arguments: MacroArguments,
        replacement_tokens: TokenListId,
        replacement_origins: OriginListId,
    ) -> InputLevelId {
        let definition_origin = self
            .state
            .macro_definition_provenance(definition)
            .definition_origin();
        let parent = self.command.parameters.parent_invocation();
        let invocation =
            self.state
                .macro_invocation_origin(definition, call_site, definition_origin, parent);
        self.command.push_macro_activation(
            definition,
            arguments,
            invocation,
            replacement_tokens,
            replacement_origins,
        )
    }
}

#[cfg(test)]
mod tests {
    use tex_state::Universe;
    use tex_state::macro_store::MacroMeaning;
    use tex_state::meaning::MeaningFlags;
    use tex_state::token::OriginId;

    use super::*;
    use crate::{CommandHostCapabilities, CommandHostContext, CommandRuntime, CommandState};

    #[test]
    fn macro_activations_allocate_nested_invocation_provenance() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
        let empty = universe.intern_token_list(&[]);
        let definition =
            universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, empty));
        let mut capabilities = CommandHostCapabilities::default();
        let outer_invocation;
        let inner_invocation;
        {
            let mut processor = CommandProcessor::new(
                &mut command,
                &mut runtime,
                universe.command_context(),
                CommandHostContext::new(&mut capabilities),
            );
            processor.push_macro_activation(
                definition,
                OriginId::UNKNOWN,
                MacroArguments::default(),
                empty,
                tex_state::ids::OriginListId::EMPTY,
            );
            outer_invocation = processor
                .command
                .parameters
                .activations
                .last()
                .expect("outer activation")
                .invocation;
            processor.push_macro_activation(
                definition,
                OriginId::UNKNOWN,
                MacroArguments::default(),
                empty,
                tex_state::ids::OriginListId::EMPTY,
            );
            inner_invocation = processor
                .command
                .parameters
                .activations
                .last()
                .expect("inner activation")
                .invocation;
        }

        assert_ne!(outer_invocation, inner_invocation);
        assert_eq!(command.parameters.activations.len(), 2);
        assert_eq!(
            universe.macro_invocation_provenance_stats().invocations(),
            2
        );
    }
}

/// Future-relevant expansion facts.
///
/// Per-request fuel is deliberately absent: it is call-local and recreated
/// when an executor step is retried. Caches and profiling likewise belong to
/// [`crate::CommandRuntime`].
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct ExpansionState {
    pub(crate) cumulative_expansions: u64,
    pub(crate) next_resource_resolution: u64,
    pub(crate) pending_diagnostics: Vec<u64>,
    pub(crate) observed_dependencies: Vec<u64>,
    pub(crate) semantic_barriers: Vec<u64>,
    pub(crate) profile: CommandProfile,
}
