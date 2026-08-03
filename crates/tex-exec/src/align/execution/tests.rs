use super::*;
use crate::align::support::{align_state, alignment_mode, cell_mode, row_mode};
use crate::{AlignColumn, AlignmentPackSpec, install_unexpandable_primitives};
use tex_lex::{InputStack, MemoryInput};
use tex_state::env::banks::TokParam;
use tex_state::ids::{GlueId, TokenListId};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};
use tex_state::{EffectRecord, GroupKind, PrintSink};

fn state(kind: AlignmentKind) -> AlignState {
    AlignState::new(
        kind,
        AlignmentPackSpec::Natural,
        vec![AlignColumn {
            u_template: TokenListId::EMPTY,
            v_template: TokenListId::EMPTY,
        }],
        vec![GlueId::ZERO, GlueId::ZERO],
        GlueId::ZERO,
        None,
    )
}

fn alignment_nest(kind: AlignmentKind) -> (ModeNest, usize) {
    let mut nest = ModeNest::new();
    nest.push(alignment_mode(kind)).expect("test mode push");
    let level = nest.depth() - 1;
    nest.current_list_mutation().set_align_state(state(kind));
    (nest, level)
}

#[test]
fn init_row_switches_mode_and_copies_leading_tabskip() {
    for kind in [AlignmentKind::HAlign, AlignmentKind::VAlign] {
        let (mut nest, align_level) = alignment_nest(kind);

        init_row(align_level, &mut nest).expect("alignment test precondition");

        assert_eq!(nest.current_mode(), row_mode(kind));
        assert_eq!(cell_mode(kind), row_mode(kind));
        assert_eq!(
            nest.current_list().nodes(),
            &[Node::Glue {
                spec: GlueId::ZERO,
                kind: GlueKind::TabSkip,
                leader: None,
            }]
        );
        if kind == AlignmentKind::HAlign {
            assert_eq!(
                nest.current_list().space_factor(),
                1000,
                "the internal zero aux projects as TeX's effective default space factor"
            );
        }
        let alignment = align_state(&nest, align_level).expect("alignment test precondition");
        assert_eq!(alignment.current_row(), 0);
        assert_eq!(alignment.current_col(), 0);
        assert_eq!(alignment.current_span(), 1);
    }
}

#[test]
fn init_span_sets_mode_aux_and_cur_span() {
    for kind in [AlignmentKind::HAlign, AlignmentKind::VAlign] {
        assert_eq!(cell_mode(kind), row_mode(kind));
        let mut state = state(kind);

        state.start_row();
        state.start_cell(2, 3);
        assert_eq!(state.current_row(), 0);
        assert_eq!(state.current_col(), 2);
        assert_eq!(state.current_span(), 3);

        state.finish_cell(5);
        assert_eq!(state.current_col(), 5);
        assert_eq!(state.current_span(), 1);

        state.start_cell(5, 2);
        state.finish_row();
        assert_eq!(state.current_row(), 1);
        assert_eq!(state.current_col(), 0);
        assert_eq!(state.current_span(), 1);
    }
}

#[test]
fn align_peek_selects_noalign_finish_crcr_or_row() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    install_unexpandable_primitives(&mut stores);
    stores.enter_group_with_kind(GroupKind::Align);
    stores.enter_group_with_kind(GroupKind::Align);
    let mut input = InputStack::new(MemoryInput::new("\\noalign{\\vskip2pt}\\crcr x"));
    input.begin_alignment();
    let (mut nest, align_level) = alignment_nest(AlignmentKind::HAlign);
    let mut execution = crate::ExecutionContext::new("texput");

    let first = align_peek(
        align_level,
        &mut nest,
        &mut input,
        &mut stores,
        &mut execution,
    )
    .expect("alignment test precondition")
    .expect("ordinary row token follows noalign and crcr");

    assert_eq!(
        first.semantic_token(),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        }
    );
    assert!(
        input.alignment_state_is(1_000_000),
        "TeX82 §785 resets align_state before expandable lookahead"
    );
    assert!(
        nest.list(align_level)
            .expect("alignment test precondition")
            .nodes()
            .iter()
            .any(|node| {
                matches!(
                    node,
                    Node::Glue {
                        spec,
                        kind: GlueKind::Normal,
                        ..
                    } if stores.glue(*spec).width == Scaled::from_raw(2 * Scaled::UNITY)
                )
            })
    );

    let mut closing_stores = crate::test_harness::universe_with_plain_catcodes();
    install_unexpandable_primitives(&mut closing_stores);
    closing_stores.enter_group_with_kind(GroupKind::Align);
    closing_stores.enter_group_with_kind(GroupKind::Align);
    let mut closing_input = InputStack::new(MemoryInput::new("}"));
    closing_input.begin_alignment();
    let (mut closing_nest, closing_level) = alignment_nest(AlignmentKind::HAlign);
    let mut closing_execution = crate::ExecutionContext::new("texput");

    assert!(
        align_peek(
            closing_level,
            &mut closing_nest,
            &mut closing_input,
            &mut closing_stores,
            &mut closing_execution,
        )
        .expect("alignment test precondition")
        .is_none()
    );
    assert!(
        closing_input.alignment_state_is(999_999),
        "the delivered closing brace remains accounted when fin_align is selected"
    );
}

fn row_state(
    stores: &mut Universe,
    kind: AlignmentKind,
    column_count: usize,
    loop_start: Option<usize>,
) -> AlignState {
    let v_template = stores.intern_token_list(&[stores.frozen_end_template_token()]);
    AlignState::new(
        kind,
        AlignmentPackSpec::Natural,
        vec![
            AlignColumn {
                u_template: TokenListId::EMPTY,
                v_template,
            };
            column_count
        ],
        vec![GlueId::ZERO; column_count + 1],
        GlueId::ZERO,
        loop_start,
    )
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

fn execute_test_row(
    kind: AlignmentKind,
    column_count: usize,
    loop_start: Option<usize>,
    source: &str,
    finish: bool,
) -> (Universe, ModeNest, usize, bool) {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    install_unexpandable_primitives(&mut stores);
    stores.enter_group_with_kind(GroupKind::Align);
    let mut input = InputStack::new(MemoryInput::new(source));
    input.begin_alignment();
    input.set_alignment_scanner_phase(tex_lex::AlignmentScannerPhase::BetweenEntries);
    let mut nest = ModeNest::new();
    nest.push(alignment_mode(kind)).expect("test mode push");
    let align_level = nest.depth() - 1;
    let state = row_state(&mut stores, kind, column_count, loop_start);
    nest.current_list_mutation().set_align_state(state);
    init_row(align_level, &mut nest).expect("row initialization succeeds");
    let mut execution = crate::ExecutionContext::new("texput");
    let first = next_non_space_protected(&mut input, &mut stores, &mut execution)
        .expect("row lookahead succeeds")
        .expect("test row has a terminator");
    let mut migrations = Vec::new();
    let extra = execute_row(
        align_level,
        first,
        &mut migrations,
        &mut nest,
        &mut input,
        &mut stores,
        &mut execution,
    )
    .expect("row executes");
    if finish {
        fin_row(
            align_level,
            migrations,
            &mut nest,
            &mut stores,
            &mut execution,
        )
        .expect("row packages");
    }
    (stores, nest, align_level, extra)
}

#[test]
fn fin_col_delimiter_matrix_and_periodic_extension() {
    for (source, columns, loop_start, expected_spans) in [
        ("&\\cr", 2, None, vec![0, 0]),
        ("\\span\\cr", 2, None, vec![1]),
        ("\\crcr", 1, None, vec![0]),
        ("&&\\cr", 2, Some(1), vec![0, 0, 0]),
    ] {
        let (_stores, nest, _, extra) =
            execute_test_row(AlignmentKind::HAlign, columns, loop_start, source, false);
        assert!(!extra, "source {source:?} must not use extra-tab recovery");
        let spans = nest
            .current_list()
            .nodes()
            .iter()
            .filter_map(|node| match node {
                Node::Unset(cell) => Some(cell.span_count),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(spans, expected_spans, "source {source:?}");
    }
}

#[test]
fn fin_col_extra_tab_recovers_to_cr() {
    for source in ["&", "\\span"] {
        let (stores, nest, align_level, extra) =
            execute_test_row(AlignmentKind::HAlign, 1, None, source, false);
        assert!(extra, "source {source:?} must end the row recoverably");
        assert!(terminal_text(&stores).contains("Extra alignment tab has been changed to \\cr"));
        let state = align_state(&nest, align_level).expect("alignment state remains live");
        assert!(!state.suppress_redundant_cr());
        assert_eq!(state.current_col(), 1);
        assert_eq!(state.current_span(), 1);
    }
}

#[test]
fn fin_row_packages_modes_adjustments_every_cr_and_peek() {
    for kind in [AlignmentKind::HAlign, AlignmentKind::VAlign] {
        let (stores, nest, align_level, extra) = execute_test_row(kind, 1, None, "\\cr", true);
        assert!(!extra);
        assert_eq!(nest.depth(), align_level + 1);
        let [Node::Unset(row)] = nest.current_list().nodes() else {
            panic!("fin_row must append exactly one unset row");
        };
        assert_eq!(row.kind, crate::align::packaging::row_unset_kind(kind));
        assert_eq!(
            align_state(&nest, align_level)
                .expect("alignment state remains live")
                .current_row(),
            1
        );
        assert!(!stores.nodes(row.children).is_empty());
    }

    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let every_cr = stores.intern_token_list(&[Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    }]);
    stores.set_tok_param(TokParam::EVERY_CR, every_cr);
    let mut input = InputStack::new(MemoryInput::new(""));
    replay_everycr(&mut input, &stores);
    assert_eq!(
        input
            .next_token(&mut stores)
            .expect("every_cr replay reads"),
        Some(Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        })
    );
}

#[test]
fn cell_adjustments_leave_cells_in_exact_row_order() {
    // TeX82 §§796 and 799 migrate each halign cell's adjustments out of the
    // packaged cell, then splice them immediately after the completed row in
    // cell order.
    let (stores, nest, align_level, extra) = execute_test_row(
        AlignmentKind::HAlign,
        2,
        None,
        "\\kern1pt\\vadjust{\\kern2pt}&\\kern3pt\\vadjust{\\kern4pt}\\cr",
        true,
    );
    assert!(!extra);
    let nodes = nest
        .list(align_level)
        .expect("alignment list remains available")
        .nodes();
    let [
        Node::Unset(row),
        Node::Kern { amount: first, .. },
        Node::Kern { amount: second, .. },
    ] = nodes
    else {
        panic!("§799 must leave one row followed by its two cell adjustments: {nodes:?}");
    };
    assert_eq!(
        (*first, *second),
        (
            Scaled::from_raw(2 * Scaled::UNITY),
            Scaled::from_raw(4 * Scaled::UNITY)
        )
    );
    let row_children = stores.nodes(row.children).testing_decoded();
    assert!(
        row_children
            .iter()
            .all(|node| !matches!(node, Node::Adjust(_))),
        "migrated adjustments must not remain inside either packaged cell"
    );
}

#[test]
fn insert_finished_alignment_list_dispatches_by_enclosing_mode() {
    for enclosing in [Mode::InternalVertical, Mode::RestrictedHorizontal] {
        let mut stores = crate::test_harness::universe_with_plain_catcodes();
        let mut nest = ModeNest::new();
        nest.push(enclosing).expect("test mode push");
        nest.current_list_mutation()
            .set_prev_depth(Scaled::from_raw(7));
        append_finished_alignment(
            &mut nest,
            &mut stores,
            FinishedAlignment {
                nodes: vec![Node::Rule {
                    width: Some(Scaled::from_raw(3)),
                    height: Some(Scaled::from_raw(2)),
                    depth: Some(Scaled::from_raw(1)),
                }],
                aux_prev_depth: Some(Scaled::from_raw(11)),
                aux_space_factor: Some(1234),
            },
        );

        assert!(matches!(nest.current_list().nodes(), [Node::Rule { .. }]));
        if enclosing == Mode::InternalVertical {
            assert_eq!(nest.current_list().prev_depth(), Some(Scaled::from_raw(11)));
        } else {
            assert_eq!(nest.current_list().prev_depth(), Some(Scaled::from_raw(7)));
            assert_eq!(nest.current_list().space_factor(), 1234);
        }
    }

    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut outer = ModeNest::new();
    append_finished_alignment(
        &mut outer,
        &mut stores,
        FinishedAlignment {
            nodes: vec![Node::Rule {
                width: Some(Scaled::from_raw(3)),
                height: Some(Scaled::from_raw(2)),
                depth: Some(Scaled::from_raw(1)),
            }],
            aux_prev_depth: None,
            aux_space_factor: None,
        },
    );
    build_page_if_outer_vertical(&outer, &mut stores)
        .expect("outer vertical insertion invokes the page builder");
    assert!(
        stores
            .current_page_nodes()
            .iter()
            .any(|node| matches!(node, Node::Rule { .. }))
    );
}

fn exhausted_v_template_input(
    stores: &mut Universe,
    group: GroupKind,
) -> (InputStack, TracedTokenWord) {
    let template = stores.intern_token_list(&[Token::Char {
        ch: 'v',
        cat: Catcode::Letter,
    }]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.begin_alignment();
    input.begin_alignment_cell(None, template);
    let delimiter = TracedTokenWord::pack(
        Token::Char {
            ch: '&',
            cat: Catcode::AlignmentTab,
        },
        tex_state::token::OriginId::UNKNOWN,
    );
    assert!(input.intercept_alignment_token(
        delimiter,
        tex_lex::AlignmentTokenDelivery::Other,
        Some(tex_lex::AlignmentTerminator::Tab),
    ));
    assert_eq!(
        input.next_token(stores).expect("v-template token reads"),
        Some(Token::Char {
            ch: 'v',
            cat: Catcode::Letter,
        })
    );
    assert!(input.has_exhausted_alignment_v_template(stores));
    stores.enter_group_with_kind(group);
    (
        input,
        TracedTokenWord::pack(
            stores.frozen_endv_token(),
            tex_state::token::OriginId::UNKNOWN,
        ),
    )
}

#[test]
fn do_endv_validates_template_and_group_matrix() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let ordinary = TracedTokenWord::pack(
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        },
        tex_state::token::OriginId::UNKNOWN,
    );
    let mut empty_input = InputStack::new(MemoryInput::new(""));
    assert_eq!(
        do_endv(ordinary, &mut empty_input, &mut stores)
            .expect("ordinary command is outside do_endv"),
        DoEndV::NotApplicable
    );
    let unbacked_endv = TracedTokenWord::pack(
        stores.frozen_endv_token(),
        tex_state::token::OriginId::UNKNOWN,
    );
    assert_eq!(
        do_endv(unbacked_endv, &mut empty_input, &mut stores)
            .expect("endv without v-template ancestry is rejected by the gate"),
        DoEndV::NotApplicable
    );

    let (mut aligned_input, aligned_endv) =
        exhausted_v_template_input(&mut stores, GroupKind::Align);
    assert_eq!(
        do_endv(aligned_endv, &mut aligned_input, &mut stores)
            .expect("exhausted v-template in align group finishes the cell"),
        DoEndV::FinishCell
    );

    let mut wrong_stores = crate::test_harness::universe_with_plain_catcodes();
    let (mut wrong_input, wrong_endv) =
        exhausted_v_template_input(&mut wrong_stores, GroupKind::SemiSimple);
    assert_eq!(
        do_endv(wrong_endv, &mut wrong_input, &mut wrong_stores)
            .expect("wrong group follows off_save recovery"),
        DoEndV::Recovered
    );
}
