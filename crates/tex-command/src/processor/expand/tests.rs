mod kernel;
mod nesting;
mod resident_runs;

#[path = "tests/delivery.rs"]
mod delivery;
#[path = "tests/expanded_body.rs"]
mod expanded_body;
#[path = "tests/expressions.rs"]
mod expressions;
#[path = "tests/font_and_names.rs"]
mod font_and_names;
#[path = "tests/macro_delivery.rs"]
mod macro_delivery;
#[path = "tests/main_loop.rs"]
mod main_loop;
#[path = "tests/number_condition.rs"]
mod number_condition;
#[path = "tests/pdf_queries.rs"]
mod pdf_queries;
#[path = "tests/preflight.rs"]
mod preflight;
#[path = "tests/protected_replay.rs"]
mod protected_replay;

use tex_state::env::AssignmentScope;
use tex_state::env::banks::IntParam;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, MeaningWord};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

use crate::{
    CommandDeliveryBoundary, CommandDialect, CommandHostCapabilities, CommandObservation,
    CommandObserver, CommandProfile, CommandState, InputTransition, RecoveryKind,
};

#[derive(Default)]
struct RecordingObserver(Vec<CommandObservation>);

impl CommandObserver for RecordingObserver {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

#[derive(Default)]
struct RecordingCharacterConsumer {
    characters: String,
    origins: Vec<OriginId>,
    fallback_borrowed: bool,
    stop_after: Option<usize>,
}

impl<G> crate::MainCharacterConsumer<G> for RecordingCharacterConsumer {
    fn admit<'state, 'admission, 'fuel, 'effects, 'run>(
        &mut self,
        state: &'state mut tex_state::CommandContext<'admission, G>,
        _fuel: &'fuel mut crate::CommandFuel,
        _diagnostic_effects: &'effects mut tex_state::diagnostic::DiagnosticEffects,
        input: crate::MainCharacterInput<'run>,
    ) -> crate::CharacterRunAdmission {
        match input {
            crate::MainCharacterInput::Borrowed(run) => {
                if self.fallback_borrowed {
                    return crate::CharacterRunAdmission::scalar_fallback();
                }
                for (index, &byte) in run.bytes().iter().enumerate() {
                    if !byte.is_ascii()
                        || !matches!(
                            state.catcode(char::from(byte)),
                            Catcode::Letter | Catcode::Other
                        )
                    {
                        return crate::CharacterRunAdmission::with_fallback(
                            u32::try_from(index).expect("test source run fits u32"),
                            false,
                            crate::CharacterRunFallback::LexicalBoundary,
                        );
                    }
                    self.characters.push(char::from(byte));
                    self.origins.push(run.origin(index));
                }
                crate::CharacterRunAdmission::new(
                    u32::try_from(run.bytes().len()).expect("test source run fits u32"),
                    true,
                )
            }
            crate::MainCharacterInput::Scalar { ch, origin } => {
                self.characters.push(ch);
                self.origins.push(origin);
                crate::CharacterRunAdmission::new(1, self.stop_after != Some(self.characters.len()))
            }
        }
    }
}

fn install_static<G>(universe: &mut tex_state::Universe<G>, name: &str, meaning: Meaning) -> Token {
    let symbol = universe.intern(name).expect("intern primitive");
    universe
        .assign_meaning(
            symbol,
            MeaningWord::from_static(meaning),
            AssignmentScope::Global,
        )
        .expect("install primitive");
    Token::Cs(symbol.symbol())
}

fn collect_expanded_characters<G>(
    universe: &mut tex_state::Universe<G>,
    command: &mut CommandState<G>,
) -> String {
    collect_expanded_meanings(universe, command)
        .into_iter()
        .map(|meaning| match meaning {
            Meaning::CharToken { ch, .. } => ch,
            other => panic!("expected expanded character, found {other:?}"),
        })
        .collect()
}

fn collect_expanded_meanings<G>(
    universe: &mut tex_state::Universe<G>,
    command: &mut CommandState<G>,
) -> Vec<Meaning> {
    let mut capabilities = CommandHostCapabilities::default();
    let mut fuel = crate::CommandFuelLedger::default();
    let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
    let mut context = universe.command_context().expect("command context");
    let mut processor = crate::test_harness::processor(
        command,
        &mut context,
        &mut capabilities,
        &mut fuel,
        &mut diagnostic_effects,
    );
    let mut output = Vec::new();
    while let Some(command) = processor.get_x_token().expect("expanded delivery") {
        match command.meaning() {
            tex_state::meaning::ResolvedMeaning::Static(meaning) => {
                output.push(meaning);
            }
            other => panic!("expected unexpandable meaning, found {other:?}"),
        }
    }
    output
}

fn other(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Other,
    }
}

fn letter(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Letter,
    }
}

#[cfg(feature = "profiling")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OrdinaryDeliveryEvidence {
    slot_initializations: u64,
    rich_materializations: u64,
    resolved_writes: u64,
    expanded_classifications: u64,
    command_clones: u64,
    token_frame_steps: u64,
    meaning_lookups: u64,
    expanded_deliveries: u64,
    #[cfg(feature = "profiling")]
    allocations: u64,
    #[cfg(feature = "profiling")]
    allocated_bytes: u64,
}

#[cfg(feature = "profiling")]
fn empty_macro_delivery_evidence(expansions: usize) -> OrdinaryDeliveryEvidence {
    crate::test_harness::with_universe(|universe| {
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("empty definition");
        let symbol = universe.intern("m").expect("macro name");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("macro meaning");
        let terminal = Token::Char {
            ch: 'Z',
            cat: Catcode::Letter,
        };
        let mut input = Vec::with_capacity((expansions + 1) * 2);
        for _ in 0..2 {
            input.resize(input.len() + expansions, Token::Cs(symbol.symbol()));
            input.push(terminal);
        }

        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, input);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut destination = None;

        {
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            assert_eq!(
                processor
                    .get_x_token_into(&mut destination)
                    .expect("warm ordinary expanded delivery"),
                crate::DeliveryStatus::Command
            );
        }
        assert_eq!(
            destination
                .take()
                .expect("warm terminal command")
                .spelling()
                .semantic_token(),
            terminal
        );
        let before_ownership = crate::command::command_ownership_counters();
        let classifications_before = super::expanded_classifications();
        let work_before = fuel.work();

        #[cfg(feature = "profiling")]
        let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
        #[cfg(feature = "profiling")]
        let before_allocations =
            tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        {
            #[cfg(feature = "profiling")]
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            let mut processor = crate::test_harness::processor(
                &mut command,
                &mut context,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            assert_eq!(
                processor
                    .preflight_command_into(&mut destination)
                    .expect("preflight expansion delivery"),
                crate::DeliveryStatus::Command
            );
        }
        #[cfg(feature = "profiling")]
        let after_allocations =
            tex_state::measurement::hot_core_thread_allocation_measurement(owner);

        let delivered = destination.expect("terminal command");
        assert_eq!(delivered.spelling().semantic_token(), terminal);
        assert_eq!(
            delivered.meaning(),
            Meaning::CharToken {
                ch: 'Z',
                cat: Catcode::Letter,
            }
        );
        let after_ownership = crate::command::command_ownership_counters();
        let work = fuel.work();
        OrdinaryDeliveryEvidence {
            slot_initializations: after_ownership.slot_initializations
                - before_ownership.slot_initializations,
            rich_materializations: after_ownership.rich_materializations
                - before_ownership.rich_materializations,
            resolved_writes: after_ownership.resolved_writes - before_ownership.resolved_writes,
            expanded_classifications: super::expanded_classifications() - classifications_before,
            command_clones: after_ownership.clones - before_ownership.clones,
            token_frame_steps: work.token_frame_steps - work_before.token_frame_steps,
            meaning_lookups: work.meaning_lookups - work_before.meaning_lookups,
            expanded_deliveries: work.expanded_deliveries - work_before.expanded_deliveries,
            #[cfg(feature = "profiling")]
            allocations: after_allocations.calls - before_allocations.calls,
            #[cfg(feature = "profiling")]
            allocated_bytes: after_allocations.requested_bytes - before_allocations.requested_bytes,
        }
    })
}
