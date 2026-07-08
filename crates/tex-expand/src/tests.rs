use crate::{
    EngineMode, ExpandableOpcode, ExpansionHooks, NoopRecorder, ReadRecorder, dispatch,
    dispatch_expandable_opcode, get_x_token, get_x_token_with_hooks, get_x_token_with_recorder,
};
use std::collections::HashMap;
use tex_lex::{InputStack, MemoryInput, TokenListReplayKind};
use tex_state::Universe;
use tex_state::glue::{GlueSpec, Order};
use tex_state::interner::Symbol;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::node::{BoxNode, BoxNodeFields, Node, Sign};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};

#[derive(Default)]
struct CountingRecorder {
    reads: usize,
}

impl ReadRecorder for CountingRecorder {
    fn record_meaning(&mut self, _symbol: Symbol, _meaning: Meaning) {
        self.reads += 1;
    }
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
        crate::Dispatch::Deliver(token)
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
fn get_x_token_delivers_unexpandable_control_sequence() {
    let mut stores = Universe::new();
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
    let mut input = InputStack::new(MemoryInput::new(""));
    let list = stores.intern_token_list(&[Token::Cs(relax)]);
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
        Some(Token::Cs(relax))
    );
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
        Some(Token::Cs(relax))
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
    stores.set_macro_meaning(
        macro_cs,
        MacroMeaning::new(MeaningFlags::EMPTY, params, body),
    );
    let invocation = stores.intern_token_list(&[Token::Cs(macro_cs)]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(invocation, TokenListReplayKind::Inserted);

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
            macro_arguments
        }) if *token_list == body && macro_arguments.is_empty()
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
fn recorder_observes_one_meaning_read_per_control_sequence_token() {
    let mut stores = Universe::new();
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
    let list = stores.intern_token_list(&[Token::Cs(relax)]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);
    let mut recorder = CountingRecorder::default();

    assert_eq!(
        get_x_token_with_recorder(&mut input, &mut stores, &mut recorder)
            .expect("expansion should succeed"),
        Some(Token::Cs(relax))
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

    let input_list =
        stores.intern_token_list(&[Token::Cs(expandafter), char_token('a'), Token::Cs(macro_cs)]);
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
        Token::Cs(expandafter),
        Token::Cs(expandafter),
        Token::Cs(expandafter),
        Token::Cs(first),
        Token::Cs(second),
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
        Token::Cs(noexpand),
        Token::Cs(macro_cs),
        Token::Cs(macro_cs),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(input_list, TokenListReplayKind::Inserted);

    assert_eq!(
        get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
        Some(Token::Cs(macro_cs))
    );
    assert_eq!(
        get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
        Some(char_token('x'))
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
        Token::Cs(expandafter),
        char_token('a'),
        Token::Cs(noexpand),
        Token::Cs(macro_cs),
        Token::Cs(macro_cs),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(input_list, TokenListReplayKind::Inserted);

    assert_eq!(
        get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
        Some(char_token('a'))
    );
    assert_eq!(
        get_x_token(&mut input, &mut stores).expect("expansion should succeed"),
        Some(Token::Cs(macro_cs))
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
        Token::Cs(csname),
        char_token('f'),
        char_token('o'),
        char_token('o'),
        Token::Cs(endcsname),
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
        Token::Cs(csname),
        char_token('f'),
        Token::Cs(macro_cs),
        Token::Cs(endcsname),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(input_list, TokenListReplayKind::Inserted);

    assert_eq!(
        get_x_token(&mut input, &mut stores).expect("csname expansion should succeed"),
        Some(Token::Cs(
            stores
                .symbol("fbar")
                .expect("expanded name should be interned")
        ))
    );
}

#[test]
fn csname_reports_non_character_material_after_expansion() {
    let mut stores = Universe::new();
    let (csname, endcsname) = csname_primitives(&mut stores);
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
    let input_list =
        stores.intern_token_list(&[Token::Cs(csname), Token::Cs(relax), Token::Cs(endcsname)]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(input_list, TokenListReplayKind::Inserted);

    assert!(matches!(
        get_x_token(&mut input, &mut stores),
        Err(crate::ExpandError::NonCharacterInCsName(Token::Cs(found))) if found == relax
    ));
}

#[test]
fn csname_preserves_existing_meaning_for_ifx_relax_comparison() {
    let mut stores = Universe::new();
    let (csname, endcsname) = csname_primitives(&mut stores);
    let existing = stores.intern("known");
    stores.set_meaning(existing, Meaning::CharGiven('K'));
    let input_list = stores.intern_token_list(&[
        Token::Cs(csname),
        char_token('k'),
        char_token('n'),
        char_token('o'),
        char_token('w'),
        char_token('n'),
        Token::Cs(endcsname),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(input_list, TokenListReplayKind::Inserted);

    assert_eq!(
        get_x_token(&mut input, &mut stores).expect("csname expansion should succeed"),
        Some(Token::Cs(existing))
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
        Token::Cs(csname),
        char_token('n'),
        char_token('e'),
        char_token('w'),
        Token::Cs(endcsname),
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
        Token::Cs(macro_cs),
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
        Token::Cs(wrap),
        char_token('{'),
        Token::param(1),
        char_token('}'),
    ]);
    stores.set_macro_meaning(
        outer,
        MacroMeaning::new(MeaningFlags::EMPTY, outer_params, outer_body),
    );

    let invocation = stores.intern_token_list(&[
        Token::Cs(outer),
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
        Token::Cs(left),
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
        Token::Cs(right),
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
        Token::Cs(left),
        char_token('x'),
        Token::Cs(right),
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
        Token::Cs(string),
        Token::Cs(target),
        Token::Cs(string),
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
    let list = stores.intern_token_list(&[Token::Cs(string), Token::Cs(target)]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "foo");
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
        Token::Cs(number),
        Token::Char {
            ch: '-',
            cat: Catcode::Other,
        },
        Token::Cs(digits),
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
        Token::Cs(roman),
        Token::Char {
            ch: '0',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
        Token::Cs(roman),
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
        Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::Count),
    );
    let catcode = stores.intern("catcode");
    stores.set_meaning(
        catcode,
        Meaning::UnexpandablePrimitive(tex_state::meaning::UnexpandablePrimitive::CatCode),
    );
    let foo = stores.intern("foo");
    stores.set_meaning(foo, Meaning::CountRegister(300));
    stores.set_count(300, 42);
    let parskip = stores.intern("parskip");
    stores.set_meaning(parskip, Meaning::GlueParam(2));
    let glue = stores.intern_glue(GlueSpec {
        width: Scaled::from_raw(Scaled::UNITY),
        stretch: Scaled::from_raw(2 * Scaled::UNITY),
        stretch_order: Order::Fil,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
    });
    stores.set_glue_param(tex_state::env::banks::GlueParam::new(2), glue);
    let everypar = stores.intern("everypar");
    stores.set_meaning(everypar, Meaning::TokParam(1));
    let hi = stores.intern_token_list(&[char_token('h'), char_token('i')]);
    stores.set_tok_param(tex_state::env::banks::TokParam::new(1), hi);
    let mut input = InputStack::new(MemoryInput::new(
        "\\the\\count300 \\the\\foo \\the\\parskip \\the\\everypar \\the\\catcode`x",
    ));

    assert_eq!(
        next_expanded_chars(&mut input, &mut stores),
        "42421.0pt plus 2.0filhi11"
    );
}

#[test]
fn number_scanner_preserves_driver_hooks_during_nested_expansion() {
    let mut stores = Universe::new();
    let number = expandable_primitive(&mut stores, "number", ExpandablePrimitive::Number);
    let input_primitive = expandable_primitive(&mut stores, "input", ExpandablePrimitive::Input);
    let digits = stores.intern("digits");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[
        Token::Cs(input_primitive),
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
    let list = stores.intern_token_list(&[Token::Cs(number), Token::Cs(digits)]);
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
    let list = stores.intern_token_list(&[Token::Cs(meaning), Token::Cs(macro_cs)]);
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
        Token::Cs(the),
        Token::Cs(count),
        char_token('2'),
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
        Token::Cs(the),
        Token::Cs(dimen),
        char_token('3'),
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
        Token::Cs(the),
        Token::Cs(toks),
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
    let list = stores.intern_token_list(&[Token::Cs(number), char_token('7')]);
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
fn fontname_stub_consumes_selector_and_expands_empty() {
    let mut stores = Universe::new();
    expandable_primitive(&mut stores, "fontname", ExpandablePrimitive::FontName);
    let nullfont = stores.intern("nullfont");
    stores.set_meaning(nullfont, Meaning::Relax);
    let list = stores.intern_token_list(&[
        Token::Cs(stores.symbol("fontname").expect("fontname")),
        Token::Cs(nullfont),
        char_token('z'),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "z");
}

#[test]
fn mark_family_stubs_expand_empty() {
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
        Token::Cs(stores.symbol("topmark").expect("topmark")),
        Token::Cs(stores.symbol("firstmark").expect("firstmark")),
        Token::Cs(stores.symbol("botmark").expect("botmark")),
        Token::Cs(stores.symbol("splitfirstmark").expect("splitfirstmark")),
        Token::Cs(stores.symbol("splitbotmark").expect("splitbotmark")),
        char_token('z'),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "z");
}

#[test]
fn iftrue_and_iffalse_select_expected_two_limb_branches() {
    let mut stores = Universe::new();
    let (iftrue, iffalse, else_cs, fi) = conditional_primitives(&mut stores);
    let list = stores.intern_token_list(&[
        Token::Cs(iftrue),
        char_token('t'),
        Token::Cs(else_cs),
        char_token('f'),
        Token::Cs(fi),
        Token::Cs(iffalse),
        char_token('f'),
        Token::Cs(else_cs),
        char_token('t'),
        Token::Cs(fi),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "tt");
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
        Token::Cs(if_cs),
        Token::Cs(left),
        Token::Cs(right),
        char_token('y'),
        Token::Cs(else_cs),
        char_token('n'),
        Token::Cs(fi),
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
        Token::Cs(ifcat),
        char_token('a'),
        Token::Cs(macro_cs),
        char_token('y'),
        Token::Cs(else_cs),
        char_token('n'),
        Token::Cs(fi),
        Token::Cs(ifcat),
        char_token('a'),
        Token::Char {
            ch: '1',
            cat: Catcode::Other,
        },
        char_token('n'),
        Token::Cs(else_cs),
        char_token('y'),
        Token::Cs(fi),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "yy");
}

#[test]
fn ifx_compares_identical_macro_definitions_by_flags_and_hash_consed_ids() {
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
    stores.set_macro_meaning(
        left,
        MacroMeaning::new(MeaningFlags::EMPTY, params, left_body),
    );
    stores.set_macro_meaning(
        right,
        MacroMeaning::new(MeaningFlags::EMPTY, params, right_body),
    );
    stores.set_macro_meaning(
        protected,
        MacroMeaning::new(MeaningFlags::PROTECTED, params, right_body),
    );
    let list = stores.intern_token_list(&[
        Token::Cs(ifx),
        Token::Cs(left),
        Token::Cs(right),
        char_token('y'),
        Token::Cs(else_cs),
        char_token('n'),
        Token::Cs(fi),
        Token::Cs(ifx),
        Token::Cs(left),
        Token::Cs(protected),
        char_token('n'),
        Token::Cs(else_cs),
        char_token('y'),
        Token::Cs(fi),
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
        Token::Cs(ifx),
        Token::Cs(first),
        Token::Cs(second),
        char_token('y'),
        Token::Cs(else_cs),
        char_token('n'),
        Token::Cs(fi),
        Token::Cs(ifx),
        Token::Cs(macro_cs),
        char_token('a'),
        char_token('n'),
        Token::Cs(else_cs),
        char_token('y'),
        Token::Cs(fi),
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
    let list = stores.intern_token_list(&[
        Token::Cs(ifnum),
        Token::Cs(count),
        char_token('2'),
        char_token('>'),
        char_token('6'),
        char_token('y'),
        Token::Cs(else_cs),
        char_token('n'),
        Token::Cs(fi),
        Token::Cs(ifdim),
        Token::Cs(dimen),
        char_token('3'),
        char_token('='),
        char_token('1'),
        char_token('p'),
        char_token('t'),
        char_token('y'),
        Token::Cs(else_cs),
        char_token('n'),
        Token::Cs(fi),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "yy");
}

#[test]
fn ifodd_and_ifcase_select_expected_limb() {
    let mut stores = Universe::new();
    let (_, _, else_cs, fi) = conditional_primitives(&mut stores);
    let or_cs = expandable_primitive(&mut stores, "or", ExpandablePrimitive::Or);
    let ifodd = expandable_primitive(&mut stores, "ifodd", ExpandablePrimitive::IfOdd);
    let ifcase = expandable_primitive(&mut stores, "ifcase", ExpandablePrimitive::IfCase);
    let list = stores.intern_token_list(&[
        Token::Cs(ifodd),
        char_token('-'),
        char_token('3'),
        char_token('y'),
        Token::Cs(else_cs),
        char_token('n'),
        Token::Cs(fi),
        Token::Cs(ifcase),
        char_token('2'),
        char_token('z'),
        Token::Cs(or_cs),
        char_token('o'),
        Token::Cs(or_cs),
        char_token('t'),
        Token::Cs(or_cs),
        char_token('x'),
        Token::Cs(else_cs),
        char_token('e'),
        Token::Cs(fi),
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
        Token::Cs(stores.symbol("ifhmode").expect("ifhmode")),
        char_token('h'),
        Token::Cs(else_cs),
        char_token('n'),
        Token::Cs(fi),
        Token::Cs(stores.symbol("ifvmode").expect("ifvmode")),
        char_token('n'),
        Token::Cs(else_cs),
        char_token('v'),
        Token::Cs(fi),
        Token::Cs(stores.symbol("ifinner").expect("ifinner")),
        char_token('i'),
        Token::Cs(else_cs),
        char_token('n'),
        Token::Cs(fi),
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
        Token::Cs(ifvoid),
        char_token('0'),
        char_token('v'),
        Token::Cs(else_cs),
        char_token('n'),
        Token::Cs(fi),
        Token::Cs(ifhbox),
        char_token('1'),
        char_token('h'),
        Token::Cs(else_cs),
        char_token('n'),
        Token::Cs(fi),
        Token::Cs(ifvbox),
        char_token('2'),
        char_token('b'),
        Token::Cs(else_cs),
        char_token('n'),
        Token::Cs(fi),
        Token::Cs(ifhbox),
        char_token('2'),
        char_token('n'),
        Token::Cs(else_cs),
        char_token('x'),
        Token::Cs(fi),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(next_expanded_chars(&mut input, &mut stores), "vhbx");
}

#[test]
fn ifeof_uses_hook_and_noop_hook_reports_eof_stub() {
    let mut stores = Universe::new();
    let (_, _, else_cs, fi) = conditional_primitives(&mut stores);
    let ifeof = expandable_primitive(&mut stores, "ifeof", ExpandablePrimitive::IfEof);
    let list = stores.intern_token_list(&[
        Token::Cs(ifeof),
        char_token('1'),
        char_token('n'),
        Token::Cs(else_cs),
        char_token('o'),
        Token::Cs(fi),
        Token::Cs(ifeof),
        char_token('2'),
        char_token('e'),
        Token::Cs(else_cs),
        char_token('n'),
        Token::Cs(fi),
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
        Token::Cs(ifeof),
        char_token('9'),
        char_token('e'),
        Token::Cs(else_cs),
        char_token('n'),
        Token::Cs(fi),
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
        Token::Cs(iffalse),
        char_token('x'),
        Token::Cs(iftrue),
        char_token('y'),
        Token::Cs(else_cs),
        char_token('z'),
        Token::Cs(fi),
        Token::Cs(else_cs),
        char_token('t'),
        Token::Cs(fi),
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
        Token::Cs(ifcase),
        char_token('0'),
        char_token('z'),
        Token::Cs(or_cs),
        char_token('o'),
        Token::Cs(else_cs),
        char_token('e'),
        Token::Cs(fi),
        Token::Cs(ifcase),
        char_token('-'),
        char_token('1'),
        char_token('z'),
        Token::Cs(or_cs),
        char_token('o'),
        Token::Cs(else_cs),
        char_token('e'),
        Token::Cs(fi),
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
        let list = stores.intern_token_list(&[Token::Cs(control)]);
        let mut input = InputStack::new(MemoryInput::new(""));
        input.push_token_list(list, TokenListReplayKind::Inserted);

        assert!(matches!(
            get_x_token(&mut input, &mut stores),
            Err(crate::ExpandError::ExtraConditionalControl(found)) if found == expected
        ));
    }
}

#[test]
fn skipped_conditional_reports_incomplete_if_at_eof() {
    let mut stores = Universe::new();
    let (_, iffalse, _, _) = conditional_primitives(&mut stores);
    let list = stores.intern_token_list(&[Token::Cs(iffalse), char_token('x')]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert!(matches!(
        get_x_token(&mut input, &mut stores),
        Err(crate::ExpandError::IncompleteIf)
    ));
}

#[test]
fn skipped_conditional_rejects_outer_macro_tokens() {
    let mut stores = Universe::new();
    let (_, iffalse, else_cs, fi) = conditional_primitives(&mut stores);
    let outer = stores.intern("outer");
    let params = stores.intern_token_list(&[]);
    let body = stores.intern_token_list(&[char_token('x')]);
    stores.set_macro_meaning(outer, MacroMeaning::new(MeaningFlags::OUTER, params, body));
    let list = stores.intern_token_list(&[
        Token::Cs(iffalse),
        Token::Cs(outer),
        Token::Cs(else_cs),
        char_token('t'),
        Token::Cs(fi),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(list, TokenListReplayKind::Inserted);

    assert!(matches!(
        get_x_token(&mut input, &mut stores),
        Err(crate::ExpandError::ForbiddenOuterTokenInSkippedConditional { ref name })
            if name == "\\outer"
    ));
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

fn next_expanded_chars(input: &mut InputStack<MemoryInput>, stores: &mut Universe) -> String {
    let mut out = String::new();
    while let Some(token) = get_x_token(input, stores).expect("expansion should succeed") {
        let Token::Char { ch, .. } = token else {
            panic!("expected character token, got {token:?}");
        };
        out.push(ch);
    }
    out
}

fn collect_expanded(input: &mut InputStack<MemoryInput>, stores: &mut Universe) -> Vec<Token> {
    let mut out = Vec::new();
    while let Some(token) = get_x_token(input, stores).expect("expansion should succeed") {
        out.push(token);
    }
    out
}

fn next_expanded_chars_with_hooks(
    input: &mut InputStack<MemoryInput>,
    stores: &mut Universe,
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
    stores: &mut Universe,
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
        glue_set: 0.0,
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
    symbol
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
    fn open_input(&mut self, _stores: &mut Universe, name: &str) -> Result<MemoryInput, String> {
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

    fn input_stream_eof(&self, stream: u8) -> bool {
        self.eof.get(&stream).copied().unwrap_or(true)
    }
}
