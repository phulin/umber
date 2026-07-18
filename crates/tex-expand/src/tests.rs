use crate::{
    DriverExpansionMode, EngineMode, ExpandableOpcode, ExpansionContext, ExpansionMode,
    ReadDependency, ReadRecorder, RestrictedExpansionMode, dispatch, dispatch_expandable_opcode,
    dispatch_with_context, install_expandable_primitives, semantic_token,
};
use ahash::AHashMap;
#[cfg(feature = "profiling-stats")]
use tex_lex::MacroArguments;
use tex_lex::{InputStack, MemoryInput, TokenListReplayKind};
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::TokenListId;
use tex_state::interner::Symbol;
use tex_state::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::node::{BoxNode, BoxNodeFields, GlueKind, KernKind, Node, Sign};
use tex_state::page::PageMark;
use tex_state::provenance::{
    InsertedOrigin, InsertedOriginKind, MacroInvocationOrigin, OriginRecord, SynthesizedOrigin,
    SynthesizedOriginKind,
};
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{ExpansionState, InputReadState, Universe};

#[test]
fn resource_need_survives_every_nested_scanner_wrapper() {
    fn need() -> crate::ExpandError {
        crate::ExpandError::NeedResource(crate::ResourceNeed::new(115))
    }

    let errors = [
        crate::ExpandError::ScanInt(Box::new(crate::scan_int::ScanIntError::Expand(need()))),
        crate::ExpandError::ScanInt(Box::new(crate::scan_int::ScanIntError::Expand(
            crate::ExpandError::ScanInt(Box::new(crate::scan_int::ScanIntError::Expand(need()))),
        ))),
        crate::ExpandError::ScanDimen(Box::new(crate::scan_dimen::ScanDimenError::Expand(need()))),
        crate::ExpandError::ScanDimen(Box::new(crate::scan_dimen::ScanDimenError::Integer(
            crate::scan_int::ScanIntError::Expand(need()),
        ))),
        crate::ExpandError::ScanGlue(Box::new(crate::scan_glue::ScanGlueError::Expand(need()))),
        crate::ExpandError::ScanGlue(Box::new(crate::scan_glue::ScanGlueError::Dimen(
            crate::scan_dimen::ScanDimenError::Integer(crate::scan_int::ScanIntError::Expand(
                need(),
            )),
        ))),
        crate::ExpandError::ScanGeneralText(Box::new(crate::scan::ScanToksError::Expand(need()))),
    ];

    for error in errors {
        assert_eq!(error.resource_need(), Some(crate::ResourceNeed::new(115)));
    }
}

#[test]
fn paragraph_reads_are_deduplicated_at_publication() {
    let mut context = ExpansionContext::new("test");
    context.begin_paragraph_recording();
    context.record_dependency(ReadDependency::InputStack);
    context.record_dependency(ReadDependency::Meaning(7));
    context.record_dependency(ReadDependency::InputStack);

    let (reads, barriers) = context.finish_paragraph_recording();

    assert_eq!(
        reads,
        vec![ReadDependency::Meaning(7), ReadDependency::InputStack]
    );
    assert!(barriers.is_empty());
}

#[test]
fn paragraph_local_meaning_reads_are_source_proven_until_group_exit() {
    let mut context = ExpansionContext::new("test");
    let mut stores = Universe::new();
    let symbol = stores.intern("local").symbol();
    context.begin_paragraph_recording();
    context.mark_paragraph_local_meaning(symbol, 1);
    context.record_meaning(symbol, Meaning::Relax);
    context.paragraph_group_exited(0);
    context.record_meaning(symbol, Meaning::Undefined);

    let (reads, _) = context.finish_paragraph_recording();

    assert_eq!(reads, vec![ReadDependency::Meaning(symbol.raw())]);
}

#[test]
fn paragraph_meaning_read_before_local_write_remains_a_dependency() {
    let mut context = ExpansionContext::new("test");
    let mut stores = Universe::new();
    let symbol = stores.intern("local").symbol();
    context.begin_paragraph_recording();
    context.record_meaning(symbol, Meaning::Undefined);
    context.mark_paragraph_local_meaning(symbol, 1);
    context.record_meaning(symbol, Meaning::Relax);

    let (reads, _) = context.finish_paragraph_recording();

    assert_eq!(reads, vec![ReadDependency::Meaning(symbol.raw())]);
}

fn pdf_test_font(name: &str, content_hash: [u8; 32], size: i32) -> tex_state::font::LoadedFont {
    tex_state::font::LoadedFont::new(
        name,
        name,
        content_hash,
        0,
        Scaled::from_raw(655_360),
        Scaled::from_raw(size),
        vec![Scaled::from_raw(0); 7],
        tex_state::font::FontMetrics::default(),
    )
}

#[test]
fn pdf_font_enquiries_share_stable_resource_and_object_identities() {
    let mut stores = Universe::new();
    stores.enable_pdf_output();
    crate::install_pdftex_expandable_primitives(&mut stores);
    let a = stores.intern("a");
    let b = stores.intern("b");
    let c = stores.intern("c");
    let font_a = stores.intern_font_with_identifier(pdf_test_font("cmr10", [1; 32], 655_360), a);
    let font_b = stores.intern_font_with_identifier(pdf_test_font("cmr10", [1; 32], 786_432), b);
    let font_c = stores.intern_font_with_identifier(pdf_test_font("cmtt10", [2; 32], 655_360), c);
    stores.set_meaning(a, Meaning::Font(font_a));
    stores.set_meaning(b, Meaning::Font(font_b));
    stores.set_meaning(c, Meaning::Font(font_c));

    let mut input = InputStack::new(MemoryInput::new(
        "\\pdffontname\\a\\pdffontobjnum\\a\\pdffontname\\b\\pdffontobjnum\\b\\pdffontname\\c\\pdffontobjnum\\c",
    ));
    let mut output = String::new();
    while let Some(token) = crate::get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect("font enquiry expansion")
    {
        let Token::Char { ch, cat } = crate::semantic_token(token) else {
            panic!("non-character enquiry output")
        };
        assert_eq!(cat, Catcode::Other);
        output.push(ch);
    }
    assert_eq!(output, "111132");
    assert_eq!(stores.pdf_next_object_id(), 3);
    let resources = stores.pdf_font_resources().collect::<Vec<_>>();
    assert_eq!(resources.len(), 2);
    assert_eq!(
        stores
            .pdf_font_resources()
            .map(|record| (record.resource_number(), record.object_number()))
            .collect::<Vec<_>>(),
        vec![(1, 1), (3, 2)]
    );
}

#[test]
fn pdf_font_enquiries_reject_nullfont() {
    let mut stores = Universe::new();
    crate::install_pdftex_expandable_primitives(&mut stores);
    let nullfont = stores.intern("nullfont");
    stores.set_meaning(nullfont, Meaning::Font(tex_state::font::NULL_FONT));
    let mut input = InputStack::new(MemoryInput::new("\\pdffontobjnum\\nullfont"));
    let error = crate::get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect_err("nullfont must be rejected");
    assert_eq!(
        error.to_string(),
        "pdfTeX error (font): invalid font identifier."
    );
}

#[test]
fn pdf_last_object_reads_the_checkpointed_canonical_ledger() {
    let mut stores = Universe::new();
    stores.enable_pdf_output();
    crate::install_expandable_primitives(&mut stores);
    crate::install_pdftex_expandable_primitives(&mut stores);

    for (expected, reserve) in [("0", false), ("1", true)] {
        if reserve {
            assert_eq!(
                stores
                    .reserve_pdf_raw_object()
                    .expect("reserve raw object")
                    .raw(),
                1
            );
        }
        let mut input = InputStack::new(MemoryInput::new("\\the\\pdflastobj"));
        let token = get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
        )
        .expect("last-object expansion")
        .expect("one digit");
        assert_eq!(token, char_token(expected.chars().next().expect("digit")));
        assert_eq!(
            get_x_token(
                &mut input,
                &mut tex_state::ExpansionContext::new(&mut stores)
            )
            .expect("end of expansion"),
            None
        );
    }
}

#[test]
fn pdftex_absolute_conditionals_handle_signed_minimum_without_overflow() {
    assert_eq!(
        crate::conditionals::absolute_magnitude(i32::MIN),
        2_147_483_648
    );
    assert_eq!(
        crate::conditionals::absolute_magnitude(i32::MAX),
        2_147_483_647
    );
    assert_eq!(crate::conditionals::absolute_magnitude(-1), 1);
}

#[cfg(feature = "profiling-stats")]
#[test]
fn macro_site_meaning_cache_is_expansion_owned_and_guarded() {
    let mut stores = Universe::new();
    let symbol = stores.intern("cached");
    stores.set_meaning(symbol, Meaning::Relax);
    let baseline = stores.snapshot();
    let body = stores.intern_token_list(&[Token::Cs(symbol.symbol())]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_macro_body(body, MacroArguments::new());
    let mut expansion = ExpansionContext::new("texput");

    let read = input
        .next_traced_expansion_token(&mut stores)
        .expect("macro replay")
        .expect("control sequence");
    expansion.observe_read(read);
    assert_eq!(read.token(), Token::Cs(symbol.symbol()));
    assert_eq!(
        expansion.resolve_meaning(&mut input, &stores, symbol.symbol()),
        Meaning::Relax
    );

    stores.enter_group();
    stores.set_count(0, 42);
    let _ = stores.leave_group();
    assert_eq!(
        expansion.resolve_meaning(&mut input, &stores, symbol.symbol()),
        Meaning::Relax,
        "a non-meaning group must retain the guarded cache entry"
    );
    assert_eq!(
        expansion.resolve_meaning(&mut input, &stores, symbol.symbol()),
        Meaning::Relax
    );

    stores.enter_group();
    stores.set_meaning(symbol, Meaning::Undefined);
    assert_eq!(
        expansion.resolve_meaning(&mut input, &stores, symbol.symbol()),
        Meaning::Undefined
    );
    let _ = stores.leave_group();
    assert_eq!(
        expansion.resolve_meaning(&mut input, &stores, symbol.symbol()),
        Meaning::Relax
    );

    stores.enter_group();
    stores.set_meaning_global(symbol, Meaning::Undefined);
    assert_eq!(
        expansion.resolve_meaning(&mut input, &stores, symbol.symbol()),
        Meaning::Undefined
    );
    let _ = stores.leave_group();
    assert_eq!(
        expansion.resolve_meaning(&mut input, &stores, symbol.symbol()),
        Meaning::Undefined
    );

    stores.enter_group_with_kind(tex_state::GroupKind::Simple);
    stores.set_meaning(symbol, Meaning::Relax);
    assert_eq!(
        expansion.resolve_meaning(&mut input, &stores, symbol.symbol()),
        Meaning::Relax
    );
    stores
        .leave_group_with_kind(tex_state::GroupKind::Simple)
        .expect("matching typed group");
    assert_eq!(
        expansion.resolve_meaning(&mut input, &stores, symbol.symbol()),
        Meaning::Undefined
    );

    stores.set_meaning(symbol, Meaning::Relax);
    stores.rollback(&baseline);
    assert_eq!(
        expansion.resolve_meaning(&mut input, &stores, symbol.symbol()),
        Meaning::Relax
    );

    let fork = stores.clone();
    assert_eq!(
        expansion.resolve_meaning(&mut input, &fork, symbol.symbol()),
        Meaning::Relax
    );

    let stats = input.expansion_stats();
    assert_eq!(stats.meaning_cache_hits, 2);
    assert_eq!(stats.meaning_cache_misses, 9);
    assert_eq!(stats.meaning_lookups, 9);
    assert_eq!(stats.frame_step_timer_samples, 1);
    assert_eq!(stats.provenance_timer_samples, 1);
    assert_eq!(stats.classification_meaning_timer_samples, 1);
}

#[test]
fn get_x_token_converts_frozen_end_template_without_losing_origin() {
    let mut stores = Universe::new();
    let origin = stores.source_origin(tex_state::SourceId::new(7), 19, 3, 5);
    let tokens = stores.intern_token_list(&[stores.frozen_end_template_token()]);
    let origins = stores.allocate_origin_list(&[origin]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list_with_origins(tokens, origins, TokenListReplayKind::Inserted);

    let delivered = crate::get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect("frozen sentinel expansion")
    .expect("frozen endv delivery");

    assert_eq!(crate::semantic_token(delivered), stores.frozen_endv_token());
    assert_eq!(delivered.origin(), origin);
    assert_ne!(
        stores.frozen_end_template_token(),
        Token::Cs(stores.intern("endtemplate").symbol())
    );
}

#[test]
fn preamble_span_operation_expands_exactly_one_token() {
    let mut stores = Universe::new();
    let first = stores.intern("first");
    let second = stores.intern("second");
    let empty = stores.intern_token_list(&[]);
    let first_body = stores.intern_token_list(&[Token::Cs(second.symbol())]);
    let second_body = stores.intern_token_list(&[Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    }]);
    stores.set_macro_meaning(
        first,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, first_body),
    );
    stores.set_macro_meaning(
        second,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, second_body),
    );
    let mut input = InputStack::new(MemoryInput::new("\\first"));

    let delivered = crate::expand_once_then_get_token_with_context(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        &mut ExpansionContext::new("texput"),
    )
    .expect("one expansion should succeed")
    .expect("macro body should provide a raw token");

    assert_eq!(crate::semantic_token(delivered), Token::Cs(second.symbol()));
}

#[derive(Default)]
struct CountingRecorder {
    reads: usize,
}

impl ReadRecorder for CountingRecorder {
    fn record_meaning(&mut self, _symbol: Symbol, _meaning: Meaning) {
        self.reads += 1;
    }
}

fn get_x_token(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
) -> Result<Option<Token>, crate::ExpandError> {
    crate::get_x_token(input, stores).map(|token| token.map(crate::semantic_token))
}

fn get_x_token_recording(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    recorder: &mut dyn ReadRecorder,
) -> Result<Option<Token>, crate::ExpandError> {
    let mut expansion = ExpansionContext::new("texput").recording(recorder);
    crate::get_x_token_with_context(input, stores, &mut expansion)
        .map(|token| token.map(crate::semantic_token))
}

fn get_x_token_with_context(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    context: &mut MemoryResolverFixture,
) -> Result<Option<Token>, crate::ExpandError>
where
    MemoryResolver: crate::InputResolver,
{
    let mut context = context.expansion_context();
    crate::get_x_token_with_context(input, stores, &mut context)
        .map(|token| token.map(crate::semantic_token))
}

fn collect_protected_expansion(
    source: &str,
    prepared: bool,
) -> (Vec<Token>, tex_state::InputSummary, usize) {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    let macro_symbol = stores.intern("m");
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[Token::Char {
        ch: 'M',
        cat: Catcode::Letter,
    }]);
    stores.set_macro_meaning(
        macro_symbol,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, body),
    );
    let protected_symbol = stores.intern("p");
    stores.set_macro_meaning(
        protected_symbol,
        MacroMeaning::new(MeaningFlags::PROTECTED, empty, body),
    );

    let mut input = InputStack::new(MemoryInput::new(source));
    let mut recorder = CountingRecorder::default();
    let mut context = ExpansionContext::new("texput").recording(&mut recorder);
    let mut expanded = Vec::new();
    loop {
        let token = if prepared {
            let Some(first) = crate::next_prepared_expansion_token(
                &mut input,
                &mut tex_state::ExpansionContext::new(&mut stores),
                &mut context,
            )
            .expect("prepared get_next") else {
                break;
            };
            crate::get_x_or_protected_from_prepared_with_context(
                first,
                &mut input,
                &mut tex_state::ExpansionContext::new(&mut stores),
                &mut context,
            )
            .expect("prepared x_token")
        } else {
            crate::get_x_or_protected_with_context(
                &mut input,
                &mut tex_state::ExpansionContext::new(&mut stores),
                &mut context,
            )
            .expect("ordinary get_x_token")
        };
        let Some(token) = token else {
            break;
        };
        expanded.push(crate::semantic_token(token));
    }
    (expanded, input.summary(), recorder.reads)
}

#[test]
fn compulsory_macro_token_mismatch_is_consumed_and_reported() {
    let mut stores = Universe::new();
    let macro_symbol = stores.intern("bad");
    let parameters = stores.intern_token_list(&[char_token('?')]);
    let body = stores.intern_token_list(&[]);
    stores.set_macro_meaning(
        macro_symbol,
        MacroMeaning::new(MeaningFlags::EMPTY, parameters, body),
    );
    let input_tokens = stores.intern_token_list(&[
        Token::Cs(macro_symbol.symbol()),
        char_token('!'),
        char_token('O'),
        char_token('K'),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(input_tokens, TokenListReplayKind::Inserted);
    let mut context = ExpansionContext::new("texput");

    let first = crate::get_x_token_with_context(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        &mut context,
    )
    .expect("TeX recovers from a compulsory-token mismatch");

    assert_eq!(first.map(crate::semantic_token), Some(char_token('O')));
    let diagnostics = context.take_recoverable_diagnostics().collect::<Vec<_>>();
    assert!(matches!(
        diagnostics.as_slice(),
        [crate::RecoverableExpansionDiagnostic::MacroDoesNotMatchDefinition {
            macro_name,
            context,
        }] if macro_name == "\\bad" && crate::semantic_token(*context) == char_token('!')
    ));
}

#[test]
fn expandafter_replays_saved_token_when_target_macro_mismatches() {
    let mut stores = Universe::new();
    let expandafter =
        expandable_primitive(&mut stores, "expandafter", ExpandablePrimitive::ExpandAfter);
    let bad = stores.intern("bad");
    let parameters = stores.intern_token_list(&[char_token('?')]);
    let body = stores.intern_token_list(&[]);
    stores.set_macro_meaning(
        bad,
        MacroMeaning::new(MeaningFlags::EMPTY, parameters, body),
    );
    let input_tokens = stores.intern_token_list(&[
        Token::Cs(expandafter.symbol()),
        char_token('S'),
        Token::Cs(bad.symbol()),
        char_token('!'),
        char_token('T'),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(input_tokens, TokenListReplayKind::Inserted);
    let mut context = ExpansionContext::new("texput");
    let mut expanded = Vec::new();

    while let Some(token) = crate::get_x_token_with_context(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        &mut context,
    )
    .expect("TeX recovers inside expandafter")
    {
        expanded.push(crate::semantic_token(token));
    }

    assert_eq!(expanded, [char_token('S'), char_token('T')]);
    assert_eq!(context.take_recoverable_diagnostics().count(), 1);
}

#[test]
fn prepared_and_input_driven_expansion_share_dispatch_semantics() {
    let source = "\\m\\p\\noexpand\\m\\iftrue T\\else F\\fi\\csname relaxed\\endcsname";

    let ordinary = collect_protected_expansion(source, false);
    let prepared = collect_protected_expansion(source, true);

    assert_eq!(prepared.0, ordinary.0);
    assert_eq!(prepared.1, ordinary.1);
    assert_eq!(prepared.2, ordinary.2);
}

#[test]
fn dispatch_delivers_unexpandable_tokens() {
    let mut stores = Universe::new();
    let token = Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    };
    assert_eq!(
        dispatch(
            token,
            &mut InputStack::new(MemoryInput::new("")),
            &mut tex_state::ExpansionContext::new(&mut stores),
            Meaning::Relax,
        )
        .expect("dispatch should succeed"),
        crate::Dispatch::Deliver(tex_state::token::TracedTokenWord::pack(
            token,
            tex_state::token::OriginId::UNKNOWN,
        ))
    );
}

#[test]
fn expandable_dispatch_table_covers_epic_opcode_families() {
    let opcodes = [
        ExpandableOpcode::Macro,
        ExpandableOpcode::ExpandAfter,
        ExpandableOpcode::NoExpand,
        ExpandableOpcode::CsName,
        ExpandableOpcode::EndCsName,
        ExpandableOpcode::String,
        ExpandableOpcode::Number,
        ExpandableOpcode::RomanNumeral,
        ExpandableOpcode::Meaning,
        ExpandableOpcode::The,
        ExpandableOpcode::Input,
        ExpandableOpcode::If,
        ExpandableOpcode::Else,
        ExpandableOpcode::Or,
        ExpandableOpcode::Fi,
    ];

    for opcode in opcodes {
        let result = dispatch_expandable_opcode(opcode);
        assert!(result.is_ok(), "{opcode:?} should be covered");
    }
}

#[test]
fn invalid_conditional_relation_assumes_equal_and_replays_offending_token() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("!"));
    let ifnum = stores.intern("ifnum");
    let context = TracedTokenWord::pack(Token::Cs(ifnum.symbol()), OriginId::UNKNOWN);
    let relation = crate::conditionals::scan_conditional_relation_with_mode_and_context(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        &mut ExpansionContext::new("texput"),
        &mut crate::RestrictedExpansionMode,
        context,
    )
    .expect("relation scanner should insert equality");

    assert_eq!(relation, crate::conditionals::ConditionalRelation::Equal);
    let token = input
        .next_traced_token(&mut tex_state::ExpansionContext::new(&mut stores))
        .expect("read replayed token")
        .expect("replayed relation token");
    assert_eq!(crate::semantic_token(token), char_token('!'));
    let OriginRecord::Inserted(inserted) = stores.origin(token.origin()) else {
        panic!("relation token should have unread provenance");
    };
    assert_eq!(inserted.kind(), InsertedOriginKind::Unread);
    assert!(matches!(
        stores.origin(inserted.parent()),
        OriginRecord::Source(_) | OriginRecord::SourceSpan(_)
    ));
}

#[test]
fn get_x_token_delivers_unexpandable_control_sequence() {
    let mut stores = Universe::new();
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
    let mut input = InputStack::new(MemoryInput::new(""));
    let list = stores.intern_token_list(&[Token::Cs(relax.symbol())]);
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("expansion should succeed"),
        Some(Token::Cs(relax.symbol()))
    );
}

#[test]
fn get_x_token_reports_undefined_control_sequence_and_forgets_it() {
    let mut stores = Universe::new();
    let undefined = stores.intern("missing");
    let after = stores.intern("after");
    stores.set_meaning(after, Meaning::Relax);
    let mut input = InputStack::new(MemoryInput::new(""));
    let list =
        stores.intern_token_list(&[Token::Cs(undefined.symbol()), Token::Cs(after.symbol())]);
    input.push_token_list(list, TokenListReplayKind::Inserted);

    let err = get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect_err("undefined control sequence is rejected");
    assert!(matches!(
        err,
        crate::ExpandError::UndefinedControlSequence { ref name, .. } if name == "missing"
    ));
    let origin = err.primary_origin().expect("undefined control origin");
    assert_ne!(origin, OriginId::UNKNOWN);
    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("following token should still be readable"),
        Some(Token::Cs(after.symbol()))
    );
}

#[test]
fn conditional_operand_scan_reports_undefined_control_sequence() {
    let mut stores = Universe::new();
    let if_cs = expandable_primitive(&mut stores, "if", ExpandablePrimitive::If);
    let undefined = stores.intern("missing");
    let list = stores.intern_token_list(&[
        Token::Cs(if_cs.symbol()),
        Token::Cs(undefined.symbol()),
        char_token('x'),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    let err = get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect_err("undefined control sequence is rejected");
    assert!(matches!(
        err,
        crate::ExpandError::UndefinedControlSequence { ref name, .. } if name == "missing"
    ));
    assert_ne!(
        err.primary_origin().expect("undefined control origin"),
        OriginId::UNKNOWN
    );
}

#[test]
fn undefined_control_sequence_from_source_reports_source_origin() {
    let mut stores = Universe::new();
    let mut input = InputStack::new(MemoryInput::new("\\missing"));

    let err = get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect_err("undefined control sequence is rejected");

    assert!(matches!(
        err,
        crate::ExpandError::UndefinedControlSequence { ref name, .. } if name == "missing"
    ));
    let origin = err.primary_origin().expect("undefined control origin");
    assert!(matches!(stores.origin(origin), OriginRecord::SourceSpan(_)));
}

#[test]
fn get_x_token_pulls_from_source_frames_with_interner_access() {
    let mut stores = Universe::new();
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
    let mut input = InputStack::new(MemoryInput::new("x\\relax"));

    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("source expansion should succeed"),
        Some(Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        })
    );
    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("source expansion should succeed"),
        Some(Token::Cs(relax.symbol()))
    );
}

#[test]
fn get_x_token_pushes_macro_body_frame_and_continues() {
    let mut stores = Universe::new();
    let macro_cs = stores.intern("m");
    let body = stores.intern_token_list(&[
        Token::Char {
            ch: 'a',
            cat: Catcode::Letter,
        },
        Token::Char {
            ch: 'b',
            cat: Catcode::Letter,
        },
    ]);
    let params = stores.intern_token_list(&[]);
    let definition_origin = stores.source_origin(tex_state::SourceId::new(7), 30, 3, 4);
    let body_origin = stores.source_origin(tex_state::SourceId::new(7), 40, 3, 14);
    let body_origins = stores.allocate_origin_list(&[body_origin, body_origin]);
    stores.set_macro_meaning_with_provenance(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        MacroDefinitionProvenance::new(
            definition_origin,
            tex_state::ids::OriginListId::EMPTY,
            body_origins,
        ),
    );
    let Meaning::Macro { definition, .. } = stores.meaning(macro_cs) else {
        panic!("expected macro meaning");
    };
    let invocation = stores.intern_token_list(&[Token::Cs(macro_cs.symbol())]);
    let call_origin = stores.source_origin(tex_state::SourceId::new(8), 50, 5, 1);
    let invocation_origins = stores.allocate_origin_list(&[call_origin]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list_with_origins(
        invocation,
        invocation_origins,
        TokenListReplayKind::Inserted,
    );

    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("expansion should succeed"),
        Some(Token::Char {
            ch: 'a',
            cat: Catcode::Letter,
        })
    );
    assert!(matches!(
        input.summary().frames().last(),
        Some(tex_lex::InputFrameSummary::TokenList {
            token_list,
            replay_kind: TokenListReplayKind::MacroBody,
            index: 1,
            macro_arguments,
            macro_invocation,
            ..
        }) if *token_list == body
            && macro_arguments.is_empty()
            && matches!(
                stores.origin(*macro_invocation),
                OriginRecord::MacroInvocation(origin)
                    if origin == MacroInvocationOrigin::new(
                        definition,
                        call_origin,
                        definition_origin,
                        OriginId::UNKNOWN,
                    )
            )
    ));
    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("expansion should succeed"),
        Some(Token::Char {
            ch: 'b',
            cat: Catcode::Letter,
        })
    );
}

#[test]
fn expansion_fuel_stops_a_self_recursive_macro() {
    let mut stores = Universe::new();
    let recursive = stores.intern("loop");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[Token::Cs(recursive.symbol())]);
    stores.set_macro_meaning(
        recursive,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
    );
    let mut input = InputStack::new(MemoryInput::new("\\loop"));
    let mut expansion = ExpansionContext::new("texput").with_fuel(8);

    let error = crate::get_x_token_with_context(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        &mut expansion,
    )
    .expect_err("recursive expansion must exhaust its fuel");

    assert_eq!(
        error.to_string(),
        "expansion work limit of 8 steps exceeded"
    );
    assert!(matches!(
        error,
        crate::ExpandError::Captured { error, .. }
            if matches!(
                *error,
                crate::ExpandError::ExpansionWorkLimitExceeded { limit: 8 }
            )
    ));
}

#[test]
fn expansion_fuel_resets_after_a_token_is_delivered() {
    let mut stores = Universe::new();
    let macro_cs = stores.intern("finite");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x')]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
    );
    let mut input = InputStack::new(MemoryInput::new("\\finite\\finite"));
    let mut expansion = ExpansionContext::new("texput").with_fuel(2);

    for _ in 0..2 {
        assert_eq!(
            crate::get_x_token_with_context(
                &mut input,
                &mut tex_state::ExpansionContext::new(&mut stores),
                &mut expansion,
            )
            .expect("each finite expansion request has its own fuel")
            .map(crate::semantic_token),
            Some(char_token('x'))
        );
    }
}

#[test]
fn default_expansion_fuel_clears_the_measured_productive_workload_floor() {
    const MEASURED_PRODUCTIVE_WORKLOAD_MAX: u64 = 3_323_945;
    const EXPECTED_HEADROOM: u64 = 676_055;
    let expansion = ExpansionContext::new("texput");

    assert_eq!(crate::DEFAULT_EXPANSION_FUEL, 4_000_000);
    assert_eq!(
        crate::DEFAULT_EXPANSION_FUEL - MEASURED_PRODUCTIVE_WORKLOAD_MAX,
        EXPECTED_HEADROOM
    );
    assert_eq!(expansion.fuel_limit, 4_000_000);
    assert_eq!(expansion.remaining_fuel, 4_000_000);
}

#[test]
fn nested_expansion_consumes_the_parent_fuel_budget() {
    let mut expansion = ExpansionContext::new("texput").with_fuel(2);
    expansion.begin_fuel_scope();
    expansion.burn_fuel().expect("parent fuel");
    expansion.with_nested(|nested| {
        nested.begin_fuel_scope();
        nested.burn_fuel().expect("nested fuel");
        nested.end_fuel_scope();
    });

    assert!(matches!(
        expansion.burn_fuel(),
        Err(crate::ExpandError::ExpansionWorkLimitExceeded { limit: 2 })
    ));
    expansion.end_fuel_scope();
}

#[test]
fn get_x_token_expands_protected_macros_during_normal_execution() {
    let mut stores = Universe::new();
    let macro_cs = stores.intern("protectedmacro");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x')]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::PROTECTED, params, body),
    );
    let mut input = InputStack::new(MemoryInput::new("\\protectedmacro"));

    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("protected macro expansion"),
        Some(char_token('x'))
    );
}

#[test]
fn get_x_or_protected_stops_before_protected_macro_expansion() {
    // e-TeX's alignment changes use get_x_or_protected at align_peek and
    // fin_col, while ordinary command demand still expands the same macro.
    let mut stores = Universe::new();
    let macro_cs = stores.intern("protectedmacro");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x')]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::PROTECTED, params, body),
    );
    let mut input = InputStack::new(MemoryInput::new("\\protectedmacro"));

    let delivered = crate::get_x_or_protected_with_context(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        &mut ExpansionContext::new("texput"),
    )
    .expect("protected-aware expansion")
    .expect("protected macro token");
    assert_eq!(
        crate::semantic_token(delivered),
        Token::Cs(macro_cs.symbol())
    );
}

#[test]
fn get_x_or_protected_expands_tokens_returned_by_unexpanded() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let macro_cs = stores.intern("ordinarymacro");
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x')]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, body),
    );
    let mut input = InputStack::new(MemoryInput::new("\\unexpanded{\\ordinarymacro}"));

    assert_eq!(
        crate::get_x_or_protected_with_context(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
            &mut ExpansionContext::new("texput"),
        )
        .expect("protected-aware expansion")
        .map(crate::semantic_token),
        Some(char_token('x'))
    );
}

#[test]
fn alignment_x_or_protected_resumes_unexpanded_macro_replay() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let macro_cs = stores.intern("ordinarymacro");
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x')]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, body),
    );
    let mut input = InputStack::new(MemoryInput::new("\\unexpanded{\\ordinarymacro}"));

    assert_eq!(
        crate::get_alignment_x_or_protected_with_context(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
            &mut ExpansionContext::new("texput"),
        )
        .expect("alignment command-demand expansion")
        .map(crate::semantic_token),
        Some(char_token('x'))
    );
}

#[test]
fn keyword_scanner_resumes_a_macro_from_unexpanded_replay() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let keyword_cs = stores.intern("keyword");
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[
        char_token('w'),
        char_token('i'),
        char_token('d'),
        char_token('t'),
        char_token('h'),
    ]);
    stores.set_macro_meaning(
        keyword_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, body),
    );
    let mut input = InputStack::new(MemoryInput::new("\\unexpanded{\\keyword}"));

    assert!(
        crate::scan_optional_keyword_with_context(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
            &mut ExpansionContext::new("texput"),
            "width",
        )
        .expect("keyword command demand")
    );
}

#[test]
fn general_driver_expands_tokens_returned_by_unexpanded() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let macro_cs = stores.intern("ordinarymacro");
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x')]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, body),
    );
    let mut input = InputStack::new(MemoryInput::new("\\unexpanded{\\ordinarymacro}"));

    let delivered = DriverExpansionMode
        .next_expanded_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
            &mut ExpansionContext::new("texput"),
        )
        .expect("driver expansion")
        .expect("expanded macro token");

    assert_eq!(semantic_token(delivered), char_token('x'));
}

#[test]
fn backtick_constant_lookahead_resumes_unexpanded_replay() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let space_cs = stores.intern("sp");
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[Token::Char {
        ch: ' ',
        cat: Catcode::Space,
    }]);
    stores.set_macro_meaning(
        space_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, body),
    );
    let mut input = InputStack::new(MemoryInput::new("`a\\unexpanded{\\sp}z"));
    let mut expansion = ExpansionContext::new("texput");
    let context = TracedTokenWord::pack(char_token('0'), OriginId::UNKNOWN);

    let scanned = crate::scan_int::scan_int_with_mode_and_context(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        &mut expansion,
        &mut DriverExpansionMode,
        context,
    )
    .expect("backtick constant");
    assert_eq!(scanned.value(), 'a' as i32);

    let following = DriverExpansionMode
        .next_expanded_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
            &mut expansion,
        )
        .expect("following expansion")
        .expect("following token");
    assert_eq!(semantic_token(following), char_token('z'));
}

#[test]
fn ordinary_get_x_token_expands_tokens_returned_by_unexpanded() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let macro_cs = stores.intern("m");
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x')]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, body),
    );
    let mut input = InputStack::new(MemoryInput::new("\\unexpanded{\\m}"));

    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("unexpanded expansion"),
        Some(char_token('x'))
    );
}

#[test]
fn expanded_token_list_scope_preserves_unexpanded_replay_until_collection_finishes() {
    let mut stores = Universe::new();
    let macro_cs = stores.intern("m");
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x')]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, body),
    );
    let replay =
        stores.intern_token_list(&[Token::Cs(macro_cs.symbol()), Token::Cs(macro_cs.symbol())]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(replay, TokenListReplayKind::Unexpanded);
    let mut expansion = ExpansionContext::new("texput");

    let preserved = expansion.with_expanded_token_list(|expansion| {
        crate::get_x_token_with_context(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
            expansion,
        )
        .expect("expanded-list collection")
        .expect("preserved token")
    });
    assert_eq!(semantic_token(preserved), Token::Cs(macro_cs.symbol()));

    let expanded = crate::get_x_token_with_context(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        &mut expansion,
    )
    .expect("ordinary expansion after collection")
    .expect("expanded token");
    assert_eq!(semantic_token(expanded), char_token('x'));
}

#[test]
fn expanded_replays_nested_unexpanded_tokens_to_its_caller() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    crate::install_latex_expandable_primitives(&mut stores);
    let macro_cs = stores.intern("macro");
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x')]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, body),
    );
    let mut input = InputStack::new(MemoryInput::new("\\expanded{\\unexpanded{\\macro}}"));

    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("expanded expansion"),
        Some(char_token('x'))
    );
}

#[test]
fn back_input_clears_one_shot_noexpand_suppression() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    let roman = stores.intern("romannumeral");
    let suppressed = stores.intern_token_list(&[Token::Cs(roman.symbol())]);
    let mut input = InputStack::new(MemoryInput::new("0x"));
    input.push_token_list(suppressed, TokenListReplayKind::NoExpand);

    let first = crate::get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect("suppressed delivery")
    .expect("suppressed token");
    assert_eq!(crate::semantic_token(first), Token::Cs(roman.symbol()));
    crate::back_input(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        [first],
    );

    assert_eq!(
        crate::get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("replayed expansion")
        .map(crate::semantic_token),
        Some(char_token('x'))
    );
}

#[test]
fn control_sequence_brace_aliases_do_not_change_alignment_depth() {
    let mut stores = Universe::new();
    let egroup = stores.intern("egroup");
    stores.set_meaning(
        egroup,
        Meaning::CharToken {
            ch: '}',
            cat: Catcode::EndGroup,
        },
    );
    let mut input = InputStack::new(MemoryInput::new("\\egroup"));
    input.begin_alignment();
    input.set_alignment_state(0);
    input.begin_alignment_cell(None, TokenListId::EMPTY, 0);

    let token = crate::get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect("brace alias delivery")
    .expect("brace alias token");
    assert!(input.alignment_cell_at_base_depth());

    crate::back_input(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        [token],
    );
    assert!(input.alignment_cell_at_base_depth());
}

#[test]
fn expanded_is_installed_only_in_the_latex_extension_layer() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let expanded = stores.intern("expanded");
    assert_eq!(stores.meaning(expanded), Meaning::Undefined);

    crate::install_latex_expandable_primitives(&mut stores);
    assert_eq!(
        stores.meaning(expanded),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Expanded)
    );
    let filesize = stores.intern("filesize");
    assert_eq!(
        stores.meaning(filesize),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::FileSize)
    );
    let strcmp = stores.intern("strcmp");
    assert_eq!(
        stores.meaning(strcmp),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::StringCompare)
    );
    let shellescape = stores.intern("shellescape");
    assert_eq!(
        stores.meaning(shellescape),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::ShellEscape)
    );
    let creationdate = stores.intern("creationdate");
    assert_eq!(
        stores.meaning(creationdate),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::CreationDate)
    );
}

#[test]
fn shellescape_reports_the_disabled_world_policy() {
    let mut stores = Universe::new();
    crate::install_latex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("A\\shellescape B%"));
    let mut expansion = tex_state::ExpansionContext::new(&mut stores);

    assert_eq!(next_expanded_chars(&mut input, &mut expansion), "A0B");
}

#[test]
fn creationdate_reports_the_immutable_utc_job_clock() {
    let mut stores = Universe::new();
    crate::install_latex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\creationdate"));
    let mut expansion = tex_state::ExpansionContext::new(&mut stores);
    let mut context = ExpansionContext::new("texput");
    context.job_clock = tex_state::JobClock {
        time: 22 * 60,
        second: 37,
        day: 14,
        month: 7,
        year: 2026,
    };

    let mut rendered = String::new();
    while let Some(token) =
        crate::get_x_token_with_context(&mut input, &mut expansion, &mut context)
            .expect("creation date should expand")
    {
        let Token::Char { ch, cat } = crate::semantic_token(token) else {
            panic!("expected character token, got {token:?}");
        };
        assert_eq!(cat, Catcode::Other);
        rendered.push(ch);
    }
    assert_eq!(rendered, "D:20260714220037Z");
}

#[test]
fn strcmp_expands_and_compares_two_general_text_strings() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_latex_expandable_primitives(&mut stores);
    let value = stores.intern("value");
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('a')]);
    stores.set_macro_meaning(value, MacroMeaning::new(MeaningFlags::EMPTY, empty, body));
    let mut input = InputStack::new(MemoryInput::new(
        "\\strcmp{\\value}{aa},\\strcmp{same}{same},\\strcmp{z}{a}",
    ));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "-1,0,1 "
    );
}

#[test]
fn filesize_expands_the_filename_and_returns_its_byte_count() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_latex_expandable_primitives(&mut stores);
    let filename = stores.intern("filename");
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[
        char_token('a'),
        char_token('s'),
        char_token('s'),
        char_token('e'),
        char_token('t'),
    ]);
    stores.set_macro_meaning(
        filename,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, body),
    );
    let mut input = InputStack::new(MemoryInput::new("\\filesize{\\filename}"));
    let mut context = MemoryResolverFixture::new("main").with_source("asset", "hello\n");

    assert_eq!(
        next_expanded_chars_with_context(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
            &mut context,
        ),
        "6 "
    );
    assert_eq!(context.resolver.sized, vec!["asset"]);
}

#[test]
fn nested_restricted_expansion_retains_filesize_resolution() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_latex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\filesize{asset}"));
    let mut context = MemoryResolverFixture::new("main").with_source("asset", "hello\n");

    let output = {
        let mut expansion = context.expansion_context();
        expansion.with_nested(|nested| {
            let mut output = String::new();
            while let Some(token) = crate::get_x_token_without_input_open(
                &mut input,
                &mut tex_state::ExpansionContext::new(&mut stores),
                nested,
            )
            .expect("read-only file enquiry is valid in restricted expansion")
            {
                let Token::Char { ch, .. } = crate::semantic_token(token) else {
                    panic!("expected rendered filesize character")
                };
                output.push(ch);
            }
            output
        })
    };

    assert_eq!(output, "6 ");
    assert_eq!(context.resolver.sized, vec!["asset"]);
}

#[test]
fn filesize_expands_to_nothing_when_the_file_is_missing() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_latex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("a\\filesize{missing}b"));
    let mut context = MemoryResolverFixture::new("main");

    assert_eq!(
        next_expanded_chars_with_context(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
            &mut context,
        ),
        "ab "
    );
    assert_eq!(context.resolver.sized, vec!["missing"]);
}

#[test]
fn filesize_propagates_a_typed_resource_need() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_latex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\filesize{asset}"));
    let mut resolver = SuspendingResolver;
    let mut context = ExpansionContext::with_input_resolver("main", &mut resolver);

    assert!(matches!(
        crate::get_x_token_without_input_open(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
            &mut context,
        ),
        Err(crate::ExpandError::NeedResource(need))
            if need == crate::ResourceNeed::new(0)
    ));
}

#[test]
fn expanded_performs_message_style_balanced_text_expansion() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    crate::install_latex_expandable_primitives(&mut stores);
    let macro_cs = stores.intern("m");
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('X')]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, body),
    );
    let mut input = InputStack::new(MemoryInput::new("\\expanded{a \\m{b}#}%"));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "a X{b}#"
    );
}

#[test]
fn expanded_balances_braces_after_conditional_expansion() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    crate::install_latex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        r"\expanded{{{A\iffalse}}}\fi B\iffalse{{{\fi}}}%",
    ));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "{{AB}}"
    );
}

#[test]
fn expanded_expands_while_scanning_for_the_opening_brace() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_latex_expandable_primitives(&mut stores);
    let a = stores.intern("a");
    let replacement = stores.intern_token_list(&[char_token('X')]);
    let parameters = stores.intern_token_list(&[]);
    stores.set_macro_meaning(
        a,
        MacroMeaning::new(MeaningFlags::EMPTY, parameters, replacement),
    );
    let mut input = InputStack::new(MemoryInput::new("\\expanded\\expandafter{\\a Y}"));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "XY "
    );
}

#[test]
fn expanded_preserves_protected_macros_during_its_own_expansion() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_latex_expandable_primitives(&mut stores);
    let protected = stores.intern("protectedmacro");
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('X')]);
    stores.set_macro_meaning(
        protected,
        MacroMeaning::new(MeaningFlags::PROTECTED, empty, body),
    );
    let mut input = InputStack::new(MemoryInput::new("\\expanded{\\protectedmacro}"));

    let expanded = crate::get_x_or_protected_with_context(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        &mut ExpansionContext::new("texput"),
    )
    .expect("protected-aware expansion")
    .expect("expanded result");
    assert_eq!(
        crate::semantic_token(expanded),
        Token::Cs(protected.symbol())
    );
}

#[test]
fn expanded_can_return_a_noexpanded_dynamically_named_control_sequence() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_latex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\expanded{\\expandafter\\noexpand\\csname generated:name\\endcsname}",
    ));

    let token = crate::get_x_or_protected_with_context(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        &mut ExpansionContext::new("texput"),
    )
    .expect("expanded csname")
    .expect("generated control sequence");
    let generated = stores.symbol("generated:name").expect("interned csname");
    assert_eq!(crate::semantic_token(token), Token::Cs(generated.symbol()));
}

#[test]
fn unexpanded_expands_while_scanning_for_the_opening_brace() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let a = stores.intern("a");
    let replacement = stores.intern_token_list(&[Token::Char {
        ch: 'X',
        cat: Catcode::Letter,
    }]);
    let parameters = stores.intern_token_list(&[]);
    stores.set_macro_meaning(
        a,
        tex_state::macro_store::MacroMeaning::new(
            tex_state::meaning::MeaningFlags::EMPTY,
            parameters,
            replacement,
        ),
    );
    let mut input = InputStack::new(MemoryInput::new("\\unexpanded\\expandafter{\\a Y}"));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "XY "
    );
}

#[test]
fn unexpanded_accepts_a_control_sequence_with_begin_group_meaning() {
    // e-TeX manual section 3.1 uses TeX's general-text scanner, whose
    // compulsory brace test is by command meaning (for example `\bgroup`).
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let bgroup = stores.intern("bgroup");
    stores.set_meaning(
        bgroup,
        Meaning::CharToken {
            ch: '{',
            cat: Catcode::BeginGroup,
        },
    );
    let mut input = InputStack::new(MemoryInput::new("\\unexpanded\\bgroup X}"));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "X "
    );
}

#[test]
fn detokenize_outputs_space_and_other_character_tokens() {
    let mut stores = Universe::new();
    crate::install_etex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\detokenize{a \\word!#1}%"));
    let mut output = Vec::new();
    while let Some(token) = get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect("detokenize expansion")
    {
        output.push(token);
    }

    let rendered: String = output
        .iter()
        .filter_map(|token| match token {
            Token::Char { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect();
    // e-TeX short reference manual section 3.1 requires a separating space
    // after each control word, including the final control word.
    assert_eq!(rendered, "a \\word !##1");
    assert!(output.iter().all(|token| matches!(
        token,
        Token::Char {
            cat: Catcode::Space,
            ch: ' '
        } | Token::Char {
            cat: Catcode::Other,
            ..
        }
    )));
}

#[test]
fn unless_inverts_boolean_conditionals_but_not_ifcase() {
    // e-TeX short reference manual section 3.7 restricts \unless to boolean
    // conditionals; \ifcase is deliberately excluded.
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\unless\\iftrue n\\else y\\fi\\unless\\iffalse y\\else n\\fi",
    ));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "yy"
    );

    let mut invalid = InputStack::new(MemoryInput::new("\\unless\\ifcase0\\fi"));
    assert!(matches!(
        crate::get_x_token(
            &mut invalid,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        Err(crate::ExpandError::Captured { .. })
            | Err(crate::ExpandError::MissingTokenAfterPrimitive {
                opcode: ExpandableOpcode::Unless,
                ..
            })
    ));
}

#[test]
fn scantokens_relexes_text_with_current_catcodes_and_superscript_notation() {
    // e-TeX short reference manual section 3.2 requires reprocessing through
    // the input mechanism, so both current catcodes and ^^ notation apply.
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    stores.set_catcode('@', Catcode::Active);
    let active = stores.intern_active_character('@');
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('A')]);
    stores.set_macro_meaning(active, MacroMeaning::new(MeaningFlags::EMPTY, empty, body));
    let mut input = InputStack::new(MemoryInput::new("\\scantokens{@^^42}"));

    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("active character from pseudo-file"),
        Some(char_token('A'))
    );
    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("superscript notation from pseudo-file"),
        Some(char_token('B'))
    );
}

#[test]
fn scantokens_splits_raw_newlinechar_into_pseudo_file_records() {
    // e-TeX manual section 3.2 and etex.ch's pseudo_start: token_show uses
    // selector=new_string, then new_line_char splits the resulting records.
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    stores.set_int_param(tex_state::env::banks::IntParam::NEWLINE_CHAR, 10);
    stores.set_catcode('\n', Catcode::Other);
    let mut input = InputStack::new(MemoryInput::new("\\scantokens{A^^JB}%"));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "A B "
    );
}

#[test]
fn scantokens_input_summary_and_state_hash_resume_identically() {
    // e-TeX manual sections 3.2 and 3.7: a live pseudo-file and its pending
    // everyeof replay must survive the aggregate resumability boundary.
    const OUTER: &str = "\\scantokens{AB}%Z";
    const PSEUDO: &str = "AB\n";

    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let everyeof = stores.intern_token_list(&[char_token('E')]);
    stores.set_tok_param(tex_state::env::banks::TokParam::EVERY_EOF, everyeof);
    let mut input = InputStack::new(MemoryInput::new(OUTER));

    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("first pseudo-file token"),
        Some(char_token('A'))
    );
    let input_summary = input.summary();
    let state_snapshot = stores.snapshot();
    let first_tail = next_expanded_chars(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    );
    let first_hash = stores.snapshot().state_hash();

    stores.rollback(&state_snapshot);
    let mut restored = InputStack::from_summary(&input_summary, |_, _, source| {
        let full = if source.is_scantokens() {
            PSEUDO
        } else {
            OUTER
        };
        let remaining = &full[source.next_source_offset()..];
        Ok::<_, ()>(if source.is_scantokens() {
            MemoryInput::scantokens(remaining.to_owned())
        } else {
            MemoryInput::new(remaining)
        })
    })
    .expect("live scantokens input summary restores");
    let replay_tail = next_expanded_chars(
        &mut restored,
        &mut tex_state::ExpansionContext::new(&mut stores),
    );

    assert_eq!(replay_tail, first_tail);
    assert_eq!(stores.snapshot().state_hash(), first_hash);
}

#[test]
fn tracingscantokens_records_virtual_file_boundaries() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    stores.set_int_param(tex_state::env::banks::IntParam::TRACING_SCAN_TOKENS, 1);
    let mut input = InputStack::new(MemoryInput::new("\\scantokens{X}%"));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "X "
    );
    let trace = stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            tex_state::EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(trace, "( )");
}

#[test]
fn everyeof_is_inserted_at_natural_virtual_eof_but_not_endinput() {
    // e-TeX short reference manual section 3.7 requires natural real and
    // virtual EOF insertion, explicitly excluding EOF forced by \endinput.
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let everyeof = stores.intern_token_list(&[char_token('E')]);
    stores.set_tok_param(tex_state::env::banks::TokParam::EVERY_EOF, everyeof);

    let mut virtual_input = InputStack::new(MemoryInput::new("Z"));
    assert_eq!(
        get_x_token(
            &mut virtual_input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("source token"),
        Some(char_token('Z'))
    );
    assert_eq!(
        get_x_token(
            &mut virtual_input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("source endline"),
        Some(Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        })
    );
    assert_eq!(
        get_x_token(
            &mut virtual_input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("everyeof token"),
        Some(char_token('E'))
    );

    let mut forced = InputStack::new(MemoryInput::new("\\endinput Z"));
    assert_eq!(
        get_x_token(
            &mut forced,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("endinput line token"),
        Some(char_token('Z'))
    );
    assert_eq!(
        get_x_token(
            &mut forced,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("forced source endline"),
        Some(Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        })
    );
    assert_eq!(
        get_x_token(
            &mut forced,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("forced eof"),
        None
    );
}

#[test]
fn everyeof_is_visible_to_raw_scanners_before_the_outer_source() {
    let mut stores = Universe::new();
    let everyeof = stores.intern_token_list(&[char_token('E')]);
    stores.set_tok_param(tex_state::env::banks::TokParam::EVERY_EOF, everyeof);
    let mut input = InputStack::new(MemoryInput::new("O%"));
    input.push_source(MemoryInput::new("I%"));

    assert_eq!(
        crate::semantic_token(
            crate::next_semantic_raw_token(
                &mut input,
                &mut tex_state::ExpansionContext::new(&mut stores)
            )
            .expect("nested token")
            .expect("nested token present")
        ),
        char_token('I')
    );
    assert_eq!(
        crate::semantic_token(
            crate::next_semantic_raw_token(
                &mut input,
                &mut tex_state::ExpansionContext::new(&mut stores)
            )
            .expect("everyeof token")
            .expect("everyeof token present")
        ),
        char_token('E')
    );
    assert_eq!(
        crate::semantic_token(
            crate::next_semantic_raw_token(
                &mut input,
                &mut tex_state::ExpansionContext::new(&mut stores)
            )
            .expect("outer token")
            .expect("outer token present")
        ),
        char_token('O')
    );
}

#[test]
fn etex_version_and_revision_match_the_v2_reference() {
    // e-TeX short reference manual section 3.3 defines eTeXversion as an
    // internal read-only integer and eTeXrevision as catcode-12 text.
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\the\\eTeXversion\\eTeXrevision%"));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "2.6"
    );
}

#[test]
fn current_group_enquiries_read_exact_state_markers() {
    // e-TeX short reference manual section 3.3 defines level as the live
    // nesting depth and type as the documented 0..16 group classification.
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    stores.enter_group_with_kind(tex_state::GroupKind::HBox);
    stores.enter_group_with_kind(tex_state::GroupKind::SemiSimple);
    let mut input = InputStack::new(MemoryInput::new(
        "\\number\\currentgrouplevel,\\number\\currentgrouptype%",
    ));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "2,14"
    );
}

#[test]
fn current_if_enquiries_report_level_type_branch_and_unless_sign() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\iftrue\\number\\currentiflevel,\\number\\currentiftype,\\number\\currentifbranch;\\fi\
         \\unless\\iffalse\\number\\currentiflevel,\\number\\currentiftype,\\number\\currentifbranch;\\fi\
         \\iffalse X\\else\\number\\currentiflevel,\\number\\currentiftype,\\number\\currentifbranch;\\fi%",
    ));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "1,15,1;1,-16,1;1,16,-1;"
    );
}

#[test]
fn current_if_enquiries_follow_manual_type_and_branch_codes() {
    // e-TeX short reference manual section 3.3: level is conditional depth,
    // type is negated under \unless, and branch is 1/0/-1 for an available
    // alternative, operand evaluation, or a final branch respectively.
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\the\\currentiflevel,\\the\\currentiftype,\\the\\currentifbranch;\
         \\iftrue\\the\\currentiflevel,\\the\\currentiftype,\\the\\currentifbranch\\fi;\
         \\unless\\iftrue X\\else\\the\\currentiflevel,\\the\\currentiftype,\\the\\currentifbranch\\fi%",
    ));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "0,0,0;1,15,1;1,-15,-1"
    );
}

#[test]
fn ifdefined_and_ifcsname_test_without_creating_missing_names() {
    // e-TeX short reference manual section 3.3 requires \ifcsname to avoid
    // both hash-table creation and the \relax side effect of ordinary \csname.
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let known = stores.intern("known");
    stores.set_meaning(known, Meaning::Relax);
    assert!(stores.symbol("nevercreated").is_none());
    let mut input = InputStack::new(MemoryInput::new(
        "\\ifdefined\\known T\\else F\\fi\
         \\unless\\ifdefined\\missing T\\else F\\fi\
         \\ifcsname known\\endcsname T\\else F\\fi\
         \\unless\\ifcsname nevercreated\\endcsname T\\else F\\fi%",
    ));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "TTTT"
    );
    assert!(stores.symbol("nevercreated").is_none());
}

#[test]
fn failed_ifcsname_scan_does_not_leak_an_evaluating_condition() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\ifcsname missing"));

    let error = crate::get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect_err("unterminated ifcsname must fail");

    assert!(matches!(error, crate::ExpandError::MissingEndCsName { .. }));
    assert_eq!(input.condition_depth(), 0);
}

#[test]
fn ifincsname_tracks_only_live_csname_scans() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\ifincsname A\\else B\\fi\
         \\csname a\\ifincsname T\\else F\\fi b\\endcsname\
         \\ifincsname C\\else D\\fi%",
    ));

    let expanded = collect_expanded(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    );
    let generated = stores
        .symbol("aTb")
        .expect("the live csname scan should select its true branch");
    assert_eq!(
        expanded,
        vec![
            char_token('B'),
            Token::Cs(generated.symbol()),
            char_token('D')
        ]
    );
    assert_eq!(stores.meaning(generated), Meaning::Relax);
}

#[test]
fn expansion_error_captures_invocation_chain_before_macro_frame_pops() {
    let mut stores = Universe::new();
    let macro_cs = stores.intern("m");
    let missing = stores.intern("missing");
    let body = stores.intern_token_list(&[Token::Cs(missing.symbol())]);
    let params = stores.intern_token_list(&[]);
    let definition_origin = stores.source_origin(tex_state::SourceId::new(7), 10, 1, 10);
    let body_origin = stores.source_origin(tex_state::SourceId::new(7), 12, 1, 12);
    let body_origins = stores.allocate_origin_list(&[body_origin]);
    stores.set_macro_meaning_with_provenance(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        MacroDefinitionProvenance::new(
            definition_origin,
            tex_state::ids::OriginListId::EMPTY,
            body_origins,
        ),
    );
    let call = stores.intern_token_list(&[Token::Cs(macro_cs.symbol())]);
    let call_origin = stores.source_origin(tex_state::SourceId::new(8), 1, 1, 1);
    let call_origins = stores.allocate_origin_list(&[call_origin]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list_with_origins(call, call_origins, TokenListReplayKind::Inserted);

    let error = crate::get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect_err("undefined body token must diagnose");
    let site = error.diagnostic_site();
    assert_eq!(site.primary_origin(), Some(body_origin));
    let expansion_head = site.expansion_head().expect("macro expansion head");

    assert!(
        input
            .next_traced_token(&mut tex_state::ExpansionContext::new(&mut stores))
            .expect("frame pop")
            .is_none()
    );
    assert_eq!(
        error.diagnostic_site().expansion_head(),
        Some(expansion_head)
    );
}

#[test]
fn macro_replay_without_definition_provenance_degrades_to_unknown_origins() {
    let mut stores = Universe::new();
    let macro_cs = stores.intern("memoized");
    let body_token = Token::Char {
        ch: 'z',
        cat: Catcode::Letter,
    };
    let body = stores.intern_token_list(&[body_token]);
    let params = stores.intern_token_list(&[]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
    );
    let invocation = stores.intern_token_list(&[Token::Cs(macro_cs.symbol())]);
    let call_origin = stores.source_origin(tex_state::SourceId::new(12), 90, 9, 1);
    let invocation_origins = stores.allocate_origin_list(&[call_origin]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list_with_origins(
        invocation,
        invocation_origins,
        TokenListReplayKind::Inserted,
    );

    let expanded = crate::get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect("expansion should not fail without side-table provenance")
    .expect("macro body token");

    assert_eq!(crate::semantic_token(expanded), body_token);
    assert_eq!(expanded.origin(), OriginId::UNKNOWN);
}

#[test]
fn recorder_observes_one_meaning_read_per_control_sequence_token() {
    let mut stores = Universe::new();
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
    let list = stores.intern_token_list(&[Token::Cs(relax.symbol())]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);
    let mut recorder = CountingRecorder::default();

    assert_eq!(
        get_x_token_recording(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
            &mut recorder
        )
        .expect("expansion should succeed"),
        Some(Token::Cs(relax.symbol()))
    );
    assert_eq!(recorder.reads, 1);
}

#[test]
fn expandafter_expands_second_token_then_replays_saved_token_first() {
    let mut stores = Universe::new();
    let expandafter =
        expandable_primitive(&mut stores, "expandafter", ExpandablePrimitive::ExpandAfter);
    let macro_cs = stores.intern("m");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x'), char_token('y')]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
    );

    let input_list = stores.intern_token_list(&[
        Token::Cs(expandafter.symbol()),
        char_token('a'),
        Token::Cs(macro_cs.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(input_list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "axy"
    );
}

#[test]
fn restricted_expandafter_expands_a_historical_unexpanded_target() {
    let mut stores = Universe::new();
    let macro_cs = stores.intern("m");
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x')]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, body),
    );
    let target_token = Token::Cs(macro_cs.symbol());
    let target_origin = stores.inserted_origin(
        InsertedOriginKind::Unexpanded,
        target_token,
        OriginId::UNKNOWN,
    );
    let target = TracedTokenWord::pack(target_token, target_origin);
    let saved = TracedTokenWord::pack(char_token('a'), OriginId::UNKNOWN);
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut expansion = ExpansionContext::new("texput");

    RestrictedExpansionMode
        .dispatch_raw_token_after(
            saved,
            false,
            target,
            false,
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
            &mut expansion,
        )
        .expect("restricted expandafter");

    let mut context = tex_state::ExpansionContext::new(&mut stores);
    assert_eq!(
        get_x_token(&mut input, &mut context).expect("saved token"),
        Some(char_token('a'))
    );
    assert_eq!(
        get_x_token(&mut input, &mut context).expect("expanded target"),
        Some(char_token('x'))
    );
}

#[test]
fn expandafter_preserves_structural_noexpand_without_provenance() {
    let mut stores = Universe::new();
    let macro_cs = stores.intern("m");
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x')]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, empty, body),
    );
    let target = TracedTokenWord::pack(Token::Cs(macro_cs.symbol()), OriginId::UNKNOWN);
    let saved = TracedTokenWord::pack(char_token('a'), OriginId::UNKNOWN);
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut expansion = ExpansionContext::new("texput");

    RestrictedExpansionMode
        .dispatch_raw_token_after(
            saved,
            false,
            target,
            true,
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
            &mut expansion,
        )
        .expect("structurally suppressed expandafter target");

    let mut context = tex_state::ExpansionContext::new(&mut stores);
    assert_eq!(
        get_x_token(&mut input, &mut context).expect("saved token"),
        Some(char_token('a'))
    );
    assert_eq!(
        get_x_token(&mut input, &mut context).expect("suppressed target"),
        Some(Token::Cs(macro_cs.symbol())),
    );

    let saved = TracedTokenWord::pack(Token::Cs(macro_cs.symbol()), OriginId::UNKNOWN);
    let target = TracedTokenWord::pack(char_token('b'), OriginId::UNKNOWN);
    let mut input = InputStack::new(MemoryInput::new(""));
    RestrictedExpansionMode
        .dispatch_raw_token_after(
            saved,
            true,
            target,
            false,
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
            &mut expansion,
        )
        .expect("structurally suppressed saved token");
    let mut context = tex_state::ExpansionContext::new(&mut stores);
    assert_eq!(
        get_x_token(&mut input, &mut context).expect("suppressed saved token"),
        Some(Token::Cs(macro_cs.symbol())),
    );
    assert_eq!(
        get_x_token(&mut input, &mut context).expect("target token"),
        Some(char_token('b')),
    );
}

#[test]
fn expandafter_saved_brace_updates_alignment_depth_only_when_replayed() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    let empty = stores.intern_token_list(&[]);
    let mut input = InputStack::new(MemoryInput::new("\\expandafter{\\romannumeral0}"));
    input.begin_alignment();
    input.begin_alignment_cell(None, empty, stores.execution_group_depth());

    let mut context = tex_state::ExpansionContext::new(&mut stores);
    assert_eq!(
        get_x_token(&mut input, &mut context).expect("opening brace expands"),
        Some(Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        }),
    );
    assert_eq!(
        get_x_token(&mut input, &mut context).expect("closing brace expands"),
        Some(Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        }),
    );
    assert!(input.alignment_cell_at_base_depth());
}

#[test]
fn expandafter_chains_match_tex_pushback_order() {
    let mut stores = Universe::new();
    let expandafter =
        expandable_primitive(&mut stores, "expandafter", ExpandablePrimitive::ExpandAfter);
    let first = stores.intern("first");
    let second = stores.intern("second");
    let params = stores.intern_token_list(&[]);
    let first_body = stores.intern_token_list(&[char_token('1')]);
    let second_body = stores.intern_token_list(&[char_token('2')]);
    stores.set_macro_meaning(
        first,
        MacroMeaning::new(MeaningFlags::EMPTY, params, first_body),
    );
    stores.set_macro_meaning(
        second,
        MacroMeaning::new(MeaningFlags::EMPTY, params, second_body),
    );

    let input_list = stores.intern_token_list(&[
        Token::Cs(expandafter.symbol()),
        Token::Cs(expandafter.symbol()),
        Token::Cs(expandafter.symbol()),
        Token::Cs(first.symbol()),
        Token::Cs(second.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(input_list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "12"
    );
}

#[test]
fn noexpand_suppresses_next_control_sequence_for_one_get_x_token() {
    let mut stores = Universe::new();
    let noexpand = expandable_primitive(&mut stores, "noexpand", ExpandablePrimitive::NoExpand);
    let macro_cs = stores.intern("m");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x')]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
    );
    let input_list = stores.intern_token_list(&[
        Token::Cs(noexpand.symbol()),
        Token::Cs(macro_cs.symbol()),
        Token::Cs(macro_cs.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(input_list, TokenListReplayKind::Inserted);

    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("expansion should succeed"),
        Some(Token::Cs(macro_cs.symbol()))
    );
    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("expansion should succeed"),
        Some(char_token('x'))
    );
}

#[test]
fn noexpand_delivers_inserted_origin_for_suppressed_token() {
    let mut stores = Universe::new();
    let noexpand = expandable_primitive(&mut stores, "noexpand", ExpandablePrimitive::NoExpand);
    let relax = stores.intern_relaxed_control_sequence("relax");
    let noexpand_origin = stores.source_origin(tex_state::SourceId::new(20), 100, 10, 1);
    let target_origin = stores.source_origin(tex_state::SourceId::new(20), 110, 10, 11);
    let input_list =
        stores.intern_token_list(&[Token::Cs(noexpand.symbol()), Token::Cs(relax.symbol())]);
    let origins = stores.allocate_origin_list(&[noexpand_origin, target_origin]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list_with_origins(input_list, origins, TokenListReplayKind::Inserted);

    let traced = crate::get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect("noexpand should succeed")
    .expect("suppressed token should be delivered");

    assert_eq!(traced.token(), Some(Token::Cs(relax.symbol())));
    assert_eq!(
        stores.origin(traced.origin()),
        OriginRecord::Inserted(InsertedOrigin::new(
            InsertedOriginKind::NoExpand,
            Token::Cs(relax.symbol()),
            target_origin,
        ))
    );
}

#[test]
fn expandafter_preserves_noexpand_for_later_frame_step() {
    let mut stores = Universe::new();
    let expandafter =
        expandable_primitive(&mut stores, "expandafter", ExpandablePrimitive::ExpandAfter);
    let noexpand = expandable_primitive(&mut stores, "noexpand", ExpandablePrimitive::NoExpand);
    let macro_cs = stores.intern("m");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x')]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
    );
    let input_list = stores.intern_token_list(&[
        Token::Cs(expandafter.symbol()),
        char_token('a'),
        Token::Cs(noexpand.symbol()),
        Token::Cs(macro_cs.symbol()),
        Token::Cs(macro_cs.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(input_list, TokenListReplayKind::Inserted);

    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("expansion should succeed"),
        Some(char_token('a'))
    );
    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("expansion should succeed"),
        Some(Token::Cs(macro_cs.symbol()))
    );
    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("expansion should succeed"),
        Some(char_token('x'))
    );
}

#[test]
fn csname_interns_undefined_name_and_assigns_relax() {
    let mut stores = Universe::new();
    let (csname, endcsname) = csname_primitives(&mut stores);
    let input_list = stores.intern_token_list(&[
        Token::Cs(csname.symbol()),
        char_token('f'),
        char_token('o'),
        char_token('o'),
        Token::Cs(endcsname.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(input_list, TokenListReplayKind::Inserted);

    let created = stores.symbol("foo");
    assert!(created.is_none());
    let token = get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect("csname expansion should succeed")
    .expect("csname should emit a token");
    let Token::Cs(created) = token else {
        panic!("expected control sequence, got {token:?}");
    };

    assert_eq!(stores.resolve(created), "foo");
    assert_eq!(stores.meaning(created), Meaning::Relax);
}

#[test]
fn csname_expands_name_pieces_before_interning() {
    let mut stores = Universe::new();
    let (csname, endcsname) = csname_primitives(&mut stores);
    let macro_cs = stores.intern("piece");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('b'), char_token('a'), char_token('r')]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
    );
    let input_list = stores.intern_token_list(&[
        Token::Cs(csname.symbol()),
        char_token('f'),
        Token::Cs(macro_cs.symbol()),
        Token::Cs(endcsname.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(input_list, TokenListReplayKind::Inserted);

    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("csname expansion should succeed"),
        Some(Token::Cs(
            stores
                .symbol("fbar")
                .expect("expanded name should be interned")
                .symbol()
        ))
    );
}

#[test]
fn csname_reexpands_a_macro_result_with_synthesized_provenance() {
    let mut stores = Universe::new();
    let (csname, endcsname) = csname_primitives(&mut stores);
    let false_macro = stores.intern("us@false");
    let let_cs = stores.intern("let");
    stores.set_meaning(
        let_cs,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Let),
    );

    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[Token::Cs(let_cs.symbol())]);
    let definition_origin = stores.source_origin(tex_state::SourceId::new(25), 40, 4, 1);
    let body_origins = stores.allocate_repeated_origin_list(definition_origin, 1);
    stores.set_macro_meaning_with_provenance(
        false_macro,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        MacroDefinitionProvenance::new(
            definition_origin,
            tex_state::ids::OriginListId::EMPTY,
            body_origins,
        ),
    );
    let Meaning::Macro { definition, .. } = stores.meaning(false_macro) else {
        panic!("expected macro meaning");
    };

    let input_tokens = [
        Token::Cs(csname.symbol()),
        char_token('u'),
        char_token('s'),
        char_token('@'),
        char_token('f'),
        char_token('a'),
        char_token('l'),
        char_token('s'),
        char_token('e'),
        Token::Cs(endcsname.symbol()),
    ];
    let csname_origin = stores.source_origin(tex_state::SourceId::new(26), 80, 8, 1);
    let origins = stores.allocate_repeated_origin_list(csname_origin, input_tokens.len());
    let input_list = stores.intern_token_list(&input_tokens);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list_with_origins(input_list, origins, TokenListReplayKind::Inserted);

    let delivered = crate::get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect("csname macro result should expand")
    .expect("macro body should deliver its unexpandable token");
    assert_eq!(delivered.token(), Some(Token::Cs(let_cs.symbol())));

    let summary = input.summary();
    let Some(tex_lex::InputFrameSummary::TokenList {
        macro_invocation, ..
    }) = summary.frames().last()
    else {
        panic!("macro body should remain on the input stack");
    };
    let OriginRecord::MacroInvocation(invocation) = stores.origin(*macro_invocation) else {
        panic!("macro body should retain an invocation origin");
    };
    assert_eq!(invocation.definition(), definition);
    assert_eq!(
        stores.origin(invocation.invocation()),
        OriginRecord::Synthesized(SynthesizedOrigin::new(
            SynthesizedOriginKind::Expansion,
            csname_origin,
        ))
    );
}

#[test]
fn csname_recovers_from_non_character_material_after_expansion() {
    let mut stores = Universe::new();
    let (csname, endcsname) = csname_primitives(&mut stores);
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
    let input_list = stores.intern_token_list(&[
        Token::Cs(csname.symbol()),
        Token::Cs(relax.symbol()),
        Token::Cs(endcsname.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(input_list, TokenListReplayKind::Inserted);

    let Some(Token::Cs(empty)) = get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect("csname recovery should succeed") else {
        panic!("expected recovered empty control sequence");
    };
    assert_eq!(stores.resolve(empty), "");
    assert_eq!(stores.meaning(empty), Meaning::Relax);

    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("pushed-back token should expand"),
        Some(Token::Cs(relax.symbol()))
    );
    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("remaining endcsname should be delivered"),
        Some(Token::Cs(endcsname.symbol()))
    );
}

#[test]
fn csname_preserves_existing_meaning_for_ifx_relax_comparison() {
    let mut stores = Universe::new();
    let (csname, endcsname) = csname_primitives(&mut stores);
    let existing = stores.intern("known");
    stores.set_meaning(existing, Meaning::CharGiven('K'));
    let input_list = stores.intern_token_list(&[
        Token::Cs(csname.symbol()),
        char_token('k'),
        char_token('n'),
        char_token('o'),
        char_token('w'),
        char_token('n'),
        Token::Cs(endcsname.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(input_list, TokenListReplayKind::Inserted);

    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("csname expansion should succeed"),
        Some(Token::Cs(existing.symbol()))
    );
    assert_eq!(stores.meaning(existing), Meaning::CharGiven('K'));
}

#[test]
fn csname_created_undefined_name_is_meaning_equal_to_relax() {
    let mut stores = Universe::new();
    let (csname, endcsname) = csname_primitives(&mut stores);
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
    let input_list = stores.intern_token_list(&[
        Token::Cs(csname.symbol()),
        char_token('n'),
        char_token('e'),
        char_token('w'),
        Token::Cs(endcsname.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(input_list, TokenListReplayKind::Inserted);

    let Some(Token::Cs(created)) = get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect("csname expansion should succeed") else {
        panic!("expected created control sequence");
    };

    assert_eq!(stores.meaning(created), stores.meaning(relax));
}

#[test]
fn macro_body_replay_substitutes_frozen_argument_lists() {
    let mut stores = Universe::new();
    let macro_cs = stores.intern("m");
    let params = stores.intern_token_list(&[Token::param(1)]);
    let body = stores.intern_token_list(&[char_token('a'), Token::param(1), char_token('b')]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
    );
    let invocation = stores.intern_token_list(&[
        Token::Cs(macro_cs.symbol()),
        char_token('{'),
        char_token('x'),
        char_token('y'),
        char_token('}'),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(invocation, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "axyb"
    );
}

#[test]
fn macro_argument_replay_delivers_call_site_argument_origins() {
    let mut stores = Universe::new();
    let macro_cs = stores.intern("m");
    let params = stores.intern_token_list(&[Token::param(1)]);
    let body = stores.intern_token_list(&[Token::param(1)]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
    );
    let argument_origin = stores.source_origin(tex_state::SourceId::new(9), 70, 7, 5);
    let invocation = stores.intern_token_list(&[char_token('x')]);
    let invocation_origins = stores.allocate_origin_list(&[argument_origin]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list_with_origins(
        invocation,
        invocation_origins,
        TokenListReplayKind::Inserted,
    );
    let meaning = stores.meaning(macro_cs);
    let crate::Dispatch::Push {
        token_list,
        origin_list,
        macro_arguments,
        macro_invocation,
        ..
    } = dispatch(
        Token::Cs(macro_cs.symbol()),
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        meaning,
    )
    .expect("macro dispatch should succeed")
    else {
        panic!("expected macro body push");
    };
    input.push_macro_body_with_origins_and_invocation(
        token_list,
        origin_list,
        macro_arguments,
        macro_invocation,
    );

    let replayed = input
        .next_traced_token(&mut tex_state::ExpansionContext::new(&mut stores))
        .expect("replay should succeed")
        .expect("argument token should replay");

    assert_eq!(replayed.token(), Some(char_token('x')));
    assert_eq!(replayed.origin(), argument_origin);
}

#[test]
fn macro_body_delivery_does_not_write_provenance_per_token() {
    let mut stores = Universe::new();
    let macro_cs = stores.intern("m");
    let params = stores.intern_token_list(&[]);
    let body_tokens = [char_token('a'), char_token('b'), char_token('c')];
    let body = stores.intern_token_list(&body_tokens);
    let definition_origin = stores.source_origin(tex_state::SourceId::new(1), 0, 1, 1);
    let body_origins = stores.allocate_repeated_origin_list(definition_origin, body_tokens.len());
    stores.set_macro_meaning_with_provenance(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
        MacroDefinitionProvenance::new(
            definition_origin,
            tex_state::ids::OriginListId::EMPTY,
            body_origins,
        ),
    );
    let invocation_origin = stores.source_origin(tex_state::SourceId::new(1), 10, 1, 11);
    let mut input = InputStack::new(MemoryInput::new(""));
    let meaning = stores.meaning(macro_cs);
    let crate::Dispatch::Push {
        token_list,
        origin_list,
        macro_arguments,
        macro_invocation,
        ..
    } = dispatch_with_context(
        Token::Cs(macro_cs.symbol()),
        invocation_origin,
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        &mut ExpansionContext::new("texput"),
        meaning,
    )
    .expect("macro dispatch should succeed")
    else {
        panic!("expected macro body push");
    };
    input.push_macro_body_with_origins_and_invocation(
        token_list,
        origin_list,
        macro_arguments,
        macro_invocation,
    );
    let after_dispatch = stores.provenance_stats();

    assert_eq!(
        collect_expanded(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        body_tokens.to_vec()
    );
    let after_delivery = stores.provenance_stats();
    assert_eq!(
        after_delivery.origin_records(),
        after_dispatch.origin_records()
    );
    assert_eq!(
        after_delivery.origin_list_spans(),
        after_dispatch.origin_list_spans()
    );
    assert_eq!(
        after_delivery.origin_list_entries(),
        after_dispatch.origin_list_entries()
    );
}

#[test]
fn generated_value_tokens_share_one_synthesized_origin_record() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    let number = stores.symbol("number").expect("number primitive");
    let input_tokens = [
        Token::Cs(number.symbol()),
        char_token('1'),
        char_token('2'),
        char_token('3'),
        char_token('4'),
    ];
    let input = stores.intern_token_list(&input_tokens);
    let call_origin = stores.source_origin(tex_state::SourceId::new(4), 40, 4, 1);
    let input_origins = stores.allocate_repeated_origin_list(call_origin, input_tokens.len());
    let mut input_stack = InputStack::new(MemoryInput::new(""));
    input_stack.push_token_list_with_origins(input, input_origins, TokenListReplayKind::Inserted);
    let before = stores.provenance_stats();

    assert_eq!(
        collect_expanded(
            &mut input_stack,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        vec![
            char_token('1'),
            char_token('2'),
            char_token('3'),
            char_token('4')
        ]
    );
    let growth = stores.provenance_stats().saturating_sub(before);

    assert_eq!(growth.origin_records(), 1);
    assert_eq!(growth.origin_list_spans(), 0);
    assert_eq!(growth.origin_list_entries(), 0);
}

#[test]
fn nested_macro_calls_replay_arguments_from_outer_frozen_frame() {
    let mut stores = Universe::new();
    let wrap = stores.intern("wrap");
    let wrap_params = stores.intern_token_list(&[Token::param(1)]);
    let wrap_body = stores.intern_token_list(&[char_token('['), Token::param(1), char_token(']')]);
    stores.set_macro_meaning(
        wrap,
        MacroMeaning::new(MeaningFlags::EMPTY, wrap_params, wrap_body),
    );

    let outer = stores.intern("outer");
    let outer_params = stores.intern_token_list(&[Token::param(1)]);
    let outer_body = stores.intern_token_list(&[
        Token::Cs(wrap.symbol()),
        char_token('{'),
        Token::param(1),
        char_token('}'),
    ]);
    stores.set_macro_meaning(
        outer,
        MacroMeaning::new(MeaningFlags::EMPTY, outer_params, outer_body),
    );

    let invocation = stores.intern_token_list(&[
        Token::Cs(outer.symbol()),
        char_token('{'),
        char_token('x'),
        char_token('y'),
        char_token('}'),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(invocation, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "[xy]"
    );
}

#[test]
fn identical_macro_bodies_keep_shared_body_identity_with_distinct_arguments() {
    let mut stores = Universe::new();
    let left = stores.intern("left");
    let right = stores.intern("right");
    let params = stores.intern_token_list(&[Token::param(1)]);
    let first_body = stores.intern_token_list(&[Token::param(1), char_token('!')]);
    let second_body = stores.intern_token_list(&[Token::param(1), char_token('!')]);
    assert_eq!(first_body, second_body);
    stores.set_macro_meaning(
        left,
        MacroMeaning::new(MeaningFlags::EMPTY, params, first_body),
    );
    stores.set_macro_meaning(
        right,
        MacroMeaning::new(MeaningFlags::EMPTY, params, second_body),
    );

    let left_arg = stores.intern_token_list(&[char_token('x')]);
    let mut left_input = InputStack::new(MemoryInput::new(""));
    left_input.push_token_list(left_arg, TokenListReplayKind::Inserted);
    let left_meaning = stores.meaning(left);
    let left_dispatch = dispatch(
        Token::Cs(left.symbol()),
        &mut left_input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        left_meaning,
    )
    .expect("left dispatch should succeed");
    let crate::Dispatch::Push {
        token_list: left_body,
        macro_arguments: left_arguments,
        ..
    } = left_dispatch
    else {
        panic!("expected left macro body push");
    };
    assert_eq!(left_body, first_body);
    assert_eq!(
        left_arguments.get(1).expect("left #1")[0].token(),
        Some(char_token('x'))
    );

    let right_arg = stores.intern_token_list(&[char_token('y')]);
    let mut right_input = InputStack::new(MemoryInput::new(""));
    right_input.push_token_list(right_arg, TokenListReplayKind::Inserted);
    let right_meaning = stores.meaning(right);
    let right_dispatch = dispatch(
        Token::Cs(right.symbol()),
        &mut right_input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        right_meaning,
    )
    .expect("right dispatch should succeed");
    let crate::Dispatch::Push {
        token_list: right_body,
        macro_arguments: right_arguments,
        ..
    } = right_dispatch
    else {
        panic!("expected right macro body push");
    };
    assert_eq!(right_body, second_body);
    assert_eq!(
        right_arguments.get(1).expect("right #1")[0].token(),
        Some(char_token('y'))
    );

    let invocation = stores.intern_token_list(&[
        Token::Cs(left.symbol()),
        char_token('x'),
        Token::Cs(right.symbol()),
        char_token('y'),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(invocation, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "x!y!"
    );
}

#[test]
fn string_respects_escapechar_and_renders_other_catcodes() {
    let mut stores = Universe::new();
    let string = expandable_primitive(&mut stores, "string", ExpandablePrimitive::String);
    let target = stores.intern("foo");
    let list = stores.intern_token_list(&[
        Token::Cs(string.symbol()),
        Token::Cs(target.symbol()),
        Token::Cs(string.symbol()),
        Token::Char {
            ch: 'a',
            cat: Catcode::Letter,
        },
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        collect_expanded(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        vec![
            Token::Char {
                ch: '\\',
                cat: Catcode::Other
            },
            Token::Char {
                ch: 'f',
                cat: Catcode::Other
            },
            Token::Char {
                ch: 'o',
                cat: Catcode::Other
            },
            Token::Char {
                ch: 'o',
                cat: Catcode::Other
            },
            Token::Char {
                ch: 'a',
                cat: Catcode::Other
            },
        ]
    );
}

#[test]
fn string_omits_invalid_escapechar() {
    let mut stores = Universe::new();
    stores.set_int_param(tex_state::env::banks::IntParam::ESCAPE_CHAR, -1);
    let string = expandable_primitive(&mut stores, "string", ExpandablePrimitive::String);
    let target = stores.intern("foo");
    let list = stores.intern_token_list(&[Token::Cs(string.symbol()), Token::Cs(target.symbol())]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "foo"
    );
}

#[test]
fn token_show_text_matches_tex_print_cs_classes() {
    let mut stores = Universe::new();
    let multiletter = stores.intern("foo");
    let multiother = stores.intern("@@");
    let single = stores.intern("@");
    let empty = stores.intern("");
    let active = stores.intern_active_character('~');

    let render = |stores: &Universe, token| {
        let mut text = String::new();
        crate::append_token_show_text(stores, token, &mut text);
        text
    };

    assert_eq!(render(&stores, Token::Cs(multiletter.symbol())), "\\foo ");
    assert_eq!(render(&stores, Token::Cs(multiother.symbol())), "\\@@ ");
    assert_eq!(render(&stores, Token::Cs(single.symbol())), "\\@");
    assert_eq!(
        render(&stores, Token::Cs(empty.symbol())),
        "\\csname\\endcsname "
    );
    assert_eq!(render(&stores, Token::Cs(active.symbol())), "~");

    stores.set_catcode('@', Catcode::Letter);
    assert_eq!(render(&stores, Token::Cs(single.symbol())), "\\@ ");
}

#[test]
fn number_and_romannumeral_scan_expanded_integer_edge_cases() {
    let mut stores = Universe::new();
    let number = expandable_primitive(&mut stores, "number", ExpandablePrimitive::Number);
    let roman = expandable_primitive(
        &mut stores,
        "romannumeral",
        ExpandablePrimitive::RomanNumeral,
    );
    let digits = stores.intern("digits");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('1'), char_token('9')]);
    stores.set_macro_meaning(digits, MacroMeaning::new(MeaningFlags::EMPTY, params, body));
    let list = stores.intern_token_list(&[
        Token::Cs(number.symbol()),
        Token::Char {
            ch: '-',
            cat: Catcode::Other,
        },
        Token::Cs(digits.symbol()),
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
        Token::Cs(roman.symbol()),
        Token::Char {
            ch: '0',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
        Token::Cs(roman.symbol()),
        Token::Char {
            ch: '4',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: '0',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: '0',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: '0',
            cat: Catcode::Other,
        },
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "-19mmmm"
    );
}

#[test]
fn number_renders_a_nested_numexpr_and_consumes_its_relax() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    let numexpr = stores.intern("numexpr");
    stores.set_meaning(
        numexpr,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NumExpr),
    );
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
    let mut input = InputStack::new(MemoryInput::new("\\number\\numexpr0+85\\relax%"));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "85"
    );

    let mut input = InputStack::new(MemoryInput::new("\\the\\numexpr0+85\\relax%"));
    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "85"
    );
}

#[test]
fn the_renders_assignable_registers_parameters_and_code_tables() {
    let mut stores = Universe::new();
    expandable_primitive(&mut stores, "the", ExpandablePrimitive::The);
    let count = stores.intern("count");
    stores.set_meaning(
        count,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count),
    );
    let catcode = stores.intern("catcode");
    stores.set_meaning(
        catcode,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CatCode),
    );
    let foo = stores.intern("foo");
    stores.set_meaning(foo, Meaning::CountRegister(300));
    stores.set_count(300, 42);
    stores.set_int_param_global(tex_state::env::banks::IntParam::ETEX_EXTENDED_MODE, 1);
    let parskip = stores.intern("parskip");
    stores.set_meaning(parskip, Meaning::GlueParam(2));
    let thinmuskip = stores.intern("thinmuskip");
    stores.set_meaning(thinmuskip, Meaning::MuGlueParam(15));
    let glue = stores.intern_glue(GlueSpec {
        width: Scaled::from_raw(Scaled::UNITY),
        stretch: Scaled::from_raw(2 * Scaled::UNITY),
        stretch_order: Order::Fil,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
    });
    stores.set_glue_param(tex_state::env::banks::GlueParam::new(2), glue);
    let muglue = stores.intern_glue(GlueSpec {
        width: Scaled::from_raw(3 * Scaled::UNITY),
        stretch: Scaled::from_raw(4 * Scaled::UNITY),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(5 * Scaled::UNITY),
        shrink_order: Order::Normal,
    });
    stores.set_glue_param(tex_state::env::banks::GlueParam::new(15), muglue);
    let everypar = stores.intern("everypar");
    stores.set_meaning(everypar, Meaning::TokParam(1));
    let hi = stores.intern_token_list(&[char_token('h'), char_token('i')]);
    stores.set_tok_param(tex_state::env::banks::TokParam::new(1), hi);
    let mut input = InputStack::new(MemoryInput::new(
        "\\the\\count300 \\the\\foo \\the\\parskip \\the\\thinmuskip \\the\\everypar \\the\\catcode`x",
    ));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "42421.0pt plus 2.0fil3.0mu plus 4.0mu minus 5.0muhi11"
    );
}

#[test]
fn the_recovers_non_internal_target_with_integer_zero() {
    let mut stores = Universe::new();
    expandable_primitive(&mut stores, "the", ExpandablePrimitive::The);
    let active = stores.intern_active_character('~');
    stores.set_meaning(active, Meaning::CountRegister(7));
    stores.set_catcode('~', Catcode::Active);
    stores.set_count(7, 42);
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
    let mut input = InputStack::new(MemoryInput::new("\\the e\\the\\relax\\the~%"));
    let mut expansion = ExpansionContext::new("texput");
    let mut output = String::new();

    while let Some(token) = crate::get_x_token_with_context(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        &mut expansion,
    )
    .expect("invalid the target recovers")
    {
        if let Token::Char { ch, .. } = semantic_token(token) {
            output.push(ch);
        }
    }

    assert_eq!(output, "0042");
    let diagnostics = expansion.take_recoverable_diagnostics().collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 2);
    assert!(matches!(
        diagnostics[0],
        crate::RecoverableExpansionDiagnostic::InvalidTheTarget { context }
            if crate::semantic_token(context) == char_token('e')
    ));
    assert!(matches!(
        diagnostics[1],
        crate::RecoverableExpansionDiagnostic::InvalidTheTarget { context }
            if crate::semantic_token(context) == Token::Cs(relax.symbol())
    ));
}

#[test]
fn the_records_exact_code_dependencies_that_table_mutations_invalidate() {
    let mut stores = Universe::new();
    expandable_primitive(&mut stores, "the", ExpandablePrimitive::The);
    let catcode = stores.intern("catcode");
    stores.set_meaning(
        catcode,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CatCode),
    );
    let value = stores.intern("value");
    stores.set_meaning(value, Meaning::CountRegister(7));
    stores.set_count(7, 41);
    let mut input = InputStack::new(MemoryInput::new("\\the\\value \\the\\catcode`x"));
    let mut reads = crate::ReadSetRecorder::default();
    while get_x_token_recording(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        &mut reads,
    )
    .expect("recorded expansion")
    .is_some()
    {}

    let dependencies = reads.dependencies().collect::<Vec<_>>();
    assert!(dependencies.contains(&crate::ReadDependency::Cell {
        bank: crate::ReadBank::Count,
        index: 7,
    }));
    assert!(
        !dependencies.contains(&crate::ReadDependency::CodeGeneration(
            crate::ReadCodeTable::Catcode,
        ))
    );
    assert!(dependencies.contains(&crate::ReadDependency::Code {
        table: crate::ReadCodeTable::Catcode,
        scalar: 'x' as u32,
    }));

    stores.set_count(7, 42);
    stores.set_catcode('x', Catcode::Active);
    assert_eq!(stores.count(7), 42);
    assert_eq!(stores.catcode('x'), Catcode::Active);
}

#[test]
fn number_scanner_preserves_session_context_during_nested_expansion() {
    let mut stores = Universe::new();
    let number = expandable_primitive(&mut stores, "number", ExpandablePrimitive::Number);
    let input_primitive = expandable_primitive(&mut stores, "input", ExpandablePrimitive::Input);
    let digits = stores.intern("digits");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[
        Token::Cs(input_primitive.symbol()),
        char_token('d'),
        char_token('i'),
        char_token('g'),
        char_token('s'),
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
    ]);
    stores.set_macro_meaning(digits, MacroMeaning::new(MeaningFlags::EMPTY, params, body));
    let list = stores.intern_token_list(&[Token::Cs(number.symbol()), Token::Cs(digits.symbol())]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);
    let mut context = MemoryResolverFixture::new("job").with_source("digs", "42");

    assert_eq!(
        next_expanded_chars_with_context(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
            &mut context
        ),
        "42"
    );
    assert_eq!(context.resolver.opened, vec!["digs"]);
}

#[test]
fn meaning_renders_macro_text_and_output_catcodes() {
    let mut stores = Universe::new();
    let meaning = expandable_primitive(&mut stores, "meaning", ExpandablePrimitive::Meaning);
    let macro_cs = stores.intern("m");
    let params = stores.intern_token_list(&[Token::param(1)]);
    let body = stores.intern_token_list(&[
        char_token('a'),
        Token::param(1),
        Token::Char {
            ch: '#',
            cat: Catcode::Parameter,
        },
    ]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
    );
    let list =
        stores.intern_token_list(&[Token::Cs(meaning.symbol()), Token::Cs(macro_cs.symbol())]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    let tokens = collect_expanded(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    );
    let text = tokens
        .iter()
        .map(|token| match token {
            Token::Char { ch, .. } => *ch,
            other => panic!("expected character token, got {other:?}"),
        })
        .collect::<String>();

    assert_eq!(text, "macro:#1->a#1##");
    assert!(tokens.iter().all(|token| matches!(
        token,
        Token::Char {
            cat: Catcode::Other | Catcode::Space,
            ..
        }
    )));
}

#[test]
fn meaning_uses_tex_printable_forms_for_nonprinting_macro_tokens() {
    // tex.web sections 49 and 262: show_token_list uses the preloaded
    // single-character strings, which make control bytes visible.
    let mut stores = Universe::new();
    let macro_cs = stores.intern("m");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[
        Token::Char {
            ch: '\r',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: '\u{7f}',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: '\u{80}',
            cat: Catcode::Other,
        },
    ]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
    );

    assert_eq!(
        crate::meaning_text(&stores, Token::Cs(macro_cs.symbol())),
        "macro:->^^M^^?^^80"
    );
}

#[test]
fn meaning_uses_the_canonical_name_for_a_radical_alias() {
    let mut stores = Universe::new();
    let alias = stores.intern("sqrtsign");
    stores.set_meaning(
        alias,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Radical),
    );

    assert_eq!(
        crate::meaning_text(&stores, Token::Cs(alias.symbol())),
        "\\radical"
    );
}

#[test]
fn meaning_uses_registered_names_for_primitive_aliases() {
    let mut stores = Universe::new();
    let expanded = Meaning::ExpandablePrimitive(ExpandablePrimitive::Expanded);
    stores.install_primitive_meaning("expanded", expanded);
    let expanded_alias = stores.intern("expandedalias");
    stores.set_meaning(expanded_alias, expanded);

    let mark = Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Mark);
    stores.install_primitive_meaning("mark", mark);
    let mark_alias = stores.intern("markalias");
    stores.set_meaning(mark_alias, mark);

    assert_eq!(
        crate::meaning_text(&stores, Token::Cs(expanded_alias.symbol())),
        "\\expanded"
    );
    assert_eq!(
        crate::meaning_text(&stores, Token::Cs(mark_alias.symbol())),
        "\\mark"
    );
}

#[test]
fn meaning_renders_macro_prefixes_in_tex_order() {
    let mut stores = Universe::new();
    let macro_cs = stores.intern("prefixed");
    let empty = stores.intern_token_list(&[]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(
            MeaningFlags::PROTECTED | MeaningFlags::LONG | MeaningFlags::OUTER,
            empty,
            empty,
        ),
    );

    assert_eq!(
        crate::meaning_text(&stores, Token::Cs(macro_cs.symbol())),
        "\\protected\\long\\outer macro:->"
    );
}

#[test]
fn meaning_resolves_an_active_character_macro() {
    let mut stores = Universe::new();
    let active = stores.intern_active_character('~');
    let empty = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x')]);
    stores.set_macro_meaning(
        active,
        MacroMeaning::new(MeaningFlags::PROTECTED, empty, body),
    );

    assert_eq!(
        crate::meaning_text(
            &stores,
            Token::Char {
                ch: '~',
                cat: Catcode::Active,
            }
        ),
        "\\protected macro:->x"
    );
}

#[test]
fn meaning_reports_a_font_selection_by_font_identity() {
    let mut stores = Universe::new();
    let alias = stores.intern("array_alias");
    stores.set_meaning(alias, Meaning::Font(tex_state::font::NULL_FONT));

    assert_eq!(
        crate::meaning_text(&stores, Token::Cs(alias.symbol())),
        "select font nullfont"
    );
}

#[test]
fn the_renders_supported_registers_and_token_registers() {
    let mut stores = Universe::new();
    let the = expandable_primitive(&mut stores, "the", ExpandablePrimitive::The);
    let count = stores.intern("count");
    let dimen = stores.intern("dimen");
    let toks = stores.intern("toks");
    stores.set_meaning(
        count,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count),
    );
    stores.set_meaning(
        dimen,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen),
    );
    stores.set_meaning(
        toks,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Toks),
    );
    stores.set_count(2, -42);
    stores.set_dimen(3, tex_state::scaled::Scaled::from_raw(65_537));
    let toks_value = stores.intern_token_list(&[
        Token::Char {
            ch: 'A',
            cat: Catcode::Letter,
        },
        Token::Char {
            ch: '!',
            cat: Catcode::Other,
        },
    ]);
    stores.set_toks(4, toks_value);
    let list = stores.intern_token_list(&[
        Token::Cs(the.symbol()),
        Token::Cs(count.symbol()),
        char_token('2'),
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
        Token::Cs(the.symbol()),
        Token::Cs(dimen.symbol()),
        char_token('3'),
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
        Token::Cs(the.symbol()),
        Token::Cs(toks.symbol()),
        char_token('4'),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "-421.00002ptA!"
    );
}

#[test]
fn rendered_output_uses_transient_replay_without_durable_identity() {
    let mut stores = Universe::new();
    let number = expandable_primitive(&mut stores, "number", ExpandablePrimitive::Number);
    let list = stores.intern_token_list(&[Token::Cs(number.symbol()), char_token('7')]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("number should expand"),
        Some(Token::Char {
            ch: '7',
            cat: Catcode::Other
        })
    );
    assert!(
        matches!(
            input.summary().frames().last(),
            Some(tex_lex::InputFrameSummary::TransientTokenList {
                tokens,
                replay_kind: TokenListReplayKind::Inserted,
                ..
            }) if tokens.is_empty()
        ),
        "rendered output must not acquire a permanent token-list id"
    );
}

#[test]
fn number_output_tokens_share_synthesized_origin_from_primitive() {
    let mut stores = Universe::new();
    let number = expandable_primitive(&mut stores, "number", ExpandablePrimitive::Number);
    let number_origin = stores.source_origin(tex_state::SourceId::new(21), 120, 12, 1);
    let digit_origin = stores.source_origin(tex_state::SourceId::new(21), 128, 12, 9);
    let list =
        stores.intern_token_list(&[Token::Cs(number.symbol()), char_token('4'), char_token('2')]);
    let origins = stores.allocate_origin_list(&[number_origin, digit_origin, digit_origin]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list_with_origins(list, origins, TokenListReplayKind::Inserted);

    let first = crate::get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect("number should expand")
    .expect("first digit should be delivered");
    let second = crate::get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect("number should continue")
    .expect("second digit should be delivered");

    assert_eq!(first.token(), Some(char_token('4')));
    assert_eq!(second.token(), Some(char_token('2')));
    assert_eq!(first.origin(), second.origin());
    assert_eq!(
        stores.origin(first.origin()),
        OriginRecord::Synthesized(SynthesizedOrigin::new(
            SynthesizedOriginKind::ValueRendering,
            number_origin,
        ))
    );
}

#[test]
fn input_pushes_driver_source_and_returns_to_calling_source() {
    let mut stores = Universe::new();
    stores.set_int_param(tex_state::env::banks::IntParam::END_LINE_CHAR, 13);
    expandable_primitive(&mut stores, "input", ExpandablePrimitive::Input);
    let mut input = InputStack::new(MemoryInput::new("\\input{inc}z"));
    let mut context = MemoryResolverFixture::new("main").with_source("inc", "ab");

    assert_eq!(
        next_expanded_chars_with_context(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
            &mut context
        ),
        "ab z "
    );
    assert_eq!(context.resolver.opened, vec!["inc"]);
}

#[test]
fn input_propagates_a_typed_resource_need() {
    let mut stores = Universe::new();
    expandable_primitive(&mut stores, "input", ExpandablePrimitive::Input);
    let mut input = InputStack::new(MemoryInput::new("\\input{inc}"));
    let mut resolver = SuspendingResolver;
    let mut context = ExpansionContext::with_input_resolver("main", &mut resolver);

    assert!(matches!(
        crate::get_x_token_with_context(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
            &mut context,
        ),
        Err(crate::ExpandError::NeedResource(need))
            if need == crate::ResourceNeed::new(0)
    ));
}

#[test]
fn input_strips_filename_quotes_and_accepts_spaces_inside_them() {
    let mut stores = Universe::new();
    stores.set_int_param(tex_state::env::banks::IntParam::END_LINE_CHAR, 13);
    expandable_primitive(&mut stores, "input", ExpandablePrimitive::Input);
    let mut input = InputStack::new(MemoryInput::new("\\input \"inc file\" z"));
    let mut context = MemoryResolverFixture::new("main").with_source("inc file", "ab");

    assert_eq!(
        next_expanded_chars_with_context(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
            &mut context
        ),
        "ab z "
    );
    assert_eq!(context.resolver.opened, vec!["inc file"]);
}

#[test]
fn input_filename_stops_before_an_unexpandable_control_sequence() {
    let mut stores = Universe::new();
    stores.set_int_param(tex_state::env::banks::IntParam::END_LINE_CHAR, -1);
    expandable_primitive(&mut stores, "input", ExpandablePrimitive::Input);
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
    let mut input = InputStack::new(MemoryInput::new("\\input inc\\relax"));
    let mut context = MemoryResolverFixture::new("main").with_source("inc", "");

    let token = get_x_token_with_context(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        &mut context,
    )
    .expect("input expansion succeeds")
    .expect("filename terminator is replayed");
    assert_eq!(token, Token::Cs(relax.symbol()));
    assert_eq!(context.resolver.opened, vec!["inc"]);
}

#[test]
fn endinput_finishes_current_line_then_pops_source() {
    let mut stores = Universe::new();
    stores.set_int_param(tex_state::env::banks::IntParam::END_LINE_CHAR, 13);
    expandable_primitive(&mut stores, "input", ExpandablePrimitive::Input);
    expandable_primitive(&mut stores, "endinput", ExpandablePrimitive::EndInput);
    let mut input = InputStack::new(MemoryInput::new("\\input{inc}z"));
    let mut context = MemoryResolverFixture::new("main").with_source("inc", "a\\endinput b\nc");

    assert_eq!(
        next_expanded_chars_with_context(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
            &mut context
        ),
        "ab z "
    );
}

#[test]
fn jobname_expands_from_driver_hook_as_rendered_tokens() {
    let mut stores = Universe::new();
    expandable_primitive(&mut stores, "jobname", ExpandablePrimitive::JobName);
    let mut input = InputStack::new(MemoryInput::new("\\jobname"));
    let mut context = MemoryResolverFixture::new("paper");

    let tokens = collect_expanded_with_context(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        &mut context,
    );
    let text = tokens
        .iter()
        .map(|token| match token {
            Token::Char { ch, .. } => *ch,
            other => panic!("expected character token, got {other:?}"),
        })
        .collect::<String>();

    assert_eq!(text, "paper");
    assert!(tokens.iter().all(|token| matches!(
        token,
        Token::Char {
            cat: Catcode::Other,
            ..
        }
    )));
}

#[test]
fn fontname_renders_real_font_selector_name() {
    let mut stores = Universe::new();
    expandable_primitive(&mut stores, "fontname", ExpandablePrimitive::FontName);
    let nullfont = stores.intern("nullfont");
    stores.set_meaning(nullfont, Meaning::Font(tex_state::font::NULL_FONT));
    let list = stores.intern_token_list(&[
        Token::Cs(stores.symbol("fontname").expect("fontname").symbol()),
        Token::Cs(nullfont.symbol()),
        char_token('z'),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "nullfontz"
    );
}

#[test]
fn the_fontdimen_accepts_current_font_with_exact_output_and_trace() {
    #[derive(Default)]
    struct SymbolRecorder(Vec<Symbol>);

    impl ReadRecorder for SymbolRecorder {
        fn record_meaning(&mut self, symbol: Symbol, _meaning: Meaning) {
            self.0.push(symbol);
        }
    }

    let mut stores = Universe::new();
    let the = expandable_primitive(&mut stores, "the", ExpandablePrimitive::The);
    let fontdimen = stores.intern("fontdimen");
    stores.set_meaning(
        fontdimen,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::FontDimen),
    );
    let font = stores.intern("font");
    stores.set_meaning(
        font,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Font),
    );
    let nullfont = stores.intern("nullfont");
    stores.set_meaning(nullfont, Meaning::Font(tex_state::font::NULL_FONT));
    stores.set_current_font_selector(nullfont, tex_state::font::NULL_FONT);
    stores
        .set_font_dimen(
            tex_state::font::NULL_FONT,
            1,
            Scaled::from_raw(Scaled::UNITY + Scaled::UNITY / 2),
        )
        .expect("current font parameter is writable");

    let invocation = stores.source_origin(tex_state::SourceId::new(9), 90, 9, 1);
    let tokens = stores.intern_token_list(&[
        Token::Cs(the.symbol()),
        Token::Cs(fontdimen.symbol()),
        char_token('1'),
        Token::Cs(font.symbol()),
    ]);
    let origins = stores.allocate_repeated_origin_list(invocation, 4);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list_with_origins(tokens, origins, TokenListReplayKind::Inserted);
    let mut recorder = SymbolRecorder::default();
    let mut expansion = ExpansionContext::new("texput").recording(&mut recorder);
    let mut output = Vec::new();
    while let Some(token) = crate::get_x_token_with_context(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        &mut expansion,
    )
    .expect("fontdimen expansion should succeed")
    {
        output.push(token);
    }

    let text = output
        .iter()
        .map(
            |token| match token.token().expect("rendered token decodes") {
                Token::Char { ch, .. } => ch,
                token => panic!("expected rendered character, got {token:?}"),
            },
        )
        .collect::<String>();
    assert_eq!(text, "1.5pt");
    assert!(recorder.0.contains(&the));
    assert!(recorder.0.contains(&fontdimen.symbol()));
    assert!(recorder.0.contains(&font.symbol()));

    let rendered_origin = output[0].origin();
    assert!(output.iter().all(|token| token.origin() == rendered_origin));
    assert_eq!(
        stores.origin(rendered_origin),
        OriginRecord::Synthesized(SynthesizedOrigin::new(
            SynthesizedOriginKind::ValueRendering,
            invocation,
        ))
    );
}

#[test]
fn the_fontdimen_renders_zero_for_unavailable_parameter() {
    let mut stores = Universe::new();
    let the = expandable_primitive(&mut stores, "the", ExpandablePrimitive::The);
    let fontdimen = stores.intern("fontdimen");
    stores.set_meaning(
        fontdimen,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::FontDimen),
    );
    let font = stores.intern("font");
    stores.set_meaning(
        font,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Font),
    );
    let nullfont = stores.intern("nullfont");
    stores.set_meaning(nullfont, Meaning::Font(tex_state::font::NULL_FONT));
    stores.set_current_font_selector(nullfont, tex_state::font::NULL_FONT);
    assert_eq!(stores.font_parameter_count(tex_state::font::NULL_FONT), 7);
    stores
        .set_font_dimen(
            tex_state::font::NULL_FONT,
            1,
            tex_state::scaled::Scaled::from_raw(123),
        )
        .expect("first nullfont parameter is writable");

    let invocation = stores.source_origin(tex_state::SourceId::new(10), 100, 10, 1);
    let tokens = stores.intern_token_list(&[
        Token::Cs(the.symbol()),
        Token::Cs(fontdimen.symbol()),
        char_token('3'),
        char_token('2'),
        char_token('7'),
        char_token('6'),
        char_token('9'),
        Token::Cs(font.symbol()),
    ]);
    let origins = stores.allocate_repeated_origin_list(invocation, 8);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list_with_origins(tokens, origins, TokenListReplayKind::Inserted);

    let mut output = String::new();
    while let Some(token) = crate::get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect("unavailable fontdimen yields zero")
    {
        let Token::Char { ch, .. } = token.token().expect("rendered token") else {
            panic!("fontdimen should render characters");
        };
        output.push(ch);
    }
    assert_eq!(output, "0.0pt");
}

#[test]
fn the_math_family_fonts_expand_to_identifier_tokens_with_trace_and_reads() {
    #[derive(Default)]
    struct SymbolRecorder(Vec<Symbol>);

    impl ReadRecorder for SymbolRecorder {
        fn record_meaning(&mut self, symbol: Symbol, _meaning: Meaning) {
            self.0.push(symbol);
        }
    }

    let mut stores = Universe::new();
    let the = expandable_primitive(&mut stores, "the", ExpandablePrimitive::The);
    let nullfont = stores.intern("nullfont");
    stores.set_meaning(nullfont, Meaning::Font(tex_state::font::NULL_FONT));
    stores.set_font_identifier_symbol(tex_state::font::NULL_FONT, nullfont);
    let family_primitives = [
        (
            "textfont",
            UnexpandablePrimitive::TextFont,
            tex_state::math::MathFontSize::Text,
        ),
        (
            "scriptfont",
            UnexpandablePrimitive::ScriptFont,
            tex_state::math::MathFontSize::Script,
        ),
        (
            "scriptscriptfont",
            UnexpandablePrimitive::ScriptScriptFont,
            tex_state::math::MathFontSize::ScriptScript,
        ),
    ];
    let mut input_tokens = Vec::new();
    let mut primitive_symbols = Vec::new();
    for (name, primitive, size) in family_primitives {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
        stores.set_math_family_font(size, 1, tex_state::font::NULL_FONT, true);
        primitive_symbols.push(symbol);
        input_tokens.extend([
            Token::Cs(the.symbol()),
            Token::Cs(symbol.symbol()),
            char_token('1'),
        ]);
    }
    let invocation = stores.source_origin(tex_state::SourceId::new(11), 110, 11, 1);
    let tokens = stores.intern_token_list(&input_tokens);
    let origins = stores.allocate_repeated_origin_list(invocation, input_tokens.len());
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list_with_origins(tokens, origins, TokenListReplayKind::Inserted);
    let mut recorder = SymbolRecorder::default();
    let mut expansion = ExpansionContext::new("texput").recording(&mut recorder);
    let mut output = Vec::new();
    while let Some(token) = crate::get_x_token_with_context(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
        &mut expansion,
    )
    .expect("math-family font identifiers should expand")
    {
        output.push(token);
    }

    assert_eq!(output.len(), 3);
    assert!(
        output
            .iter()
            .all(|token| token.token() == Some(Token::Cs(nullfont.symbol())))
    );
    assert!(recorder.0.contains(&the));
    for symbol in primitive_symbols {
        assert!(recorder.0.contains(&symbol.symbol()));
    }
    for token in output {
        assert_eq!(
            stores.origin(token.origin()),
            OriginRecord::Synthesized(SynthesizedOrigin::new(
                SynthesizedOriginKind::ValueRendering,
                invocation,
            ))
        );
    }
}

#[test]
fn the_math_family_font_substitutes_family_zero_for_out_of_range_number() {
    let mut stores = Universe::new();
    let the = expandable_primitive(&mut stores, "the", ExpandablePrimitive::The);
    let textfont = stores.intern("textfont");
    stores.set_meaning(
        textfont,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::TextFont),
    );
    let invocation = stores.source_origin(tex_state::SourceId::new(12), 120, 12, 1);
    let number_origin = stores.source_origin(tex_state::SourceId::new(12), 129, 12, 10);
    let tokens = stores.intern_token_list(&[
        Token::Cs(the.symbol()),
        Token::Cs(textfont.symbol()),
        char_token('1'),
        char_token('6'),
    ]);
    let origins =
        stores.allocate_origin_list(&[invocation, invocation, number_origin, number_origin]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list_with_origins(tokens, origins, TokenListReplayKind::Inserted);

    let error = crate::get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect_err("the substituted null font has no printable control sequence");
    assert!(matches!(
        error,
        crate::ExpandError::UnsupportedTheTarget { .. }
    ));
}

#[test]
fn mark_family_primitives_expand_stored_page_marks() {
    let mut stores = Universe::new();
    crate::install_etex_expandable_primitives(&mut stores);
    for (name, primitive) in [
        ("topmark", ExpandablePrimitive::TopMark),
        ("firstmark", ExpandablePrimitive::FirstMark),
        ("botmark", ExpandablePrimitive::BotMark),
        ("splitfirstmark", ExpandablePrimitive::SplitFirstMark),
        ("splitbotmark", ExpandablePrimitive::SplitBotMark),
    ] {
        expandable_primitive(&mut stores, name, primitive);
    }
    let list = stores.intern_token_list(&[
        Token::Cs(stores.symbol("topmark").expect("topmark").symbol()),
        Token::Cs(stores.symbol("firstmark").expect("firstmark").symbol()),
        Token::Cs(stores.symbol("botmark").expect("botmark").symbol()),
        Token::Cs(
            stores
                .symbol("splitfirstmark")
                .expect("splitfirstmark")
                .symbol(),
        ),
        Token::Cs(
            stores
                .symbol("splitbotmark")
                .expect("splitbotmark")
                .symbol(),
        ),
        char_token('z'),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "z"
    );

    let top = stores.intern_token_list(&[char_token('T')]);
    let first = stores.intern_token_list(&[char_token('F')]);
    let bot = stores.intern_token_list(&[char_token('B')]);
    let split_first = stores.intern_token_list(&[char_token('S')]);
    let split_bot = stores.intern_token_list(&[char_token('s')]);
    stores.set_page_mark(PageMark::Top, top);
    stores.set_page_mark(PageMark::First, first);
    stores.set_page_mark(PageMark::Bot, bot);
    stores.set_page_mark(PageMark::SplitFirst, split_first);
    stores.set_page_mark(PageMark::SplitBot, split_bot);
    let list = stores.intern_token_list(&[
        Token::Cs(stores.symbol("topmark").expect("topmark").symbol()),
        Token::Cs(stores.symbol("firstmark").expect("firstmark").symbol()),
        Token::Cs(stores.symbol("botmark").expect("botmark").symbol()),
        Token::Cs(
            stores
                .symbol("splitfirstmark")
                .expect("splitfirstmark")
                .symbol(),
        ),
        Token::Cs(
            stores
                .symbol("splitbotmark")
                .expect("splitbotmark")
                .symbol(),
        ),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "TFBSs"
    );

    let class_zero = stores.intern_token_list(&[char_token('Z')]);
    stores.set_page_mark(PageMark::Top, class_zero);
    let mut input = InputStack::new(MemoryInput::new(r"\topmarks-1"));
    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "Z"
    );
    assert!(
        stores
            .world()
            .effect_records()
            .iter()
            .any(|record| matches!(
                record,
                tex_state::EffectRecord::StreamWrite { text, .. }
                    if text.contains("Bad register code")
            ))
    );
}

#[test]
fn iffontchar_recovers_a_missing_font_identifier_as_nullfont() {
    let mut stores = Universe::new();
    crate::install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(r"\iffontchar\else\fi"));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        ""
    );
    assert!(stores.world().effect_records().iter().any(|record| {
        matches!(
            record,
            tex_state::EffectRecord::StreamWrite { text, .. }
                if text.contains("Missing font identifier")
        )
    }));
}

#[test]
fn etex_mark_class_primitives_scan_class_and_expand_its_marks() {
    let mut stores = Universe::new();
    crate::install_etex_expandable_primitives(&mut stores);
    let top = stores.intern_token_list(&[char_token('T')]);
    let first = stores.intern_token_list(&[char_token('F')]);
    let bot = stores.intern_token_list(&[char_token('B')]);
    let split_first = stores.intern_token_list(&[char_token('S')]);
    let split_bot = stores.intern_token_list(&[char_token('s')]);
    stores.set_page_mark_class(PageMark::Top, 27, top);
    stores.set_page_mark_class(PageMark::First, 27, first);
    stores.set_page_mark_class(PageMark::Bot, 27, bot);
    stores.set_page_mark_class(PageMark::SplitFirst, 27, split_first);
    stores.set_page_mark_class(PageMark::SplitBot, 27, split_bot);
    let mut input = InputStack::new(MemoryInput::new(
        r"\topmarks27\firstmarks27\botmarks27\splitfirstmarks27\splitbotmarks27",
    ));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "TFBSs"
    );
}

#[test]
fn iftrue_and_iffalse_select_expected_two_limb_branches() {
    let mut stores = Universe::new();
    let (iftrue, iffalse, else_cs, fi) = conditional_primitives(&mut stores);
    let list = stores.intern_token_list(&[
        Token::Cs(iftrue.symbol()),
        char_token('t'),
        Token::Cs(else_cs.symbol()),
        char_token('f'),
        Token::Cs(fi.symbol()),
        Token::Cs(iffalse.symbol()),
        char_token('f'),
        Token::Cs(else_cs.symbol()),
        char_token('t'),
        Token::Cs(fi.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "tt"
    );
}

#[test]
fn unless_inverts_boolean_conditionals_without_leaking_frames() {
    let mut stores = Universe::new();
    crate::install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\unless\\iftrue f\\else t\\fi\\unless\\iffalse t\\else f\\fi%",
    ));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "tt"
    );
    assert!(input.current_condition().is_none());
}

#[test]
fn unless_inverts_scanned_numeric_condition() {
    let mut stores = Universe::new();
    crate::install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\unless\\ifnum1<2 f\\else t\\fi%"));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "t"
    );
}

#[test]
fn if_expands_to_two_unexpandable_character_tokens_before_comparing_charcodes() {
    let mut stores = Universe::new();
    let (_, _, else_cs, fi) = conditional_primitives(&mut stores);
    let if_cs = expandable_primitive(&mut stores, "if", ExpandablePrimitive::If);
    let left = stores.intern("left");
    let right = stores.intern("right");
    let params = stores.intern_token_list(&[]);
    let left_body = stores.intern_token_list(&[char_token('a')]);
    let right_body = stores.intern_token_list(&[Token::Char {
        ch: 'a',
        cat: Catcode::Other,
    }]);
    stores.set_macro_meaning(
        left,
        MacroMeaning::new(MeaningFlags::EMPTY, params, left_body),
    );
    stores.set_macro_meaning(
        right,
        MacroMeaning::new(MeaningFlags::EMPTY, params, right_body),
    );
    let list = stores.intern_token_list(&[
        Token::Cs(if_cs.symbol()),
        Token::Cs(left.symbol()),
        Token::Cs(right.symbol()),
        char_token('y'),
        Token::Cs(else_cs.symbol()),
        char_token('n'),
        Token::Cs(fi.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "y"
    );
}

#[test]
fn if_compares_two_unexpandable_control_sequences_as_character_code_256() {
    let mut stores = Universe::new();
    let (_, _, else_cs, fi) = conditional_primitives(&mut stores);
    let if_cs = expandable_primitive(&mut stores, "if", ExpandablePrimitive::If);
    let left = stores.intern("left-relax");
    let right = stores.intern("right-relax");
    stores.set_meaning(left, Meaning::Relax);
    stores.set_meaning(right, Meaning::Relax);
    let list = stores.intern_token_list(&[
        Token::Cs(if_cs.symbol()),
        Token::Cs(left.symbol()),
        Token::Cs(right.symbol()),
        char_token('y'),
        Token::Cs(else_cs.symbol()),
        char_token('n'),
        Token::Cs(fi.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "y"
    );
}

#[test]
fn ifcat_compares_category_codes_after_expansion() {
    let mut stores = Universe::new();
    let (_, _, else_cs, fi) = conditional_primitives(&mut stores);
    let ifcat = expandable_primitive(&mut stores, "ifcat", ExpandablePrimitive::IfCat);
    let macro_cs = stores.intern("letter");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('b')]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
    );
    let list = stores.intern_token_list(&[
        Token::Cs(ifcat.symbol()),
        char_token('a'),
        Token::Cs(macro_cs.symbol()),
        char_token('y'),
        Token::Cs(else_cs.symbol()),
        char_token('n'),
        Token::Cs(fi.symbol()),
        Token::Cs(ifcat.symbol()),
        char_token('a'),
        Token::Char {
            ch: '1',
            cat: Catcode::Other,
        },
        char_token('n'),
        Token::Cs(else_cs.symbol()),
        char_token('y'),
        Token::Cs(fi.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "yy"
    );
}

#[test]
fn if_and_ifcat_use_character_alias_command_meanings() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    let letter = stores.intern("letter");
    stores.set_meaning(
        letter,
        Meaning::CharToken {
            ch: 'a',
            cat: Catcode::Letter,
        },
    );
    let mut input = InputStack::new(MemoryInput::new(
        "\\if a\\letter y\\else n\\fi\\ifcat z\\letter y\\else n\\fi",
    ));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "yy"
    );
}

#[test]
fn ifx_compares_macro_definitions_semantically_ignoring_origin_lists() {
    let mut stores = Universe::new();
    let (_, _, else_cs, fi) = conditional_primitives(&mut stores);
    let ifx = expandable_primitive(&mut stores, "ifx", ExpandablePrimitive::IfX);
    let left = stores.intern("left");
    let right = stores.intern("right");
    let protected = stores.intern("protected");
    let params = stores.intern_token_list(&[Token::param(1)]);
    let left_body = stores.intern_token_list(&[Token::param(1), char_token('!')]);
    let right_body = stores.intern_token_list(&[Token::param(1), char_token('!')]);
    assert_eq!(left_body, right_body);
    let left_origin = stores.source_origin(tex_state::SourceId::new(1), 10, 2, 3);
    let right_origin = stores.source_origin(tex_state::SourceId::new(2), 20, 4, 5);
    let param_origins = stores.allocate_origin_list(&[left_origin]);
    let left_origins = stores.allocate_origin_list(&[left_origin, left_origin]);
    let right_origins = stores.allocate_origin_list(&[right_origin, right_origin]);
    stores.set_macro_meaning_with_provenance(
        left,
        MacroMeaning::new(MeaningFlags::EMPTY, params, left_body),
        MacroDefinitionProvenance::new(left_origin, param_origins, left_origins),
    );
    stores.set_macro_meaning_with_provenance(
        right,
        MacroMeaning::new(MeaningFlags::EMPTY, params, right_body),
        MacroDefinitionProvenance::new(right_origin, param_origins, right_origins),
    );
    stores.set_macro_meaning(
        protected,
        MacroMeaning::new(MeaningFlags::PROTECTED, params, right_body),
    );
    let list = stores.intern_token_list(&[
        Token::Cs(ifx.symbol()),
        Token::Cs(left.symbol()),
        Token::Cs(right.symbol()),
        char_token('y'),
        Token::Cs(else_cs.symbol()),
        char_token('n'),
        Token::Cs(fi.symbol()),
        Token::Cs(ifx.symbol()),
        Token::Cs(left.symbol()),
        Token::Cs(protected.symbol()),
        char_token('n'),
        Token::Cs(else_cs.symbol()),
        char_token('y'),
        Token::Cs(fi.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "yy"
    );
}

#[test]
fn ifx_compares_an_active_character_with_a_control_sequence_alias_by_meaning() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    stores.set_catcode('<', Catcode::Active);
    let active = stores.intern_active_character('<');
    let alias = stores.intern("next");
    let body = stores.intern_token_list(&[char_token('x')]);
    let meaning = MacroMeaning::new(MeaningFlags::EMPTY, TokenListId::EMPTY, body);
    stores.set_macro_meaning(active, meaning);
    stores.set_macro_meaning(alias, meaning);
    let mut input = InputStack::new(MemoryInput::new("\\ifx<\\next y\\else n\\fi"));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "y"
    );
}

#[test]
fn ifx_compares_a_character_with_a_control_sequence_alias_by_meaning() {
    let mut stores = Universe::new();
    let (_, _, else_cs, fi) = conditional_primitives(&mut stores);
    let ifx = expandable_primitive(&mut stores, "ifx", ExpandablePrimitive::IfX);
    let alias = stores.intern("punctuation");
    stores.set_meaning(
        alias,
        Meaning::CharToken {
            ch: ',',
            cat: Catcode::Other,
        },
    );
    let list = stores.intern_token_list(&[
        Token::Cs(ifx.symbol()),
        Token::Cs(alias.symbol()),
        Token::Char {
            ch: ',',
            cat: Catcode::Other,
        },
        char_token('y'),
        Token::Cs(else_cs.symbol()),
        char_token('n'),
        Token::Cs(fi.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "y"
    );
}

#[test]
fn ifx_uses_meaning_word_equality_for_non_macros_without_expansion() {
    let mut stores = Universe::new();
    let (_, _, else_cs, fi) = conditional_primitives(&mut stores);
    let ifx = expandable_primitive(&mut stores, "ifx", ExpandablePrimitive::IfX);
    let first = stores.intern("first");
    let second = stores.intern("second");
    let macro_cs = stores.intern("macro");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('a')]);
    stores.set_meaning(first, Meaning::CharGiven('a'));
    stores.set_meaning(second, Meaning::CharGiven('a'));
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
    );
    let list = stores.intern_token_list(&[
        Token::Cs(ifx.symbol()),
        Token::Cs(first.symbol()),
        Token::Cs(second.symbol()),
        char_token('y'),
        Token::Cs(else_cs.symbol()),
        char_token('n'),
        Token::Cs(fi.symbol()),
        Token::Cs(ifx.symbol()),
        Token::Cs(macro_cs.symbol()),
        char_token('a'),
        char_token('n'),
        Token::Cs(else_cs.symbol()),
        char_token('y'),
        Token::Cs(fi.symbol()),
        Token::Cs(ifx.symbol()),
        Token::Cs(first.symbol()),
        char_token('a'),
        char_token('n'),
        Token::Cs(else_cs.symbol()),
        char_token('y'),
        Token::Cs(fi.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "yyy"
    );
}

#[test]
fn ifx_treats_a_noexpanded_expandable_operand_as_relax() {
    let mut stores = Universe::new();
    crate::install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\ifx\\noexpand\\empty\\empty n\\else y\\fi%",
    ));
    let empty = stores.intern("empty");
    let token_list = stores.intern_token_list(&[]);
    stores.set_macro_meaning(
        empty,
        MacroMeaning::new(MeaningFlags::EMPTY, token_list, token_list),
    );

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "y"
    );
}

#[test]
fn expandafter_preserves_noexpand_relax_meaning_for_ifx() {
    let mut stores = Universe::new();
    crate::install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let eval = stores.intern("eval");
    let empty = stores.intern("empty");
    let expandafter = stores.intern("expandafter");
    let ifx = stores.intern("ifx");
    let noexpand = stores.intern("noexpand");
    let else_cs = stores.intern("else");
    let fi = stores.intern("fi");
    let token_list = stores.intern_token_list(&[]);
    stores.set_macro_meaning(
        empty,
        MacroMeaning::new(MeaningFlags::EMPTY, token_list, token_list),
    );
    let parameter = stores.intern_token_list(&[Token::param(1)]);
    let body = stores.intern_token_list(&[
        Token::Cs(expandafter.symbol()),
        Token::Cs(ifx.symbol()),
        Token::Cs(noexpand.symbol()),
        Token::param(1),
        Token::param(1),
        char_token('n'),
        Token::Cs(else_cs.symbol()),
        char_token('y'),
        Token::Cs(fi.symbol()),
    ]);
    stores.set_macro_meaning(
        eval,
        MacroMeaning::new(MeaningFlags::EMPTY, parameter, body),
    );
    let mut input = InputStack::new(MemoryInput::new("\\eval\\empty%"));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "y"
    );
}

#[test]
fn ifnum_consumes_a_char_token_space_after_expanded_digits() {
    let mut stores = Universe::new();
    let (_, _, else_cs, fi) = conditional_primitives(&mut stores);
    let ifnum = expandable_primitive(&mut stores, "ifnum", ExpandablePrimitive::IfNum);
    let string = expandable_primitive(&mut stores, "string", ExpandablePrimitive::String);
    let stop = stores.intern("stop");
    stores.set_meaning(
        stop,
        Meaning::CharToken {
            ch: ' ',
            cat: Catcode::Space,
        },
    );
    let list = stores.intern_token_list(&[
        Token::Cs(ifnum.symbol()),
        char_token('9'),
        char_token('<'),
        char_token('1'),
        Token::Cs(string.symbol()),
        char_token('1'),
        Token::Cs(stop.symbol()),
        char_token('t'),
        Token::Cs(else_cs.symbol()),
        char_token('f'),
        Token::Cs(fi.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "t"
    );
}

#[test]
fn ifnum_and_ifdim_compare_scanned_values() {
    let mut stores = Universe::new();
    let (_, _, else_cs, fi) = conditional_primitives(&mut stores);
    let ifnum = expandable_primitive(&mut stores, "ifnum", ExpandablePrimitive::IfNum);
    let ifdim = expandable_primitive(&mut stores, "ifdim", ExpandablePrimitive::IfDim);
    stores.set_count(2, 7);
    stores.set_dimen(3, Scaled::from_raw(Scaled::UNITY));
    let count = stores.intern("count");
    let dimen = stores.intern("dimen");
    stores.set_meaning(
        count,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count),
    );
    stores.set_meaning(
        dimen,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen),
    );
    let list = stores.intern_token_list(&[
        Token::Cs(ifnum.symbol()),
        Token::Cs(count.symbol()),
        char_token('2'),
        char_token('>'),
        char_token('6'),
        char_token('y'),
        Token::Cs(else_cs.symbol()),
        char_token('n'),
        Token::Cs(fi.symbol()),
        Token::Cs(ifdim.symbol()),
        Token::Cs(dimen.symbol()),
        char_token('3'),
        char_token('='),
        char_token('1'),
        char_token('p'),
        char_token('t'),
        char_token('y'),
        Token::Cs(else_cs.symbol()),
        char_token('n'),
        Token::Cs(fi.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "yy"
    );
}

#[test]
fn ifdim_compares_named_skip_registers_by_width_only() {
    let mut stores = Universe::new();
    let (_, _, else_cs, fi) = conditional_primitives(&mut stores);
    let ifdim = expandable_primitive(&mut stores, "ifdim", ExpandablePrimitive::IfDim);
    let named_skip = stores.intern("namedskip");
    stores.set_meaning(named_skip, Meaning::SkipRegister(42));
    let glue = stores.intern_glue(tex_state::glue::GlueSpec {
        width: Scaled::from_raw(2 * Scaled::UNITY),
        stretch: Scaled::from_raw(20 * Scaled::UNITY),
        stretch_order: tex_state::glue::Order::Fill,
        shrink: Scaled::from_raw(10 * Scaled::UNITY),
        shrink_order: tex_state::glue::Order::Fil,
    });
    stores.set_skip(42, glue);
    let list = stores.intern_token_list(&[
        Token::Cs(ifdim.symbol()),
        Token::Cs(named_skip.symbol()),
        char_token('<'),
        char_token('3'),
        char_token('p'),
        char_token('t'),
        char_token('y'),
        Token::Cs(else_cs.symbol()),
        char_token('n'),
        Token::Cs(fi.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "y"
    );
}

#[test]
fn ifdim_operand_nested_conditional_completes_exact_outer_frame() {
    let mut stores = Universe::new();
    let (iftrue, _, else_cs, fi) = conditional_primitives(&mut stores);
    let ifdim = expandable_primitive(&mut stores, "ifdim", ExpandablePrimitive::IfDim);
    let selected_skip = stores.intern("selectedskip");
    stores.set_meaning(selected_skip, Meaning::SkipRegister(42));
    let glue = stores.intern_glue(GlueSpec {
        width: Scaled::from_raw(4 * Scaled::UNITY),
        ..GlueSpec::ZERO
    });
    stores.set_skip(42, glue);

    let choose_skip = stores.intern("chooseskip");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[
        Token::Cs(iftrue.symbol()),
        Token::Cs(selected_skip.symbol()),
        Token::Cs(else_cs.symbol()),
        char_token('0'),
        char_token('p'),
        char_token('t'),
        Token::Cs(fi.symbol()),
    ]);
    stores.set_macro_meaning(
        choose_skip,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
    );

    let list = stores.intern_token_list(&[
        Token::Cs(ifdim.symbol()),
        Token::Cs(choose_skip.symbol()),
        char_token('<'),
        char_token('3'),
        char_token('p'),
        char_token('t'),
        char_token('y'),
        Token::Cs(else_cs.symbol()),
        char_token('n'),
        Token::Cs(fi.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "n"
    );
    assert!(input.current_condition().is_none());
}

#[test]
fn conditional_operand_recovery_preserves_nested_and_outer_frame_identity() {
    let mut stores = Universe::new();
    let (iftrue, _, else_cs, fi) = conditional_primitives(&mut stores);
    let if_cs = expandable_primitive(&mut stores, "if", ExpandablePrimitive::If);
    let list = stores.intern_token_list(&[
        Token::Cs(if_cs.symbol()),
        Token::Cs(iftrue.symbol()),
        char_token('a'),
        char_token('b'),
        Token::Cs(else_cs.symbol()),
        char_token('c'),
        Token::Cs(fi.symbol()),
        char_token('x'),
        Token::Cs(else_cs.symbol()),
        char_token('y'),
        Token::Cs(fi.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "y"
    );
    assert!(input.current_condition().is_none());
}

#[test]
fn ifnum_internal_operand_does_not_eagerly_expand_following_else() {
    let mut stores = Universe::new();
    let (_, _, else_cs, fi) = conditional_primitives(&mut stores);
    let ifnum = expandable_primitive(&mut stores, "ifnum", ExpandablePrimitive::IfNum);
    let count = stores.intern("count");
    let limit = stores.intern("limit");
    stores.set_meaning(
        count,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count),
    );
    stores.set_meaning(limit, Meaning::CountRegister(20));
    stores.set_count(11, 10);
    stores.set_count(20, 255);
    let list = stores.intern_token_list(&[
        Token::Cs(ifnum.symbol()),
        Token::Cs(count.symbol()),
        char_token('1'),
        char_token('1'),
        char_token('<'),
        Token::Cs(limit.symbol()),
        Token::Cs(else_cs.symbol()),
        char_token('n'),
        Token::Cs(fi.symbol()),
        char_token('y'),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);
    assert_eq!(
        collect_expanded(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        vec![char_token('y')]
    );
}

#[test]
fn ifodd_and_ifcase_select_expected_limb() {
    let mut stores = Universe::new();
    let (_, _, else_cs, fi) = conditional_primitives(&mut stores);
    let or_cs = expandable_primitive(&mut stores, "or", ExpandablePrimitive::Or);
    let ifodd = expandable_primitive(&mut stores, "ifodd", ExpandablePrimitive::IfOdd);
    let ifcase = expandable_primitive(&mut stores, "ifcase", ExpandablePrimitive::IfCase);
    let list = stores.intern_token_list(&[
        Token::Cs(ifodd.symbol()),
        char_token('-'),
        char_token('3'),
        char_token('y'),
        Token::Cs(else_cs.symbol()),
        char_token('n'),
        Token::Cs(fi.symbol()),
        Token::Cs(ifcase.symbol()),
        char_token('2'),
        char_token('z'),
        Token::Cs(or_cs.symbol()),
        char_token('o'),
        Token::Cs(or_cs.symbol()),
        char_token('t'),
        Token::Cs(or_cs.symbol()),
        char_token('x'),
        Token::Cs(else_cs.symbol()),
        char_token('e'),
        Token::Cs(fi.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "yt"
    );
}

#[test]
fn mode_predicates_use_driver_hook() {
    let mut stores = Universe::new();
    let (_, _, else_cs, fi) = conditional_primitives(&mut stores);
    for (name, primitive) in [
        ("ifvmode", ExpandablePrimitive::IfVMode),
        ("ifhmode", ExpandablePrimitive::IfHMode),
        ("ifmmode", ExpandablePrimitive::IfMMode),
        ("ifinner", ExpandablePrimitive::IfInner),
    ] {
        expandable_primitive(&mut stores, name, primitive);
    }
    let list = stores.intern_token_list(&[
        Token::Cs(stores.symbol("ifhmode").expect("ifhmode").symbol()),
        char_token('h'),
        Token::Cs(else_cs.symbol()),
        char_token('n'),
        Token::Cs(fi.symbol()),
        Token::Cs(stores.symbol("ifvmode").expect("ifvmode").symbol()),
        char_token('n'),
        Token::Cs(else_cs.symbol()),
        char_token('v'),
        Token::Cs(fi.symbol()),
        Token::Cs(stores.symbol("ifinner").expect("ifinner").symbol()),
        char_token('i'),
        Token::Cs(else_cs.symbol()),
        char_token('n'),
        Token::Cs(fi.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);
    let mut context = MemoryResolverFixture::new("main")
        .with_mode(EngineMode::Horizontal)
        .with_inner(true);

    assert_eq!(
        next_expanded_chars_with_context(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
            &mut context
        ),
        "hvi"
    );
}

#[test]
fn box_predicates_read_box_register_state() {
    let mut stores = Universe::new();
    let (_, _, else_cs, fi) = conditional_primitives(&mut stores);
    let ifvoid = expandable_primitive(&mut stores, "ifvoid", ExpandablePrimitive::IfVoid);
    let ifhbox = expandable_primitive(&mut stores, "ifhbox", ExpandablePrimitive::IfHBox);
    let ifvbox = expandable_primitive(&mut stores, "ifvbox", ExpandablePrimitive::IfVBox);
    let hbox = boxed_list(&mut stores, BoxKindForTest::HBox);
    let vbox = boxed_list(&mut stores, BoxKindForTest::VBox);
    stores.set_box_reg(1, hbox);
    stores.set_box_reg(2, vbox);
    let list = stores.intern_token_list(&[
        Token::Cs(ifvoid.symbol()),
        char_token('0'),
        char_token('v'),
        Token::Cs(else_cs.symbol()),
        char_token('n'),
        Token::Cs(fi.symbol()),
        Token::Cs(ifhbox.symbol()),
        char_token('1'),
        char_token('h'),
        Token::Cs(else_cs.symbol()),
        char_token('n'),
        Token::Cs(fi.symbol()),
        Token::Cs(ifvbox.symbol()),
        char_token('2'),
        char_token('b'),
        Token::Cs(else_cs.symbol()),
        char_token('n'),
        Token::Cs(fi.symbol()),
        Token::Cs(ifhbox.symbol()),
        char_token('2'),
        char_token('n'),
        Token::Cs(else_cs.symbol()),
        char_token('x'),
        Token::Cs(fi.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "vhbx"
    );
}

#[test]
fn margin_kern_enquiries_find_named_kerns_inside_skip_glue() {
    let mut stores = Universe::new();
    let left = expandable_primitive(
        &mut stores,
        "leftmarginkern",
        ExpandablePrimitive::LeftMarginKern,
    );
    let right = expandable_primitive(
        &mut stores,
        "rightmarginkern",
        ExpandablePrimitive::RightMarginKern,
    );
    let zero = stores.intern_glue(GlueSpec::ZERO);
    let children = stores.freeze_node_list(&[
        Node::Glue {
            spec: zero,
            kind: GlueKind::LeftSkip,
            leader: None,
        },
        Node::Kern {
            amount: Scaled::from_raw(-5 * 65_536),
            kind: KernKind::LeftMargin,
        },
        Node::Kern {
            amount: Scaled::from_raw(-7 * 65_536),
            kind: KernKind::RightMargin,
        },
        Node::Glue {
            spec: zero,
            kind: GlueKind::RightSkip,
            leader: None,
        },
    ]);
    let hbox = BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    });
    let root = stores.freeze_node_list(&[Node::HList(hbox)]);
    stores.set_box_reg(1, root);
    let input_tokens = stores.intern_token_list(&[
        Token::Cs(left.symbol()),
        char_token('1'),
        Token::Cs(right.symbol()),
        char_token('1'),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(input_tokens, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "-5.0pt-7.0pt"
    );
}

#[test]
fn margin_kern_enquiry_rejects_void_register() {
    let mut stores = Universe::new();
    let left = expandable_primitive(
        &mut stores,
        "leftmarginkern",
        ExpandablePrimitive::LeftMarginKern,
    );
    let input_tokens = stores.intern_token_list(&[Token::Cs(left.symbol()), char_token('0')]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(input_tokens, TokenListReplayKind::Inserted);

    let error = get_x_token(
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect_err("void box register must be rejected");
    assert_eq!(
        error.to_string(),
        "pdfTeX error (marginkern): a non-empty hbox expected"
    );
}

#[test]
fn ifeof_reads_world_stream_state_directly() {
    let mut stores = Universe::new();
    let (_, _, else_cs, fi) = conditional_primitives(&mut stores);
    let ifeof = expandable_primitive(&mut stores, "ifeof", ExpandablePrimitive::IfEof);
    let list = stores.intern_token_list(&[
        Token::Cs(ifeof.symbol()),
        char_token('1'),
        char_token('n'),
        Token::Cs(else_cs.symbol()),
        char_token('o'),
        Token::Cs(fi.symbol()),
        Token::Cs(ifeof.symbol()),
        char_token('2'),
        char_token('e'),
        Token::Cs(else_cs.symbol()),
        char_token('n'),
        Token::Cs(fi.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);
    stores
        .world_mut()
        .write_file("stream-one.tex", "unread")
        .expect("seed stream input");
    stores
        .world_mut()
        .open_in(tex_state::StreamSlot::new(1), "stream-one.tex")
        .expect("open seeded stream input");
    let mut context = MemoryResolverFixture::new("main");

    assert_eq!(
        next_expanded_chars_with_context(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores),
            &mut context
        ),
        "oe"
    );

    let list = stores.intern_token_list(&[
        Token::Cs(ifeof.symbol()),
        char_token('9'),
        char_token('e'),
        Token::Cs(else_cs.symbol()),
        char_token('n'),
        Token::Cs(fi.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "e"
    );
}

#[test]
fn skipped_false_limb_tracks_nested_conditionals() {
    let mut stores = Universe::new();
    let (iftrue, iffalse, else_cs, fi) = conditional_primitives(&mut stores);
    let list = stores.intern_token_list(&[
        Token::Cs(iffalse.symbol()),
        char_token('x'),
        Token::Cs(iftrue.symbol()),
        char_token('y'),
        Token::Cs(else_cs.symbol()),
        char_token('z'),
        Token::Cs(fi.symbol()),
        Token::Cs(else_cs.symbol()),
        char_token('t'),
        Token::Cs(fi.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "t"
    );
}

#[test]
fn skipped_false_limb_tracks_nested_etex_conditionals() {
    let mut stores = Universe::new();
    install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\iffalse x\\ifdefined\\missing y\\fi\\else t\\fi",
    ));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "t"
    );
}

#[test]
fn skipped_false_limb_resolves_active_conditional_meanings() {
    let mut stores = Universe::new();
    let (_, iffalse, _, _) = conditional_primitives(&mut stores);
    let active_iftrue = active_expandable_primitive(&mut stores, '?', ExpandablePrimitive::IfTrue);
    let active_else = active_expandable_primitive(&mut stores, '~', ExpandablePrimitive::Else);
    let active_fi = active_expandable_primitive(&mut stores, '!', ExpandablePrimitive::Fi);
    let list = stores.intern_token_list(&[
        Token::Cs(iffalse.symbol()),
        char_token('x'),
        active_iftrue,
        char_token('y'),
        active_else,
        char_token('z'),
        active_fi,
        active_else,
        char_token('t'),
        active_fi,
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "t"
    );
}

#[test]
fn ifcase_selects_selected_limb_and_else_fallback() {
    let mut stores = Universe::new();
    let (_, _, else_cs, fi) = conditional_primitives(&mut stores);
    let ifcase = expandable_primitive(&mut stores, "ifcase", ExpandablePrimitive::IfCase);
    let or_cs = expandable_primitive(&mut stores, "or", ExpandablePrimitive::Or);
    let list = stores.intern_token_list(&[
        Token::Cs(ifcase.symbol()),
        char_token('0'),
        char_token('z'),
        Token::Cs(or_cs.symbol()),
        char_token('o'),
        Token::Cs(else_cs.symbol()),
        char_token('e'),
        Token::Cs(fi.symbol()),
        Token::Cs(ifcase.symbol()),
        char_token('-'),
        char_token('1'),
        char_token('z'),
        Token::Cs(or_cs.symbol()),
        char_token('o'),
        Token::Cs(else_cs.symbol()),
        char_token('e'),
        Token::Cs(fi.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "ze"
    );
}

#[test]
fn else_or_fi_report_extra_without_open_conditional() {
    for (name, primitive, expected) in [
        ("else", ExpandablePrimitive::Else, "else"),
        ("or", ExpandablePrimitive::Or, "or"),
        ("fi", ExpandablePrimitive::Fi, "fi"),
    ] {
        let mut stores = Universe::new();
        let control = expandable_primitive(&mut stores, name, primitive);
        let list = stores.intern_token_list(&[Token::Cs(control.symbol())]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(list, TokenListReplayKind::Inserted);

        assert!(matches!(
            get_x_token(&mut input, &mut tex_state::ExpansionContext::new(&mut stores)),
            Err(crate::ExpandError::ExtraConditionalControl { name: found, .. }) if found == expected
        ));
    }
}

#[test]
fn conditional_recovery_uses_frozen_relax_when_live_relax_is_rebound() {
    let mut stores = Universe::new();
    stores.register_primitive_meaning("relax", Meaning::Relax);
    let live_relax = stores.intern("relax");
    let fi = expandable_primitive(&mut stores, "fi", ExpandablePrimitive::Fi);
    stores.set_meaning(
        live_relax,
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Fi),
    );
    let mut input = InputStack::new(MemoryInput::new(""));
    crate::conditionals::begin_if_evaluation(
        &mut input,
        TracedTokenWord::pack(Token::Cs(fi), OriginId::UNKNOWN),
        crate::conditionals::ConditionMetadata::new(0, false),
    );

    crate::conditionals::handle_fi(
        Token::Cs(fi),
        OriginId::UNKNOWN,
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect("conditional recovery");

    let inserted_relax = input
        .next_traced_token(&mut stores)
        .expect("read inserted relax")
        .expect("inserted relax token");
    let inserted_relax = semantic_token(inserted_relax);
    assert_eq!(
        stores.frozen_primitive_meaning(inserted_relax),
        Some(Meaning::Relax)
    );
    assert_ne!(inserted_relax, Token::Cs(live_relax.symbol()));
    assert_eq!(
        input
            .next_traced_token(&mut stores)
            .expect("read replayed fi")
            .map(semantic_token),
        Some(Token::Cs(fi))
    );
}

#[test]
fn conditional_recovery_orders_relax_before_unread_token_from_active_replay() {
    let mut stores = Universe::new();
    stores.register_primitive_meaning("relax", Meaning::Relax);
    let fi = expandable_primitive(&mut stores, "fi", ExpandablePrimitive::Fi);
    let fi_token = TracedTokenWord::pack(Token::Cs(fi), OriginId::UNKNOWN);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_transient_tokens(vec![fi_token], TokenListReplayKind::Inserted);
    let triggering = input
        .next_traced_token(&mut stores)
        .expect("read triggering replay")
        .expect("fi token");
    crate::conditionals::begin_if_evaluation(
        &mut input,
        fi_token,
        crate::conditionals::ConditionMetadata::new(19, false),
    );

    crate::conditionals::handle_fi(
        semantic_token(triggering),
        triggering.origin(),
        &mut input,
        &mut tex_state::ExpansionContext::new(&mut stores),
    )
    .expect("conditional recovery");

    let relax = input
        .next_traced_token(&mut stores)
        .expect("read inserted relax")
        .expect("inserted relax");
    assert_eq!(
        stores.frozen_primitive_meaning(semantic_token(relax)),
        Some(Meaning::Relax)
    );
    assert_eq!(
        input
            .next_traced_token(&mut stores)
            .expect("read unread fi")
            .map(semantic_token),
        Some(Token::Cs(fi))
    );
}

#[test]
fn skipped_conditional_reports_incomplete_if_at_eof() {
    let mut stores = Universe::new();
    let (_, iffalse, _, _) = conditional_primitives(&mut stores);
    let list = stores.intern_token_list(&[Token::Cs(iffalse.symbol()), char_token('x')]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert!(matches!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        Err(crate::ExpandError::IncompleteIf { .. })
    ));
}

#[test]
fn skipped_conditional_closes_and_replays_outer_macro_token() {
    let mut stores = Universe::new();
    let (_, iffalse, else_cs, fi) = conditional_primitives(&mut stores);
    let outer = stores.intern("outer");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x')]);
    stores.set_macro_meaning(outer, MacroMeaning::new(MeaningFlags::OUTER, params, body));
    let list = stores.intern_token_list(&[
        Token::Cs(iffalse.symbol()),
        Token::Cs(outer.symbol()),
        Token::Cs(else_cs.symbol()),
        char_token('t'),
        Token::Cs(fi.symbol()),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        get_x_token(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        )
        .expect("outer token should be replayed"),
        Some(char_token('x'))
    );
    assert!(input.current_condition().is_none());
}

#[test]
fn skipped_source_text_is_lexed_with_current_catcodes() {
    let mut stores = Universe::new();
    stores.set_int_param(tex_state::env::banks::IntParam::END_LINE_CHAR, -1);
    stores.set_catcode('@', Catcode::Escape);
    conditional_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("@iffalse@iftrue bad@fi@else good@fi"));

    assert_eq!(
        next_expanded_chars(
            &mut input,
            &mut tex_state::ExpansionContext::new(&mut stores)
        ),
        "good"
    );
}

fn next_expanded_chars(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
) -> String {
    let mut out = String::new();
    while let Some(token) = get_x_token(input, stores).expect("expansion should succeed") {
        let Token::Char { ch, .. } = token else {
            panic!("expected character token, got {token:?}");
        };
        out.push(ch);
    }
    out
}

fn collect_expanded(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
) -> Vec<Token> {
    let mut out = Vec::new();
    while let Some(token) = get_x_token(input, stores).expect("expansion should succeed") {
        out.push(token);
    }
    out
}

fn next_expanded_chars_with_context(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    context: &mut MemoryResolverFixture,
) -> String {
    let mut out = String::new();
    while let Some(token) =
        get_x_token_with_context(input, stores, context).expect("expansion should succeed")
    {
        let Token::Char { ch, .. } = token else {
            panic!("expected character token, got {token:?}");
        };
        out.push(ch);
    }
    out
}

fn collect_expanded_with_context(
    input: &mut InputStack,
    stores: &mut tex_state::ExpansionContext<'_>,
    context: &mut MemoryResolverFixture,
) -> Vec<Token> {
    let mut out = Vec::new();
    while let Some(token) =
        get_x_token_with_context(input, stores, context).expect("expansion should succeed")
    {
        out.push(token);
    }
    out
}

fn char_token(ch: char) -> Token {
    let cat = match ch {
        '{' => Catcode::BeginGroup,
        '}' => Catcode::EndGroup,
        '0'..='9' | '[' | ']' | '!' | '<' | '=' | '>' | '-' => Catcode::Other,
        _ => Catcode::Letter,
    };
    Token::Char { ch, cat }
}

fn active_token(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Active,
    }
}

fn active_expandable_primitive(
    stores: &mut Universe,
    ch: char,
    primitive: ExpandablePrimitive,
) -> Token {
    let symbol = stores.intern_active_character(ch);
    stores.set_meaning(symbol, Meaning::ExpandablePrimitive(primitive));
    active_token(ch)
}

#[derive(Clone, Copy)]
enum BoxKindForTest {
    HBox,
    VBox,
}

fn boxed_list(stores: &mut Universe, kind: BoxKindForTest) -> tex_state::ids::NodeListId {
    let empty = stores.freeze_node_list(&[]);
    let box_node = BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: empty,
    });
    match kind {
        BoxKindForTest::HBox => stores.freeze_node_list(&[Node::HList(box_node)]),
        BoxKindForTest::VBox => stores.freeze_node_list(&[Node::VList(box_node)]),
    }
}

fn expandable_primitive(
    stores: &mut Universe,
    name: &str,
    primitive: ExpandablePrimitive,
) -> Symbol {
    let symbol = stores.intern(name);
    stores.set_meaning(symbol, Meaning::ExpandablePrimitive(primitive));
    symbol.symbol()
}

fn csname_primitives(stores: &mut Universe) -> (Symbol, Symbol) {
    (
        expandable_primitive(stores, "csname", ExpandablePrimitive::CsName),
        expandable_primitive(stores, "endcsname", ExpandablePrimitive::EndCsName),
    )
}

fn conditional_primitives(stores: &mut Universe) -> (Symbol, Symbol, Symbol, Symbol) {
    (
        expandable_primitive(stores, "iftrue", ExpandablePrimitive::IfTrue),
        expandable_primitive(stores, "iffalse", ExpandablePrimitive::IfFalse),
        expandable_primitive(stores, "else", ExpandablePrimitive::Else),
        expandable_primitive(stores, "fi", ExpandablePrimitive::Fi),
    )
}

struct MemoryResolverFixture {
    job_name: String,
    resolver: MemoryResolver,
    engine: crate::EngineStateSnapshot,
}

struct MemoryResolver {
    sources: AHashMap<String, String>,
    opened: Vec<String>,
    sized: Vec<String>,
}

struct SuspendingResolver;

impl crate::InputResolver for SuspendingResolver {
    fn open_input(
        &mut self,
        _input: &mut dyn InputReadState,
        _name: &str,
        request_index: u64,
    ) -> crate::ResourceResult<Box<dyn tex_lex::InputSource>> {
        Ok(crate::ResourceLookup::NeedResource(
            crate::ResourceNeed::new(request_index),
        ))
    }

    fn input_file_size(
        &mut self,
        _input: &mut dyn InputReadState,
        _name: &str,
        request_index: u64,
    ) -> crate::ResourceResult<u64> {
        Ok(crate::ResourceLookup::NeedResource(
            crate::ResourceNeed::new(request_index),
        ))
    }
}

impl MemoryResolverFixture {
    fn new(job_name: &str) -> Self {
        Self {
            job_name: job_name.to_owned(),
            resolver: MemoryResolver {
                sources: AHashMap::new(),
                opened: Vec::new(),
                sized: Vec::new(),
            },
            engine: crate::EngineStateSnapshot::default(),
        }
    }

    fn with_source(mut self, name: &str, input: &str) -> Self {
        self.resolver
            .sources
            .insert(name.to_owned(), input.to_owned());
        self
    }

    fn with_mode(mut self, mode: EngineMode) -> Self {
        self.engine.mode = mode;
        self
    }

    fn with_inner(mut self, inner: bool) -> Self {
        self.engine.is_inner_mode = inner;
        self
    }

    fn expansion_context(&mut self) -> ExpansionContext<'_>
    where
        MemoryResolver: crate::InputResolver,
    {
        let mut context = ExpansionContext::with_input_resolver(&self.job_name, &mut self.resolver);
        context.engine = self.engine;
        context
    }
}

impl crate::InputResolver for MemoryResolver {
    fn open_input(
        &mut self,
        _input: &mut dyn InputReadState,
        name: &str,
        _request_index: u64,
    ) -> crate::ResourceResult<Box<dyn tex_lex::InputSource>> {
        let Some(source) = self.sources.get(name) else {
            return Ok(crate::ResourceLookup::Unavailable);
        };
        self.opened.push(name.to_owned());
        Ok(crate::ResourceLookup::Available(Box::new(
            MemoryInput::new(source.clone()),
        )))
    }

    fn input_file_size(
        &mut self,
        _input: &mut dyn InputReadState,
        name: &str,
        _request_index: u64,
    ) -> crate::ResourceResult<u64> {
        self.sized.push(name.to_owned());
        Ok(self
            .sources
            .get(name)
            .map_or(crate::ResourceLookup::Unavailable, |source| {
                crate::ResourceLookup::Available(
                    u64::try_from(source.len()).expect("test source length should fit in u64"),
                )
            }))
    }
}
