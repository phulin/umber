//! Ordinary expanded-command delivery.

use tex_state::ids::{MacroDefinitionId, OriginListId, TokenListId};
use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::token::OriginId;

use crate::input::{
    BackupTreatment, InputLevelId, ReplayTrace, RetirementBehavior, SharedTokenBuffer,
    TokenBehavior, TokenPayload,
};
use crate::macro_call::MacroArguments;
use crate::profile::CommandProfile;
use crate::{CommandError, CurrentCommand};

use super::CommandProcessor;

impl CommandProcessor<'_> {
    /// Delivers one ordinary expanded command through TeX.web's `get_x_token`.
    ///
    /// This is the sole production expanded loop. Expansion mutates the
    /// canonical command state and restarts here; it never returns a
    /// push-bearing dispatch result or enters a second interpreter.
    pub fn get_x_token(&mut self) -> Result<Option<CurrentCommand>, CommandError> {
        self.command.transient.active_expansion_depth += 1;
        let result = self.get_x_token_scalar();
        self.command.transient.active_expansion_depth -= 1;
        result
    }

    fn get_x_token_scalar(&mut self) -> Result<Option<CurrentCommand>, CommandError> {
        loop {
            let Some(command) = self.get_next()? else {
                return Ok(None);
            };
            if !is_expandable(command.meaning()) {
                return Ok(Some(command));
            }
            self.expand(command)?;
        }
    }

    /// TeX.web's scalar `expand`: each case changes the active input/state
    /// directly, then returns to [`Self::get_x_token_scalar`].
    fn expand(&mut self, command: CurrentCommand) -> Result<(), CommandError> {
        self.command.expansion.cumulative_expansions =
            self.command.expansion.cumulative_expansions.wrapping_add(1);
        match command.meaning() {
            Meaning::Macro { .. } => {
                self.macro_call(command)?;
                Ok(())
            }
            Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand) => self.expand_noexpand(),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter) => {
                self.expand_expandafter()
            }
            Meaning::ExpandablePrimitive(primitive) => {
                Err(CommandError::UnsupportedExpandablePrimitive(primitive))
            }
            _ => Err(CommandError::InputInvariant),
        }
    }

    /// TeX.web's `\noexpand`: read normally, then replay exactly one target
    /// from a backed-up level carrying the non-sticky suppression treatment.
    fn expand_noexpand(&mut self) -> Result<(), CommandError> {
        let target = self.get_token()?.ok_or(CommandError::InputInvariant)?;
        self.back_input_with_treatment(target, BackupTreatment::SuppressExpandableControlSequence)
    }

    /// TeX.web's `\expandafter`: preserve the first token, expand (or back
    /// up) the second token, then put the first token above the resulting
    /// input. The first delivery is intentionally replayed through an
    /// explicit backed-up level because it is no longer the latest delivery.
    fn expand_expandafter(&mut self) -> Result<(), CommandError> {
        let first = self.get_token()?.ok_or(CommandError::InputInvariant)?;
        let second = self.get_token()?.ok_or(CommandError::InputInvariant)?;
        if is_expandable(second.meaning()) {
            self.expand(second)?;
        } else {
            self.back_input(second)?;
        }
        self.replay_expandafter_first(first);
        Ok(())
    }

    fn replay_expandafter_first(&mut self, command: CurrentCommand) {
        self.undo_alignment_delivery(&command);
        self.command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![command.spelling()])),
            TokenBehavior::BackedUp(BackupTreatment::Ordinary),
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
    }

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

fn is_expandable(meaning: Meaning) -> bool {
    matches!(
        meaning,
        Meaning::Macro { .. } | Meaning::ExpandablePrimitive(_)
    )
}

#[cfg(test)]
mod tests {
    use tex_state::Universe;
    use tex_state::macro_store::MacroMeaning;
    use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
    use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

    use super::*;
    use crate::input::{ReplayTrace, RetirementBehavior};
    use crate::{CommandHostCapabilities, CommandHostContext, CommandRuntime, CommandState};

    fn traced(token: Token) -> TracedTokenWord {
        TracedTokenWord::pack(token, OriginId::UNKNOWN)
    }

    fn processor<'a>(
        command: &'a mut CommandState,
        runtime: &'a mut CommandRuntime,
        universe: &'a mut Universe,
        capabilities: &'a mut CommandHostCapabilities,
    ) -> CommandProcessor<'a> {
        CommandProcessor::new(
            command,
            runtime,
            universe.command_context(),
            CommandHostContext::new(capabilities),
        )
    }

    fn install_macro(
        universe: &mut Universe,
        name: &str,
        replacement: Token,
    ) -> tex_state::interner::Symbol {
        let name = universe.intern(name).symbol();
        let empty = universe.intern_token_list(&[]);
        let replacement = universe.intern_token_list(&[replacement]);
        let definition =
            universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, replacement));
        universe.set_meaning(
            name,
            Meaning::Macro {
                flags: MeaningFlags::EMPTY,
                definition,
            },
        );
        name
    }

    #[test]
    fn ordinary_loop_expands_macro_body_on_the_canonical_raw_path() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
        let macro_name = install_macro(
            &mut universe,
            "m",
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
        );
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(macro_name))])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

        let delivered = processor
            .get_x_token()
            .expect("macro expands")
            .expect("body token");
        assert_eq!(
            delivered.spelling().semantic_token(),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter
            }
        );
        assert_eq!(processor.command.expansion.cumulative_expansions, 1);
        assert_eq!(processor.command.transient.active_expansion_depth, 0);
    }

    #[test]
    fn noexpand_suppresses_one_macro_delivery_without_changing_its_spelling() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
        let noexpand = universe.intern("noexpand").symbol();
        universe.set_meaning(
            noexpand,
            Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand),
        );
        let macro_name = install_macro(
            &mut universe,
            "m",
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
        );
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![
                traced(Token::Cs(noexpand)),
                traced(Token::Cs(macro_name)),
            ])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

        let delivered = processor
            .get_x_token()
            .expect("noexpand completes")
            .expect("target");
        assert_eq!(delivered.spelling().semantic_token(), Token::Cs(macro_name));
        assert_eq!(delivered.meaning(), Meaning::Relax);
        assert_eq!(processor.command.expansion.cumulative_expansions, 1);
    }

    #[test]
    fn expandafter_expands_second_token_before_replaying_first() {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
        let expandafter = universe.intern("expandafter").symbol();
        universe.set_meaning(
            expandafter,
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter),
        );
        let macro_name = install_macro(
            &mut universe,
            "m",
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
        );
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![
                traced(Token::Cs(expandafter)),
                traced(Token::Char {
                    ch: 'a',
                    cat: Catcode::Letter,
                }),
                traced(Token::Cs(macro_name)),
            ])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut runtime, &mut universe, &mut capabilities);

        let first = processor
            .get_x_token()
            .expect("expandafter completes")
            .expect("first token");
        let second = processor
            .get_x_token()
            .expect("macro body follows")
            .expect("body token");
        assert_eq!(
            first.spelling().semantic_token(),
            Token::Char {
                ch: 'a',
                cat: Catcode::Letter
            }
        );
        assert_eq!(
            second.spelling().semantic_token(),
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter
            }
        );
        assert_eq!(processor.command.expansion.cumulative_expansions, 2);
    }

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
