use tex_state::env::AssignmentScope;
use tex_state::meaning::{Meaning, MeaningWord, UnexpandablePrimitive};
use tex_state::token::{Catcode, Token};

use crate::{CommandHostCapabilities, CommandProfile, CommandState, processor::DeliveryStatus};

fn other(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Other,
    }
}

#[test]
fn numexpr_honors_precedence_and_leaves_its_relax_terminator_consumed() {
    crate::test_harness::with_universe(|universe| {
        let numexpr = universe.intern("numexpr").expect("numexpr");
        universe
            .assign_meaning(
                numexpr,
                MeaningWord::from_static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::NumExpr,
                )),
                AssignmentScope::Global,
            )
            .expect("numexpr meaning");
        let relax = universe.intern("relax").expect("relax");
        universe
            .assign_meaning(
                relax,
                MeaningWord::from_static(Meaning::Relax),
                AssignmentScope::Global,
            )
            .expect("relax meaning");
        let mut command = CommandState::new(CommandProfile::ETEX26);
        crate::test_harness::push(
            &mut command,
            [
                Token::Cs(numexpr.symbol()),
                other('2'),
                other('+'),
                other('3'),
                other('*'),
                other('4'),
                Token::Cs(relax.symbol()),
                other('X'),
            ],
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut scalar = crate::ScalarScanFrame::default();
        assert_eq!(
            processor.scan_integer_into(&mut scalar),
            crate::ScalarScanStatus::Complete
        );
        let scanned = scalar.take_integer();
        assert_eq!(scanned.value, 14);
        let mut following = None;
        assert_eq!(
            processor
                .get_x_token_into(&mut following)
                .expect("following"),
            DeliveryStatus::Command
        );
        assert_eq!(
            following
                .expect("following token")
                .spelling()
                .semantic_token(),
            other('X')
        );
    });
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_numexpr_destination_path_allocates_zero_heap() {
    crate::test_harness::with_universe(|universe| {
        const SCANS: usize = 4_097;
        let numexpr = universe.intern("numexpr").expect("numexpr");
        universe
            .assign_meaning(
                numexpr,
                MeaningWord::from_static(Meaning::UnexpandablePrimitive(
                    UnexpandablePrimitive::NumExpr,
                )),
                AssignmentScope::Global,
            )
            .expect("numexpr meaning");
        let relax = universe.intern("relax").expect("relax");
        universe
            .assign_meaning(
                relax,
                MeaningWord::from_static(Meaning::Relax),
                AssignmentScope::Global,
            )
            .expect("relax meaning");
        let expression = [
            Token::Cs(numexpr.symbol()),
            other('2'),
            other('+'),
            other('3'),
            other('*'),
            other('4'),
            Token::Cs(relax.symbol()),
        ];
        let mut command = CommandState::new(CommandProfile::ETEX26);
        crate::test_harness::push(
            &mut command,
            (0..SCANS).flat_map(|_| expression.iter().cloned()),
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut scalar = crate::ScalarScanFrame::default();
        assert_eq!(
            processor.scan_integer_into(&mut scalar),
            crate::ScalarScanStatus::Complete
        );
        assert_eq!(scalar.take_integer().value, 14);

        let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            for _ in 1..SCANS {
                assert_eq!(
                    processor.scan_integer_into(&mut scalar),
                    crate::ScalarScanStatus::Complete
                );
                assert_eq!(scalar.take_integer().value, 14);
            }
        }
        let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
    });
}
