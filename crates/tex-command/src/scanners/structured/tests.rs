use tex_state::meaning::{MeaningFlags, MeaningWord};
use tex_state::token::{Catcode, Token};

use super::{FileNameComponents, WriteStreamSelector};
use crate::{AlignmentIdentity, CommandHostCapabilities, CommandState};

fn other(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Other,
    }
}

fn space() -> Token {
    Token::Char {
        ch: ' ',
        cat: Catcode::Space,
    }
}

fn scan_math_delimiter(tokens: impl IntoIterator<Item = Token>) -> (u32, u32, Option<Token>) {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, tokens);
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
        let expected_code = processor.state.delcode('.') as u32;
        let code = processor
            .scan_math_delimiter_boundary(super::MathDelimiterBoundaryKind::Left)
            .expect("delimiter boundary")
            .delimiter
            .code;
        let mut destination = None;
        let next = match processor
            .get_token_into(&mut destination)
            .expect("following raw token")
        {
            crate::DeliveryStatus::End => None,
            crate::DeliveryStatus::Command => Some(
                destination
                    .take()
                    .expect("raw token destination")
                    .spelling()
                    .semantic_token(),
            ),
            other => panic!("unexpected delivery status: {other:?}"),
        };
        (expected_code, code, next)
    })
}

fn scan_write_stream(tokens: impl IntoIterator<Item = Token>) -> WriteStreamSelector {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, tokens);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        )
        .scan_write_stream()
        .expect("write stream")
    })
}

#[test]
fn startup_name_components_use_tex_delimiters_on_every_host() {
    assert_eq!(
        FileNameComponents::from_tex_name(r"volume:dir\trip.more.tex"),
        FileNameComponents {
            area: r"volume:dir\".to_owned(),
            name: "trip".to_owned(),
            extension: ".more.tex".to_owned(),
        }
    );
}

#[test]
fn write_stream_scan_keeps_texs_two_out_of_range_classes_distinct() {
    assert_eq!(
        scan_write_stream([other('-'), other('1')]),
        WriteStreamSelector::Negative
    );
    assert_eq!(
        scan_write_stream([other('1'), other('6')]),
        WriteStreamSelector::AboveRange
    );
    assert_eq!(
        scan_write_stream([other('7')]),
        WriteStreamSelector::Stream(7)
    );
}

#[test]
fn normalized_write_stream_numbers_match_texs_reserved_slots() {
    assert_eq!(WriteStreamSelector::Negative.normalized_number(), 17);
    assert_eq!(WriteStreamSelector::AboveRange.normalized_number(), 16);
    assert_eq!(WriteStreamSelector::Stream(4).normalized_number(), 4);
}

#[test]
fn math_delimiter_consumes_one_expanded_command_before_adjacent_source() {
    let (expected_code, direct_code, direct_next) = scan_math_delimiter([other('.'), other('.')]);
    assert_eq!(direct_code, expected_code);
    assert_eq!(direct_next, Some(other('.')));
    let (expected_code, spaced_code, spaced_next) =
        scan_math_delimiter([space(), other('.'), space(), other('.')]);
    assert_eq!(spaced_code, expected_code);
    assert_eq!(spaced_next, Some(space()));

    crate::test_harness::with_universe(|universe| {
        let replacement = [tex_state::token::TokenWord::pack(other('.'))];
        let definition = universe
            .allocate_definition(&[], &replacement)
            .expect("delimiter macro definition");
        let symbol = universe.intern("delimarg").expect("delimiter macro");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                tex_state::env::AssignmentScope::Global,
            )
            .expect("delimiter macro meaning");

        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, [Token::Cs(symbol.symbol()), other('.')]);
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
        let expected_code = processor.state.delcode('.') as u32;
        let ownership_before = crate::command::command_ownership_counters();
        let scanned = processor
            .scan_math_delimiter_boundary(super::MathDelimiterBoundaryKind::Left)
            .expect("macro delimiter boundary");
        let ownership_after = crate::command::command_ownership_counters();
        assert_eq!(scanned.delimiter.code, expected_code);
        assert_eq!(
            ownership_after.rich_materializations, ownership_before.rich_materializations,
            "compact delimiter delivery must not materialize its operand"
        );
        let mut destination = None;
        assert_eq!(
            processor
                .get_token_into(&mut destination)
                .expect("adjacent source token"),
            crate::DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .expect("adjacent token destination")
                .spelling()
                .semantic_token(),
            other('.')
        );
    });
}

#[test]
fn math_delimiter_recovery_backs_up_only_the_rejected_operand() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(&mut command, [other('x'), other('.')]);
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
        let scanned = processor
            .scan_math_delimiter_boundary(super::MathDelimiterBoundaryKind::Left)
            .expect("delimiter recovery");
        assert!(scanned.delimiter.recovered);
        assert!(scanned.delimiter.missing_delimiter);

        let mut destination = None;
        assert_eq!(
            processor
                .get_token_into(&mut destination)
                .expect("rejected delimiter replay"),
            crate::DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .take()
                .expect("rejected token destination")
                .spelling()
                .semantic_token(),
            other('x')
        );
        assert_eq!(
            processor
                .get_token_into(&mut destination)
                .expect("following source token"),
            crate::DeliveryStatus::Command
        );
        assert_eq!(
            destination
                .take()
                .expect("following token destination")
                .spelling()
                .semantic_token(),
            other('.')
        );
    });
}

#[test]
fn fresh_active_character_is_a_definition_target_without_recovery() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        crate::test_harness::push(
            &mut command,
            [
                Token::Char {
                    ch: '~',
                    cat: Catcode::Active,
                },
                Token::Char {
                    ch: '{',
                    cat: Catcode::BeginGroup,
                },
                Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter,
                },
                Token::Char {
                    ch: '}',
                    cat: Catcode::EndGroup,
                },
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

        let scanned = processor
            .scan_macro_definition(false, false)
            .expect("active-character definition");
        assert!(processor.take_semantic_diagnostics().is_empty());
        let target = scanned.target;
        drop(processor);
        drop(context);
        assert_eq!(universe.resolve(target), Some("~"));
    });
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_ordinary_let_reads_only_the_compact_macro_key_without_allocation() {
    crate::test_harness::with_universe(|universe| {
        let source_id = universe.intern("letsource").expect("source name");
        let source = source_id.symbol();
        let target = universe.intern("lettarget").expect("target name").symbol();
        let definition = universe
            .allocate_definition(
                &[],
                &[tex_state::token::TokenWord::pack(Token::frozen_relax())],
            )
            .expect("source definition");
        universe
            .assign_meaning(
                source_id,
                tex_state::MeaningWord::macro_definition(
                    tex_state::meaning::MeaningFlags::EMPTY,
                    definition,
                ),
                tex_state::AssignmentScope::Global,
            )
            .expect("source meaning");

        let one_scan = [Token::Cs(target), other('='), Token::Cs(source)];
        let mut command = CommandState::default();
        crate::test_harness::push(&mut command, one_scan.into_iter().chain(one_scan));
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

        let (_, warm) = processor.scan_let_assignment(false).expect("warm let scan");
        assert!(matches!(warm, tex_state::ResolvedMeaning::Macro { .. }));
        let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        let retain_before = tex_state::definition_retain_count();
        let (_, measured) = {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            processor
                .scan_let_assignment(false)
                .expect("measured let scan")
        };
        let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);

        assert!(matches!(measured, tex_state::ResolvedMeaning::Macro { .. }));
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
        assert_eq!(tex_state::definition_retain_count(), retain_before);
    });
}

#[test]
fn preamble_span_expansion_retires_its_command_before_raw_refill() {
    // TeX82 §759 implements the preamble `\span` transition as
    // `expand; get_token`: expansion consumes `\noexpand`, then the fresh raw
    // fetch delivers its suppressed operand. The two commands must never own
    // the destination simultaneously.
    crate::test_harness::with_universe(|universe| {
        crate::install_tex82_unexpandable_primitives(universe);
        crate::install_tex82_expandable_primitives(universe);
        let span = universe.primitive_token("span").expect("span primitive");
        let noexpand = universe
            .primitive_token("noexpand")
            .expect("noexpand primitive");
        let cr = universe.primitive_token("cr").expect("cr primitive");
        let x = Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        };

        let mut command = CommandState::default();
        let _operation = command.begin_attempt_operation();
        let alignment = AlignmentIdentity::new(1);
        command.begin_alignment(alignment);
        crate::test_harness::push(
            &mut command,
            [
                span,
                noexpand,
                x,
                Token::Char {
                    ch: '#',
                    cat: Catcode::Parameter,
                },
                cr,
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

        processor
            .begin_alignment_preamble_scan(None)
            .expect("span expansion continues through raw preamble delivery");
        drop(processor);
        drop(context);

        let preamble = command
            .take_completed_alignment_preamble(alignment)
            .expect("completed preamble");
        assert_eq!(preamble.columns.len(), 1);
        let u_template = preamble.columns[0]
            .u_template
            .expect("ordinary column has a u-template");
        let words = command
            .attempt
            .arena()
            .token_words(u_template)
            .expect("live attempt template");
        assert_eq!(words.len(), 1);
        assert_eq!(words.first().expect("one token").semantic_token(), x);
        assert!(
            command
                .attempt
                .arena()
                .token_words(preamble.columns[0].v_template)
                .expect("live attempt template")
                .is_empty()
        );
    });
}
