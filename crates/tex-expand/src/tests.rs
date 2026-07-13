use crate::{
    EngineMode, ExpandableOpcode, ExpansionHooks, NoopExpansionHooks, NoopRecorder, ReadRecorder,
    dispatch, dispatch_expandable_opcode, dispatch_with_hooks, install_expandable_primitives,
};
use std::collections::HashMap;
use tex_lex::{InputStack, MemoryInput, TokenListReplayKind};
use tex_state::glue::{GlueSpec, Order};
use tex_state::interner::Symbol;
use tex_state::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags, UnexpandablePrimitive};
use tex_state::node::{BoxNode, BoxNodeFields, Node, Sign};
use tex_state::page::PageMark;
use tex_state::provenance::{
    InsertedOrigin, InsertedOriginKind, MacroInvocationOrigin, OriginRecord, SynthesizedOrigin,
    SynthesizedOriginKind,
};
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{ExpansionState, InputOpenState, InputReadState, Universe};

#[test]
fn get_x_token_converts_frozen_end_template_without_losing_origin() {
    let mut stores = Universe::new();
    let origin = stores.source_origin(tex_state::SourceId::new(7), 19, 3, 5);
    let tokens = stores.intern_token_list(&[stores.frozen_end_template_token()]);
    let origins = stores.allocate_origin_list(&[origin]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list_with_origins(tokens, origins, TokenListReplayKind::Inserted);

    let delivered = crate::get_x_token(&mut input, &mut stores)
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

    let delivered = crate::expand_once_then_get_token_with_hooks(
        &mut input,
        &mut stores,
        &mut NoopRecorder,
        &mut NoopExpansionHooks,
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

fn get_x_token<S>(
    input: &mut InputStack<S>,
    stores: &mut (impl ExpansionState + InputOpenState),
) -> Result<Option<Token>, crate::ExpandError>
where
    S: tex_lex::InputSource,
{
    crate::get_x_token(input, stores).map(|token| token.map(crate::semantic_token))
}

fn get_x_token_with_recorder<S, R>(
    input: &mut InputStack<S>,
    stores: &mut (impl ExpansionState + InputOpenState),
    recorder: &mut R,
) -> Result<Option<Token>, crate::ExpandError>
where
    S: tex_lex::InputSource,
    R: ReadRecorder,
{
    crate::get_x_token_with_recorder(input, stores, recorder)
        .map(|token| token.map(crate::semantic_token))
}

fn get_x_token_with_hooks<S, H>(
    input: &mut InputStack<S>,
    stores: &mut (impl ExpansionState + InputOpenState),
    hooks: &mut H,
) -> Result<Option<Token>, crate::ExpandError>
where
    S: tex_lex::InputSource,
    H: ExpansionHooks<S>,
{
    crate::get_x_token_with_hooks(input, stores, hooks)
        .map(|token| token.map(crate::semantic_token))
}

#[test]
fn noop_recorder_has_no_state() {
    assert_eq!(core::mem::size_of::<NoopRecorder>(), 0);
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
            &mut stores,
            &mut NoopRecorder,
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
    let relation = crate::conditionals::scan_conditional_relation_with_expander_and_hooks(
        &mut input,
        &mut stores,
        &mut NoopRecorder,
        &mut crate::NoopExpansionHooks,
        &mut crate::NoInputExpandNext,
        context,
    )
    .expect("relation scanner should insert equality");

    assert_eq!(relation, crate::conditionals::ConditionalRelation::Equal);
    let token = input
        .next_traced_token(&mut stores)
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
        get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
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

    let err =
        get_x_token(&mut input, &mut stores).expect_err("undefined control sequence is rejected");
    assert!(matches!(
        err,
        crate::ExpandError::UndefinedControlSequence { ref name, .. } if name == "missing"
    ));
    let origin = err.primary_origin().expect("undefined control origin");
    assert_ne!(origin, OriginId::UNKNOWN);
    assert_eq!(
        get_x_token(&mut input, &mut stores).expect("following token should still be readable"),
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

    let err =
        get_x_token(&mut input, &mut stores).expect_err("undefined control sequence is rejected");
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

    let err =
        get_x_token(&mut input, &mut stores).expect_err("undefined control sequence is rejected");

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
        get_x_token(&mut input, &mut stores).expect("source expansion should succeed"),
        Some(Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        })
    );
    assert_eq!(
        get_x_token(&mut input, &mut stores).expect("source expansion should succeed"),
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
        get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
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
        get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
        Some(Token::Char {
            ch: 'b',
            cat: Catcode::Letter,
        })
    );
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
        get_x_token(&mut input, &mut stores).expect("protected macro expansion"),
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

    let delivered = crate::get_x_or_protected_with_recorder_and_hooks(
        &mut input,
        &mut stores,
        &mut NoopRecorder,
        &mut NoopExpansionHooks,
    )
    .expect("protected-aware expansion")
    .expect("protected macro token");
    assert_eq!(
        crate::semantic_token(delivered),
        Token::Cs(macro_cs.symbol())
    );
}

#[test]
fn unexpanded_delivers_general_text_without_expanding_macros() {
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
        get_x_token(&mut input, &mut stores).expect("unexpanded expansion"),
        Some(Token::Cs(macro_cs.symbol()))
    );
}

#[test]
fn detokenize_outputs_space_and_other_character_tokens() {
    let mut stores = Universe::new();
    crate::install_etex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\detokenize{a \\word!}%"));
    let mut output = Vec::new();
    while let Some(token) = get_x_token(&mut input, &mut stores).expect("detokenize expansion") {
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
    assert_eq!(rendered, "a \\word !");
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "yy");

    let mut invalid = InputStack::new(MemoryInput::new("\\unless\\ifcase0\\fi"));
    assert!(matches!(
        crate::get_x_token(&mut invalid, &mut stores),
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
        get_x_token(&mut input, &mut stores).expect("active character from pseudo-file"),
        Some(char_token('A'))
    );
    assert_eq!(
        get_x_token(&mut input, &mut stores).expect("superscript notation from pseudo-file"),
        Some(char_token('B'))
    );
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
        get_x_token(&mut virtual_input, &mut stores).expect("source token"),
        Some(char_token('Z'))
    );
    assert_eq!(
        get_x_token(&mut virtual_input, &mut stores).expect("source endline"),
        Some(Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        })
    );
    assert_eq!(
        get_x_token(&mut virtual_input, &mut stores).expect("everyeof token"),
        Some(char_token('E'))
    );

    let mut forced = InputStack::new(MemoryInput::new("\\endinput Z"));
    assert_eq!(
        get_x_token(&mut forced, &mut stores).expect("endinput line token"),
        Some(char_token('Z'))
    );
    assert_eq!(
        get_x_token(&mut forced, &mut stores).expect("forced source endline"),
        Some(Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        })
    );
    assert_eq!(
        get_x_token(&mut forced, &mut stores).expect("forced eof"),
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
            crate::next_semantic_raw_token(&mut input, &mut stores)
                .expect("nested token")
                .expect("nested token present")
        ),
        char_token('I')
    );
    assert_eq!(
        crate::semantic_token(
            crate::next_semantic_raw_token(&mut input, &mut stores)
                .expect("everyeof token")
                .expect("everyeof token present")
        ),
        char_token('E')
    );
    assert_eq!(
        crate::semantic_token(
            crate::next_semantic_raw_token(&mut input, &mut stores)
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "2.6");
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "2,14");
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
        next_expanded_chars(&mut input, &mut stores),
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
        next_expanded_chars(&mut input, &mut stores),
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "TTTT");
    assert!(stores.symbol("nevercreated").is_none());
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

    let error = crate::get_x_token(&mut input, &mut stores)
        .expect_err("undefined body token must diagnose");
    let site = error.diagnostic_site();
    assert_eq!(site.primary_origin(), Some(body_origin));
    let expansion_head = site.expansion_head().expect("macro expansion head");

    assert!(
        input
            .next_traced_token(&mut stores)
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

    let expanded = crate::get_x_token(&mut input, &mut stores)
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
        get_x_token_with_recorder(&mut input, &mut stores, &mut recorder)
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "axy");
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "12");
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
        get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
        Some(Token::Cs(macro_cs.symbol()))
    );
    assert_eq!(
        get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
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

    let traced = crate::get_x_token(&mut input, &mut stores)
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
        get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
        Some(char_token('a'))
    );
    assert_eq!(
        get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
        Some(Token::Cs(macro_cs.symbol()))
    );
    assert_eq!(
        get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
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
    let token = get_x_token(&mut input, &mut stores)
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
        get_x_token(&mut input, &mut stores).expect("csname expansion should succeed"),
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

    let delivered = crate::get_x_token(&mut input, &mut stores)
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

    let Some(Token::Cs(empty)) =
        get_x_token(&mut input, &mut stores).expect("csname recovery should succeed")
    else {
        panic!("expected recovered empty control sequence");
    };
    assert_eq!(stores.resolve(empty), "");
    assert_eq!(stores.meaning(empty), Meaning::Relax);

    assert_eq!(
        get_x_token(&mut input, &mut stores).expect("pushed-back token should expand"),
        Some(Token::Cs(relax.symbol()))
    );
    assert_eq!(
        get_x_token(&mut input, &mut stores).expect("remaining endcsname should be delivered"),
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
        get_x_token(&mut input, &mut stores).expect("csname expansion should succeed"),
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

    let Some(Token::Cs(created)) =
        get_x_token(&mut input, &mut stores).expect("csname expansion should succeed")
    else {
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "axyb");
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
        &mut stores,
        &mut NoopRecorder,
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
        .next_traced_token(&mut stores)
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
    } = dispatch_with_hooks(
        Token::Cs(macro_cs.symbol()),
        invocation_origin,
        &mut input,
        &mut stores,
        &mut NoopRecorder,
        &mut NoopExpansionHooks,
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
        collect_expanded(&mut input, &mut stores),
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
        collect_expanded(&mut input_stack, &mut stores),
        vec![
            char_token('1'),
            char_token('2'),
            char_token('3'),
            char_token('4')
        ]
    );
    let growth = stores.provenance_stats().saturating_sub(before);

    assert_eq!(growth.origin_records(), 1);
    assert_eq!(growth.origin_list_spans(), 1);
    assert_eq!(growth.origin_list_entries(), 4);
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "[xy]");
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
        &mut stores,
        &mut NoopRecorder,
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
        stores.tokens(left_arguments.get(1).expect("left #1")),
        &[char_token('x')]
    );

    let right_arg = stores.intern_token_list(&[char_token('y')]);
    let mut right_input = InputStack::new(MemoryInput::new(""));
    right_input.push_token_list(right_arg, TokenListReplayKind::Inserted);
    let right_meaning = stores.meaning(right);
    let right_dispatch = dispatch(
        Token::Cs(right.symbol()),
        &mut right_input,
        &mut stores,
        &mut NoopRecorder,
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
        stores.tokens(right_arguments.get(1).expect("right #1")),
        &[char_token('y')]
    );

    let invocation = stores.intern_token_list(&[
        Token::Cs(left.symbol()),
        char_token('x'),
        Token::Cs(right.symbol()),
        char_token('y'),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(invocation, TokenListReplayKind::Inserted);

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "x!y!");
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
        collect_expanded(&mut input, &mut stores),
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "foo");
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "-19mmmm");
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
        next_expanded_chars(&mut input, &mut stores),
        "42421.0pt plus 2.0fil3.0mu plus 4.0mu minus 5.0muhi11"
    );
}

#[test]
fn the_records_value_and_code_generation_dependencies_that_mutations_invalidate() {
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
    while get_x_token_with_recorder(&mut input, &mut stores, &mut reads)
        .expect("recorded expansion")
        .is_some()
    {}

    let dependencies = reads.dependencies().collect::<Vec<_>>();
    assert!(dependencies.contains(&crate::ReadDependency::Cell {
        bank: crate::ReadBank::Count,
        index: 7,
    }));
    assert!(
        dependencies.contains(&crate::ReadDependency::CodeGeneration(
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
fn number_scanner_preserves_driver_hooks_during_nested_expansion() {
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
    let mut hooks = MemoryHooks::new("job").with_source("digs", "42");

    assert_eq!(
        next_expanded_chars_with_hooks(&mut input, &mut stores, &mut hooks),
        "42"
    );
    assert_eq!(hooks.opened, vec!["digs"]);
}

#[test]
fn meaning_renders_macro_text_and_output_catcodes() {
    let mut stores = Universe::new();
    let meaning = expandable_primitive(&mut stores, "meaning", ExpandablePrimitive::Meaning);
    let macro_cs = stores.intern("m");
    let params = stores.intern_token_list(&[Token::param(1)]);
    let body = stores.intern_token_list(&[char_token('a'), Token::param(1)]);
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
    );
    let list =
        stores.intern_token_list(&[Token::Cs(meaning.symbol()), Token::Cs(macro_cs.symbol())]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    let tokens = collect_expanded(&mut input, &mut stores);
    let text = tokens
        .iter()
        .map(|token| match token {
            Token::Char { ch, .. } => *ch,
            other => panic!("expected character token, got {other:?}"),
        })
        .collect::<String>();

    assert_eq!(text, "macro:#1->a#1");
    assert!(tokens.iter().all(|token| matches!(
        token,
        Token::Char {
            cat: Catcode::Other | Catcode::Space,
            ..
        }
    )));
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
        next_expanded_chars(&mut input, &mut stores),
        "-421.00002ptA!"
    );
}

#[test]
fn rendered_output_is_frozen_and_rollback_removes_it() {
    let mut stores = Universe::new();
    let snapshot = stores.snapshot();
    let number = expandable_primitive(&mut stores, "number", ExpandablePrimitive::Number);
    let list = stores.intern_token_list(&[Token::Cs(number.symbol()), char_token('7')]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        get_x_token(&mut input, &mut stores).expect("number should expand"),
        Some(Token::Char {
            ch: '7',
            cat: Catcode::Other
        })
    );
    let rendered = match input.summary().frames().last() {
        Some(tex_lex::InputFrameSummary::TokenList { token_list, .. }) => *token_list,
        other => panic!("expected rendered token-list frame, got {other:?}"),
    };

    stores.rollback(&snapshot);
    let err = std::panic::catch_unwind(|| stores.tokens(rendered));
    assert!(
        err.is_err(),
        "rendered output must be rollback-coupled frozen content"
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

    let first = crate::get_x_token(&mut input, &mut stores)
        .expect("number should expand")
        .expect("first digit should be delivered");
    let second = crate::get_x_token(&mut input, &mut stores)
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
    let mut hooks = MemoryHooks::new("main").with_source("inc", "ab");

    assert_eq!(
        next_expanded_chars_with_hooks(&mut input, &mut stores, &mut hooks),
        "ab z "
    );
    assert_eq!(hooks.opened, vec!["inc"]);
}

#[test]
fn endinput_finishes_current_line_then_pops_source() {
    let mut stores = Universe::new();
    stores.set_int_param(tex_state::env::banks::IntParam::END_LINE_CHAR, 13);
    expandable_primitive(&mut stores, "input", ExpandablePrimitive::Input);
    expandable_primitive(&mut stores, "endinput", ExpandablePrimitive::EndInput);
    let mut input = InputStack::new(MemoryInput::new("\\input{inc}z"));
    let mut hooks = MemoryHooks::new("main").with_source("inc", "a\\endinput b\nc");

    assert_eq!(
        next_expanded_chars_with_hooks(&mut input, &mut stores, &mut hooks),
        "ab z "
    );
}

#[test]
fn jobname_expands_from_driver_hook_as_rendered_tokens() {
    let mut stores = Universe::new();
    expandable_primitive(&mut stores, "jobname", ExpandablePrimitive::JobName);
    let mut input = InputStack::new(MemoryInput::new("\\jobname"));
    let mut hooks = MemoryHooks::new("paper");

    let tokens = collect_expanded_with_hooks(&mut input, &mut stores, &mut hooks);
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "nullfontz");
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
            true,
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
    let mut output = Vec::new();
    while let Some(token) = crate::get_x_token_with_recorder(&mut input, &mut stores, &mut recorder)
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
            true,
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
    while let Some(token) =
        crate::get_x_token(&mut input, &mut stores).expect("unavailable fontdimen yields zero")
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
    let mut output = Vec::new();
    while let Some(token) = crate::get_x_token_with_recorder(&mut input, &mut stores, &mut recorder)
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

    let error = crate::get_x_token(&mut input, &mut stores)
        .expect_err("the substituted null font has no printable control sequence");
    assert!(matches!(
        error,
        crate::ExpandError::UnsupportedTheTarget { .. }
    ));
}

#[test]
fn mark_family_primitives_expand_stored_page_marks() {
    let mut stores = Universe::new();
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "z");

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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "TFBSs");
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "tt");
}

#[test]
fn unless_inverts_boolean_conditionals_without_leaking_frames() {
    let mut stores = Universe::new();
    crate::install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\unless\\iftrue f\\else t\\fi\\unless\\iffalse t\\else f\\fi%",
    ));

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "tt");
    assert!(input.current_condition().is_none());
}

#[test]
fn unless_inverts_scanned_numeric_condition() {
    let mut stores = Universe::new();
    crate::install_expandable_primitives(&mut stores);
    crate::install_etex_expandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\unless\\ifnum1<2 f\\else t\\fi%"));

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "t");
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "y");
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "yy");
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "yy");
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
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "yy");
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "yy");
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "y");
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "n");
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "y");
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
        collect_expanded(&mut input, &mut stores),
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "yt");
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
    let mut hooks = MemoryHooks::new("main")
        .with_mode(EngineMode::Horizontal)
        .with_inner(true);

    assert_eq!(
        next_expanded_chars_with_hooks(&mut input, &mut stores, &mut hooks),
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "vhbx");
}

#[test]
fn ifeof_uses_hook_and_default_world_stream_state() {
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
    let mut hooks = MemoryHooks::new("main")
        .with_eof(1, false)
        .with_eof(2, true);

    assert_eq!(
        next_expanded_chars_with_hooks(&mut input, &mut stores, &mut hooks),
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "e");
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "t");
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "t");
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "ze");
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
            get_x_token(&mut input, &mut stores),
            Err(crate::ExpandError::ExtraConditionalControl { name: found, .. }) if found == expected
        ));
    }
}

#[test]
fn skipped_conditional_reports_incomplete_if_at_eof() {
    let mut stores = Universe::new();
    let (_, iffalse, _, _) = conditional_primitives(&mut stores);
    let list = stores.intern_token_list(&[Token::Cs(iffalse.symbol()), char_token('x')]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert!(matches!(
        get_x_token(&mut input, &mut stores),
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
        get_x_token(&mut input, &mut stores).expect("outer token should be replayed"),
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

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "good");
}

fn next_expanded_chars(
    input: &mut InputStack<MemoryInput>,
    stores: &mut (impl ExpansionState + InputOpenState),
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
    input: &mut InputStack<MemoryInput>,
    stores: &mut (impl ExpansionState + InputOpenState),
) -> Vec<Token> {
    let mut out = Vec::new();
    while let Some(token) = get_x_token(input, stores).expect("expansion should succeed") {
        out.push(token);
    }
    out
}

fn next_expanded_chars_with_hooks(
    input: &mut InputStack<MemoryInput>,
    stores: &mut (impl ExpansionState + InputOpenState),
    hooks: &mut MemoryHooks,
) -> String {
    let mut out = String::new();
    while let Some(token) =
        get_x_token_with_hooks(input, stores, hooks).expect("expansion should succeed")
    {
        let Token::Char { ch, .. } = token else {
            panic!("expected character token, got {token:?}");
        };
        out.push(ch);
    }
    out
}

fn collect_expanded_with_hooks(
    input: &mut InputStack<MemoryInput>,
    stores: &mut (impl ExpansionState + InputOpenState),
    hooks: &mut MemoryHooks,
) -> Vec<Token> {
    let mut out = Vec::new();
    while let Some(token) =
        get_x_token_with_hooks(input, stores, hooks).expect("expansion should succeed")
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

struct MemoryHooks {
    job_name: String,
    sources: HashMap<String, String>,
    opened: Vec<String>,
    mode: EngineMode,
    inner: bool,
    eof: HashMap<u8, bool>,
}

impl MemoryHooks {
    fn new(job_name: &str) -> Self {
        Self {
            job_name: job_name.to_owned(),
            sources: HashMap::new(),
            opened: Vec::new(),
            mode: EngineMode::Vertical,
            inner: false,
            eof: HashMap::new(),
        }
    }

    fn with_source(mut self, name: &str, input: &str) -> Self {
        self.sources.insert(name.to_owned(), input.to_owned());
        self
    }

    fn with_mode(mut self, mode: EngineMode) -> Self {
        self.mode = mode;
        self
    }

    fn with_inner(mut self, inner: bool) -> Self {
        self.inner = inner;
        self
    }

    fn with_eof(mut self, stream: u8, eof: bool) -> Self {
        self.eof.insert(stream, eof);
        self
    }
}

impl ExpansionHooks<MemoryInput> for MemoryHooks {
    fn open_input<C: InputReadState>(
        &mut self,
        _input: &mut C,
        name: &str,
    ) -> Result<MemoryInput, String> {
        let source = self
            .sources
            .get(name)
            .ok_or_else(|| "missing memory source".to_owned())?;
        self.opened.push(name.to_owned());
        Ok(MemoryInput::new(source.clone()))
    }

    fn job_name(&self) -> &str {
        &self.job_name
    }

    fn mode(&self) -> EngineMode {
        self.mode
    }

    fn is_inner_mode(&self) -> bool {
        self.inner
    }

    fn input_stream_eof(&self, _stores: &impl ExpansionState, stream: u8) -> bool {
        self.eof.get(&stream).copied().unwrap_or(true)
    }
}
