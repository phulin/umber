use super::*;
use tex_lex::MemoryInput;
use tex_state::GroupKind;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::math::{MathChar, MathNoad, NoadClass, NoadKind};
use tex_state::node::{BoxNode, BoxNodeFields, Node, Sign};
use tex_state::scaled::GlueSetRatio;
use tex_state::{EffectRecord, PrintSink};

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw * Scaled::UNITY)
}

fn terminal_text(stores: &Universe) -> String {
    stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|record| match record {
            EffectRecord::StreamWrite {
                sink: PrintSink::Terminal | PrintSink::TerminalAndLog | PrintSink::Log,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn display_alignment_finish_assignments_delimiters_and_spacing() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\count0=7 $$x"));
    let mut execution = crate::ExecutionContext::new("texput");

    finish_display_alignment_assignments(&mut input, &mut stores, &mut execution)
        .expect("post-alignment assignments execute");
    assert_eq!(stores.count(0), 7);
    let fallback = stores.synthetic_origin(tex_state::provenance::SyntheticOriginKind::Test);
    assert_ne!(
        consume_display_alignment_closer(&mut input, &mut stores, &mut execution, fallback)
            .expect("double math shift closes"),
        fallback
    );
    assert_eq!(
        tex_expand::semantic_token(
            input
                .next_traced_token(&mut stores)
                .expect("following token reads")
                .expect("following token remains")
        ),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        }
    );

    let mut missing = InputStack::new(MemoryInput::new("q"));
    assert_eq!(
        consume_display_alignment_closer(&mut missing, &mut stores, &mut execution, fallback)
            .expect("missing closer recovers"),
        fallback
    );
    assert!(terminal_text(&stores).contains("Missing $$ inserted"));
    assert_eq!(
        tex_expand::semantic_token(
            missing
                .next_traced_token(&mut stores)
                .expect("backed-up token reads")
                .expect("non-math command is backed up")
        ),
        Token::Char {
            ch: 'q',
            cat: Catcode::Letter,
        }
    );

    stores.set_int_param(IntParam::PRE_DISPLAY_PENALTY, 11);
    stores.set_int_param(IntParam::POST_DISPLAY_PENALTY, 22);
    stores.set_dimen_param(DimenParam::DISPLAY_INDENT, sp(5));
    let above = stores.intern_glue(GlueSpec {
        width: sp(3),
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
    });
    let below = stores.intern_glue(GlueSpec {
        width: sp(4),
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
    });
    stores.set_glue_param(GlueParam::ABOVE_DISPLAY_SKIP, above);
    stores.set_glue_param(GlueParam::BELOW_DISPLAY_SKIP, below);
    let empty = stores.freeze_node_list(&[]);
    let alignment = Node::HList(BoxNode::new(BoxNodeFields {
        width: sp(9),
        height: sp(2),
        depth: sp(1),
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: empty,
    }));
    let second_alignment = alignment.clone();
    let mut nest = ModeNest::new();
    nest.push(Mode::InternalVertical).expect("test mode push");
    nest.current_list_mutation().set_prev_depth(sp(10));

    finish_display_alignment(
        &mut nest,
        &mut stores,
        crate::align::FinishedAlignment {
            nodes: vec![alignment, second_alignment],
            aux_prev_depth: Some(sp(7)),
        },
    )
    .expect("display list material inserts");

    let nodes = nest.current_list().nodes();
    assert!(matches!(
        nodes,
        [
            Node::Penalty(11),
            Node::Glue { spec: above_spec, kind: GlueKind::AboveDisplaySkip, .. },
            Node::HList(first_row),
            Node::HList(second_row),
            Node::Penalty(22),
            Node::Glue { spec: below_spec, kind: GlueKind::BelowDisplaySkip, .. },
        ] if *above_spec == above
            && *below_spec == below
            && first_row.display
            && second_row.display
            // §812's display insertion only marks the material as display
            // content; the `\displayindent` shift is §800's `o`, applied by
            // §806/§807 while `fin_align` sets the boxes.
            && first_row.shift == Scaled::from_raw(0)
            && second_row.shift == Scaled::from_raw(0)
    ));
    assert_eq!(nest.current_list().prev_depth(), Some(sp(7)));

    let mut resume_input = InputStack::new(MemoryInput::new("z"));
    resume_after_display_alignment(&mut nest, &mut resume_input, &mut stores, Vec::new())
        .expect("post-display scanning resumes");
    assert_eq!(nest.current_mode(), Mode::Horizontal);
    assert_eq!(
        tex_expand::semantic_token(
            resume_input
                .next_traced_token(&mut stores)
                .expect("resumed token reads")
                .expect("resumed token remains")
        ),
        Token::Char {
            ch: 'z',
            cat: Catcode::Letter,
        }
    );
}

#[test]
fn forbidden_setbox_scans_target_but_leaves_box_command_and_body_owned_by_input() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\setbox17 = \\hbox{owned}"));
    let mut execution = crate::ExecutionContext::new("texput");

    finish_display_alignment_assignments(&mut input, &mut stores, &mut execution)
        .expect("forbidden setbox rejection is recoverable");

    assert!(terminal_text(&stores).contains("Improper \\setbox"));
    let next = input
        .next_traced_token(&mut stores)
        .expect("remaining input reads")
        .expect("box command remains input");
    assert_eq!(
        stores.meaning(match tex_expand::semantic_token(next) {
            Token::Cs(symbol) => symbol,
            token => panic!("expected box command, got {token:?}"),
        }),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HBox)
    );
}

fn horizontal_nest(mode: Mode) -> ModeNest {
    let mut nest = ModeNest::new();
    nest.push(mode).expect("test mode push");
    nest
}

fn next_semantic(input: &mut InputStack, stores: &mut Universe) -> Option<Token> {
    input
        .next_traced_token(stores)
        .expect("input read succeeds")
        .map(tex_expand::semantic_token)
}

fn pending_terminal_text(stores: &Universe) -> String {
    stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            tex_state::EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn display_line_geometry_uses_the_second_paragraph_line_shape() {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_dimen_param(DimenParam::H_SIZE, sp(100));
    let mut nest = ModeNest::new();

    assert_eq!(
        crate::assignments::display_line_dimensions(&nest, &stores),
        tex_typeset::linebreak::LineDimensions {
            indent: Scaled::from_raw(0),
            width: sp(100),
        }
    );

    stores.set_dimen_param(DimenParam::HANG_INDENT, sp(20));
    stores.set_int_param(IntParam::HANG_AFTER, 1);
    assert_eq!(
        crate::assignments::display_line_dimensions(&nest, &stores),
        tex_typeset::linebreak::LineDimensions {
            indent: sp(20),
            width: sp(80),
        },
        "positive hangindent narrows and indents the second line"
    );

    stores.set_dimen_param(DimenParam::HANG_INDENT, sp(-20));
    stores.set_int_param(IntParam::HANG_AFTER, -2);
    assert_eq!(
        crate::assignments::display_line_dimensions(&nest, &stores),
        tex_typeset::linebreak::LineDimensions {
            indent: Scaled::from_raw(0),
            width: sp(80),
        },
        "negative hangafter applies the hanging shape through line two"
    );

    nest.set_enclosing_vertical_prev_graf(7);
    stores.set_paragraph_shape(
        &[
            tex_state::ParagraphShapeLine {
                indent: sp(3),
                width: sp(70),
            },
            tex_state::ParagraphShapeLine {
                indent: sp(9),
                width: sp(60),
            },
        ],
        false,
    );
    assert_eq!(
        crate::assignments::display_line_dimensions(&nest, &stores),
        tex_typeset::linebreak::LineDimensions {
            indent: sp(9),
            width: sp(60),
        },
        "the final parshape entry repeats independently of prevgraf"
    );
}

#[test]
fn pre_display_size_uses_natural_glue_width_until_the_set_ratio_matters() {
    let mut stores = Universe::new_with_plain_catcodes();
    let glue = stores.intern_glue(GlueSpec {
        width: sp(7),
        stretch: sp(11),
        stretch_order: Order::Fil,
        shrink: sp(5),
        shrink_order: Order::Fill,
    });
    let children = stores.freeze_node_list(&[
        Node::Kern {
            amount: sp(2),
            kind: tex_state::node::KernKind::Explicit,
        },
        Node::Glue {
            spec: glue,
            kind: GlueKind::Normal,
            leader: None,
        },
        Node::Rule {
            width: Some(sp(13)),
            height: None,
            depth: None,
        },
    ]);
    let line = |glue_sign, glue_order| {
        BoxNode::new(BoxNodeFields {
            width: sp(200),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: sp(3),
            display: false,
            glue_set: GlueSetRatio::from_ratio_parts(37, 10),
            glue_sign,
            glue_order,
            children,
        })
    };

    let natural = pre_display_size(&stores, &line(Sign::Normal, Order::Normal));
    assert_eq!(natural, sp(25));
    assert_eq!(
        pre_display_size(&stores, &line(Sign::Stretching, Order::Normal)),
        natural,
        "a nonparticipating glue order keeps its natural width regardless of glue_set"
    );
    assert_eq!(
        pre_display_size(&stores, &line(Sign::Shrinking, Order::Normal)),
        natural,
        "the shrink branch also ignores a nonparticipating glue order"
    );
    assert_eq!(
        pre_display_size(&stores, &line(Sign::Stretching, Order::Fil)),
        Scaled::MAX_DIMEN
    );
    assert_eq!(
        pre_display_size(&stores, &line(Sign::Shrinking, Order::Fill)),
        Scaled::MAX_DIMEN
    );
}

#[test]
fn tex82_math_entry_display_probe_and_eqno_mode_matrix() {
    let mut stores = Universe::new_with_plain_catcodes();
    let hook = stores.intern_token_list(&[Token::Char {
        ch: 'q',
        cat: Catcode::Letter,
    }]);
    stores.set_tok_param(TokParam::EVERY_MATH, hook);
    let mut nest = horizontal_nest(Mode::Horizontal);
    let mut input = InputStack::new(MemoryInput::new("x"));
    let mut execution = crate::ExecutionContext::new("texput");
    assert_eq!(
        enter_math(&mut nest, &mut input, &mut stores, &mut execution).expect("inline math enters"),
        DispatchAction::Continue
    );
    assert_eq!(nest.current_mode(), Mode::Math);
    assert_eq!(stores.innermost_group_kind(), Some(GroupKind::MathShift));
    assert!(matches!(
        next_semantic(&mut input, &mut stores),
        Some(Token::Char { ch: 'q', .. })
    ));
    assert!(matches!(
        next_semantic(&mut input, &mut stores),
        Some(Token::Char { ch: 'x', .. })
    ));

    let mut stores = Universe::new_with_plain_catcodes();
    let mut restricted = horizontal_nest(Mode::RestrictedHorizontal);
    let mut input = InputStack::new(MemoryInput::new("$"));
    let mut execution = crate::ExecutionContext::new("texput");
    enter_math(&mut restricted, &mut input, &mut stores, &mut execution)
        .expect("restricted hmode enters inline math");
    assert_eq!(restricted.current_mode(), Mode::Math);
    assert!(matches!(
        next_semantic(&mut input, &mut stores),
        Some(Token::Char {
            cat: Catcode::MathShift,
            ..
        })
    ));

    let mut stores = Universe::new_with_plain_catcodes();
    let mut display = horizontal_nest(Mode::Horizontal);
    let mut input = InputStack::new(MemoryInput::new("$"));
    let mut execution = crate::ExecutionContext::new("texput");
    enter_math(&mut display, &mut input, &mut stores, &mut execution)
        .expect("paired shifts enter display math");
    assert_eq!(display.current_mode(), Mode::DisplayMath);
    assert_eq!(stores.innermost_group_kind(), Some(GroupKind::MathShift));
    testing_start_eq_no(&mut display, &mut stores, UnexpandablePrimitive::LeftEqNo)
        .expect("leqno starts only in display math");
    assert_eq!(display.current_mode(), Mode::Math);
    assert!(display.current_list().display_eq_no().is_some());
    assert_eq!(stores.innermost_group_kind(), Some(GroupKind::MathShift));

    let mut stores = Universe::new_with_plain_catcodes();
    let mut inline = horizontal_nest(Mode::Math);
    assert!(testing_start_eq_no(&mut inline, &mut stores, UnexpandablePrimitive::EqNo).is_err());
}

#[test]
fn finish_display_math_packages_width_equation_number_and_migration_matrix() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut nest = horizontal_nest(Mode::Horizontal);
    let mut opening = InputStack::new(MemoryInput::new(""));
    let mut execution = crate::ExecutionContext::new("texput");
    enter_math(&mut nest, &mut opening, &mut stores, &mut execution).expect("inline math enters");
    finish_math(
        &mut nest,
        &mut opening,
        &mut stores,
        &mut execution,
        OriginId::UNKNOWN,
    )
    .expect("inline math exits");
    assert_eq!(nest.current_mode(), Mode::Horizontal);
    assert_eq!(stores.innermost_group_kind(), None);
    assert!(matches!(
        nest.current_list().nodes(),
        [Node::MathOn(_), Node::MathOff(_)]
    ));
    assert!(
        terminal_text(&stores).contains("Math formula deleted: Insufficient symbol fonts"),
        "TeX82 §1194 checks math font families even for an empty mlist"
    );
    assert_eq!(nest.current_list().space_factor(), 1000);

    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut display = horizontal_nest(Mode::Horizontal);
    let mut opening = InputStack::new(MemoryInput::new("$"));
    let mut execution = crate::ExecutionContext::new("texput");
    enter_math(&mut display, &mut opening, &mut stores, &mut execution).expect("display enters");
    let mut closing = InputStack::new(MemoryInput::new("$x"));
    finish_math(
        &mut display,
        &mut closing,
        &mut stores,
        &mut execution,
        OriginId::UNKNOWN,
    )
    .expect("paired display shifts exit");
    assert_eq!(display.current_mode(), Mode::Horizontal);
    assert!(
        stores
            .current_page_nodes()
            .iter()
            .chain(stores.page_contributions().iter())
            .any(|node| matches!(node, Node::HList(boxed) if boxed.display))
    );
    assert!(matches!(
        next_semantic(&mut closing, &mut stores),
        Some(Token::Char { ch: 'x', .. })
    ));

    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut display = horizontal_nest(Mode::Horizontal);
    let mut opening = InputStack::new(MemoryInput::new("$"));
    let mut execution = crate::ExecutionContext::new("texput");
    enter_math(&mut display, &mut opening, &mut stores, &mut execution).expect("display enters");
    testing_start_eq_no(&mut display, &mut stores, UnexpandablePrimitive::EqNo)
        .expect("eqno enters negative math mode");
    let mut closing = InputStack::new(MemoryInput::new("$z"));
    finish_math(
        &mut display,
        &mut closing,
        &mut stores,
        &mut execution,
        OriginId::UNKNOWN,
    )
    .expect("eqno and display finish together");
    assert_eq!(display.current_mode(), Mode::Horizontal);
    assert_eq!(stores.innermost_group_kind(), None);
    assert!(
        terminal_text(&stores).contains("Math formula deleted: Insufficient symbol fonts"),
        "TeX82 §1194 checks the empty equation-number mlist before fin_mlist"
    );
    assert!(matches!(
        next_semantic(&mut closing, &mut stores),
        Some(Token::Char { ch: 'z', .. })
    ));

    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut display = horizontal_nest(Mode::Horizontal);
    let mut opening = InputStack::new(MemoryInput::new("$"));
    let mut execution = crate::ExecutionContext::new("texput");
    enter_math(&mut display, &mut opening, &mut stores, &mut execution).expect("display enters");
    let mut malformed = InputStack::new(MemoryInput::new("r"));
    finish_math(
        &mut display,
        &mut malformed,
        &mut stores,
        &mut execution,
        OriginId::UNKNOWN,
    )
    .expect("missing second shift recovers");
    assert!(pending_terminal_text(&stores).contains("Display math should end with $$"));
    assert!(matches!(
        next_semantic(&mut malformed, &mut stores),
        Some(Token::Char { ch: 'r', .. })
    ));

    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut inline = horizontal_nest(Mode::Horizontal);
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut execution = crate::ExecutionContext::new("texput");
    enter_math(&mut inline, &mut input, &mut stores, &mut execution).expect("inline enters");
    inline
        .current_list_mutation()
        .push(Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::MathChar(MathChar {
                family: 0,
                character: 'a',
                origin: OriginId::UNKNOWN,
            }),
        )));
    finish_math(
        &mut inline,
        &mut input,
        &mut stores,
        &mut execution,
        OriginId::UNKNOWN,
    )
    .expect("missing math fonts diagnose before lowering");
    assert!(pending_terminal_text(&stores).contains("fontdimen"));
    assert!(matches!(
        inline.current_list().nodes(),
        [Node::MathOn(_), Node::MathOff(_)]
    ));
}
