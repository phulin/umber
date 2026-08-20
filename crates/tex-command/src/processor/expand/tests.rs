use std::sync::Arc;

use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::page::PageMark;
use tex_state::token::{Catcode, Token};
use tex_state::{
    DependencyEngineField, DependencyKey, DependencyWorldField, JobClock, ShellEscapePolicy,
    Universe, World,
};

use super::*;
use crate::conditionals::{ConditionalKind, IfLimit};
use crate::input::{ReplayTrace, RetirementBehavior, SharedTokenBuffer};
use crate::observation::{
    CommandDeliveryBoundary, CommandObservation, DiagnosticArgument, InputTransition,
    ObservationValue, ObservedToken,
};
use crate::processor::{DefinitionContext, ScannerStatus, ScannerWarning, TokenBuilderId};
use crate::test_harness::{Recorder, ScannerRig, diagnostic_text, processor, traced};
use crate::{
    CommandHostCapabilities, CommandHostContext, CommandState, RegisteredSourceKind,
    SourceRegistration,
};

#[test]
fn pinned_ranked_expansion_set_uses_the_borrowed_dispatch_lane() {
    assert!(is_ranked_fused_expansion(Meaning::Macro {
        flags: MeaningFlags::EMPTY,
        definition: tex_state::ids::MacroDefinitionId::testing_new(1),
    }));
    let ranked = [
        ExpandablePrimitive::ExpandAfter,
        ExpandablePrimitive::Fi,
        ExpandablePrimitive::IfX,
        ExpandablePrimitive::IfNum,
        ExpandablePrimitive::If,
        ExpandablePrimitive::CsName,
        ExpandablePrimitive::NoExpand,
        ExpandablePrimitive::Detokenize,
        ExpandablePrimitive::String,
        ExpandablePrimitive::IfFalse,
        ExpandablePrimitive::RomanNumeral,
        ExpandablePrimitive::Else,
        ExpandablePrimitive::Expanded,
        ExpandablePrimitive::IfCsName,
        ExpandablePrimitive::Number,
        ExpandablePrimitive::The,
    ];
    for primitive in ranked {
        assert!(
            is_ranked_fused_expansion(Meaning::ExpandablePrimitive(primitive)),
            "{primitive:?} must stay on the borrowed expansion lane"
        );
    }
    for primitive in [
        ExpandablePrimitive::Input,
        ExpandablePrimitive::Scantokens,
        ExpandablePrimitive::PdfMatch,
    ] {
        assert!(
            !is_ranked_fused_expansion(Meaning::ExpandablePrimitive(primitive)),
            "{primitive:?} is an explicit cold fallback"
        );
    }
}

fn install_macro(
    universe: &mut Universe,
    name: &str,
    replacement: Token,
) -> tex_state::interner::Symbol {
    install_macro_with_flags(universe, name, replacement, MeaningFlags::EMPTY)
}

fn install_macro_with_flags(
    universe: &mut Universe,
    name: &str,
    replacement: Token,
    flags: MeaningFlags,
) -> tex_state::interner::Symbol {
    let name = universe.intern(name).symbol();
    let empty = universe.intern_token_list(&[]);
    let replacement = universe.intern_token_list(&[replacement]);
    let definition = universe.intern_macro(MacroMeaning::new(flags, empty, replacement));
    universe.set_meaning(
        name,
        Meaning::Macro {
            flags,
            definition: definition.id(),
        },
    );
    name
}

fn delivery_and_backup_script(recorder: &Recorder, command_name: &str) -> Vec<&'static str> {
    recorder
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Command(record)
                if record.command == command_name
                    && record.boundary == CommandDeliveryBoundary::Raw =>
            {
                Some("raw")
            }
            CommandObservation::Command(record)
                if record.command == command_name
                    && record.boundary == CommandDeliveryBoundary::Expanded =>
            {
                Some("expanded")
            }
            CommandObservation::Input(record) if record.transition == InputTransition::Backup => {
                Some("backup")
            }
            _ => None,
        })
        .collect()
}

#[test]
fn absent_observer_does_not_build_raw_or_expanded_delivery_payloads() {
    // Observation is detached instrumentation around TeX82 §§341 and 380.
    // The production scalar path must not allocate or resolve an observation
    // payload when no consumer is attached.
    fn run(observed: bool) -> usize {
        let mut command = CommandState::default();
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(b"x".as_slice()),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut universe = Universe::new_with_plain_catcodes();
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        if observed {
            processor = processor.with_observer(&mut recorder);
        }

        assert!(
            processor
                .get_x_token()
                .expect("expanded delivery succeeds")
                .is_some()
        );
        processor.observation_payloads_built()
    }

    assert_eq!(run(false), 0);
    assert_eq!(run(true), 2);
}

#[test]
fn macro_heavy_delivery_has_an_exact_monotonic_work_vector() {
    const INVOCATIONS: u64 = 256;

    let mut command = CommandState::default();
    let mut universe = Universe::new_with_plain_catcodes();
    let macro_symbol = install_macro(
        &mut universe,
        "workmacro",
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
    );
    crate::test_harness::push(
        &mut command,
        (0..INVOCATIONS).map(|_| Token::Cs(macro_symbol)),
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut fuel = crate::CommandFuelLedger::new(10_000).expect("bounded test fuel");
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_fuel(fuel.fuel_mut());
        for _ in 0..INVOCATIONS {
            assert!(processor.get_x_token().expect("macro expands").is_some());
        }
        assert!(processor.get_x_token().expect("input ends").is_none());
    }

    assert_eq!(
        fuel.work(),
        crate::CommandWorkCounters {
            fuel_charges: INVOCATIONS * 2 + 1,
            token_frame_steps: INVOCATIONS * 2,
            expanded_deliveries: INVOCATIONS,
            meaning_lookups: INVOCATIONS,
            scanner_tokens: 0,
            write_expansions: 0,
        }
    );
}

#[test]
fn replay_completion_survives_a_descendant_macro_across_processor_episodes() {
    // TeX82 §§357 and 390: retiring the stored replay before installing the
    // final macro's replacement does not permit the source below that replay
    // to resume.  The replacement can deliver an unexpandable command to main
    // control, ending this borrow, so the pending ownership boundary must be
    // command state rather than call-local processor state.
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"z".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens beneath replay");
    let mut universe = Universe::new_with_plain_catcodes();
    let relax = universe.intern("relax").symbol();
    universe.set_meaning(relax, Meaning::Relax);
    let final_macro = install_macro(&mut universe, "finalmacro", Token::Cs(relax));
    let replay_tokens = universe.intern_token_list_ref(&[Token::Cs(final_macro)]);
    let replay = command.push_discretionary_episode(
        &universe.command_context(),
        tex_state::input::TracedTokenList::synthetic(replay_tokens),
    );
    let mut capabilities = CommandHostCapabilities::default();

    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let crate::CommandReplayDelivery::Command(delivery) = processor
            .get_x_token_with_replay_completion()
            .expect("final macro expands")
            .expect("replacement command delivers")
        else {
            panic!("the replacement command precedes replay completion");
        };
        assert_eq!(delivery.meaning(), Meaning::Relax);
    }

    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    assert!(
        matches!(
            processor
                .get_x_token_with_replay_completion()
                .expect("descendant macro retires"),
            Some(crate::CommandReplayDelivery::Completed(completed)) if completed == replay
        ),
        "the replay ownership boundary must surface before source resumes",
    );
}

#[test]
fn macro_expanded_alignment_lookahead_is_observed_before_backup() {
    // TeX82 §§380, 785, 789: `align_peek` completes `get_x_token` before
    // `init_col` backs its ordinary command up. This bounded source-free
    // microfixture models `\def\bf{\fam...}` at the start of an alignment
    // entry, the first long-document occurrence that exposed the ordering.
    let mut rig = ScannerRig::plain();
    let fam = rig.scenario.universe.intern("fam").symbol();
    rig.scenario.universe.set_meaning(
        fam,
        Meaning::IntParam(tex_state::env::banks::IntParam::FAM.raw()),
    );
    let bf = install_macro(&mut rig.scenario.universe, "bf", Token::Cs(fam));
    rig.scenario.command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(bf))])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut processor = rig.processor();

    let lookahead = processor
        .next_alignment_lookahead()
        .expect("alignment lookahead expands")
        .expect("macro replacement produces a command");
    assert!(matches!(
        lookahead,
        super::AlignmentLookahead::PendingExpanded(_)
    ));
    assert_eq!(
        lookahead.command().meaning(),
        Meaning::IntParam(tex_state::env::banks::IntParam::FAM.raw())
    );
    processor
        .back_alignment_lookahead(lookahead)
        .expect("ordinary init_col backup succeeds");
    processor
        .get_x_token()
        .expect("backed command redelivers")
        .expect("backed command exists");

    assert_eq!(
        delivery_and_backup_script(&rig.recorder, "assign_int"),
        ["raw", "expanded", "backup", "raw", "expanded",],
        "§380's completed expansion precedes §789's one backup and one replay"
    );
}

#[test]
fn direct_alignment_lookahead_keeps_raw_expanded_backup_replay_order() {
    // TeX82 §§341, 380, 785, 789: a directly fetched unexpandable command
    // completes raw and expanded delivery before init_col backs it up.
    let mut rig = ScannerRig::plain();
    let fam = rig.scenario.universe.intern("fam").symbol();
    rig.scenario.universe.set_meaning(
        fam,
        Meaning::IntParam(tex_state::env::banks::IntParam::FAM.raw()),
    );
    rig.scenario.command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(fam))])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut processor = rig.processor();

    let lookahead = processor
        .next_alignment_lookahead()
        .expect("direct alignment lookahead succeeds")
        .expect("direct command exists");
    assert!(matches!(lookahead, super::AlignmentLookahead::Committed(_)));
    processor
        .back_alignment_lookahead(lookahead)
        .expect("direct command backup succeeds");
    processor
        .get_x_token()
        .expect("backed direct command redelivers")
        .expect("backed direct command exists");

    assert_eq!(
        delivery_and_backup_script(&rig.recorder, "assign_int"),
        ["raw", "expanded", "backup", "raw", "expanded",]
    );
}

#[test]
fn consumed_macro_alignment_lookahead_commits_once_without_backup() {
    // TeX82 §§380/785 consume no_align/crcr/closing-brace/omit lookahead in
    // place. Model that branch directly: its pending expansion commits once
    // and creates no backup replay.
    let mut rig = ScannerRig::plain();
    let omit = rig.scenario.universe.intern("omit").symbol();
    rig.scenario.universe.set_meaning(
        omit,
        Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Omit),
    );
    let macro_name = install_macro(&mut rig.scenario.universe, "next", Token::Cs(omit));
    rig.scenario.command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(macro_name))])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut processor = rig.processor();

    let lookahead = processor
        .next_alignment_lookahead()
        .expect("consumed lookahead expands")
        .expect("macro replacement produces a command");
    assert!(matches!(
        lookahead,
        super::AlignmentLookahead::PendingExpanded(_)
    ));
    assert_eq!(
        lookahead.command().meaning(),
        Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Omit)
    );
    let _ = processor.commit_alignment_lookahead_delivery(lookahead);

    assert_eq!(
        delivery_and_backup_script(&rig.recorder, "omit"),
        ["raw", "expanded"],
        "consumed lookahead has one completed expanded delivery and no backup"
    );
}

#[test]
fn etex_protected_alignment_lookahead_is_raw_and_nonpending() {
    // e-TeX 2.6 [37.785] get_x_or_protected returns a protected macro from
    // get_token, without an expanded delivery pending in observer transport.
    let mut command = CommandState::new(CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let relax = universe.intern("relax").symbol();
    universe.set_meaning(relax, Meaning::Relax);
    let protected = install_macro_with_flags(
        &mut universe,
        "protected",
        Token::Cs(relax),
        MeaningFlags::PROTECTED,
    );
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(protected))])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let mut processor =
        processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);

    let lookahead = processor
        .next_alignment_lookahead()
        .expect("protected lookahead succeeds")
        .expect("protected command exists");
    assert!(matches!(
        lookahead.command().meaning(),
        Meaning::Macro { .. }
    ));
    assert!(matches!(lookahead, super::AlignmentLookahead::Committed(_)));
    assert_eq!(
        recorder
            .0
            .iter()
            .filter(|event| matches!(
                event,
                CommandObservation::Command(record)
                    if record.boundary == CommandDeliveryBoundary::Expanded
            ))
            .count(),
        0,
        "protected lookahead never fabricates an expanded delivery"
    );
}

#[test]
fn cyclic_macro_exhausts_shared_command_fuel() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let cycle = universe.intern("cycle").symbol();
    let empty = universe.intern_token_list(&[]);
    let replacement = universe.intern_token_list(&[Token::Cs(cycle)]);
    let definition =
        universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, replacement));
    universe.set_meaning(
        cycle,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: definition.id(),
        },
    );
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(cycle))])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut fuel = crate::CommandFuelLedger::new(7).expect("valid test limit");
    let error = CommandProcessor::new(
        &mut command,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .with_fuel(fuel.fuel_mut())
    .get_x_token()
    .expect_err("cyclic expansion must terminate inside tex-command");
    assert_eq!(
        error,
        crate::CommandError::FuelExhausted {
            limit: 7,
            burned: 7,
            work: crate::CommandWorkCounters {
                fuel_charges: 7,
                token_frame_steps: 7,
                meaning_lookups: 7,
                ..crate::CommandWorkCounters::default()
            },
        }
    );
    assert_eq!(fuel.burned(), 7);
    assert_eq!(
        command.transient.active_expansion_depth, 0,
        "the typed delivery guard balances expansion depth on error"
    );
}

fn install_expandable(
    universe: &mut Universe,
    name: &str,
    primitive: ExpandablePrimitive,
) -> tex_state::interner::Symbol {
    let symbol = universe.intern(name).symbol();
    universe.set_meaning(symbol, Meaning::ExpandablePrimitive(primitive));
    symbol
}

fn set_margin_kern_test_box(
    universe: &mut Universe,
    index: u16,
    horizontal: bool,
    children: Vec<tex_state::node::Node>,
) {
    use tex_state::glue::Order;
    use tex_state::node::{BoxLr, BoxNode, BoxNodeFields, Node, Sign};
    use tex_state::scaled::GlueSetRatio;

    let children = universe.publish_page_nodes(&children);
    let boxed = BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        box_lr: BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    });
    let root = universe.publish_page_nodes(&[if horizontal {
        Node::HList(boxed)
    } else {
        Node::VList(boxed)
    }]);
    universe.assign_page_box_global(index, root);
}

#[test]
fn frozen_end_template_delivers_endv_fresh_and_after_format_load() {
    // TeX82 §§375, 780: both `endtemplate` control sequences are inaccessible
    // frozen slots. Expanding the first delivers the second as `endv`; format
    // loading must preserve that internal meaning without exposing a named
    // primitive to user input.
    let fresh = crate::test_harness::universe_with_plain_catcodes();
    assert_eq!(fresh.symbol("endtemplate"), None);
    let format = fresh.dump_format().expect("quiescent format");
    let loaded = Universe::from_format(World::default(), &format).expect("load format");

    for mut universe in [fresh, loaded] {
        assert_eq!(universe.symbol("endtemplate"), None);
        assert_eq!(universe.primitive_meaning("endtemplate"), None);
        let frozen_end_template = universe.command_context().frozen_end_template_token();

        let mut command = CommandState::default();
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(frozen_end_template)])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let delivered = processor(&mut command, &mut universe, &mut capabilities)
            .get_x_token()
            .expect("end_template expansion succeeds")
            .expect("frozen endv is delivered");

        assert_eq!(delivered.meaning(), Meaning::EndV);
        assert!(delivered.spelling().semantic_token().is_frozen_endv());
        assert_eq!(universe.symbol("endtemplate"), None);
    }
}

#[test]
fn etex_unexpanded_reenters_the_current_expansion_loop() {
    // e-TeX 2.6 etex.ch §27.465 routes `\unexpanded` through
    // `scan_general_text`, then returns its token list through `the_toks`.
    // Outside an expanded token-list collector, that list is ordinary
    // `ins_list` input and the enclosing `get_x_token` expands it normally.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let unexpanded =
        install_expandable(&mut universe, "unexpanded", ExpandablePrimitive::Unexpanded);
    let payload = install_macro(
        &mut universe,
        "payload",
        Token::Char {
            ch: 'X',
            cat: Catcode::Letter,
        },
    );
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(unexpanded)),
            traced(Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            }),
            traced(Token::Cs(payload)),
            traced(Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            }),
            traced(Token::Cs(payload)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut fuel = crate::CommandFuelLedger::new(32).expect("finite test fuel");
    let mut processor =
        processor(&mut command, &mut universe, &mut capabilities).with_fuel(fuel.fuel_mut());

    let expanded = processor
        .get_x_token()
        .expect("unexpanded scan succeeds")
        .expect("expanded token is returned");
    assert_eq!(
        expanded.spelling().semantic_token(),
        Token::Char {
            ch: 'X',
            cat: Catcode::Letter,
        }
    );
    assert!(!is_expandable_command(&expanded));
    assert_eq!(rendered(&mut processor), "X");
    assert!(fuel.burned() <= 32);
}

#[test]
fn pdftex_expanded_collects_then_reenters_the_current_expansion_loop() {
    // pdftex.web §§495 and 1535: `\expanded` uses
    // `scan_toks(false, true)`, then inserts the collected list. The nested
    // `\unexpanded` suppresses expansion only while that list is collected;
    // the inserted result is expanded normally by the enclosing fetch.
    let mut command = CommandState::new(crate::CommandProfile::PDFTEX14029);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    install_expandable(&mut universe, "expanded", ExpandablePrimitive::Expanded);
    install_expandable(&mut universe, "unexpanded", ExpandablePrimitive::Unexpanded);
    install_macro(
        &mut universe,
        "payload",
        Token::Char {
            ch: 'X',
            cat: Catcode::Letter,
        },
    );
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            br"\expanded{\unexpanded{\payload}}\payload".as_slice(),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut capabilities = CommandHostCapabilities::default();
    let mut fuel = crate::CommandFuelLedger::new(64).expect("finite test fuel");
    let mut processor =
        processor(&mut command, &mut universe, &mut capabilities).with_fuel(fuel.fuel_mut());

    assert_eq!(rendered(&mut processor), "XX");
    assert!(fuel.burned() <= 64);
}

#[test]
fn pdftex_expanded_collects_the_numexpr_result_without_its_terminator() {
    // pdftex.web §§495 and 1535: the expanded scan uses TeX's §478 direct
    // `\the` path. An e-TeX expression contributes its rendered value while
    // the terminating `\relax` remains consumed by the internal scan.
    let mut command = CommandState::new(crate::CommandProfile::PDFTEX14029);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.set_catcode('_', Catcode::Letter);
    universe.set_catcode(':', Catcode::Letter);
    install_expandable(&mut universe, "expanded", ExpandablePrimitive::Expanded);
    let the = install_expandable(&mut universe, "the", ExpandablePrimitive::The);
    let numexpr = universe.intern("numexpr").symbol();
    universe.set_meaning(
        numexpr,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NumExpr),
    );
    let eval_end = universe.intern("__int_eval_end:").symbol();
    universe.set_meaning(eval_end, Meaning::Relax);
    let parameters = universe.intern_token_list(&[Token::Param(1)]);
    let replacement = universe.intern_token_list(&[
        Token::Cs(the),
        Token::Cs(numexpr),
        Token::Param(1),
        Token::Cs(eval_end),
    ]);
    let definition = universe.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        parameters,
        replacement,
    ));
    let int_eval = universe.intern("int_eval:n").symbol();
    universe.set_meaning(
        int_eval,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: definition.id(),
        },
    );
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            br#"\expanded{\int_eval:n{"41+1}}%"#.as_slice(),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(rendered(&mut processor), "66");
}

#[test]
fn pdftex_string_compare_expands_both_balanced_operands() {
    // pdftex.web §§1535 compares the two independently expanded token
    // strings bytewise and returns only -1, 0, or 1. Macro expansion and the
    // empty-string case challenge both operand boundaries.
    for (source, expected) in [
        (br"\pdfstrcmp{a}{b}%".as_slice(), "-1"),
        (br"\pdfstrcmp{b}{a}%".as_slice(), "1"),
        (br"\pdfstrcmp{\payload}{x}%".as_slice(), "0"),
        (br"\pdfstrcmp{}{}%".as_slice(), "0"),
    ] {
        let mut command = CommandState::new(crate::CommandProfile::PDFTEX14029);
        let mut universe = crate::test_harness::universe_with_plain_catcodes();
        install_expandable(
            &mut universe,
            "pdfstrcmp",
            ExpandablePrimitive::StringCompare,
        );
        install_macro(
            &mut universe,
            "payload",
            Token::Char {
                ch: 'x',
                cat: Catcode::Letter,
            },
        );
        let input = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                source,
            ))
            .expect("source registers");
        command.open_registered_source(input).expect("source opens");
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);

        assert_eq!(rendered(&mut processor), expected);
    }
}

#[test]
fn pdftex_escape_string_expands_and_escapes_only_pdf_literal_bytes() {
    // pdftex.web §495 and utils.c `escapestring`: scan one expanded balanced
    // operand, prefix the three PDF literal delimiters, octal-encode bytes
    // outside !..~, and leave safe endpoints unchanged. Fresh and loaded
    // universes are the format-registration negative control.
    let mut fresh = crate::test_harness::universe_with_plain_catcodes();
    crate::primitives::install_pdftex_expandable_primitives(&mut fresh);
    let format = fresh.dump_format().expect("quiescent pdfTeX format");
    let mut loaded = Universe::from_format(World::default(), &format).expect("format loads");
    crate::primitives::register_pdftex_expandable_primitives(&mut loaded);

    for mut universe in [fresh, loaded] {
        let escape = universe
            .symbol("pdfescapestring")
            .expect("pdfTeX spelling is installed");
        assert_eq!(
            universe.meaning(escape.symbol()),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfEscapeString),
        );
        install_macro(
            &mut universe,
            "backslash",
            Token::Char {
                ch: '\\',
                cat: Catcode::Other,
            },
        );
        install_macro(
            &mut universe,
            "highbyte",
            Token::Char {
                ch: '\u{80}',
                cat: Catcode::Other,
            },
        );
        install_macro(
            &mut universe,
            "spacechar",
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
        );
        let mut command = CommandState::new(crate::CommandProfile::PDFTEX14029);
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                br"\pdfescapestring{(\backslash)\highbyte\spacechar A!~}%".as_slice(),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);

        assert_eq!(rendered(&mut processor), r"\(\\\)\200\040A!~");
        let returned = recorder.0.iter().find_map(|record| match record {
            CommandObservation::TokenList(record) if record.purpose == "pdf_escape_string" => {
                Some(record)
            }
            _ => None,
        });
        assert_eq!(
            returned
                .expect("conversion return is observed")
                .tokens
                .len(),
            r"\(\\\)\200\040A!~".len(),
        );
    }
}

#[test]
fn pdftex_escape_hex_expands_bytes_as_uppercase_other_character_pairs() {
    // pdftex.web §§494 and 496--497: `\pdfescapehex` scans one expanded
    // balanced operand and returns two uppercase hexadecimal digits per byte
    // through TeX82 §464's `str_toks`. Fresh and loaded universes prove both
    // INITEX installation and format-registry reconstruction.
    let mut fresh = crate::test_harness::universe_with_plain_catcodes();
    crate::primitives::install_pdftex_expandable_primitives(&mut fresh);
    let format = fresh.dump_format().expect("quiescent pdfTeX format");
    let mut loaded = Universe::from_format(World::default(), &format).expect("format loads");
    crate::primitives::register_pdftex_expandable_primitives(&mut loaded);

    for mut universe in [fresh, loaded] {
        let escape = universe
            .symbol("pdfescapehex")
            .expect("pdfTeX spelling is installed");
        assert_eq!(
            universe.meaning(escape.symbol()),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfEscapeHex),
        );
        for (name, ch) in [
            ("backslash", '\\'),
            ("zero", '\0'),
            ("delete", '\u{7f}'),
            ("highbyte", '\u{80}'),
            ("maxbyte", '\u{ff}'),
        ] {
            install_macro(
                &mut universe,
                name,
                Token::Char {
                    ch,
                    cat: Catcode::Other,
                },
            );
        }
        let mut command = CommandState::new(crate::CommandProfile::PDFTEX14029);
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                br"\pdfescapehex{A (\backslash)\zero\delete\highbyte\maxbyte}\pdfescapehex{}X%"
                    .as_slice(),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        let mut tokens = Vec::new();
        while let Some(delivery) = processor.get_x_token().expect("hex escaping expands") {
            tokens.push(delivery.spelling().semantic_token());
        }

        assert_eq!(
            tokens.pop(),
            Some(Token::Char {
                ch: 'X',
                cat: Catcode::Letter,
            }),
            "both balanced operands are consumed and the sentinel remains",
        );
        assert_eq!(
            tokens
                .iter()
                .map(|token| match token {
                    Token::Char { ch, .. } => *ch,
                    _ => panic!("hex conversion returned a non-character token"),
                })
                .collect::<String>(),
            "4120285C29007F80FF",
        );
        assert!(tokens.iter().all(|token| matches!(
            token,
            Token::Char {
                cat: Catcode::Other,
                ..
            }
        )));
        let returned_lengths = recorder
            .0
            .iter()
            .filter_map(|record| match record {
                CommandObservation::TokenList(record) if record.purpose == "pdf_escape_hex" => {
                    Some(record.tokens.len())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            returned_lengths,
            vec![18, 0],
            "the empty operand emits no bytes but still completes its conversion",
        );
    }
}

#[test]
fn pdftex_unescape_hex_expands_and_decodes_valid_nibbles_fresh_and_loaded() {
    // pdftex.web §§494 and 496--497 plus utils.c `unescapehex`: scan one
    // expanded balanced operand, ignore invalid bytes without breaking a
    // nibble pair, decode either hex case, and zero-pad a final high nibble.
    // TeX82 §464 makes a decoded space category 10 and every other byte
    // category 12. Fresh and loaded universes cover both registry paths.
    let mut fresh = crate::test_harness::universe_with_plain_catcodes();
    crate::primitives::install_pdftex_expandable_primitives(&mut fresh);
    let format = fresh.dump_format().expect("quiescent pdfTeX format");
    let mut loaded = Universe::from_format(World::default(), &format).expect("format loads");
    crate::primitives::register_pdftex_expandable_primitives(&mut loaded);

    for mut universe in [fresh, loaded] {
        let unescape = universe
            .symbol("pdfunescapehex")
            .expect("pdfTeX spelling is installed");
        assert_eq!(
            universe.meaning(unescape.symbol()),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfUnescapeHex),
        );
        install_macro(
            &mut universe,
            "one",
            Token::Char {
                ch: '1',
                cat: Catcode::Other,
            },
        );
        install_macro(
            &mut universe,
            "invalidbyte",
            Token::Char {
                ch: '\u{80}',
                cat: Catcode::Other,
            },
        );
        let mut command = CommandState::new(crate::CommandProfile::PDFTEX14029);
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                br"\pdfunescapehex{4\invalidbyte\one 2?0aF0G0f}\pdfunescapehex{}\pdfunescapehex{xyz!?}X%"
                    .as_slice(),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        let mut tokens = Vec::new();
        while let Some(delivery) = processor.get_x_token().expect("hex unescaping expands") {
            tokens.push(delivery.spelling().semantic_token());
        }

        assert_eq!(
            tokens.pop(),
            Some(Token::Char {
                ch: 'X',
                cat: Catcode::Letter,
            }),
            "all three balanced operands are consumed and the sentinel remains",
        );
        assert_eq!(
            tokens,
            vec![
                Token::Char {
                    ch: 'A',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: ' ',
                    cat: Catcode::Space,
                },
                Token::Char {
                    ch: '\u{af}',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '\0',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: '\u{f0}',
                    cat: Catcode::Other,
                },
            ],
        );
        let returned_lengths = recorder
            .0
            .iter()
            .filter_map(|record| match record {
                CommandObservation::TokenList(record) if record.purpose == "pdf_unescape_hex" => {
                    Some(record.tokens.len())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            returned_lengths,
            vec![5, 0, 0],
            "empty and all-invalid operands return empty inserted lists",
        );
    }
}

#[test]
fn pdftex_margin_kern_enquiries_use_typed_box_edges_fresh_and_loaded() {
    use tex_state::font::NULL_FONT;
    use tex_state::glue::GlueSpec;
    use tex_state::node::{GlueKind, MarginKernSide, Node, Whatsit};

    // pdftex.web §470 scans the extended register range and walks only the
    // requested hlist edge. The loaded case proves that both primitive
    // identities retain the same typed state query across a format boundary.
    let mut fresh = crate::test_harness::universe_with_plain_catcodes();
    crate::primitives::install_pdftex_expandable_primitives(&mut fresh);
    let format = fresh.dump_format().expect("quiescent pdfTeX format");
    let mut loaded = Universe::from_format(World::default(), &format).expect("format loads");
    crate::primitives::register_pdftex_expandable_primitives(&mut loaded);

    for mut universe in [fresh, loaded] {
        let nonzero_glue = universe.intern_glue(GlueSpec {
            width: Scaled::from_raw(Scaled::UNITY),
            ..GlueSpec::ZERO
        });
        set_margin_kern_test_box(
            &mut universe,
            32_767,
            true,
            vec![
                Node::Penalty(10_000),
                Node::Glue {
                    spec: nonzero_glue,
                    kind: GlueKind::LeftSkip,
                    leader: None,
                },
                Node::MarginKern {
                    amount: Scaled::from_raw(-5 * Scaled::UNITY),
                    side: MarginKernSide::Left,
                    font: NULL_FONT,
                    ch: b'L',
                },
                Node::Char {
                    font: NULL_FONT,
                    ch: 'x',
                    origin: tex_state::provenance::OriginRef::unknown(),
                },
                Node::MarginKern {
                    amount: Scaled::from_raw(-7 * Scaled::UNITY),
                    side: MarginKernSide::Right,
                    font: NULL_FONT,
                    ch: b'R',
                },
                Node::Glue {
                    spec: nonzero_glue,
                    kind: GlueKind::RightSkip,
                    leader: None,
                },
                Node::Penalty(10_000),
            ],
        );
        // The opposite edge's line skip and a referenced form are both
        // deliberate blockers, not members of pdfTeX's skipable set.
        set_margin_kern_test_box(
            &mut universe,
            1,
            true,
            vec![
                Node::Glue {
                    spec: nonzero_glue,
                    kind: GlueKind::RightSkip,
                    leader: None,
                },
                Node::MarginKern {
                    amount: Scaled::from_raw(-Scaled::UNITY),
                    side: MarginKernSide::Left,
                    font: NULL_FONT,
                    ch: b'L',
                },
            ],
        );
        set_margin_kern_test_box(
            &mut universe,
            2,
            true,
            vec![
                Node::MarginKern {
                    amount: Scaled::from_raw(-Scaled::UNITY),
                    side: MarginKernSide::Right,
                    font: NULL_FONT,
                    ch: b'R',
                },
                Node::Whatsit(Whatsit::PdfRefXForm {
                    object: 1,
                    width: Scaled::from_raw(0),
                    height: Scaled::from_raw(0),
                    depth: Scaled::from_raw(0),
                }),
            ],
        );
        set_margin_kern_test_box(&mut universe, 3, true, vec![]);
        set_margin_kern_test_box(&mut universe, 4, false, vec![]);

        let mut command = CommandState::new(crate::CommandProfile::PDFTEX14029);
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                br"\leftmarginkern32767/\rightmarginkern32767/\leftmarginkern1/\rightmarginkern2/\leftmarginkern3%".as_slice(),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut capabilities = CommandHostCapabilities::default();
        {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities);
            assert_eq!(rendered(&mut processor), "-5.0pt/-7.0pt/0.0pt/0.0pt/0.0pt");
        }

        for source in [br"\leftmarginkern4%".as_slice(), br"\rightmarginkern5%"] {
            let mut command = CommandState::new(crate::CommandProfile::PDFTEX14029);
            let source = command
                .register_source(SourceRegistration::new(
                    RegisteredSourceKind::Generated,
                    source,
                ))
                .expect("invalid source registers");
            command
                .open_registered_source(source)
                .expect("invalid source opens");
            let mut processor = processor(&mut command, &mut universe, &mut capabilities);
            let error = processor
                .get_x_token()
                .expect_err("void and non-hlist boxes are rejected");
            assert_eq!(
                error.to_string(),
                "pdfTeX error (marginkern): a non-empty hbox expected"
            );
        }
    }
}

#[test]
fn pdftex_color_stack_init_scans_canonical_modes_and_allocates_job_state() {
    // pdftex.web §495: the first optional `page` selects page-start
    // restoration, the independent `direct`/`page` keyword selects framing,
    // and an omitted mode means origin framing. The general-text operand is
    // expanded before allocation and the conversion returns the stack ID.
    // Fresh and loaded universes are the format-registration negative control.
    let mut fresh = crate::test_harness::universe_with_plain_catcodes();
    crate::primitives::install_pdftex_expandable_primitives(&mut fresh);
    let format = fresh.dump_format().expect("quiescent pdfTeX format");
    let mut loaded = Universe::from_format(World::default(), &format).expect("format loads");
    crate::primitives::register_pdftex_expandable_primitives(&mut loaded);

    for mut universe in [fresh, loaded] {
        let init = universe
            .symbol("pdfcolorstackinit")
            .expect("pdfTeX spelling is installed");
        assert_eq!(
            universe.meaning(init.symbol()),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfColorStackInit),
        );
        install_macro(
            &mut universe,
            "payload",
            Token::Char {
                ch: 'D',
                cat: Catcode::Letter,
            },
        );
        let mut command = CommandState::new(crate::CommandProfile::PDFTEX14029);
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                br"\pdfcolorstackinit{O}\pdfcolorstackinit page direct{\payload}\pdfcolorstackinit page page{P}\pdfcolorstackinit page{R}%".as_slice(),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut capabilities = CommandHostCapabilities::default();

        {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities);
            assert_eq!(rendered(&mut processor), "1234");
        }

        let origin = universe
            .apply_pdf_color_stack(
                1,
                tex_state::PdfColorStackTarget::Page,
                &tex_state::PdfColorStackAction::Current,
            )
            .expect("origin stack exists");
        assert_eq!(origin.mode, tex_state::PdfColorStackMode::Origin);
        assert_eq!(origin.payload, b"O");
        let direct = universe
            .apply_pdf_color_stack(
                2,
                tex_state::PdfColorStackTarget::Page,
                &tex_state::PdfColorStackAction::Current,
            )
            .expect("direct stack exists");
        assert_eq!(direct.mode, tex_state::PdfColorStackMode::Direct);
        assert_eq!(direct.payload, b"D");
        let page = universe
            .apply_pdf_color_stack(
                3,
                tex_state::PdfColorStackTarget::Page,
                &tex_state::PdfColorStackAction::Current,
            )
            .expect("page stack exists");
        assert_eq!(page.mode, tex_state::PdfColorStackMode::Page);
        assert_eq!(page.payload, b"P");
        let single_page_keyword = universe
            .apply_pdf_color_stack(
                4,
                tex_state::PdfColorStackTarget::Page,
                &tex_state::PdfColorStackAction::Current,
            )
            .expect("single-page-keyword stack exists");
        assert_eq!(
            single_page_keyword.mode,
            tex_state::PdfColorStackMode::Origin,
            "one `page` keyword is the restoration flag, not the framing mode",
        );
        assert_eq!(single_page_keyword.payload, b"R");

        universe.enable_pdf_output();
        let restorations = universe.pdf_page_color_stack_restorations();
        assert_eq!(
            restorations
                .into_iter()
                .map(|emission| (emission.mode, emission.payload))
                .collect::<Vec<_>>(),
            [
                (tex_state::PdfColorStackMode::Direct, b"D".to_vec()),
                (tex_state::PdfColorStackMode::Page, b"P".to_vec()),
                (tex_state::PdfColorStackMode::Origin, b"R".to_vec()),
            ],
        );
    }
}

#[test]
fn creation_date_uses_each_jobs_tracked_clock_fresh_and_after_format_load() {
    // pdftex.web §1590: `pdf_creation_date_code` inserts the immutable
    // job-start timestamp. The LaTeX compatibility spelling and pdfTeX
    // spelling are aliases of that one conversion, while a loaded format
    // must read the new job's World clock rather than format-time state.
    let fresh_clock = JobClock {
        time: 13 * 60 + 36,
        second: 0,
        day: 9,
        month: 7,
        year: 2026,
    };
    let loaded_clock = JobClock {
        time: 2 * 60 + 3,
        second: 4,
        day: 5,
        month: 6,
        year: 2042,
    };

    let mut latex_fresh = Universe::with_world(World::memory_with_clock(fresh_clock));
    crate::primitives::install_latex_expandable_primitives(&mut latex_fresh);
    let latex_format = latex_fresh.dump_format().expect("quiescent LaTeX format");
    let mut latex_loaded =
        Universe::from_format(World::memory_with_clock(loaded_clock), &latex_format)
            .expect("LaTeX format loads");
    crate::primitives::register_latex_expandable_primitives(&mut latex_loaded);

    let mut pdftex = Universe::with_world(World::memory_with_clock(fresh_clock));
    crate::primitives::install_pdftex_expandable_primitives(&mut pdftex);

    for (mut universe, profile, spelling, expected) in [
        (
            latex_fresh,
            crate::CommandProfile::ETEX26,
            "creationdate",
            "D:20260709133600Z",
        ),
        (
            latex_loaded,
            crate::CommandProfile::ETEX26,
            "creationdate",
            "D:20420605020304Z",
        ),
        (
            pdftex,
            crate::CommandProfile::PDFTEX14029,
            "pdfcreationdate",
            "D:20260709133600Z",
        ),
    ] {
        let primitive = universe
            .symbol(spelling)
            .expect("profile spelling is installed");
        assert_eq!(
            universe.meaning(primitive),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::CreationDate)
        );
        let mut command = CommandState::new(profile);
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(
                primitive.symbol(),
            ))])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mark = universe
            .begin_tracked_region()
            .expect("start tracked conversion");
        {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities);
            assert_eq!(rendered(&mut processor), expected);
        }
        let dependencies = universe
            .finish_tracked_region(mark)
            .expect("conversion remains memoizable");
        assert!(dependencies.observations().iter().any(|observation| {
            observation.key
                == DependencyKey::World {
                    field: DependencyWorldField::JobClock,
                    index: 0,
                }
        }));
    }
}

#[test]
fn latex_shell_escape_reports_each_loaded_jobs_tracked_policy() {
    // pdfTeX and XeTeX change section [53a]: the shell-escape enquiry is 0
    // when disabled, 1 when unrestricted, and 2 when restricted. The LaTeX
    // compatibility spelling shares that policy and a loaded format must use
    // the new job's World rather than the construction policy.
    fn world(policy: ShellEscapePolicy) -> World {
        World::memory_with_pdftex_inputs(JobClock::DEFAULT, 0, 0, policy)
    }

    let mut disabled = Universe::with_world(world(ShellEscapePolicy::Disabled));
    crate::primitives::install_latex_expandable_primitives(&mut disabled);
    let format = disabled.dump_format().expect("quiescent LaTeX format");
    let mut restricted = Universe::from_format(world(ShellEscapePolicy::Restricted), &format)
        .expect("LaTeX format loads");
    crate::primitives::register_latex_expandable_primitives(&mut restricted);
    let mut enabled = Universe::with_world(world(ShellEscapePolicy::Enabled));
    crate::primitives::install_latex_expandable_primitives(&mut enabled);

    for (mut universe, expected) in [(disabled, "0"), (enabled, "1"), (restricted, "2")] {
        let primitive = universe
            .symbol("shellescape")
            .expect("LaTeX compatibility spelling is installed");
        assert_eq!(
            universe.meaning(primitive),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ShellEscape)
        );
        let mut command = CommandState::new(crate::CommandProfile::ETEX26);
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(
                primitive.symbol(),
            ))])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mark = universe
            .begin_tracked_region()
            .expect("start tracked conversion");
        {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities);
            assert_eq!(rendered(&mut processor), expected);
        }
        let dependencies = universe
            .finish_tracked_region(mark)
            .expect("shell status remains memoizable");
        assert!(dependencies.observations().iter().any(|observation| {
            observation.key == DependencyKey::Engine(DependencyEngineField::PdfShellEscape)
        }));
    }
}

#[test]
fn etex_scantokens_retokenizes_balanced_text_as_nested_lines() {
    // e-TeX 2.6 etex.ch §53a: pseudo_start applies token_show, splits at the
    // live \newlinechar, and reads the result under the live catcode table.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.set_int_param(IntParam::NEWLINE_CHAR, i32::from(b'|'));
    universe.set_catcode('a', Catcode::Other);
    let every_eof = universe.intern_token_list(&[Token::Char {
        ch: 'E',
        cat: Catcode::Letter,
    }]);
    universe.set_tok_param(tex_state::env::banks::TokParam::EVERY_EOF, every_eof);
    let scantokens =
        install_expandable(&mut universe, "scantokens", ExpandablePrimitive::Scantokens);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(scantokens)),
            traced(Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            }),
            traced(Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            }),
            traced(Token::Char {
                ch: '|',
                cat: Catcode::Other,
            }),
            traced(Token::Char {
                ch: 'b',
                cat: Catcode::Letter,
            }),
            traced(Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            }),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut fuel = crate::CommandFuelLedger::new(64).expect("finite test fuel");
    let mut recorder = Recorder::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities)
        .with_fuel(fuel.fuel_mut())
        .with_observer(&mut recorder);
    let mut output = Vec::new();
    while let Some(delivery) = processor.get_x_token().expect("scantokens expands") {
        output.push(delivery.spelling().semantic_token());
    }
    assert_eq!(
        processor.command.stack_usage().buffer_stack,
        7,
        "e-TeX §53a counts the four-byte pseudo_input block and both §1334 margins"
    );
    assert_eq!(
        output.first(),
        Some(&Token::Char {
            ch: 'a',
            cat: Catcode::Other,
        })
    );
    assert!(output.iter().any(|token| {
        matches!(
            token,
            Token::Char {
                ch: 'b',
                cat: Catcode::Letter
            }
        )
    }));
    assert_eq!(
        output.last(),
        Some(&Token::Char {
            ch: 'E',
            cat: Catcode::Letter,
        }),
        "\\everyeof must replay after the pseudo-file's final line"
    );
    assert!(fuel.burned() <= 64);
    assert!(
        !recorder
            .0
            .iter()
            .any(|event| matches!(event, CommandObservation::ScannerStatus(_))),
        "e-TeX §53a scan_general_text does not publish TeX82 scan_toks status observations"
    );
    assert_eq!(
        recorder
            .0
            .iter()
            .filter(|event| matches!(
                event,
                CommandObservation::TokenList(record)
                    if record.transition == "complete" && record.purpose == "scantokens"
            ))
            .count(),
        1
    );
    assert!(recorder.0.iter().any(|event| matches!(
        event,
        CommandObservation::Input(crate::InputRecord {
            transition: crate::InputTransition::Push,
            reason: crate::InputReason::Source,
            source_name: Some(crate::SourceNameClass::Terminal),
            ..
        })
    )));
    let generated = recorder
        .0
        .iter()
        .find_map(|observation| match observation {
            CommandObservation::GeneratedSource(record) => Some(record),
            _ => None,
        })
        .expect("scantokens backing is observable before its source push");
    assert_eq!(generated.name, "^^R");
    assert_eq!(generated.source.bytes.as_ref(), b"a\nb\n");
}

#[test]
fn etex_scantokens_records_file_warning_open_depths() {
    // e-TeX 2.6 [23.328] records `grp_stack`/`if_stack` for pseudo-files just
    // as `begin_file_reading` does for ordinary `\input` levels.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.enter_group_with_kind(tex_state::GroupKind::SemiSimple);
    let scantokens =
        install_expandable(&mut universe, "scantokens", ExpandablePrimitive::Scantokens);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(scantokens)),
            traced(Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            }),
            traced(Token::Char {
                ch: 'X',
                cat: Catcode::Letter,
            }),
            traced(Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            }),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        assert_eq!(
            processor
                .get_x_token()
                .expect("scantokens expands")
                .expect("pseudo-source token")
                .spelling()
                .semantic_token(),
            Token::Char {
                ch: 'X',
                cat: Catcode::Letter,
            }
        );
    }
    let level = command
        .top_input_level_identity()
        .expect("pseudo-source remains live");
    assert_eq!(
        command.source_open_depths(level),
        Some(crate::input::SourceOpenDepths {
            group_lineages: universe
                .group_frames()
                .map(|frame| frame.lineage())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            conditional_identities: Box::new([]),
        })
    );
}

#[test]
fn etex_scantokens_pseudo_source_name_tracks_tracing() {
    assert_eq!(scantokens_source_name(0), "^^R");
    assert_eq!(scantokens_source_name(-1), "^^R");
    assert_eq!(scantokens_source_name(1), "^^S");
    assert_eq!(scantokens_numeric_name(0), 18);
    assert_eq!(scantokens_numeric_name(-1), 18);
    assert_eq!(scantokens_numeric_name(1), 19);
}

#[test]
fn nested_scantokens_error_context_crosses_numeric_names_18_and_19() {
    // e-TeX 2.6 merged §§22 and 53a: pseudo_start assigns numeric name 18
    // (or 19 while tracing), and show_context stops only at name>19. Both
    // nested pseudo-files therefore remain visible above the ordinary file.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let source = command
        .register_source(
            SourceRegistration::new(
                RegisteredSourceKind::Generated,
                br"\scantokens{\scantokens{\undefined}X}Y".as_slice(),
            )
            .with_name("outer.tex"),
        )
        .expect("outer file registers");
    command
        .open_registered_source(source)
        .expect("outer file opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    install_expandable(&mut universe, "scantokens", ExpandablePrimitive::Scantokens);
    universe.set_int_param(tex_state::env::banks::IntParam::TRACING_SCAN_TOKENS, 1);
    universe.set_int_param(tex_state::env::banks::IntParam::new(54), 10);
    let mut capabilities = CommandHostCapabilities::default();

    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let resumed = processor
            .get_x_token()
            .expect("nested scantokens recovery is finite")
            .expect("inner pseudo-file resumes");
        assert_eq!(
            resumed.spelling().semantic_token(),
            Token::Char {
                ch: 'X',
                cat: Catcode::Letter,
            }
        );
    }
    let diagnostics = command.take_semantic_diagnostics();
    let [crate::CommandSemanticDiagnostic::UndefinedControlSequence { context }] =
        diagnostics.as_slice()
    else {
        panic!("one undefined-control-sequence diagnostic expected");
    };
    assert_eq!(
        context,
        "\nl.1 \\undefined\n              \nl.1 \\scantokens {\\undefined }\n                             X\nl.1 \\scantokens{\\scantokens{\\undefined}X}\n                                         Y"
    );
}

#[test]
fn scantokens_everyeof_context_traverses_to_ordinary_file() {
    // e-TeX 2.6 merged §§22 and 53a: §24.362's everyeof token list sits above
    // the exhausted name-18 pseudo-file, and context traversal reaches the
    // enclosing real file before stopping.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let source = command
        .register_source(
            SourceRegistration::new(
                RegisteredSourceKind::Generated,
                br"\scantokens{A}Z".as_slice(),
            )
            .with_name("outer.tex"),
        )
        .expect("outer file registers");
    command
        .open_registered_source(source)
        .expect("outer file opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    install_expandable(&mut universe, "scantokens", ExpandablePrimitive::Scantokens);
    universe.set_int_param(tex_state::env::banks::IntParam::new(54), 10);
    universe.set_int_param(tex_state::env::banks::IntParam::TRACING_SCAN_TOKENS, 1);
    let undefined = universe.intern("undefined").symbol();
    let every_eof = universe.intern_token_list(&[Token::Cs(undefined)]);
    universe.set_tok_param(tex_state::env::banks::TokParam::EVERY_EOF, every_eof);
    let mut capabilities = CommandHostCapabilities::default();

    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let first = processor
            .get_x_token()
            .expect("scantokens expands")
            .expect("A");
        assert_eq!(
            first.spelling().semantic_token(),
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter
            }
        );
        loop {
            let resumed = processor
                .get_x_token()
                .expect("everyeof recovery is finite")
                .expect("outer file resumes");
            if resumed.spelling().semantic_token()
                == (Token::Char {
                    ch: 'Z',
                    cat: Catcode::Letter,
                })
            {
                break;
            }
        }
    }
    let diagnostics = command.take_semantic_diagnostics();
    let [crate::CommandSemanticDiagnostic::UndefinedControlSequence { context }] =
        diagnostics.as_slice()
    else {
        panic!("one undefined-control-sequence diagnostic expected");
    };
    assert_eq!(
        context,
        "\n<everyeof> \\undefined \n                      \nl.2 \n    \nl.1 \\scantokens{A}\n                  Z"
    );
    assert_eq!(
        command.take_file_framing_events(),
        [crate::FileFramingEvent::Close],
        "§370's pending error must precede §362's later pseudo-file close"
    );
}

#[test]
fn trace_after_expansion_errors_stays_behind_their_reports() {
    // TeX82 §§345/367/370/380: undefined expansion reports synchronously,
    // invalid-character restart reports next, and only then may the following
    // begin-group command be traced.
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"\\undefined\x7f{".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.set_catcode('\u{7f}', Catcode::Invalid);
    universe.set_int_param(tex_state::env::banks::IntParam::TRACING_COMMANDS, 2);
    let mut capabilities = CommandHostCapabilities::default();

    let next = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let next = processor
            .get_x_token()
            .expect("recovery remains finite")
            .expect("following command remains");
        processor.print_command_trace(crate::PrintCommand::from_current(&next));
        next
    };
    assert!(matches!(
        next.meaning(),
        Meaning::CharToken {
            ch: '{',
            cat: Catcode::BeginGroup
        }
    ));
    let diagnostics = command.take_semantic_diagnostics();
    assert!(
        matches!(
            diagnostics.as_slice(),
            [
                crate::CommandSemanticDiagnostic::UndefinedControlSequence { .. },
                crate::CommandSemanticDiagnostic::Recoverable { message, .. },
                crate::CommandSemanticDiagnostic::Trace { text, .. }
            ] if message == "Text line contains an invalid character"
                && text == "{begin-group character {}"
        ),
        "{diagnostics:?}"
    );
}

#[test]
fn etex_scantokens_null_everyeof_has_no_token_list_retirement() {
    // e-TeX 2.6 etex.ch §24.362 tests `every_eof<>null` before
    // `begin_token_list`. The default null parameter must therefore return
    // directly from the pseudo-file to its enclosing input level.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let scantokens =
        install_expandable(&mut universe, "scantokens", ExpandablePrimitive::Scantokens);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(scantokens)),
            traced(Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            }),
            traced(Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            }),
            traced(Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            }),
            traced(Token::Char {
                ch: 'Z',
                cat: Catcode::Letter,
            }),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut fuel = crate::CommandFuelLedger::new(64).expect("finite test fuel");
    let mut recorder = Recorder::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities)
        .with_fuel(fuel.fuel_mut())
        .with_observer(&mut recorder);
    let mut output = Vec::new();
    while let Some(delivery) = processor.get_x_token().expect("scantokens expands") {
        output.push(delivery.spelling().semantic_token());
    }

    assert!(output.contains(&Token::Char {
        ch: 'a',
        cat: Catcode::Letter,
    }));
    assert_eq!(
        output.last(),
        Some(&Token::Char {
            ch: 'Z',
            cat: Catcode::Letter,
        })
    );
    assert!(!recorder.0.iter().any(|event| matches!(
        event,
        CommandObservation::Input(crate::InputRecord {
            transition: InputTransition::Retire,
            reason: crate::InputReason::Recovery,
            ..
        })
    )));
}

#[test]
fn etex_scantokens_defined_empty_everyeof_pushes_and_retires_before_close() {
    // e-TeX 2.6 etex.ch §24.362 tests pointer presence, not list length.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.set_tok_param(
        tex_state::env::banks::TokParam::EVERY_EOF,
        TokenListId::EMPTY,
    );
    let scantokens =
        install_expandable(&mut universe, "scantokens", ExpandablePrimitive::Scantokens);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(scantokens)),
            traced(Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            }),
            traced(Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            }),
            traced(Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            }),
            traced(Token::Char {
                ch: 'Z',
                cat: Catcode::Letter,
            }),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut fuel = crate::CommandFuelLedger::new(64).expect("finite test fuel");
    let mut recorder = Recorder::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities)
        .with_fuel(fuel.fuel_mut())
        .with_observer(&mut recorder);
    let mut output = Vec::new();
    while let Some(delivery) = processor.get_x_token().expect("scantokens expands") {
        output.push(delivery.spelling().semantic_token());
    }

    assert_eq!(
        output.last(),
        Some(&Token::Char {
            ch: 'Z',
            cat: Catcode::Letter,
        })
    );
    assert_eq!(
        recorder
            .0
            .iter()
            .filter(|event| matches!(
                event,
                CommandObservation::Input(crate::InputRecord {
                    transition: InputTransition::Retire,
                    reason: crate::InputReason::EveryEof,
                    ..
                })
            ))
            .count(),
        1,
        "the present empty everyeof level retires before pseudo_close resumes Z"
    );
    let every_eof_push = recorder
        .0
        .iter()
        .position(|event| {
            matches!(
                event,
                CommandObservation::Input(crate::InputRecord {
                    transition: InputTransition::Push,
                    reason: crate::InputReason::EveryEof,
                    ..
                })
            )
        })
        .expect("present empty everyeof pushes");
    let source_retirement = recorder
        .0
        .iter()
        .position(|event| {
            matches!(
                event,
                CommandObservation::Input(crate::InputRecord {
                    transition: InputTransition::Retire,
                    reason: crate::InputReason::Source,
                    ..
                })
            )
        })
        .expect("pseudo-file retires");
    assert!(
        every_eof_push < source_retirement,
        "etex.ch §24.362 begins everyeof before §329 retires the pseudo-file"
    );
}

#[test]
fn etex_detokenize_projects_token_show_text_without_expansion() {
    // e-TeX 2.6 etex.ch §53a: scan_general_text is unexpanded, token_show
    // separates a control word, and str_toks makes only spaces category 10.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let detokenize =
        install_expandable(&mut universe, "detokenize", ExpandablePrimitive::Detokenize);
    let payload = install_macro(
        &mut universe,
        "payload",
        Token::Char {
            ch: 'X',
            cat: Catcode::Letter,
        },
    );
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(detokenize)),
            traced(Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            }),
            traced(Token::Cs(payload)),
            traced(Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            }),
            traced(Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            }),
            traced(Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            }),
            traced(Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            }),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut fuel = crate::CommandFuelLedger::new(32).expect("finite test fuel");
    let mut processor =
        processor(&mut command, &mut universe, &mut capabilities).with_fuel(fuel.fuel_mut());
    let mut output = Vec::new();
    while let Some(delivery) = processor.get_x_token().expect("detokenize expands") {
        output.push(delivery.spelling().semantic_token());
    }
    let text = output
        .iter()
        .map(|token| match token {
            Token::Char { ch, .. } => *ch,
            _ => panic!("detokenize returned a non-character token"),
        })
        .collect::<String>();
    assert_eq!(text, "\\payload ##{}");
    assert!(output.iter().all(|token| matches!(
        token,
        Token::Char {
            ch: ' ',
            cat: Catcode::Space
        } | Token::Char {
            cat: Catcode::Other,
            ..
        }
    )));
    assert!(fuel.burned() <= 32);
}

#[test]
fn etex_detokenize_observes_live_escape_and_control_sequence_kinds() {
    // e-TeX §53a delegates spelling to token_show: active characters have no
    // escape, control symbols have no separator, and \csname\endcsname uses
    // the live escape character.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.set_int_param(IntParam::ESCAPE_CHAR, i32::from(b'!'));
    let detokenize =
        install_expandable(&mut universe, "detokenize", ExpandablePrimitive::Detokenize);
    let active = universe.intern_active_character('~').symbol();
    let symbol = universe.intern("@").symbol();
    let empty = universe.intern("").symbol();
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(detokenize)),
            traced(Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            }),
            traced(Token::Cs(active)),
            traced(Token::Cs(symbol)),
            traced(Token::Cs(empty)),
            traced(Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            }),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    assert_eq!(rendered(&mut processor), "~!@!csname!endcsname ");
}

#[test]
fn etex_detokenize_the_toks_microfixture_matches_fresh_and_loaded_formats() {
    // e-TeX 2.6 etex.ch §§[25.386], [27.465]: a numbered mark enquiry is
    // still unexpanded general text here, while detokenize's converted
    // character list is returned through `the_toks` and joins the enclosing
    // expanded collector directly. The format round trip is a negative
    // control for primitive-table reconstruction.
    let mut fresh = crate::test_harness::universe_with_plain_catcodes();
    crate::primitives::install_etex_expandable_primitives(&mut fresh);
    let format = fresh.dump_format().expect("quiescent e-TeX format");
    let mut loaded = Universe::from_format(World::default(), &format).expect("format loads");
    crate::primitives::register_etex_expandable_primitives(&mut loaded);

    for mut universe in [fresh, loaded] {
        let mut command = CommandState::new(crate::CommandProfile::ETEX26);
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(
                    include_bytes!("../fixtures/etex-detokenize-the-toks.tex").as_slice(),
                ),
            ))
            .expect("microfixture registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let result = {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities)
                .with_observer(&mut recorder);
            processor
                .scan_toks(crate::scan_toks::ScanToksMode::General { expanded: true })
                .expect("expanded collection succeeds")
        };

        let rendered = universe
            .tokens(result.replacement_text.token_list())
            .iter()
            .map(|token| match token {
                Token::Char { ch, .. } => *ch,
                _ => panic!("detokenize must return only character tokens"),
            })
            .collect::<String>();
        assert_eq!(rendered, "\\splitfirstmarks 0");
        let token_lists = recorder
            .0
            .iter()
            .filter_map(|record| match record {
                CommandObservation::TokenList(record) => Some(record),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            token_lists
                .iter()
                .map(|record| (record.transition, record.purpose))
                .collect::<Vec<_>>(),
            [
                ("complete", "detokenize"),
                ("splice", "the_toks"),
                ("complete", "expanded_scan_toks"),
            ]
        );
        assert!(
            token_lists[0]
                .tokens
                .iter()
                .all(|token| matches!(token, ObservedToken::Character { .. }))
        );
        assert_eq!(token_lists[0].tokens, token_lists[1].tokens);
        assert!(
            !recorder
                .0
                .iter()
                .any(|record| matches!(record, CommandObservation::Recovery(_)))
        );
    }
}

fn rendered(processor: &mut CommandProcessor<'_>) -> String {
    let mut text = String::new();
    while let Some(command) = processor.get_x_token().expect("conversion expands") {
        let Token::Char { ch, .. } = command.spelling().semantic_token() else {
            panic!("expected rendered character")
        };
        text.push(ch);
    }
    text
}

#[test]
fn pdf_ximage_bbox_reads_typed_metadata_without_allocating() {
    // pdftex.web §470 scans the existing ximage identity and coordinate in
    // that order, then renders the detached page-box value as a dimension.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let bbox = install_expandable(
        &mut universe,
        "pdfximagebbox",
        ExpandablePrimitive::PdfXImageBBox,
    );
    let id = tex_state::PdfExternalImageId::new(7).expect("image id");
    universe
        .register_pdf_external_image(
            id,
            tex_state::PdfExternalImageMetadata::PdfPage {
                total_pages: 1,
                page_box: tex_state::PdfPageBox {
                    left: Scaled::from_raw(-2),
                    bottom: Scaled::from_raw(3),
                    right: Scaled::from_raw(10),
                    top: Scaled::from_raw(20),
                },
                rotation: tex_state::PdfPageRotation::None,
                page: 1,
                has_page_group: false,
                pdf_version: (1, 4),
            },
        )
        .expect("register detached metadata");
    let initial_object = universe.pdf_next_object_id();
    let initial_hash = universe.snapshot().state_hash();
    let mut input = vec![traced(Token::Cs(bbox))];
    input.extend("7 3".chars().map(|ch| {
        traced(Token::Char {
            ch,
            cat: if ch == ' ' {
                Catcode::Space
            } else {
                Catcode::Other
            },
        })
    }));
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(input)),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );

    let mut capabilities = CommandHostCapabilities::default();
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        assert_eq!(rendered(&mut processor), "0.00015pt");
    }
    assert_eq!(universe.pdf_next_object_id(), initial_object);
    assert_eq!(universe.snapshot().state_hash(), initial_hash);
}

#[test]
fn the_invalid_operand_reports_exactly_and_does_not_replay_it() {
    // TeX82 §§465/467: `the_toks` owns its operand. A non-internal command is
    // consumed by the recovery, replaced with integer zero, and expansion
    // resumes at the following source token without replaying the operand.
    use tex_state::meaning::UnexpandablePrimitive as P;

    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(br"\the\hbox Z".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    install_expandable(&mut universe, "the", ExpandablePrimitive::The);
    let hbox = universe.intern("hbox").symbol();
    universe.set_meaning(hbox, Meaning::UnexpandablePrimitive(P::HBox));
    let mut capabilities = CommandHostCapabilities::default();

    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let zero = processor
            .get_x_token()
            .expect("invalid operand recovers")
            .expect("zero substitution");
        assert_eq!(
            zero.spelling().semantic_token(),
            Token::Char {
                ch: '0',
                cat: Catcode::Other
            }
        );
        let following = processor
            .get_x_token()
            .expect("expansion resumes")
            .expect("following source token");
        assert_eq!(
            following.spelling().semantic_token(),
            Token::Char {
                ch: 'Z',
                cat: Catcode::Letter
            }
        );
        while let Some(command) = processor.get_x_token().expect("source retires") {
            assert_ne!(
                command.spelling().semantic_token(),
                Token::Cs(hbox),
                "the invalid operand is consumed exactly once"
            );
        }
    }
    assert_eq!(
        diagnostic_text(&universe),
        "! You can't use `\\hbox' after \\the.\nl.1 \\the\\hbox\n              Z\nI'm forgetting what you said and using zero instead.\n\n"
    );
}

fn chars(processor: &mut CommandProcessor<'_>) -> String {
    let mut text = String::new();
    while let Some(command) = processor.get_x_token().expect("input expands") {
        if let Token::Char { ch, .. } = command.spelling().semantic_token() {
            text.push(ch);
        }
    }
    text
}

fn letters(text: &str) -> Vec<Token> {
    text.chars()
        .map(|ch| Token::Char {
            ch,
            cat: Catcode::Letter,
        })
        .collect()
}

#[test]
fn input_uses_only_capability_registered_backing_and_returns_to_parent() {
    let mut command = CommandState::default();
    let parent = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"\\input{inc}z".as_slice()),
        ))
        .expect("parent registers");
    command
        .open_registered_source(parent)
        .expect("parent opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    install_expandable(&mut universe, "input", ExpandablePrimitive::Input);
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.register_input(
        "inc.tex",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(b"ab".as_slice()),
        ),
    );
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(chars(&mut processor), "ab z ");
}

#[test]
fn unavailable_expandable_input_carries_its_triggering_origin() {
    let mut command = CommandState::default();
    let parent = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"\\input absent".as_slice()),
        ))
        .expect("parent registers");
    command
        .open_registered_source(parent)
        .expect("parent opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    install_expandable(&mut universe, "input", ExpandablePrimitive::Input);
    universe.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.mark_input_unavailable("absent.tex");
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    let error = processor
        .get_x_token()
        .expect_err("authoritatively unavailable input is fatal in nonstop mode");
    let (error, origin) = match error {
        CommandError::AtOrigin { error, origin } => (error, origin),
        other => panic!("expandable input error lacks its delivery origin: {other:?}"),
    };
    assert!(matches!(*error, CommandError::Fatal(_)));
    assert_ne!(origin, OriginId::UNKNOWN);
}

#[test]
fn recursive_input_during_filename_scan_inserts_frozen_relax_before_restored_input() {
    // TeX82 §§378/527: a recursively expanded `\input` cannot start another
    // filename scan. It is restored beneath inaccessible `frozen_relax`, so
    // the active scan stops first and ordinary expansion rereads the original
    // command only after the filename boundary has ended.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let input = install_expandable(&mut universe, "input", ExpandablePrimitive::Input);
    let mut tokens = vec![Token::Cs(input)];
    tokens.extend(letters("inc"));
    tokens.push(Token::Char {
        ch: ' ',
        cat: Catcode::Space,
    });
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(
            tokens.into_iter().map(traced).collect::<Vec<_>>(),
        )),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    command
        .begin_file_name()
        .expect("outer filename scan begins");

    let mut capabilities = CommandHostCapabilities::default();
    capabilities.register_input(
        "inc.tex",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(b"z".as_slice()),
        ),
    );
    let mut recorder = Recorder::default();

    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        let boundary = processor
            .get_x_token()
            .expect("recursive input recovery succeeds")
            .expect("frozen relax terminates the filename scan");
        assert_eq!(boundary.spelling().semantic_token(), Token::frozen_relax());
        assert_eq!(boundary.meaning(), Meaning::Relax);
    }
    assert!(command.name_in_progress());
    assert!(
        !recorder.0.iter().any(|observation| matches!(
            observation,
            CommandObservation::Effect(effect)
                if effect.kind == crate::ObservationEffectKind::Input
        )),
        "the child must not open before the frozen-relax boundary"
    );

    command.end_file_name();
    let child_token = {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        processor
            .get_x_token()
            .expect("restored input expands")
            .expect("child source delivers its first token")
    };
    assert_eq!(
        child_token.spelling().semantic_token(),
        Token::Char {
            ch: 'z',
            cat: Catcode::Letter,
        }
    );
    assert_eq!(
        recorder
            .0
            .iter()
            .filter(|observation| matches!(
                observation,
                CommandObservation::Command(delivery)
                    if delivery.boundary == CommandDeliveryBoundary::Raw
                        && delivery.command == "input"
            ))
            .count(),
        2,
        "the original input is encountered once recursively and reread once"
    );
    assert_eq!(
        recorder
            .0
            .iter()
            .filter(|observation| matches!(
                observation,
                CommandObservation::Effect(effect)
                    if effect.kind == crate::ObservationEffectKind::Input
                        && effect.channel == "inc.tex"
            ))
            .count(),
        1,
        "the restored input opens the child exactly once"
    );
}

#[test]
fn recursive_input_retires_inserted_relax_before_restored_input_diagnostic() {
    // TeX82 §§310, 314, 378, 527: `insert_relax` creates two input levels and
    // retypes only the frozen-relax level as `inserted`. Once that terminator
    // has been read, the next expanded fetch retires its depleted level before
    // diagnosing the separately backed-up command as `<recently read>`.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let input = install_expandable(&mut universe, "input", ExpandablePrimitive::Input);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(input))])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    command
        .begin_file_name()
        .expect("outer filename scan begins");

    let mut capabilities = CommandHostCapabilities::default();
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let boundary = processor
            .get_x_token()
            .expect("recursive input recovery succeeds")
            .expect("frozen relax terminates the filename scan");
        assert_eq!(boundary.spelling().semantic_token(), Token::frozen_relax());
    }

    command.end_file_name();
    universe.set_meaning(input, Meaning::Undefined);
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        assert!(
            processor
                .get_x_token()
                .expect("restored undefined input is diagnosed")
                .is_none()
        );
    }
    let diagnostic = command
        .take_semantic_diagnostics()
        .into_iter()
        .find_map(|diagnostic| match diagnostic {
            crate::CommandSemanticDiagnostic::UndefinedControlSequence { context } => Some(context),
            _ => None,
        })
        .expect("undefined input diagnostic is queued");
    assert!(
        diagnostic.contains("<recently read> \\input "),
        "{diagnostic:?}"
    );
    assert!(!diagnostic.contains("<inserted text>"), "{diagnostic:?}");
}

fn effect_text(universe: &Universe) -> String {
    universe
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            tex_state::EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn cross_file_nesting_warning_text(tracing_nesting: i32, conditional: bool) -> String {
    let mut command = CommandState::default();
    if conditional {
        command.conditions.push(ConditionalKind::IfTrue, 3);
    }
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"a".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let level = command
        .top_input_level_identity()
        .expect("source level is live");
    command.record_source_open_depths(
        level,
        if conditional {
            Box::new([])
        } else {
            Box::new([0])
        },
        command
            .conditions
            .frames
            .iter()
            .map(|frame| frame.identity.0)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.set_int_param(
        tex_state::env::banks::IntParam::TRACING_NESTING,
        tracing_nesting,
    );
    let mut capabilities = CommandHostCapabilities::default();
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("source reads")
            .expect("source token is delivered");
        if conditional {
            let frame = processor
                .command
                .conditions
                .current()
                .expect("condition is live")
                .clone();
            processor.warn_cross_file_conditional_close(&frame);
        } else {
            processor.warn_cross_file_group_close(1, "simple group", 3);
        }
    }
    effect_text(&universe)
}

fn cross_file_nesting_warning_with_macro_context() -> String {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"\\m ".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let level = command
        .top_input_level_identity()
        .expect("source level is live");
    command.record_source_open_depths(level, Box::new([0]), Box::new([]));
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    install_macro(
        &mut universe,
        "m",
        Token::Char {
            ch: 'a',
            cat: Catcode::Letter,
        },
    );
    universe.set_int_param(tex_state::env::banks::IntParam::TRACING_NESTING, 2);
    let mut capabilities = CommandHostCapabilities::default();
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("macro expands")
            .expect("replacement token is delivered");
        processor.warn_cross_file_group_close(1, "simple group", 3);
    }
    effect_text(&universe)
}

fn file_boundary_nesting_warning_text(tracing_nesting: i32, conditional: bool) -> String {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"a".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.set_int_param(
        tex_state::env::banks::IntParam::TRACING_NESTING,
        tracing_nesting,
    );
    let mut capabilities = CommandHostCapabilities::default();
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("source reads")
            .expect("source token is delivered");
    }
    if conditional {
        command.conditions.push(ConditionalKind::IfFalse, 4);
    } else {
        universe.enter_group_with_kind(tex_state::GroupKind::Simple);
    }
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor.warn_file_boundary_incomplete(
            crate::input::SourceOpenDepths {
                group_lineages: Box::new([]),
                conditional_identities: Box::new([]),
            },
            None,
        );
    }
    effect_text(&universe)
}

#[test]
fn file_warning_reports_replaced_saved_group_before_coexisting_conditional() {
    // e-TeX 2.6 [23.328] saves `cur_boundary`, not just `cur_level`. Closing
    // that boundary, replacing it at the same level, and opening another
    // group above it must report both replacement groups before the
    // conditional loop begins.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.set_int_param(tex_state::env::banks::IntParam::TRACING_NESTING, 1);
    universe.enter_group_with_kind(tex_state::GroupKind::Simple);
    universe.enter_group_with_kind(tex_state::GroupKind::AdjustedHBox);
    let open_depths = crate::input::SourceOpenDepths {
        group_lineages: universe
            .group_frames()
            .map(|frame| frame.lineage())
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        conditional_identities: Box::new([]),
    };
    let _ = universe.leave_group();
    universe.enter_group_with_kind_at_line(tex_state::GroupKind::VTop, 5);
    universe.enter_group_with_kind_at_line(tex_state::GroupKind::MathShift, 7);
    command.conditions.push(ConditionalKind::IfFalse, 9);
    let mut capabilities = CommandHostCapabilities::default();
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor.warn_file_boundary_incomplete(open_depths, None);
    }

    let output = effect_text(&universe);
    let math = output
        .find("math shift group (level 3)")
        .expect("math warning");
    let vtop = output.find("vtop group (level 2)").expect("vtop warning");
    let conditional = output.find("\\iffalse").expect("conditional warning");
    assert!(math < vtop && vtop < conditional, "{output:?}");
}

#[test]
fn tracingnesting_two_shows_context_after_cross_file_warnings() {
    // e-TeX 2.6 [23.328] shares this exact tail between `group_warning` and
    // `if_warning`: value 1 prints only the warning, while value 2 additionally
    // calls `show_context` against the live input cursor.
    for conditional in [false, true] {
        let terse = cross_file_nesting_warning_text(1, conditional);
        let contextual = cross_file_nesting_warning_text(2, conditional);
        assert!(terse.contains("of a different file"), "{terse:?}");
        assert!(!terse.contains("l.1"), "{terse:?}");
        assert!(
            contextual.contains("of a different file\nl.1"),
            "{contextual:?}"
        );
    }
}

#[test]
fn tracingnesting_two_preserves_macro_context_print_ln_separator() {
    let contextual = cross_file_nesting_warning_with_macro_context();
    assert!(
        contextual.contains("of a different file\n\n\\m ->a"),
        "{contextual:?}"
    );
}

#[test]
fn tracingnesting_two_shows_context_after_file_boundary_warnings() {
    // e-TeX 2.6 [23.328]'s `file_warning` shares the context threshold for
    // both of its incomplete-group and incomplete-conditional loops.
    for conditional in [false, true] {
        let terse = file_boundary_nesting_warning_text(1, conditional);
        let contextual = file_boundary_nesting_warning_text(2, conditional);
        assert!(terse.contains("is incomplete\n"), "{terse:?}");
        assert!(!terse.contains("l.1"), "{terse:?}");
        assert!(contextual.contains("is incomplete\nl.1"), "{contextual:?}");
    }
}

#[test]
fn tracingnesting_warns_when_a_file_ends_with_an_open_group() {
    // e-TeX 2.6 [23.328]'s `file_warning`: a group opened while reading
    // "inc.tex" and never closed before that file's natural EOF.
    let mut command = CommandState::default();
    let parent = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"\\input{inc}z".as_slice()),
        ))
        .expect("parent registers");
    command
        .open_registered_source(parent)
        .expect("parent opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    install_expandable(&mut universe, "input", ExpandablePrimitive::Input);
    universe.set_int_param(tex_state::env::banks::IntParam::TRACING_NESTING, 2);
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.register_input(
        "inc.tex",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(b"a".as_slice()),
        ),
    );
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        // Drains through `\input`'s expansion into "inc.tex"'s one character,
        // leaving the child source the live level.
        let delivered = processor
            .get_x_token()
            .expect("input expands")
            .expect("a token is delivered");
        assert_eq!(
            delivered.spelling().semantic_token(),
            Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            }
        );
    }
    // Simulate a `{` read from "inc.tex" that is never matched by a `}`
    // before the file's natural EOF. `enter_group_with_kind` (not
    // `..._at_line`) matches this raw-processor test's own tokens, which
    // carry no source line either.
    universe.enter_group_with_kind(tex_state::GroupKind::Simple);
    let text = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        chars(&mut processor)
    };
    assert_eq!(
        text.trim_start(),
        "z ",
        "the parent still resumes normally after the child ends"
    );
    let reported = effect_text(&universe);
    assert!(
        reported.contains("Warning: end of file when simple group (level 1) is incomplete\n"),
        "{reported:?}"
    );
    assert!(reported.contains("is incomplete\nl.2 a"), "{reported:?}");
}

#[test]
fn tracingnesting_file_warning_renders_saved_conditional_branch_and_line() {
    // e-TeX 2.6 [23.328]'s `file_warning` prints `\else` exactly for a
    // live frame whose saved `if_limit` is `fi_code`, followed by the saved
    // `if_line` through [49.3715]'s `print_if_line`.
    let mut command = CommandState::default();
    let parent = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"\\input{inc}z".as_slice()),
        ))
        .expect("parent registers");
    command
        .open_registered_source(parent)
        .expect("parent opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    install_expandable(&mut universe, "input", ExpandablePrimitive::Input);
    universe.set_int_param(tex_state::env::banks::IntParam::TRACING_NESTING, 1);
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.register_input(
        "inc.tex",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(b"a".as_slice()),
        ),
    );
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("input expands")
            .expect("child token is delivered");
    }
    let condition = command.conditions.push(ConditionalKind::IfFalse, 4);
    assert!(command.conditions.change_if_limit(condition, IfLimit::Fi));
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        chars(&mut processor);
    }
    assert!(
        effect_text(&universe).contains(
            "Warning: end of file when \\iffalse\\else entered on line 4 is incomplete\n"
        ),
        "{:?}",
        effect_text(&universe)
    );
}

#[test]
fn disabled_tracingnesting_emits_no_file_boundary_warning() {
    let mut command = CommandState::default();
    let parent = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"\\input{inc}z".as_slice()),
        ))
        .expect("parent registers");
    command
        .open_registered_source(parent)
        .expect("parent opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    install_expandable(&mut universe, "input", ExpandablePrimitive::Input);
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.register_input(
        "inc.tex",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(b"a".as_slice()),
        ),
    );
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("input expands")
            .expect("a token is delivered");
    }
    universe.enter_group_with_kind(tex_state::GroupKind::Simple);
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        chars(&mut processor);
    }
    assert!(!effect_text(&universe).contains("Warning"));
}

#[test]
fn endinput_keeps_its_line_but_retires_nested_source_before_the_next_line() {
    let mut command = CommandState::default();
    let parent = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"\\input{inc}z".as_slice()),
        ))
        .expect("parent registers");
    command
        .open_registered_source(parent)
        .expect("parent opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    install_expandable(&mut universe, "input", ExpandablePrimitive::Input);
    install_expandable(&mut universe, "endinput", ExpandablePrimitive::EndInput);
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.register_input(
        "inc.tex",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(b"a\\endinput b\nc".as_slice()),
        ),
    );
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(chars(&mut processor), "ab z ");
}

#[test]
fn child_endinput_retires_true_to_false_before_multiline_parent_resumes() {
    let mut command = CommandState::default();
    let parent = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"\\input{inc}\np".as_slice()),
        ))
        .expect("parent registers");
    command
        .open_registered_source(parent)
        .expect("parent opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    install_expandable(&mut universe, "input", ExpandablePrimitive::Input);
    install_expandable(&mut universe, "endinput", ExpandablePrimitive::EndInput);
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.register_input(
        "inc.tex",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(b"c\\endinput\nx".as_slice()),
        ),
    );
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        assert_eq!(chars(&mut processor), "c p ");
    }
    assert!(
        !command.input.force_eof,
        "TeX82 §362 clears force_eof before retiring the child"
    );
}

#[test]
fn jobname_and_mark_retrieval_replay_deterministic_state_values() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let jobname = install_expandable(&mut universe, "jobname", ExpandablePrimitive::JobName);
    let topmark = install_expandable(&mut universe, "topmark", ExpandablePrimitive::TopMark);
    let mark = universe.intern_token_list(&[Token::Char {
        ch: 'M',
        cat: Catcode::Letter,
    }]);
    universe.set_page_mark(PageMark::Top, mark);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(jobname)),
            traced(Token::Cs(topmark)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.set_job_name("paper");
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(rendered(&mut processor), "paperM");
}

#[test]
fn empty_mark_enquiries_match_fresh_and_loaded_etex_formats() {
    // TeX82 §386 and e-TeX 2.6 etex.ch [25.386] begin a mark-text token list
    // whenever the selected mark pointer is non-null, including a pointer to
    // an empty list. The absent class-four control still skips that level and
    // backs up its nonnumeric terminator through the ordinary §325 path.
    let mut fresh = crate::test_harness::universe_with_plain_catcodes();
    crate::primitives::install_tex82_expandable_primitives(&mut fresh);
    crate::primitives::install_etex_expandable_primitives(&mut fresh);
    let format = fresh.dump_format().expect("quiescent e-TeX format");
    let mut loaded = Universe::from_format(World::default(), &format).expect("format loads");
    crate::primitives::register_etex_expandable_primitives(&mut loaded);

    for mut universe in [fresh, loaded] {
        let mut command = CommandState::new(crate::CommandProfile::ETEX26);
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(include_bytes!("../fixtures/empty-marks.tex").as_slice()),
            ))
            .expect("microfixture registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        // e-TeX's sparse-array and class-zero `cur_mark` pointers are non-null
        // even when the referenced token list is empty. The class-four
        // enquiry is the absent control.
        universe.set_page_mark_class(PageMark::SplitFirst, 3, TokenListId::EMPTY);
        universe.set_page_mark(PageMark::First, TokenListId::EMPTY);
        assert_eq!(
            universe.page_mark_class_value(PageMark::SplitFirst, 3),
            Some(TokenListId::EMPTY)
        );
        assert_eq!(
            universe.page_mark_class_value(PageMark::First, 0),
            Some(TokenListId::EMPTY)
        );
        let output = {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities)
                .with_observer(&mut recorder);
            assert_eq!(
                processor
                    .state
                    .page_mark_class_value(PageMark::SplitFirst, 3),
                Some(TokenListId::EMPTY)
            );
            rendered(&mut processor)
        };

        assert_eq!(output, "X ");
        assert_eq!(
            recorder
                .0
                .iter()
                .filter(|event| matches!(
                    event,
                    CommandObservation::Input(crate::InputRecord {
                        transition: InputTransition::Push | InputTransition::Retire,
                        reason: crate::InputReason::Mark,
                        ..
                    })
                ))
                .count(),
            6
        );
        assert!(recorder.0.iter().any(|event| matches!(
            event,
            CommandObservation::Input(crate::InputRecord {
                transition: InputTransition::Backup,
                reason: crate::InputReason::Backup,
                ..
            })
        )));
    }
}

#[test]
fn etex_mark_class_enquiries_share_extended_register_scan_and_recovery() {
    // e-TeX 2.6 `etex.ch` [26.1178]: all five class enquiries use the same
    // `scan_register_num` as `\marks`, including invalid-to-zero recovery.
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let primitives = [
        (
            "topmarks",
            ExpandablePrimitive::TopMarks,
            PageMark::Top,
            'A',
        ),
        (
            "firstmarks",
            ExpandablePrimitive::FirstMarks,
            PageMark::First,
            'B',
        ),
        (
            "botmarks",
            ExpandablePrimitive::BotMarks,
            PageMark::Bot,
            'C',
        ),
        (
            "splitfirstmarks",
            ExpandablePrimitive::SplitFirstMarks,
            PageMark::SplitFirst,
            'D',
        ),
        (
            "splitbotmarks",
            ExpandablePrimitive::SplitBotMarks,
            PageMark::SplitBot,
            'E',
        ),
    ];
    let mut input = Vec::new();
    for (name, primitive, mark, value) in primitives {
        let symbol = install_expandable(&mut universe, name, primitive);
        let tokens = universe.intern_token_list(&[Token::Char {
            ch: value,
            cat: Catcode::Letter,
        }]);
        universe.set_page_mark_class(mark, 32_767, tokens);
        input.push(traced(Token::Cs(symbol)));
        input.extend("32767 ".chars().map(|ch| {
            traced(Token::Char {
                ch,
                cat: if ch == ' ' {
                    Catcode::Space
                } else {
                    Catcode::Other
                },
            })
        }));
    }
    let topmarks = universe.intern("topmarks").symbol();
    let zero = universe.intern_token_list(&[Token::Char {
        ch: 'Z',
        cat: Catcode::Letter,
    }]);
    universe.set_page_mark_class(PageMark::Top, 0, zero);
    input.push(traced(Token::Cs(topmarks)));
    input.extend("-1".chars().map(|ch| {
        traced(Token::Char {
            ch,
            cat: Catcode::Other,
        })
    }));
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(input)),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        assert_eq!(rendered(&mut processor), "ABCDEZ");
    }

    // e-TeX's `scan_register_num` rejects `-1` and reports it from inside the
    // scan, so `\topmarks-1` reads mark class zero and the diagnostic is
    // already on the channel.
    let reported: String = universe
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            tex_state::EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        reported.contains("! Bad register code (-1)."),
        "expected e-TeX's register-range report, got {reported:?}"
    );
}

#[test]
fn etex_revision_uses_the_canonical_conversion_token_path() {
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let revision = install_expandable(
        &mut universe,
        "eTeXrevision",
        ExpandablePrimitive::ETeXRevision,
    );
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(revision))])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    assert_eq!(rendered(&mut processor), ".6");
}

#[test]
fn pdftex_banner_is_operand_free_conversion_text_fresh_and_loaded() {
    // pdftex.web §§494 and 496--498: `\pdftexbanner` scans no operand and
    // returns `pdftex_banner` through `str_toks`, whose spaces are category 10
    // and whose other bytes are category 12. `utils.c::makepdftexbanner`
    // appends the pinned TeX Live and kpathsea identities. Exercise both
    // INITEX installation and post-format registry reconstruction.
    const BANNER: &str =
        "This is pdfTeX, Version 3.141592653-2.6-1.40.29 (TeX Live 2026) kpathsea version 6.4.2";

    let mut fresh = crate::test_harness::universe_with_plain_catcodes();
    crate::primitives::install_pdftex_expandable_primitives(&mut fresh);
    let format = fresh.dump_format().expect("quiescent pdfTeX format");
    let mut loaded = Universe::from_format(World::default(), &format).expect("format loads");
    crate::primitives::register_pdftex_expandable_primitives(&mut loaded);

    for mut universe in [fresh, loaded] {
        let banner = universe
            .symbol("pdftexbanner")
            .expect("pdfTeX banner spelling is installed");
        assert_eq!(
            universe.meaning(banner),
            Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfTeXBanner),
        );

        let mut command = CommandState::new(crate::CommandProfile::PDFTEX14029);
        let source = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                br"\pdftexbanner X%".as_slice(),
            ))
            .expect("source registers");
        command
            .open_registered_source(source)
            .expect("source opens");
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let mut tokens = Vec::new();
        while let Some(delivery) = processor.get_x_token().expect("banner expands") {
            tokens.push(delivery.spelling().semantic_token());
        }

        assert_eq!(
            tokens.pop(),
            Some(Token::Char {
                ch: 'X',
                cat: Catcode::Letter,
            }),
            "the sentinel remains because the conversion scans no operand",
        );
        assert_eq!(
            tokens
                .iter()
                .map(|token| match token {
                    Token::Char { ch, .. } => *ch,
                    _ => panic!("banner conversion returned a non-character token"),
                })
                .collect::<String>(),
            BANNER,
        );
        assert!(tokens.iter().all(|token| match token {
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            } => true,
            Token::Char {
                ch,
                cat: Catcode::Other,
            } => *ch != ' ',
            _ => false,
        }));
    }
}

#[test]
fn scalar_conversions_render_immutable_other_character_tokens() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let number = install_expandable(&mut universe, "number", ExpandablePrimitive::Number);
    let roman = install_expandable(
        &mut universe,
        "romannumeral",
        ExpandablePrimitive::RomanNumeral,
    );
    let string = install_expandable(&mut universe, "string", ExpandablePrimitive::String);
    let target = universe.intern("target").symbol();
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(number)),
            traced(Token::Char {
                ch: '-',
                cat: Catcode::Other,
            }),
            traced(Token::Char {
                ch: '4',
                cat: Catcode::Other,
            }),
            traced(Token::Char {
                ch: '2',
                cat: Catcode::Other,
            }),
            traced(Token::Cs(roman)),
            traced(Token::Char {
                ch: '9',
                cat: Catcode::Other,
            }),
            traced(Token::Cs(string)),
            traced(Token::Cs(target)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    assert_eq!(rendered(&mut processor), "-42ix\\target");
}

#[test]
fn conversion_rendering_publishes_recovery_input_before_its_first_token() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let number = install_expandable(&mut universe, "number", ExpandablePrimitive::Number);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(number)),
            traced(Token::Char {
                ch: '-',
                cat: Catcode::Other,
            }),
            traced(Token::Char {
                ch: '4',
                cat: Catcode::Other,
            }),
            traced(Token::Char {
                ch: '2',
                cat: Catcode::Other,
            }),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        assert_eq!(rendered(&mut processor), "-42");
    }

    let scanner = recorder
        .0
        .iter()
        .position(|record| matches!(record, CommandObservation::Scanner(scanner) if scanner.kind == "integer" && scanner.value == ObservationValue::Integer(-42)))
        .expect("number scanner is observed before conversion output");
    let recovery = recorder
        .0
        .iter()
        .position(|record| matches!(record, CommandObservation::Input(input) if input.transition == InputTransition::Recovery))
        .expect("conversion output creates a recovery input level");
    let inserted = recorder
        .0
        .iter()
        .position(|record| matches!(record, CommandObservation::Recovery(recovery)
            if recovery.kind == RecoveryKind::InsertedToken
                && matches!(recovery.tokens.as_slice(), [crate::ObservedToken::Character { character: '-', catcode: Catcode::Other }, ..])))
        .expect("conversion output reports its inserted minus token");
    let raw = recorder
        .0
        .iter()
        .enumerate()
        .skip(recovery + 1)
        .position(|(_, record)| matches!(record, CommandObservation::Command(command) if command.boundary == CommandDeliveryBoundary::Raw && matches!(command.spelling, crate::ObservedToken::Character { character: '-', catcode: Catcode::Other })))
        .map(|offset| recovery + 1 + offset)
        .expect("rendered minus returns through raw delivery");
    assert!(scanner < recovery && recovery < inserted && inserted < raw);
}

#[test]
fn string_reads_its_target_with_normal_scanner_status_then_restores_definition() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let string = install_expandable(&mut universe, "string", ExpandablePrimitive::String);
    let target = install_macro(
        &mut universe,
        "constructedname",
        Token::Char {
            ch: 'X',
            cat: Catcode::Letter,
        },
    );
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(string)),
            traced(Token::Cs(target)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let _prior = command.begin_scanner_status(ScannerStatus::Defining(DefinitionContext {
        target: None,
        builder: TokenBuilderId(1),
        warning: ScannerWarning(1),
    }));
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        for expected in "\\constructedname".chars() {
            let command = processor
                .get_x_token()
                .expect("string conversion expands")
                .expect("string conversion produces a character");
            assert!(
                matches!(command.spelling().semantic_token(), Token::Char { ch, .. } if ch == expected)
            );
        }
    }

    let status_exit = recorder
        .0
        .iter()
        .position(|record| matches!(record, CommandObservation::ScannerStatus(status) if status.from == "defining" && status.to == "normal"))
        .expect("string leaves defining status before its target");
    let target_delivery = recorder
        .0
        .iter()
        .position(|record| matches!(record, CommandObservation::Command(command) if command.boundary == CommandDeliveryBoundary::Raw && matches!(command.spelling, crate::ObservedToken::ControlSequence(ref name) if name == "constructedname")))
        .expect("string target is delivered raw");
    let status_restore = recorder
        .0
        .iter()
        .rposition(|record| matches!(record, CommandObservation::ScannerStatus(status) if status.from == "normal" && status.to == "defining"))
        .expect("string restores defining status after its target");
    let recovery = recorder
        .0
        .iter()
        .position(|record| matches!(record, CommandObservation::Input(input) if input.transition == InputTransition::Recovery))
        .expect("string conversion installs its inserted output");
    assert!(status_exit < target_delivery);
    assert!(target_delivery < status_restore);
    assert!(status_restore < recovery);
    assert!(matches!(
        command.scanner.status(),
        ScannerStatus::Defining(_)
    ));
}

#[test]
fn the_toks_pushes_immutable_stored_input_without_reading_beyond_target() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let the = install_expandable(&mut universe, "the", ExpandablePrimitive::The);
    let register = universe.intern("stored").symbol();
    universe.set_meaning(register, Meaning::ToksRegister(7));
    let stored = universe.intern_token_list(&[Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    }]);
    universe.set_toks(7, stored);
    let trailing = Token::Char {
        ch: 'z',
        cat: Catcode::Letter,
    };
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(the)),
            traced(Token::Cs(register)),
            traced(trailing),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    let opener = processor.get_next().expect("raw the").expect("the command");
    processor.expand(&opener).expect("the inserts stored list");
    assert!(
        matches!(processor.command.input.levels.last(), Some(crate::input::InputLevel::Tokens(cursor))
        if matches!(&cursor.payload, TokenPayload::Packed(chunk)
            if chunk.word(0).map(|word| word.semantic_token()) == Some(Token::Char { ch: 'x', cat: Catcode::Letter })))
    );
    // TeX82 §467 hands §465's copy to `ins_list`, so the level carries
    // §307's `inserted` token type and retires as a recovery, never as an
    // ordinary stored token list.
    assert!(
        matches!(processor.command.input.levels.last(), Some(crate::input::InputLevel::Tokens(cursor))
        if cursor.trace == ReplayTrace::Inserted && cursor.behavior == TokenBehavior::Recovery)
    );
    assert_eq!(
        processor
            .get_x_token()
            .expect("stored token")
            .expect("x")
            .spelling()
            .semantic_token(),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter
        }
    );
    assert_eq!(
        processor
            .get_x_token()
            .expect("trailing token")
            .expect("z")
            .spelling()
            .semantic_token(),
        trailing
    );
}

#[test]
fn the_renders_dimensions_glue_orders_and_mu_units_exactly() {
    use tex_state::glue::{GlueSpec, Order};
    use tex_state::scaled::Scaled;

    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let the = install_expandable(&mut universe, "the", ExpandablePrimitive::The);

    let dimen = universe.intern("testdimen").symbol();
    universe.set_meaning(dimen, Meaning::DimenRegister(0));
    universe.set_dimen(0, Scaled::from_raw(Scaled::UNITY + Scaled::UNITY / 2));

    let skip = universe.intern("testskip").symbol();
    universe.set_meaning(skip, Meaning::SkipRegister(0));
    let skip_value = universe.intern_glue(GlueSpec {
        width: Scaled::from_raw(2 * Scaled::UNITY),
        stretch: Scaled::from_raw(3 * Scaled::UNITY),
        stretch_order: Order::Fill,
        shrink: Scaled::from_raw(4 * Scaled::UNITY),
        shrink_order: Order::Normal,
    });
    universe.set_skip(0, skip_value);

    let muskip = universe.intern("testmuskip").symbol();
    universe.set_meaning(muskip, Meaning::MuskipRegister(0));
    let muskip_value = universe.intern_glue(GlueSpec {
        width: Scaled::from_raw(Scaled::UNITY),
        stretch: Scaled::from_raw(2 * Scaled::UNITY),
        stretch_order: Order::Fill,
        ..GlueSpec::ZERO
    });
    universe.set_muskip(0, muskip_value);

    let register = universe.intern("testtoks").symbol();
    universe.set_meaning(register, Meaning::ToksRegister(0));
    let copied_macro = install_macro(
        &mut universe,
        "copiedmacro",
        Token::Char {
            ch: 'Q',
            cat: Catcode::Letter,
        },
    );
    let stored = universe.intern_token_list(&[Token::Cs(copied_macro)]);
    universe.set_toks(0, stored);

    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(the)),
            traced(Token::Cs(dimen)),
            traced(Token::Cs(the)),
            traced(Token::Cs(skip)),
            traced(Token::Cs(the)),
            traced(Token::Cs(muskip)),
            traced(Token::Cs(the)),
            traced(Token::Cs(register)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    let expected = "1.5pt2.0pt plus 3.0fill minus 4.0pt1.0mu plus 2.0fill";
    let mut rendered = String::new();
    for _ in expected.chars() {
        let delivery = processor
            .get_x_token()
            .expect("the scalar value expands")
            .expect("rendered scalar token");
        let Token::Char { ch, .. } = delivery.spelling().semantic_token() else {
            panic!("the scalar rendering must contain only character tokens")
        };
        rendered.push(ch);
    }
    assert_eq!(rendered, expected);

    // Stop at the direct splice boundary: ordinary expanded delivery would
    // expand this macro on its next step, but §466 copies its token verbatim.
    let opener = processor.get_next().expect("raw the").expect("the command");
    processor.expand(&opener).expect("the inserts stored list");
    assert_eq!(processor.state.tokens(stored), &[Token::Cs(copied_macro)]);
    assert_eq!(
        processor
            .get_next()
            .expect("copied token is delivered raw")
            .expect("stored macro token")
            .spelling()
            .semantic_token(),
        Token::Cs(copied_macro)
    );
}

/// TeX82 §467's `ins_the_toks` is observed exactly like §470's `conv_toks`.
///
/// Both reach the input stack through §323's `ins_list`, so `\the` of a
/// token parameter must publish the same inserted push and the same
/// first-token recovery record that a rendered conversion does -- and a
/// leading control sequence is §289's `info(p)>=cs_token_flag` case.
#[test]
fn the_toks_publishes_an_inserted_push_naming_its_leading_control_sequence() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let the = install_expandable(&mut universe, "the", ExpandablePrimitive::The);
    // A token *parameter*, not a register: §466 copies both the same way,
    // and the divergence this test pins was `\the\headline`.
    let parameter = universe.intern("everypar").symbol();
    universe.set_meaning(parameter, Meaning::TokParam(1));
    let leading = universe.intern("hfil").symbol();
    let stored = universe.intern_token_list(&[Token::Cs(leading)]);
    universe.set_tok_param(tex_state::env::banks::TokParam::EVERY_PAR, stored);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(the)),
            traced(Token::Cs(parameter)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        let opener = processor.get_next().expect("raw the").expect("the command");
        processor.expand(&opener).expect("the inserts its copy");
    }
    let push = recorder
        .0
        .iter()
        .position(|record| {
            matches!(record, CommandObservation::Input(input)
                if input.transition == InputTransition::Recovery
                    && input.reason == crate::observation::InputReason::Recovery)
        })
        .expect("the_toks installs an observed inserted level");
    assert!(matches!(
        &recorder.0[push + 1],
        CommandObservation::Recovery(recovery)
            if recovery.kind == RecoveryKind::InsertedControlSequence
                && recovery.tokens
                    == vec![crate::observation::ObservedToken::ControlSequence("hfil".into())]
    ));
}

#[test]
fn ordinary_loop_expands_macro_body_on_the_canonical_raw_path() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
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
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

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
fn next_non_blank_x_token_expands_across_levels_and_preserves_the_stopping_delivery() {
    // TeX82 §§406/1045 require `get_x_token`, not raw delivery: spacer
    // commands produced by a macro are skipped even after its replacement
    // level retires. The first non-spacer remains the exact source-attributed
    // delivery that stopped the loop; it is neither backed up nor rebuilt.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let macro_name = universe.intern("spaces").symbol();
    let empty = universe.intern_token_list(&[]);
    let replacement = universe.intern_token_list(&[
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
    ]);
    let definition =
        universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, replacement));
    universe.set_meaning(
        macro_name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: definition.id(),
        },
    );
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"\\spaces X".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    let delivered = processor
        .next_non_blank_x_token()
        .expect("expanded scan succeeds")
        .expect("source character remains");
    assert_eq!(
        delivered.spelling().semantic_token(),
        Token::Char {
            ch: 'X',
            cat: Catcode::Letter,
        }
    );
    assert!(
        delivered.source_location().is_some(),
        "the stopping source token retains its physical provenance"
    );
    assert_eq!(processor.command.expansion.cumulative_expansions, 1);
}

#[test]
fn next_non_blank_x_token_does_not_skip_relax() {
    // §406 differs deliberately from §404: only spacer commands are skipped.
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let relax = universe.intern("relax").symbol();
    universe.set_meaning(relax, Meaning::Relax);
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"  \\relax X".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    let delivered = processor
        .next_non_blank_x_token()
        .expect("expanded scan succeeds")
        .expect("relax remains");
    assert_eq!(delivered.meaning(), Meaning::Relax);
    assert_eq!(delivered.spelling().semantic_token(), Token::Cs(relax));
}

#[test]
fn completed_expansion_rolls_back_to_the_exact_scalar_input_state() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
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
    let expected = command.clone();
    let snapshot = command.snapshot();
    let mut capabilities = CommandHostCapabilities::default();

    let first = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("macro expands")
            .expect("body token")
            .spelling()
            .semantic_token()
    };
    command.rollback(snapshot).expect("rollback succeeds");
    assert_eq!(command, expected);

    let replayed = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("rolled-back macro expands")
            .expect("replayed body token")
            .spelling()
            .semantic_token()
    };
    assert_eq!(replayed, first);
}

fn command_and_diagnostic_observations(records: &[CommandObservation]) -> Vec<CommandObservation> {
    records
        .iter()
        .filter(|record| {
            matches!(
                record,
                CommandObservation::Command(command)
                    if matches!(command.command.as_str(), "undefined_cs" | "letter")
            ) || matches!(record, CommandObservation::Diagnostic(_))
        })
        .cloned()
        .collect()
}

/// TeX82 §§365/370: a raw fetch with `no_new_control_sequence` frozen maps an
/// unknown multiletter name to §222's dummy `undefined_control_sequence`.
/// Since §207 puts `undefined_cs` above `max_command`, §380 reports it through
/// §370, substitutes nothing, and resumes the same loop at the following
/// source token exactly once.
#[test]
fn frozen_undefined_control_sequence_reports_then_resumes_source_once() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"\\never A".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        let resumed = processor
            .get_x_token()
            .expect("undefined recovery is finite")
            .expect("following source token resumes");
        assert_eq!(
            resumed.spelling().semantic_token(),
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter
            }
        );
        while let Some(command) = processor.get_x_token().expect("source retires") {
            assert_ne!(
                command.spelling().semantic_token(),
                Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter
                },
                "the following source token is delivered only once"
            );
        }
    }
    assert!(universe.symbol("never").is_none());
    assert!(matches!(
        command.take_semantic_diagnostics().as_slice(),
        [crate::CommandSemanticDiagnostic::UndefinedControlSequence { .. }]
    ));
    assert!(command.take_semantic_diagnostics().is_empty());

    let records = command_and_diagnostic_observations(&recorder.0);
    assert!(matches!(
        records.as_slice(),
        [
            CommandObservation::Command(raw_undefined),
            CommandObservation::Diagnostic(diagnostic),
            CommandObservation::Command(raw_a),
            CommandObservation::Command(expanded_a),
        ] if raw_undefined.boundary == CommandDeliveryBoundary::Raw
            && raw_undefined.command == "undefined_cs"
            && raw_undefined.spelling == ObservedToken::ControlSequence("^^@".into())
            && diagnostic.diagnostic == "undefined_control_sequence"
            && diagnostic.arguments
                == [DiagnosticArgument::Token(ObservedToken::ControlSequence("^^@".into()))]
            && raw_a.boundary == CommandDeliveryBoundary::Raw
            && raw_a.command == "letter"
            && expanded_a.boundary == CommandDeliveryBoundary::Expanded
            && expanded_a.command == "letter"
    ));
}

/// TeX82 §§370/380 still report and discard `undefined_cs` under the e-TeX
/// profile, but the pinned e-TeX 2.6 observer has no diagnostic seam at §370.
/// Its detached stream therefore retires an exhausted macro immediately after
/// the raw undefined command while the command-owned semantic diagnostic
/// remains available to the executor.
#[test]
fn etex_undefined_recovery_retires_macro_without_observer_diagnostic() {
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"A".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let undefined = universe.intern("undefined").symbol();
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(undefined))])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::MacroReplacement,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        let resumed = processor
            .get_x_token()
            .expect("undefined recovery is finite")
            .expect("enclosing source resumes");
        assert_eq!(
            resumed.spelling().semantic_token(),
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter
            }
        );
    }
    assert!(matches!(
        command.take_semantic_diagnostics().as_slice(),
        [crate::CommandSemanticDiagnostic::UndefinedControlSequence { .. }]
    ));
    assert!(matches!(
        command_and_diagnostic_observations(&recorder.0).as_slice(),
        [
            CommandObservation::Command(raw_undefined),
            CommandObservation::Command(raw_a),
            CommandObservation::Command(expanded_a),
        ] if raw_undefined.boundary == CommandDeliveryBoundary::Raw
            && raw_undefined.command == "undefined_cs"
            && raw_a.boundary == CommandDeliveryBoundary::Raw
            && raw_a.command == "letter"
            && expanded_a.boundary == CommandDeliveryBoundary::Expanded
            && expanded_a.command == "letter"
    ));
    let undefined_position = recorder
        .0
        .iter()
        .position(|record| {
            matches!(
                record,
                CommandObservation::Command(command) if command.command == "undefined_cs"
            )
        })
        .expect("raw undefined command observed");
    assert!(matches!(
        recorder.0.get(undefined_position + 1),
        Some(CommandObservation::Input(record))
            if record.transition == crate::observation::InputTransition::Retire
                && record.reason == crate::observation::InputReason::Macro
    ));
}

/// TeX82 §§366/370/380 make `x_token` expand, report, and discard
/// `undefined_cs` before returning the following unexpandable command. The
/// main-control preflight seam starts from an already raw-delivered command,
/// so it must retain that exact rule rather than returning the undefined
/// command as an expanded delivery to the executor.
#[test]
fn preflight_settlement_discards_undefined_before_returning_following_command() {
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"A".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let undefined = universe.intern("undefined").symbol();
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(undefined))])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::MacroReplacement,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        let raw_undefined = match processor
            .get_next_with_replay_completion()
            .expect("raw preflight delivery succeeds")
            .expect("raw preflight command")
        {
            crate::CommandReplayDelivery::Command(command) => command,
            crate::CommandReplayDelivery::Completed(_) => {
                panic!("raw preflight must deliver the undefined command")
            }
        };
        assert_eq!(raw_undefined.meaning(), Meaning::Undefined);

        let settled = processor
            .settle_current_command(raw_undefined)
            .expect("undefined recovery is finite")
            .expect("following source token resumes");
        assert_eq!(
            settled.spelling().semantic_token(),
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter
            }
        );
    }
    assert!(matches!(
        command.take_semantic_diagnostics().as_slice(),
        [crate::CommandSemanticDiagnostic::UndefinedControlSequence { .. }]
    ));
    assert!(
        !recorder.0.iter().any(|observation| matches!(
            observation,
            CommandObservation::Command(record)
                if record.boundary == CommandDeliveryBoundary::Expanded
                    && record.command == "undefined_cs"
        )),
        "§380 does not return undefined_cs at the expanded boundary"
    );
    let undefined_position = recorder
        .0
        .iter()
        .position(|record| {
            matches!(
                record,
                CommandObservation::Command(command)
                    if command.boundary == CommandDeliveryBoundary::Raw
                        && command.command == "undefined_cs"
            )
        })
        .expect("raw undefined command observed");
    assert!(matches!(
        recorder.0.get(undefined_position + 1),
        Some(CommandObservation::Input(record))
            if record.transition == crate::observation::InputTransition::Retire
                && record.reason == crate::observation::InputReason::Macro
    ));
}

/// TeX82 §379's `\noexpand` is the negative control: its one-shot frozen
/// relax has the spelling of an undefined control sequence but is below
/// `max_command`, so preflight settlement must return it without §370's
/// diagnostic.
#[test]
fn preflight_settlement_preserves_noexpanded_undefined_as_frozen_relax() {
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let noexpand = universe.intern("noexpand").symbol();
    universe.set_meaning(
        noexpand,
        Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand),
    );
    let undefined = universe.intern("undefined").symbol();
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(noexpand)),
            traced(Token::Cs(undefined)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    let raw_noexpand = match processor
        .get_next_with_replay_completion()
        .expect("raw preflight delivery succeeds")
        .expect("raw noexpand command")
    {
        crate::CommandReplayDelivery::Command(command) => command,
        crate::CommandReplayDelivery::Completed(_) => {
            panic!("raw preflight must deliver noexpand")
        }
    };
    let settled = processor
        .settle_current_command(raw_noexpand)
        .expect("noexpand settlement succeeds")
        .expect("frozen relax delivery");

    assert_eq!(settled.meaning(), Meaning::Relax);
    assert_eq!(settled.spelling().semantic_token(), Token::Cs(undefined));
    assert_eq!(
        settled.identity(),
        crate::command::CommandIdentity::NoExpandFrozenRelax
    );
    assert!(command.take_semantic_diagnostics().is_empty());
}

/// Bounded source fixture for the e-TeX 2.6 §370 observer boundary. The raw
/// undefined command is visible, recovery remains semantic, and detached
/// observation resumes at the following expanded token with no diagnostic
/// record inserted between them.
#[test]
fn etex_undefined_semantic_microfixture_omits_observer_diagnostic() {
    let mut command = CommandState::new(crate::CommandProfile::ETEX26);
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(
                include_bytes!("../fixtures/etex-undefined-expansion.tex").as_slice(),
            ),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        let resumed = processor
            .get_x_token()
            .expect("undefined recovery is finite")
            .expect("fixture resumes");
        assert_eq!(
            resumed.spelling().semantic_token(),
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter
            }
        );
    }
    assert!(matches!(
        command.take_semantic_diagnostics().as_slice(),
        [crate::CommandSemanticDiagnostic::UndefinedControlSequence { .. }]
    ));
    assert!(matches!(
        command_and_diagnostic_observations(&recorder.0).as_slice(),
        [
            CommandObservation::Command(raw_undefined),
            CommandObservation::Command(raw_a),
            CommandObservation::Command(expanded_a),
        ] if raw_undefined.boundary == CommandDeliveryBoundary::Raw
            && raw_undefined.command == "undefined_cs"
            && raw_a.boundary == CommandDeliveryBoundary::Raw
            && raw_a.command == "letter"
            && expanded_a.boundary == CommandDeliveryBoundary::Expanded
            && expanded_a.command == "letter"
    ));
}

#[test]
fn undefined_semantic_diagnostic_survives_unobserved_execution_and_snapshot_retry() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"\\undefined A".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let snapshot = command.snapshot();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let mut capabilities = CommandHostCapabilities::default();

    let run = |command: &mut CommandState,
               universe: &mut Universe,
               capabilities: &mut CommandHostCapabilities| {
        {
            let mut processor = processor(command, universe, capabilities);
            let resumed = processor
                .get_x_token()
                .expect("undefined recovery is finite")
                .expect("following token resumes");
            assert_eq!(
                resumed.spelling().semantic_token(),
                Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter,
                }
            );
        }
        command.take_semantic_diagnostics()
    };

    assert!(matches!(
        run(&mut command, &mut universe, &mut capabilities).as_slice(),
        [crate::CommandSemanticDiagnostic::UndefinedControlSequence { .. }]
    ));
    assert!(command.take_semantic_diagnostics().is_empty());

    command.rollback(snapshot).expect("rollback succeeds");
    assert!(
        matches!(
            run(&mut command, &mut universe, &mut capabilities).as_slice(),
            [crate::CommandSemanticDiagnostic::UndefinedControlSequence { .. }]
        ),
        "rollback replays the command-owned semantic diagnostic exactly once"
    );
}

/// TeX82 §§370/380 are independent of whether `undefined_cs` came from the
/// frozen dummy or an already interned hash entry. The command snapshot owns
/// the entire recovery episode: retry must reproduce the diagnostic ordering,
/// enclosing-input retirement, and following delivery byte-for-byte.
#[test]
fn interned_undefined_recovery_and_enclosing_resume_replay_after_rollback() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"A".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let undefined = universe.intern("undefined").symbol();
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(undefined))])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let expected = command.clone();
    let snapshot = command.snapshot();
    let mut capabilities = CommandHostCapabilities::default();

    let run = |command: &mut CommandState,
               universe: &mut Universe,
               capabilities: &mut CommandHostCapabilities| {
        let mut recorder = Recorder::default();
        {
            let mut processor =
                processor(command, universe, capabilities).with_observer(&mut recorder);
            let resumed = processor
                .get_x_token()
                .expect("undefined recovery is finite")
                .expect("enclosing source resumes");
            assert_eq!(
                resumed.spelling().semantic_token(),
                Token::Char {
                    ch: 'A',
                    cat: Catcode::Letter
                }
            );
            while let Some(command) = processor.get_x_token().expect("source retires") {
                assert_ne!(
                    command.spelling().semantic_token(),
                    Token::Char {
                        ch: 'A',
                        cat: Catcode::Letter
                    },
                    "the enclosing source token is delivered only once"
                );
            }
        }
        recorder.0
    };

    let first = run(&mut command, &mut universe, &mut capabilities);
    command.rollback(snapshot).expect("rollback succeeds");
    assert_eq!(command, expected);
    let replayed = run(&mut command, &mut universe, &mut capabilities);
    assert_eq!(replayed, first);

    let records = command_and_diagnostic_observations(&first);
    assert!(matches!(
        records.as_slice(),
        [
            CommandObservation::Command(raw_undefined),
            CommandObservation::Diagnostic(diagnostic),
            CommandObservation::Command(raw_a),
            CommandObservation::Command(expanded_a),
        ] if raw_undefined.boundary == CommandDeliveryBoundary::Raw
            && raw_undefined.command == "undefined_cs"
            && raw_undefined.spelling
                == ObservedToken::ControlSequence("undefined".into())
            && diagnostic.diagnostic == "undefined_control_sequence"
            && diagnostic.arguments == [
                DiagnosticArgument::Token(ObservedToken::ControlSequence("undefined".into()))
            ]
            && raw_a.boundary == CommandDeliveryBoundary::Raw
            && raw_a.command == "letter"
            && expanded_a.boundary == CommandDeliveryBoundary::Expanded
            && expanded_a.command == "letter"
    ));
}

#[test]
fn noexpand_suppresses_one_macro_delivery_without_changing_its_spelling() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
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
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    let delivered = processor
        .get_x_token()
        .expect("noexpand completes")
        .expect("target");
    assert_eq!(delivered.spelling().semantic_token(), Token::Cs(macro_name));
    assert_eq!(delivered.meaning(), Meaning::Relax);
    assert_eq!(
        delivered.identity(),
        crate::command::CommandIdentity::NoExpandFrozenRelax
    );
    assert_eq!(
        processor.observed_command_spelling(&delivered),
        crate::observation::ObservedToken::ControlSequence("m".into())
    );
    assert_eq!(processor.command.expansion.cumulative_expansions, 1);
}

/// TeX82 §379 tests `cur_cmd > max_command`, not merely whether a meaning
/// names an expandable primitive or macro. The `undefined_cs` command is in
/// that range too, so `\noexpand` must replay a newly entered undefined name
/// as the one-shot `relax`/`no_expand_flag` command.
#[test]
fn noexpand_suppresses_an_undefined_control_sequence() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let noexpand = universe.intern("noexpand").symbol();
    universe.set_meaning(
        noexpand,
        Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand),
    );
    let undefined = universe.intern("undefined").symbol();
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(noexpand)),
            traced(Token::Cs(undefined)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    let delivered = processor
        .get_x_token()
        .expect("noexpand completes")
        .expect("undefined target");
    assert_eq!(delivered.spelling().semantic_token(), Token::Cs(undefined));
    assert_eq!(delivered.meaning(), Meaning::Relax);
    assert_eq!(
        delivered.identity(),
        crate::command::CommandIdentity::NoExpandFrozenRelax
    );
    assert_eq!(
        processor.observed_command_spelling(&delivered),
        crate::observation::ObservedToken::ControlSequence("undefined".into())
    );
}

/// TeX82 §15 assigns `end_cs_name` a command code below `max_command`, so
/// §25's one-shot suppression marker must preserve the collector boundary.
/// §372 can then finish the name even when `\noexpand` immediately precedes
/// `\endcsname`.
#[test]
fn noexpand_preserves_endcsname_as_the_csname_collector_boundary() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let csname = install_expandable(&mut universe, "csname", ExpandablePrimitive::CsName);
    let noexpand = install_expandable(&mut universe, "noexpand", ExpandablePrimitive::NoExpand);
    let endcsname = install_expandable(&mut universe, "endcsname", ExpandablePrimitive::EndCsName);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(csname)),
            traced(Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            }),
            traced(Token::Cs(noexpand)),
            traced(Token::Cs(endcsname)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let constructed = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("csname expansion completes")
            .expect("constructed control sequence")
    };

    let Token::Cs(symbol) = constructed.spelling().semantic_token() else {
        panic!("csname must inject a control sequence");
    };
    assert_eq!(universe.resolve(symbol), "a");
    assert_eq!(universe.meaning(symbol), Meaning::Relax);
    assert!(command.expansion.pending_diagnostics.is_empty());
}

#[test]
fn expandafter_expands_second_token_before_replaying_first() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
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
    let mut recorder = Recorder::default();
    let (first, second) = {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);

        let first = processor
            .get_x_token()
            .expect("expandafter completes")
            .expect("first token");
        let second = processor
            .get_x_token()
            .expect("macro body follows")
            .expect("body token");
        assert_eq!(processor.command.expansion.cumulative_expansions, 2);
        (first, second)
    };
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
    assert!(recorder.0.iter().any(|observation| {
        matches!(
            observation,
            CommandObservation::Command(delivery)
                if delivery.boundary == CommandDeliveryBoundary::Raw
                    && delivery.command == "expand_after"
                    && delivery.command_operand == Some(0)
        )
    }));
}

#[test]
fn csname_expands_characters_then_injects_a_relaxed_named_control_sequence() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let csname = install_expandable(&mut universe, "csname", ExpandablePrimitive::CsName);
    let endcsname = install_expandable(&mut universe, "endcsname", ExpandablePrimitive::EndCsName);
    let macro_name = install_macro(
        &mut universe,
        "letter",
        Token::Char {
            ch: 'a',
            cat: Catcode::Other,
        },
    );
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(csname)),
            traced(Token::Cs(macro_name)),
            traced(Token::Char {
                ch: 'b',
                cat: Catcode::Letter,
            }),
            traced(Token::Cs(endcsname)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let delivered = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        processor
            .get_x_token()
            .expect("csname expands")
            .expect("constructed control sequence")
    };

    let Token::Cs(symbol) = delivered.spelling().semantic_token() else {
        panic!("csname must inject a control sequence");
    };
    assert_eq!(universe.meaning(symbol), Meaning::Relax);
    assert_eq!(
        universe.control_sequence_kind(symbol),
        tex_state::interner::ControlSequenceKind::Named
    );
    assert!(matches!(
        universe.origin(delivered.origin()),
        tex_state::provenance::OriginRecord::Synthesized(origin)
            if origin.kind() == SynthesizedOriginKind::Expansion
    ));
    assert_eq!(command.expansion.cumulative_expansions, 2);
}

#[test]
fn csname_empty_and_single_character_use_canonical_control_sequence_slots() {
    use tex_state::interner::ControlSequenceKind;

    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let csname = install_expandable(&mut universe, "csname", ExpandablePrimitive::CsName);
    let endcsname = install_expandable(&mut universe, "endcsname", ExpandablePrimitive::EndCsName);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(csname)),
            traced(Token::Cs(endcsname)),
            traced(Token::Cs(csname)),
            traced(Token::Char {
                ch: 'q',
                cat: Catcode::Other,
            }),
            traced(Token::Cs(endcsname)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let (null, single) = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        (
            processor
                .get_x_token()
                .expect("empty csname expands")
                .expect("null control sequence"),
            processor
                .get_x_token()
                .expect("single-character csname expands")
                .expect("single-character control sequence"),
        )
    };

    let Token::Cs(null) = null.spelling().semantic_token() else {
        panic!("empty csname must inject null_cs");
    };
    let Token::Cs(single) = single.spelling().semantic_token() else {
        panic!("single-character csname must inject a control sequence");
    };
    assert_eq!(universe.resolve(null), "");
    assert_eq!(
        universe.control_sequence_kind(null),
        ControlSequenceKind::Null
    );
    assert_eq!(universe.meaning(null), Meaning::Relax);
    assert_eq!(universe.resolve(single), "q");
    assert_eq!(
        universe.control_sequence_kind(single),
        ControlSequenceKind::SingleCharacter
    );
    assert_eq!(universe.meaning(single), Meaning::Relax);
}

#[test]
fn csname_recovers_by_backing_up_a_non_character_before_constructing_the_name() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let csname = install_expandable(&mut universe, "csname", ExpandablePrimitive::CsName);
    let endcsname = install_expandable(&mut universe, "endcsname", ExpandablePrimitive::EndCsName);
    let relax = universe.intern("r").symbol();
    universe.set_meaning(relax, Meaning::Relax);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(csname)),
            traced(Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            }),
            traced(Token::Cs(relax)),
            traced(Token::Cs(endcsname)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    let constructed = processor
        .get_x_token()
        .expect("csname recovery")
        .expect("constructed name");
    let replayed = processor
        .get_x_token()
        .expect("backed up token")
        .expect("relax");
    assert_eq!(constructed.meaning(), Meaning::Relax);
    assert_eq!(replayed.spelling().semantic_token(), Token::Cs(relax));
    assert_eq!(
        processor.command.expansion.pending_diagnostics,
        vec![MISSING_ENDCSNAME_DIAGNOSTIC]
    );
}

#[test]
fn endcsname_is_an_ordinary_loop_boundary_not_an_expandable_dispatch_error() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let endcsname = install_expandable(&mut universe, "endcsname", ExpandablePrimitive::EndCsName);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![traced(Token::Cs(endcsname))])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    let boundary = processor
        .get_x_token()
        .expect("boundary delivery")
        .expect("endcsname");
    assert_eq!(
        boundary.meaning(),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName)
    );
}

#[test]
fn macro_activations_allocate_nested_invocation_provenance() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let empty = universe.intern_token_list(&[]);
    let definition = universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, empty));
    let admitted = command.parameters.admit_macro(
        definition.id(),
        universe.macro_definition(definition.id()).meaning(),
    );
    let target = universe.intern("nested").symbol();
    let mut capabilities = CommandHostCapabilities::default();
    let outer_invocation;
    let inner_invocation;
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );
        processor.push_macro_activation(
            target,
            definition.id(),
            tex_state::token::OriginId::UNKNOWN,
            MacroArguments::default(),
            admitted,
        );
        outer_invocation = processor
            .command
            .parameters
            .activations
            .last()
            .expect("outer activation")
            .invocation;
        processor.push_macro_activation(
            target,
            definition.id(),
            tex_state::token::OriginId::UNKNOWN,
            MacroArguments::default(),
            admitted,
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

#[test]
fn meaning_reads_immutable_replacement_after_nested_macro_retirement() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let meaning = install_expandable(&mut universe, "meaning", ExpandablePrimitive::Meaning);
    let empty = universe.intern_token_list(&[]);
    let expanded = universe.intern_token_list(&letters("EXPANDED"));
    let definition = universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, expanded));
    let empty_definition =
        universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, empty));
    let admitted = command.parameters.admit_macro(
        empty_definition.id(),
        universe.macro_definition(empty_definition.id()).meaning(),
    );
    let target = universe.intern("getxresult").symbol();
    universe.set_meaning(
        target,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: definition.id(),
        },
    );
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(meaning)),
            traced(Token::Cs(target)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::Inserted,
    );
    command.push_macro_activation(
        target,
        empty_definition.id(),
        MacroArguments::default(),
        tex_state::token::OriginId::UNKNOWN,
        admitted,
        0,
    );
    command.push_macro_activation(
        target,
        empty_definition.id(),
        MacroArguments::default(),
        tex_state::token::OriginId::UNKNOWN,
        admitted,
        0,
    );

    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    assert_eq!(rendered(&mut processor), "macro:->EXPANDED");
    assert!(processor.command.parameters.activations.is_empty());
}

#[test]
fn meaning_separates_a_control_word_from_following_letters() {
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let leaf = universe.intern("leaf").symbol();
    let replacement = universe.intern_token_list(&[
        Token::Cs(leaf),
        Token::Char {
            ch: 'N',
            cat: Catcode::Letter,
        },
    ]);
    let empty = universe.intern_token_list(&[]);
    let definition =
        universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, replacement));
    let macro_name = universe.intern("result").symbol();
    universe.set_meaning(
        macro_name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: definition.id(),
        },
    );
    let command = {
        let mut state = universe.command_context();
        CurrentCommand::resolve(
            traced(Token::Cs(macro_name)),
            crate::command::DeliveryStamp::new(0, 0, 0),
            None,
            false,
            &mut state,
        )
    };

    assert_eq!(
        meaning_text(&mut universe.command_context(), &command),
        "macro:->\\leaf N"
    );
}

#[test]
fn meaning_renders_class_zero_mark_contents_but_not_etex_mark_classes() {
    // TeX82 §296 appends class-zero mark contents to the five singular mark
    // meanings. e-TeX change [20.296] excludes the plural class scanners.
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let contents = universe.intern_token_list(&letters("mark text"));
    for (index, (name, primitive, mark)) in [
        ("topmark", ExpandablePrimitive::TopMark, PageMark::Top),
        ("firstmark", ExpandablePrimitive::FirstMark, PageMark::First),
        ("botmark", ExpandablePrimitive::BotMark, PageMark::Bot),
        (
            "splitfirstmark",
            ExpandablePrimitive::SplitFirstMark,
            PageMark::SplitFirst,
        ),
        (
            "splitbotmark",
            ExpandablePrimitive::SplitBotMark,
            PageMark::SplitBot,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        universe.set_page_mark(
            mark,
            if index == 0 {
                TokenListId::EMPTY
            } else {
                contents
            },
        );
        let symbol = install_expandable(&mut universe, name, primitive);
        let command = {
            let mut state = universe.command_context();
            CurrentCommand::resolve(
                traced(Token::Cs(symbol)),
                crate::command::DeliveryStamp::new(0, 0, 0),
                None,
                false,
                &mut state,
            )
        };
        let suffix = if index == 0 { "" } else { "mark text" };
        assert_eq!(
            meaning_text(&mut universe.command_context(), &command),
            format!("\\{name}:{suffix}")
        );
    }

    let plural = install_expandable(&mut universe, "botmarks", ExpandablePrimitive::BotMarks);
    let plural = {
        let mut state = universe.command_context();
        CurrentCommand::resolve(
            traced(Token::Cs(plural)),
            crate::command::DeliveryStamp::new(0, 0, 0),
            None,
            false,
            &mut state,
        )
    };
    assert_eq!(
        meaning_text(&mut universe.command_context(), &plural),
        "\\botmarks"
    );
}

#[test]
fn meaning_renders_tex82_long_and_outer_macro_command_identity() {
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let empty = universe.intern_token_list(&[]);
    for (index, (flags, expected)) in [
        (MeaningFlags::EMPTY, "macro:->"),
        (MeaningFlags::LONG, "\\long macro:->"),
        (MeaningFlags::OUTER, "\\outer macro:->"),
        (
            MeaningFlags::LONG | MeaningFlags::OUTER,
            "\\long\\outer macro:->",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let definition = universe.intern_macro(MacroMeaning::new(flags, empty, empty));
        let macro_name = universe.intern(&format!("result{index}")).symbol();
        universe.set_meaning(
            macro_name,
            Meaning::Macro {
                flags,
                definition: definition.id(),
            },
        );
        let command = {
            let mut state = universe.command_context();
            CurrentCommand::resolve(
                traced(Token::Cs(macro_name)),
                crate::command::DeliveryStamp::new(0, 0, 0),
                None,
                false,
                &mut state,
            )
        };

        assert_eq!(
            meaning_text(&mut universe.command_context(), &command),
            expected
        );
    }
}

#[test]
fn end_template_alias_retains_outer_command_identity_and_meaning() {
    // TeX82 §§298, 336, 780: `frozen_end_template` has the inaccessible
    // `end_template` command code, which lies above `outer_call`. A `\let`
    // alias therefore remains outer and pseudoprints that command identity;
    // its user-authored control-sequence spelling must not replace it.
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let alias = universe.intern("endt").symbol();
    universe.set_meaning(
        alias,
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate),
    );
    let command = {
        let mut state = universe.command_context();
        CurrentCommand::resolve(
            traced(Token::Cs(alias)),
            crate::command::DeliveryStamp::new(0, 0, 0),
            None,
            false,
            &mut state,
        )
    };

    assert!(command.is_outer());
    assert_eq!(
        print_cmd_chr_text(
            &universe.command_context(),
            PrintCommand::from_current(&command),
        ),
        "\\outer endtemplate"
    );
    assert_eq!(
        meaning_text(&mut universe.command_context(), &command),
        "\\outer endtemplate:"
    );
}

#[test]
fn meaning_macro_prefixes_use_live_escape_character() {
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.set_int_param(IntParam::ESCAPE_CHAR, i32::from(b'|'));
    let empty = universe.intern_token_list(&[]);
    let flags = MeaningFlags::PROTECTED | MeaningFlags::LONG | MeaningFlags::OUTER;
    let definition = universe.intern_macro(MacroMeaning::new(flags, empty, empty));
    let macro_name = universe.intern("result").symbol();
    universe.set_meaning(
        macro_name,
        Meaning::Macro {
            flags,
            definition: definition.id(),
        },
    );
    let command = {
        let mut state = universe.command_context();
        CurrentCommand::resolve(
            traced(Token::Cs(macro_name)),
            crate::command::DeliveryStamp::new(0, 0, 0),
            None,
            false,
            &mut state,
        )
    };

    assert_eq!(
        meaning_text(&mut universe.command_context(), &command),
        "|protected|long|outer macro:->"
    );
}

#[test]
fn meaning_macro_token_list_distinguishes_words_symbols_spaces_and_active_chars() {
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let word = universe.intern("word").symbol();
    let symbol = universe.intern("!").symbol();
    let active = universe.intern_active_character('~').symbol();
    let empty = universe.intern_token_list(&[]);
    let replacement = universe.intern_token_list(&[
        Token::Cs(word),
        Token::Cs(symbol),
        Token::Cs(active),
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
    ]);
    let definition =
        universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, replacement));
    let macro_name = universe.intern("shown").symbol();
    universe.set_meaning(
        macro_name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: definition.id(),
        },
    );
    let command = {
        let mut state = universe.command_context();
        CurrentCommand::resolve(
            traced(Token::Cs(macro_name)),
            crate::command::DeliveryStamp::new(0, 0, 0),
            None,
            false,
            &mut state,
        )
    };

    assert_eq!(
        meaning_text(&mut universe.command_context(), &command),
        "macro:->\\word \\!~ "
    );
}

#[test]
fn print_cs_delimits_words_but_not_active_characters_or_control_symbols() {
    // TeX82 §§262–263: `print_cs` and `sprint_cs` share spelling, but only
    // `print_cs` appends a delimiter after a named control word. Meaning does
    // not affect that spelling partition.
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let primitive = universe.intern("relax").symbol();
    let macro_name = universe.intern("macro").symbol();
    let undefined = universe.intern("undefined").symbol();
    let active = universe.intern_active_character('~').symbol();
    let symbol = universe.intern("!").symbol();
    let null = universe.intern("").symbol();
    let empty = universe.intern_token_list(&[]);
    let definition = universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, empty));
    universe.set_meaning(primitive, Meaning::Relax);
    universe.set_meaning(
        macro_name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: definition.id(),
        },
    );

    for (symbol, expected) in [
        (primitive, "\\relax "),
        (macro_name, "\\macro "),
        (undefined, "\\undefined "),
        (active, "~"),
        (symbol, "\\!"),
        (null, "\\csname\\endcsname "),
    ] {
        assert_eq!(
            print_cs_text(&mut universe.command_context(), symbol),
            expected
        );
    }

    // TeX82 §262 tests the current catcode, not the character's Unicode
    // alphabetic property, when a direct-address control sequence is shown.
    universe.set_catcode('!', Catcode::Letter);
    assert_eq!(
        print_cs_text(&mut universe.command_context(), symbol),
        "\\! "
    );

    universe.set_int_param(IntParam::ESCAPE_CHAR, i32::from(b'!'));
    assert_eq!(
        print_cs_text(&mut universe.command_context(), null),
        "!csname!endcsname "
    );
}

#[test]
fn character_command_renderer_covers_tex82_print_cmd_chr_table() {
    for (cat, ch, expected) in [
        (Catcode::BeginGroup, '{', "begin-group character {"),
        (Catcode::EndGroup, '}', "end-group character }"),
        (Catcode::MathShift, '$', "math shift character $"),
        (Catcode::AlignmentTab, '&', "alignment tab character &"),
        (Catcode::EndLine, '\r', "\\crcr"),
        (Catcode::Parameter, '#', "macro parameter character #"),
        (Catcode::Superscript, '^', "superscript character ^"),
        (Catcode::Subscript, '_', "subscript character _"),
        (Catcode::Space, ' ', "blank space  "),
        (Catcode::Letter, 'a', "the letter a"),
        (Catcode::Other, '7', "the character 7"),
        (Catcode::Escape, '\\', "[uncommandable character \\]"),
        (Catcode::Ignored, '\0', "[uncommandable character ^^@]"),
        (Catcode::Active, '~', "[uncommandable character ~]"),
        (Catcode::Comment, '%', "[uncommandable character %]"),
        (Catcode::Invalid, '\u{7f}', "[uncommandable character ^^?]"),
    ] {
        assert_eq!(character_command_text(ch, cat), expected);
    }
}

#[test]
fn print_cmd_chr_preserves_delivered_command_operands_and_aliases() {
    use tex_state::font::{FontMetrics, LoadedFont};
    use tex_state::scaled::Scaled;

    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    universe.register_primitive_meaning(
        "advance",
        Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Advance),
    );
    let scaled_font = universe.intern_font(LoadedFont::new(
        "cmr10",
        "cmr10.tfm",
        [0; 32],
        0,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(12 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        FontMetrics::default(),
    ));
    let design_font = universe.intern_font(LoadedFont::new(
        "cmtt10",
        "cmtt10.tfm",
        [1; 32],
        0,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(10 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        FontMetrics::default(),
    ));

    for (index, meaning, expected) in [
        (0, Meaning::CharGiven('A'), "\\char\"41"),
        (1, Meaning::MathCharGiven(0x1234), "\\mathchar\"1234"),
        (2, Meaning::Font(scaled_font), "select font cmr10 at 12.0pt"),
        (3, Meaning::Font(design_font), "select font cmtt10"),
        (4, Meaning::EndV, "end of alignment template"),
        (
            5,
            Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Advance),
            "\\advance",
        ),
    ] {
        let alias = universe.intern(&format!("alias{index}")).symbol();
        universe.set_meaning(alias, meaning);
        let command = {
            let mut state = universe.command_context();
            CurrentCommand::resolve(
                traced(Token::Cs(alias)),
                crate::command::DeliveryStamp::new(0, index, 0),
                None,
                false,
                &mut state,
            )
        };
        assert_eq!(
            print_cmd_chr_text(
                &universe.command_context(),
                PrintCommand::from_current(&command),
            ),
            expected
        );
    }
}

#[test]
fn print_cmd_chr_renders_all_parameter_and_register_names() {
    use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
    use tex_state::font::{FontMetrics, LoadedFont};
    use tex_state::meaning::UnexpandablePrimitive as U;
    use tex_state::scaled::Scaled;

    // tex.web §§224, 230, and 298: named eqtb quantities print the inverse
    // primitive name, while register aliases print the register family and
    // numeric operand.  Keep the complete TeX82 parameter vocabulary here as
    // a direct table so two slots cannot silently acquire the same spelling.
    const INT_PARAMS: &[(&str, u16)] = &[
        ("pretolerance", 0),
        ("tolerance", 1),
        ("linepenalty", 2),
        ("hyphenpenalty", 3),
        ("exhyphenpenalty", 4),
        ("clubpenalty", 5),
        ("widowpenalty", 6),
        ("displaywidowpenalty", 7),
        ("brokenpenalty", 8),
        ("binoppenalty", 9),
        ("relpenalty", 10),
        ("predisplaypenalty", 11),
        ("postdisplaypenalty", 12),
        ("interlinepenalty", 13),
        ("doublehyphendemerits", 14),
        ("finalhyphendemerits", 15),
        ("adjdemerits", 16),
        ("mag", 17),
        ("delimiterfactor", 18),
        ("looseness", 19),
        ("time", 20),
        ("day", 21),
        ("month", 22),
        ("year", 23),
        ("showboxbreadth", 24),
        ("showboxdepth", 25),
        ("hbadness", 26),
        ("vbadness", 27),
        ("pausing", 28),
        ("tracingonline", 29),
        ("tracingmacros", 30),
        ("tracingstats", 31),
        ("globaldefs", 32),
        ("tracingparagraphs", 33),
        ("tracingpages", 34),
        ("tracingoutput", 35),
        ("tracinglostchars", 36),
        ("tracingcommands", 37),
        ("tracingrestores", 38),
        ("uchyph", 39),
        ("escapechar", 40),
        ("defaulthyphenchar", 41),
        ("defaultskewchar", 42),
        ("endlinechar", 48),
        ("newlinechar", 49),
        ("language", 50),
        ("lefthyphenmin", 51),
        ("righthyphenmin", 52),
        ("holdinginserts", 53),
        ("errorcontextlines", 54),
        ("outputpenalty", 55),
        ("maxdeadcycles", 56),
        ("hangafter", 57),
        ("floatingpenalty", 58),
        ("fam", 59),
    ];
    const ETEX_INT_PARAMS: &[(&str, u16)] = &[
        ("tracingscantokens", 61),
        ("TeXXeTstate", 62),
        ("predisplaydirection", 63),
        ("tracingassigns", 64),
        ("tracinggroups", 65),
        ("tracingifs", 66),
        ("tracingnesting", 67),
        ("savingvdiscards", 68),
        ("lastlinefit", 69),
        ("savinghyphcodes", 70),
    ];
    const DIMEN_PARAMS: &[(&str, u16)] = &[
        ("parindent", 0),
        ("mathsurround", 1),
        ("lineskiplimit", 2),
        ("hsize", 3),
        ("vsize", 4),
        ("maxdepth", 5),
        ("splitmaxdepth", 6),
        ("boxmaxdepth", 7),
        ("hfuzz", 8),
        ("vfuzz", 9),
        ("delimitershortfall", 10),
        ("nulldelimiterspace", 11),
        ("scriptspace", 12),
        ("predisplaysize", 13),
        ("displaywidth", 14),
        ("displayindent", 15),
        ("overfullrule", 16),
        ("hangindent", 17),
        ("hoffset", 18),
        ("voffset", 19),
        ("emergencystretch", 20),
    ];
    const GLUE_PARAMS: &[(&str, u16)] = &[
        ("lineskip", 0),
        ("baselineskip", 1),
        ("parskip", 2),
        ("abovedisplayskip", 3),
        ("belowdisplayskip", 4),
        ("abovedisplayshortskip", 5),
        ("belowdisplayshortskip", 6),
        ("leftskip", 7),
        ("rightskip", 8),
        ("topskip", 9),
        ("splittopskip", 10),
        ("tabskip", 11),
        ("spaceskip", 12),
        ("xspaceskip", 13),
        ("parfillskip", 14),
    ];
    const MU_GLUE_PARAMS: &[(&str, u16)] =
        &[("thinmuskip", 15), ("medmuskip", 16), ("thickmuskip", 17)];
    const TOK_PARAMS: &[(&str, u16)] = &[
        ("output", 0),
        ("everypar", 1),
        ("everymath", 2),
        ("everydisplay", 3),
        ("everyhbox", 4),
        ("everyvbox", 5),
        ("everyjob", 6),
        ("everycr", 7),
        ("errhelp", 8),
    ];
    const ETEX_TOK_PARAMS: &[(&str, u16)] = &[("everyeof", 13)];

    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let initial_escape = universe.int_param(IntParam::ESCAPE_CHAR);
    let initial_hsize = universe.dimen_param(DimenParam::H_SIZE);
    let initial_baseline = universe.glue_param(GlueParam::BASELINE_SKIP);
    let initial_every_par = universe.tok_param(TokParam::EVERY_PAR);

    let mut assert_named = |name: &str, meaning: Meaning| {
        universe.register_primitive_meaning(name, meaning);
        let canonical = universe.intern(name).symbol();
        let alias = universe.intern(&format!("alias-{name}")).symbol();
        universe.set_meaning(canonical, meaning);
        universe.set_meaning(alias, meaning);
        for symbol in [canonical, alias] {
            let command = {
                let mut state = universe.command_context();
                CurrentCommand::resolve(
                    traced(Token::Cs(symbol)),
                    crate::command::DeliveryStamp::new(0, 0, 0),
                    None,
                    false,
                    &mut state,
                )
            };
            let expected = format!("\\{name}");
            assert_eq!(
                print_cmd_chr_text(
                    &universe.command_context(),
                    PrintCommand::from_current(&command)
                ),
                expected,
                "print_cmd_chr name for {name}",
            );
            assert_eq!(
                meaning_text(&mut universe.command_context(), &command),
                expected
            );
        }
    };

    for &(name, slot) in INT_PARAMS.iter().chain(ETEX_INT_PARAMS) {
        assert_named(name, Meaning::IntParam(slot));
    }
    for &(name, slot) in DIMEN_PARAMS {
        assert_named(name, Meaning::DimenParam(slot));
    }
    for &(name, slot) in GLUE_PARAMS {
        assert_named(name, Meaning::GlueParam(slot));
    }
    for &(name, slot) in MU_GLUE_PARAMS {
        assert_named(name, Meaning::MuGlueParam(slot));
    }
    for &(name, slot) in TOK_PARAMS {
        assert_named(name, Meaning::TokParam(slot));
    }
    for &(name, slot) in ETEX_TOK_PARAMS {
        assert_named(name, Meaning::TokParam(slot));
    }
    for (family, make) in [
        ("count", Meaning::CountRegister as fn(u16) -> Meaning),
        ("dimen", Meaning::DimenRegister),
        ("skip", Meaning::SkipRegister),
        ("muskip", Meaning::MuskipRegister),
        ("toks", Meaning::ToksRegister),
    ] {
        for index in [0, 255, 256, 32_767] {
            let alias = universe.intern(&format!("alias-{family}-{index}")).symbol();
            universe.set_meaning(alias, make(index));
            let command = {
                let mut state = universe.command_context();
                CurrentCommand::resolve(
                    traced(Token::Cs(alias)),
                    crate::command::DeliveryStamp::new(0, u64::from(index), 0),
                    None,
                    false,
                    &mut state,
                )
            };
            let expected = format!("\\{family}{index}");
            assert_eq!(
                print_cmd_chr_text(
                    &universe.command_context(),
                    PrintCommand::from_current(&command)
                ),
                expected,
            );
            assert_eq!(
                meaning_text(&mut universe.command_context(), &command),
                expected
            );
        }
    }

    // Families whose selector is carried by an unexpandable command still
    // use the canonical primitive name through an alias.
    for (name, primitive) in [
        ("catcode", U::CatCode),
        ("lccode", U::LcCode),
        ("uccode", U::UcCode),
        ("sfcode", U::SfCode),
        ("mathcode", U::MathCode),
        ("delcode", U::DelCode),
        ("setbox", U::SetBox),
        ("box", U::Box),
        ("copy", U::Copy),
        ("font", U::Font),
        ("fontdimen", U::FontDimen),
    ] {
        let meaning = Meaning::UnexpandablePrimitive(primitive);
        universe.register_primitive_meaning(name, meaning);
        let alias = universe.intern(&format!("alias-{name}")).symbol();
        universe.set_meaning(alias, meaning);
        let command = {
            let mut state = universe.command_context();
            CurrentCommand::resolve(
                traced(Token::Cs(alias)),
                crate::command::DeliveryStamp::new(0, 0, 0),
                None,
                false,
                &mut state,
            )
        };
        assert_eq!(
            print_cmd_chr_text(
                &universe.command_context(),
                PrintCommand::from_current(&command)
            ),
            format!("\\{name}"),
        );
    }

    let font = universe.intern_font(LoadedFont::new(
        "cmr10",
        "cmr10.tfm",
        [7; 32],
        0,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(12 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        FontMetrics::default(),
    ));
    let font_alias = universe.intern("font-alias").symbol();
    universe.set_meaning(font_alias, Meaning::Font(font));
    let font_command = {
        let mut state = universe.command_context();
        CurrentCommand::resolve(
            traced(Token::Cs(font_alias)),
            crate::command::DeliveryStamp::new(0, 0, 0),
            None,
            false,
            &mut state,
        )
    };
    assert_eq!(
        print_cmd_chr_text(
            &universe.command_context(),
            PrintCommand::from_current(&font_command)
        ),
        "select font cmr10 at 12.0pt",
    );

    // Rendering is an immutable observation: representative cells from each
    // parameter bank retain their exact pre-render values.
    assert_eq!(universe.int_param(IntParam::ESCAPE_CHAR), initial_escape);
    assert_eq!(universe.dimen_param(DimenParam::H_SIZE), initial_hsize);
    assert_eq!(
        universe.glue_param(GlueParam::BASELINE_SKIP),
        initial_baseline
    );
    assert_eq!(universe.tok_param(TokParam::EVERY_PAR), initial_every_par);
}

#[test]
fn etex_print_cmd_chr_selector_table_is_exact_for_primitives_registers_and_aliases() {
    use tex_state::meaning::UnexpandablePrimitive as U;

    // e-TeX 2.6's merged etex.web changes [38], [43], [48]-[54], and
    // [76]-[77] extend print_cmd_chr and its meaning callers with these
    // selector families.  Keep this as one exact table: several rows share a
    // command code and differ only by chr, while the dense/sparse token
    // registers cross e-TeX's eqtb boundary without changing their spelling.
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    for (name, meaning) in [
        (
            "eTeXrevision",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ETeXRevision),
        ),
        (
            "expandafter",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::ExpandAfter),
        ),
        (
            "noexpand",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand),
        ),
        (
            "unless",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Unless),
        ),
        (
            "topmark",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::TopMark),
        ),
        (
            "firstmark",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::FirstMark),
        ),
        (
            "botmark",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::BotMark),
        ),
        (
            "splitfirstmark",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::SplitFirstMark),
        ),
        (
            "splitbotmark",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::SplitBotMark),
        ),
        (
            "topmarks",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::TopMarks),
        ),
        (
            "firstmarks",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::FirstMarks),
        ),
        (
            "botmarks",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::BotMarks),
        ),
        (
            "splitfirstmarks",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::SplitFirstMarks),
        ),
        (
            "splitbotmarks",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::SplitBotMarks),
        ),
        ("mark", Meaning::UnexpandablePrimitive(U::Mark)),
        ("marks", Meaning::UnexpandablePrimitive(U::Marks)),
        ("read", Meaning::UnexpandablePrimitive(U::Read)),
        ("readline", Meaning::UnexpandablePrimitive(U::ReadLine)),
        ("parshape", Meaning::UnexpandablePrimitive(U::ParShape)),
        (
            "parshapelength",
            Meaning::UnexpandablePrimitive(U::ParShapeLength),
        ),
        (
            "parshapeindent",
            Meaning::UnexpandablePrimitive(U::ParShapeIndent),
        ),
        (
            "parshapedimen",
            Meaning::UnexpandablePrimitive(U::ParShapeDimen),
        ),
        (
            "interlinepenalties",
            Meaning::UnexpandablePrimitive(U::InterLinePenalties),
        ),
        (
            "clubpenalties",
            Meaning::UnexpandablePrimitive(U::ClubPenalties),
        ),
        (
            "widowpenalties",
            Meaning::UnexpandablePrimitive(U::WidowPenalties),
        ),
        (
            "displaywidowpenalties",
            Meaning::UnexpandablePrimitive(U::DisplayWidowPenalties),
        ),
        (
            "the",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::The),
        ),
        (
            "unexpanded",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Unexpanded),
        ),
        (
            "detokenize",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Detokenize),
        ),
        ("toks", Meaning::UnexpandablePrimitive(U::Toks)),
        ("valign", Meaning::UnexpandablePrimitive(U::VAlign)),
        ("beginL", Meaning::UnexpandablePrimitive(U::BeginL)),
        ("endL", Meaning::UnexpandablePrimitive(U::EndL)),
        ("beginR", Meaning::UnexpandablePrimitive(U::BeginR)),
        ("endR", Meaning::UnexpandablePrimitive(U::EndR)),
        (
            "input",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Input),
        ),
        (
            "endinput",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndInput),
        ),
        (
            "scantokens",
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Scantokens),
        ),
    ] {
        universe.register_primitive_meaning(name, meaning);
        let canonical = universe.intern(name).symbol();
        let alias = universe.intern(&format!("alias-{name}")).symbol();
        universe.set_meaning(canonical, meaning);
        universe.set_meaning(alias, meaning);

        for (index, symbol) in [canonical, alias].into_iter().enumerate() {
            let command = {
                let mut state = universe.command_context();
                CurrentCommand::resolve(
                    traced(Token::Cs(symbol)),
                    crate::command::DeliveryStamp::new(
                        0,
                        u64::try_from(index).expect("two selector spellings fit u64"),
                        0,
                    ),
                    None,
                    false,
                    &mut state,
                )
            };
            assert_eq!(
                print_cmd_chr_text(
                    &universe.command_context(),
                    PrintCommand::from_current(&command),
                ),
                format!("\\{name}"),
                "selector or alias for {name}",
            );
            let meaning_suffix = if matches!(
                meaning,
                Meaning::ExpandablePrimitive(
                    ExpandablePrimitive::TopMark
                        | ExpandablePrimitive::FirstMark
                        | ExpandablePrimitive::BotMark
                        | ExpandablePrimitive::SplitFirstMark
                        | ExpandablePrimitive::SplitBotMark
                )
            ) {
                ":"
            } else {
                ""
            };
            assert_eq!(
                meaning_text(&mut universe.command_context(), &command),
                format!("\\{name}{meaning_suffix}"),
                "meaning selector or alias for {name}",
            );
        }
    }

    for index in [0, 255, 256, 32_767] {
        let symbol = universe.intern(&format!("toks-{index}")).symbol();
        universe.set_meaning(symbol, Meaning::ToksRegister(index));
        let command = {
            let mut state = universe.command_context();
            CurrentCommand::resolve(
                traced(Token::Cs(symbol)),
                crate::command::DeliveryStamp::new(0, u64::from(index), 0),
                None,
                false,
                &mut state,
            )
        };
        let expected = format!("\\toks{index}");
        assert_eq!(
            print_cmd_chr_text(
                &universe.command_context(),
                PrintCommand::from_current(&command),
            ),
            expected,
        );
        assert_eq!(
            meaning_text(&mut universe.command_context(), &command),
            expected
        );
    }
}

#[test]
fn etex_protected_parameterized_macro_meaning_renders_structural_end_match() {
    // e-TeX 2.6 merged change [38] adds protected_call while retaining
    // TeX82 §§289/294's structural end_match rendering as `->` between the
    // parameter and replacement lists.
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let parameters = universe.intern_token_list(&[
        Token::Param(1),
        Token::Char {
            ch: '!',
            cat: Catcode::Other,
        },
    ]);
    let replacement = universe.intern_token_list(&[Token::Param(1)]);
    let flags = MeaningFlags::PROTECTED;
    let definition = universe.intern_macro(MacroMeaning::new(flags, parameters, replacement));
    let original = universe.intern("protected-parameterized").symbol();
    let alias = universe.intern("protected-parameterized-alias").symbol();
    let meaning = Meaning::Macro {
        flags,
        definition: definition.id(),
    };
    universe.set_meaning(original, meaning);
    universe.set_meaning(alias, meaning);

    for (index, symbol) in [original, alias].into_iter().enumerate() {
        let command = {
            let mut state = universe.command_context();
            CurrentCommand::resolve(
                traced(Token::Cs(symbol)),
                crate::command::DeliveryStamp::new(
                    0,
                    u64::try_from(index).expect("two macro spellings fit u64"),
                    0,
                ),
                None,
                false,
                &mut state,
            )
        };
        assert_eq!(
            print_cmd_chr_text(
                &universe.command_context(),
                PrintCommand::from_current(&command),
            ),
            "\\protected macro",
        );
        assert_eq!(
            meaning_text(&mut universe.command_context(), &command),
            "\\protected macro:#1!->#1",
        );
    }
}

#[test]
fn print_cmd_chr_relax_uses_live_escapechar() {
    // TeX82 §§63/227/298: the `relax` case calls `print_esc`, so command
    // diagnostics must not bake in a backslash.
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let symbol = universe.intern("relax").symbol();
    universe.set_meaning(symbol, Meaning::Relax);
    universe.set_int_param(
        tex_state::env::banks::IntParam::ESCAPE_CHAR,
        i32::from(b'@'),
    );
    let command = {
        let mut state = universe.command_context();
        CurrentCommand::resolve(
            traced(Token::Cs(symbol)),
            crate::command::DeliveryStamp::new(0, 0, 0),
            None,
            false,
            &mut state,
        )
    };

    assert_eq!(
        print_cmd_chr_text(
            &universe.command_context(),
            PrintCommand::from_current(&command),
        ),
        "@relax"
    );
}

#[test]
fn public_append_renderers_extend_caller_owned_text() {
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let symbol = universe.intern("relax").symbol();
    universe.set_meaning(symbol, Meaning::Relax);
    let command = {
        let mut state = universe.command_context();
        CurrentCommand::resolve(
            traced(Token::Cs(symbol)),
            crate::command::DeliveryStamp::new(0, 0, 0),
            None,
            false,
            &mut state,
        )
    };

    let mut text = String::from("prefix:");
    append_print_cmd_chr_text(
        &universe.command_context(),
        PrintCommand::from_current(&command),
        &mut text,
    );
    append_character_command_text('x', Catcode::Letter, &mut text);
    append_print_esc_text(&universe.command_context(), "end", &mut text);
    append_command_token_text(&mut universe.command_context(), Token::Param(3), &mut text);

    assert_eq!(
        text,
        "prefix:\\relaxthe letter x\\endmacro parameter character #3"
    );
}

#[test]
fn token_list_control_sequences_use_live_escapechar() {
    // TeX82 §§63/294: `show_token_list` delegates control-sequence spelling
    // to `print_cs`, including the live escape prefix.
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let end = universe.intern("end").symbol();
    universe.set_int_param(tex_state::env::banks::IntParam::ESCAPE_CHAR, 256);

    assert_eq!(
        token_list_token_text(&universe.command_context(), Token::Cs(end)),
        "end "
    );

    universe.set_int_param(
        tex_state::env::banks::IntParam::ESCAPE_CHAR,
        i32::from(b'@'),
    );
    let null_cs = universe.intern("").symbol();
    assert_eq!(
        token_list_token_text(&universe.command_context(), Token::Cs(end)),
        "@end "
    );
    assert_eq!(
        token_list_token_text(&universe.command_context(), Token::Cs(null_cs)),
        "@csname@endcsname "
    );

    let symbol = universe.intern("@").symbol();
    assert_eq!(
        token_list_token_text(&universe.command_context(), Token::Cs(symbol)),
        "@@"
    );
    universe.set_catcode('@', Catcode::Letter);
    assert_eq!(
        token_list_token_text(&universe.command_context(), Token::Cs(symbol)),
        "@@ "
    );

    // TeX82 §§262/289/294: these are permanent frozen eqtb control
    // sequences, not anonymous sentinels. `show_token_list` therefore sends
    // them through `print_cs`, including its control-word delimiter.
    assert_eq!(
        token_list_token_text(&universe.command_context(), Token::frozen_relax()),
        "@relax "
    );
}

#[test]
fn token_list_display_doubles_one_stored_parameter_character() {
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let token = Token::Char {
        ch: '#',
        cat: Catcode::Parameter,
    };
    let stored = universe.intern_token_list(&[token]);

    assert_eq!(universe.tokens(stored), &[token]);
    assert_eq!(
        token_list_token_text(&universe.command_context(), token),
        "##"
    );
}

#[test]
fn meaning_renderer_covers_register_quantity_and_primitive_families() {
    use tex_state::env::banks::IntParam;
    use tex_state::meaning::{InternalInteger, UnexpandablePrimitive};
    use tex_state::page::{PageDimension, PageInteger};

    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    for (index, meaning, canonical_name, expected) in [
        (0, Meaning::CountRegister(3), "aliascount", "\\count3"),
        (1, Meaning::DimenRegister(4), "aliasdimen", "\\dimen4"),
        (2, Meaning::SkipRegister(5), "aliasskip", "\\skip5"),
        (3, Meaning::MuskipRegister(6), "aliasmuskip", "\\muskip6"),
        (4, Meaning::ToksRegister(7), "aliastoks", "\\toks7"),
        (
            5,
            Meaning::IntParam(IntParam::ESCAPE_CHAR.raw()),
            "escapechar",
            "\\escapechar",
        ),
        (
            6,
            Meaning::InternalInteger(InternalInteger::Badness),
            "badness",
            "\\badness",
        ),
        (
            7,
            Meaning::PageDimension(PageDimension::Goal),
            "pagegoal",
            "\\pagegoal",
        ),
        (
            8,
            Meaning::PageInteger(PageInteger::DeadCycles),
            "deadcycles",
            "\\deadcycles",
        ),
        (
            9,
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Meaning),
            "meaning",
            "\\meaning",
        ),
        (
            10,
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Show),
            "show",
            "\\show",
        ),
    ] {
        universe.register_primitive_meaning(canonical_name, meaning);
        let alias = universe.intern(&format!("alias{index}")).symbol();
        universe.set_meaning(alias, meaning);
        let command = {
            let mut state = universe.command_context();
            CurrentCommand::resolve(
                traced(Token::Cs(alias)),
                crate::command::DeliveryStamp::new(0, 0, 0),
                None,
                false,
                &mut state,
            )
        };
        let rendered = meaning_text(&mut universe.command_context(), &command);
        assert_eq!(rendered, expected);
        assert!(!rendered.contains("Register("));
        assert!(!rendered.contains("Primitive("));
    }
}

#[test]
fn expandafter_and_noexpand_preserve_canonical_raw_order() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let expandafter = install_expandable(
        &mut universe,
        "expandafter",
        ExpandablePrimitive::ExpandAfter,
    );
    let noexpand = install_expandable(&mut universe, "noexpand", ExpandablePrimitive::NoExpand);
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
            traced(Token::Cs(noexpand)),
            traced(Token::Cs(macro_name)),
            traced(Token::Cs(macro_name)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    let delivered = {
        let mut processor =
            processor(&mut command, &mut universe, &mut capabilities).with_observer(&mut recorder);
        let mut delivered = Vec::new();
        for _ in 0..4 {
            let command = processor
                .get_x_token()
                .expect("expanded delivery succeeds")
                .expect("planned command is delivered");
            delivered.push((command.spelling().semantic_token(), command.meaning()));
        }
        assert_eq!(processor.command.expansion.cumulative_expansions, 4);
        delivered
    };

    assert_eq!(
        delivered,
        vec![
            (
                Token::Char {
                    ch: 'a',
                    cat: Catcode::Letter,
                },
                Meaning::CharToken {
                    ch: 'a',
                    cat: Catcode::Letter,
                },
            ),
            (
                Token::Char {
                    ch: 'x',
                    cat: Catcode::Letter,
                },
                Meaning::CharToken {
                    ch: 'x',
                    cat: Catcode::Letter,
                },
            ),
            (Token::Cs(macro_name), Meaning::Relax),
            (
                Token::Char {
                    ch: 'x',
                    cat: Catcode::Letter,
                },
                Meaning::CharToken {
                    ch: 'x',
                    cat: Catcode::Letter,
                },
            ),
        ]
    );
    let raw = recorder
        .0
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Command(delivery)
                if delivery.boundary == CommandDeliveryBoundary::Raw =>
            {
                Some(delivery.spelling.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        raw,
        vec![
            crate::ObservedToken::ControlSequence("expandafter".into()),
            crate::ObservedToken::Character {
                character: 'a',
                catcode: Catcode::Letter,
            },
            crate::ObservedToken::ControlSequence("m".into()),
            crate::ObservedToken::Character {
                character: 'a',
                catcode: Catcode::Letter,
            },
            crate::ObservedToken::Character {
                character: 'x',
                catcode: Catcode::Letter,
            },
            crate::ObservedToken::ControlSequence("noexpand".into()),
            crate::ObservedToken::ControlSequence("m".into()),
            crate::ObservedToken::ControlSequence("m".into()),
            crate::ObservedToken::ControlSequence("m".into()),
            crate::ObservedToken::Character {
                character: 'x',
                catcode: Catcode::Letter,
            },
        ]
    );
}

#[test]
fn csname_expands_characters_interns_once_and_requires_endcsname() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let csname = install_expandable(&mut universe, "csname", ExpandablePrimitive::CsName);
    let endcsname = install_expandable(&mut universe, "endcsname", ExpandablePrimitive::EndCsName);
    let letter = install_macro(
        &mut universe,
        "letter",
        Token::Char {
            ch: 'a',
            cat: Catcode::Other,
        },
    );
    let relax = universe.intern("relax").symbol();
    universe.set_meaning(relax, Meaning::Relax);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(csname)),
            traced(Token::Cs(letter)),
            traced(Token::Char {
                ch: 'b',
                cat: Catcode::Letter,
            }),
            traced(Token::Cs(endcsname)),
            traced(Token::Cs(csname)),
            traced(Token::Char {
                ch: 'a',
                cat: Catcode::Other,
            }),
            traced(Token::Char {
                ch: 'b',
                cat: Catcode::Other,
            }),
            traced(Token::Cs(endcsname)),
            traced(Token::Cs(csname)),
            traced(Token::Char {
                ch: 'q',
                cat: Catcode::Letter,
            }),
            traced(Token::Cs(relax)),
            traced(Token::Cs(endcsname)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    let first = processor
        .get_x_token()
        .expect("first csname expands")
        .expect("first name is injected");
    let second = processor
        .get_x_token()
        .expect("second csname expands")
        .expect("second name is injected");
    let (Token::Cs(first_symbol), Token::Cs(second_symbol)) = (
        first.spelling().semantic_token(),
        second.spelling().semantic_token(),
    ) else {
        panic!("csname must inject control-sequence tokens");
    };
    assert_eq!(first_symbol, second_symbol);
    assert_eq!(
        processor.state.known_control_sequence("ab"),
        Some(first_symbol)
    );
    assert_eq!(first.meaning(), Meaning::Relax);
    assert_eq!(second.meaning(), Meaning::Relax);

    let partial = processor
        .get_x_token()
        .expect("missing endcsname recovers")
        .expect("partial name is injected");
    let backed = processor
        .get_x_token()
        .expect("rejected command is replayed")
        .expect("backed relax is live");
    let boundary = processor
        .get_x_token()
        .expect("original boundary remains")
        .expect("endcsname is not swallowed");
    let Token::Cs(partial_symbol) = partial.spelling().semantic_token() else {
        panic!("partial csname must still create a control sequence");
    };
    assert_eq!(
        processor.state.known_control_sequence("q"),
        Some(partial_symbol)
    );
    assert_eq!(partial.meaning(), Meaning::Relax);
    assert_eq!(backed.spelling().semantic_token(), Token::Cs(relax));
    assert_eq!(
        boundary.meaning(),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName)
    );
    assert_eq!(
        processor.command.expansion.pending_diagnostics,
        vec![MISSING_ENDCSNAME_DIAGNOSTIC]
    );
}

#[test]
fn backup_replays_the_exact_delivered_token_above_expansion() {
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(b"\\expandafter A\\m Z".as_slice()),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    install_expandable(
        &mut universe,
        "expandafter",
        ExpandablePrimitive::ExpandAfter,
    );
    install_macro(
        &mut universe,
        "m",
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);

    let first = processor
        .get_x_token()
        .expect("expandafter completes")
        .expect("first token is replayed");
    let spelling = first.spelling();
    let source_range = first.source_range();
    let source_location = first.source_location();
    let first_stamp = first.delivery_stamp();
    processor
        .back_input(first)
        .expect("exact delivery backs up");

    let replayed = processor
        .get_x_token()
        .expect("backup replays")
        .expect("backed token is live");
    assert_eq!(replayed.spelling(), spelling);
    assert_eq!(replayed.source_range(), source_range);
    assert_eq!(replayed.source_location(), source_location);
    assert_ne!(replayed.delivery_stamp(), first_stamp);
    assert!(replayed.direct_source_provenance().is_none());
    assert_eq!(
        processor
            .get_x_token()
            .expect("expanded second token remains below backup")
            .expect("macro output is live")
            .spelling()
            .semantic_token(),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        }
    );
    assert_eq!(
        processor
            .get_x_token()
            .expect("source resumes")
            .expect("following source token is live")
            .spelling()
            .semantic_token(),
        Token::Char {
            ch: 'Z',
            cat: Catcode::Letter,
        }
    );
}

#[test]
fn converted_token_lists_classify_spaces_copy_tokens_and_resume_expansion() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let number = install_expandable(&mut universe, "number", ExpandablePrimitive::Number);
    let roman = install_expandable(
        &mut universe,
        "romannumeral",
        ExpandablePrimitive::RomanNumeral,
    );
    let string = install_expandable(&mut universe, "string", ExpandablePrimitive::String);
    let meaning = install_expandable(&mut universe, "meaning", ExpandablePrimitive::Meaning);
    let fontname = install_expandable(&mut universe, "fontname", ExpandablePrimitive::FontName);
    let jobname = install_expandable(&mut universe, "jobname", ExpandablePrimitive::JobName);
    let string_target = universe.intern("target").symbol();
    let empty = universe.intern_token_list(&[]);
    let long_definition =
        universe.intern_macro(MacroMeaning::new(MeaningFlags::LONG, empty, empty));
    let long_macro = universe.intern("longmacro").symbol();
    universe.set_meaning(
        long_macro,
        Meaning::Macro {
            flags: MeaningFlags::LONG,
            definition: long_definition.id(),
        },
    );
    let font = universe.intern("nullfont-id").symbol();
    let identified_font = universe
        .try_copy_font_with_identifier(tex_state::font::NULL_FONT, font)
        .expect("font identity copies");
    universe.set_meaning(font, Meaning::Font(identified_font));
    let null_font_name = universe.font_name(identified_font).to_owned();
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(number)),
            traced(Token::Char {
                ch: '-',
                cat: Catcode::Other,
            }),
            traced(Token::Char {
                ch: '1',
                cat: Catcode::Other,
            }),
            traced(Token::Char {
                ch: '2',
                cat: Catcode::Other,
            }),
            traced(Token::Cs(roman)),
            traced(Token::Char {
                ch: '9',
                cat: Catcode::Other,
            }),
            traced(Token::Cs(string)),
            traced(Token::Cs(string_target)),
            traced(Token::Cs(meaning)),
            traced(Token::Cs(long_macro)),
            traced(Token::Cs(fontname)),
            traced(Token::Cs(font)),
            traced(Token::Cs(jobname)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    capabilities.set_job_name("paper");
    let rendered_tokens = {
        let mut processor = processor(&mut command, &mut universe, &mut capabilities);
        let mut rendered_tokens = Vec::new();
        while let Some(delivery) = processor.get_x_token().expect("conversion expands") {
            rendered_tokens.push(delivery.spelling().semantic_token());
        }
        assert_eq!(processor.command.expansion.cumulative_expansions, 6);
        rendered_tokens
    };
    let rendered = rendered_tokens
        .iter()
        .map(|token| match token {
            Token::Char { ch, .. } => *ch,
            _ => panic!("classic conversion output must be characters"),
        })
        .collect::<String>();
    assert_eq!(
        rendered,
        format!("-12ix\\target\\long macro:->{null_font_name}paper")
    );
    assert!(rendered_tokens.iter().any(|token| matches!(
        token,
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        }
    )));
    assert!(rendered_tokens.iter().all(|token| matches!(
        token,
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        } | Token::Char {
            ch: '!'..='~',
            cat: Catcode::Other,
        }
    )));
    let the = install_expandable(&mut universe, "the", ExpandablePrimitive::The);
    let copied_macro = install_macro(
        &mut universe,
        "copiedmacro",
        Token::Char {
            ch: 'Q',
            cat: Catcode::Letter,
        },
    );
    let register = universe.intern("stored").symbol();
    universe.set_meaning(register, Meaning::ToksRegister(4));
    let stored = universe.intern_token_list(&[
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
        Token::Cs(copied_macro),
        Token::Char {
            ch: 'L',
            cat: Catcode::Letter,
        },
    ]);
    universe.set_toks(4, stored);
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(the)),
            traced(Token::Cs(font)),
            traced(Token::Cs(the)),
            traced(Token::Cs(register)),
            traced(Token::Char {
                ch: 'Z',
                cat: Catcode::Letter,
            }),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    let mut copied = Vec::new();
    while let Some(delivery) = processor.get_x_token().expect("copied list expands") {
        copied.push(delivery.spelling().semantic_token());
    }
    assert_eq!(
        copied,
        vec![
            Token::Cs(font),
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            Token::Char {
                ch: 'Q',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'L',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'Z',
                cat: Catcode::Letter,
            },
        ]
    );
}

/// TeX82 §§471-472 route `font_name_code` through §577's
/// `scan_font_ident`, so an ordinary font control sequence is converted
/// directly and the enclosing expanded delivery resumes after it once.
#[test]
fn fontname_scans_a_valid_font_identifier() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe_with_plain_catcodes();
    let fontname = install_expandable(&mut universe, "fontname", ExpandablePrimitive::FontName);
    let identifier = universe.intern("selectedfont").symbol();
    let font = universe
        .try_copy_font_with_identifier(tex_state::font::NULL_FONT, identifier)
        .expect("font identity copies");
    universe.set_meaning(identifier, Meaning::Font(font));
    let expected = universe.font_name(font).to_owned();
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(vec![
            traced(Token::Cs(fontname)),
            traced(Token::Cs(identifier)),
        ])),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = processor(&mut command, &mut universe, &mut capabilities);
    assert_eq!(rendered(&mut processor), expected);
    assert_eq!(processor.command.expansion.cumulative_expansions, 1);
}

/// TeX82 §577 reports one missing-font-identifier error, backs up an invalid
/// command, and selects `nullfont`. §§467/472 then insert the rendered
/// null-font name before the rejected command is reconsidered. A following
/// macro proves that §380's enclosing expansion loop resumes once rather than
/// starting a second driver.
#[test]
fn fontname_invalid_character_and_control_recover_once_then_resume_expansion() {
    for invalid_control in [false, true] {
        let mut command = CommandState::default();
        let mut universe = crate::test_harness::universe_with_plain_catcodes();
        let fontname = install_expandable(&mut universe, "fontname", ExpandablePrimitive::FontName);
        let continuation = install_macro(
            &mut universe,
            "continue",
            Token::Char {
                ch: '!',
                cat: Catcode::Other,
            },
        );
        let invalid = if invalid_control {
            let relax = universe.intern("relax").symbol();
            universe.set_meaning(relax, Meaning::Relax);
            Token::Cs(relax)
        } else {
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            }
        };
        let null_font_name = universe.font_name(tex_state::font::NULL_FONT).to_owned();
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![
                traced(Token::Cs(fontname)),
                traced(invalid),
                traced(Token::Cs(continuation)),
            ])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        let delivered = {
            let mut processor = processor(&mut command, &mut universe, &mut capabilities)
                .with_observer(&mut recorder);
            let mut delivered = Vec::new();
            while let Some(command) = processor.get_x_token().expect("recovery is finite") {
                delivered.push(command.spelling().semantic_token());
            }
            assert_eq!(processor.command.expansion.cumulative_expansions, 2);
            delivered
        };

        let mut expected = null_font_name
            .chars()
            .map(|ch| Token::Char {
                ch,
                cat: if ch == ' ' {
                    Catcode::Space
                } else {
                    Catcode::Other
                },
            })
            .collect::<Vec<_>>();
        expected.push(invalid);
        expected.push(Token::Char {
            ch: '!',
            cat: Catcode::Other,
        });
        assert_eq!(delivered, expected);

        // TeX82 §577 still prints the missing-identifier error and performs
        // §327's `back_error`, but the canonical WEB observer assigns that
        // text-only report no semantic diagnostic event. This is especially
        // important when §336's conditional-limit recovery supplied the
        // rejected `\relax`: an invented event shifts the whole trace.
        let diagnostics = recorder
            .0
            .iter()
            .filter_map(|record| match record {
                CommandObservation::Diagnostic(diagnostic)
                    if diagnostic.diagnostic == "missing_font_identifier" =>
                {
                    Some(diagnostic)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(diagnostics.is_empty());
    }
}
