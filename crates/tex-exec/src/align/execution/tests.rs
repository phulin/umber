use super::*;
use crate::align::support::{align_state, alignment_mode, cell_mode, row_mode};
use crate::{AlignColumn, AlignmentPackSpec, install_unexpandable_primitives};
use tex_lex::{InputStack, MemoryInput};
use tex_state::GroupKind;
use tex_state::ids::{GlueId, TokenListId};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};

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
    nest.push(alignment_mode(kind));
    let level = nest.depth() - 1;
    nest.current_list_mut().set_align_state(state(kind));
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
    let mut stores = Universe::new_with_plain_catcodes();
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
        tex_expand::semantic_token(first),
        Token::Char {
            ch: 'x',
            cat: Catcode::Letter,
        }
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

    let mut closing_stores = Universe::new_with_plain_catcodes();
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
}
