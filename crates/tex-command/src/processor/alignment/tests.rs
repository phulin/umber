use super::*;
use crate::{CommandState, DeliveryStamp};
use tex_state::Universe;
use tex_state::input::TracedTokenList;
use tex_state::meaning::{ExpandablePrimitive, Meaning, UnexpandablePrimitive};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

fn resolve(stores: &mut Universe, token: Token) -> CurrentCommand {
    let mut context = stores.command_context();
    CurrentCommand::resolve(
        TracedTokenWord::pack(token, OriginId::UNKNOWN),
        DeliveryStamp::new(17, 23, 29),
        None,
        false,
        &mut context,
    )
}

fn templates(stores: &mut Universe) -> AlignmentCellTemplates {
    let u_template = stores.intern_token_list_ref(&[Token::Char {
        ch: 'u',
        cat: Catcode::Letter,
    }]);
    let v_template = stores.intern_token_list_ref(&[Token::Char {
        ch: 'v',
        cat: Catcode::Letter,
    }]);
    AlignmentCellTemplates {
        u_template: Some(TracedTokenList::synthetic(u_template)),
        v_template: TracedTokenList::synthetic(v_template),
    }
}

#[test]
fn alignment_stack_globals_initialize_to_null() {
    let state = AlignmentDeliveryState::default();

    assert_eq!(state.align_state, TOP_LEVEL_ALIGN_STATE);
    assert!(state.align_stack.is_empty());
    assert!(state.active_alignment.is_none());
    assert!(state.suspended.is_empty());
    assert!(state.active_cell.is_none());
    assert!(state.completed_preamble.is_none());
    assert!(state.pending_fin_col_delimiter.is_none());
    assert!(state.extra_tab_recovery.is_none());
}

#[test]
fn push_pop_alignment_restores_all_tex82_fields() {
    let mut stores = Universe::new();
    let outer_templates = templates(&mut stores);
    let outer = AlignmentIdentity::new(41);
    let inner = AlignmentIdentity::new(43);
    let mut state = AlignmentDeliveryState {
        align_state: 17,
        ..AlignmentDeliveryState::default()
    };

    state.begin_alignment(outer);
    assert_eq!(state.align_stack, [17]);
    assert_eq!(state.align_state, PREAMBLE_ALIGN_STATE);
    state
        .begin_cell(outer, outer_templates.clone())
        .expect("alignment test precondition");
    state
        .mark_u_template_installed(outer)
        .expect("alignment test precondition");
    state.align_state = 3;
    state
        .suspend_alignment(outer)
        .expect("alignment test precondition");

    assert_eq!(state.active_alignment, None);
    assert_eq!(state.suspended.len(), 1);
    assert_eq!(state.align_stack, [17]);

    state.begin_alignment(inner);
    assert_eq!(state.align_stack, [17, 3]);
    assert_eq!(state.align_state, PREAMBLE_ALIGN_STATE);
    state
        .finish_alignment(inner)
        .expect("alignment test precondition");
    assert_eq!(state.align_state, 3);
    assert_eq!(state.align_stack, [17]);

    state
        .resume_alignment(outer)
        .expect("alignment test precondition");
    let restored = state.active_cell.as_ref().expect("outer cell restores");
    assert_eq!(restored.alignment, outer);
    assert_eq!(restored.templates, outer_templates);
    assert!(restored.u_template_installed);
    assert_eq!(state.suspended, []);

    state
        .finish_alignment(outer)
        .expect("alignment test precondition");
    assert_eq!(state.align_state, 17);
    assert!(state.align_stack.is_empty());
    assert!(state.active_alignment.is_none());
    assert!(state.active_cell.is_none());
    assert!(state.completed_preamble.is_none());
    assert!(state.pending_fin_col_delimiter.is_none());
}

#[test]
fn cell_template_delivery_matrix() {
    let mut stores = Universe::new();
    let templates = templates(&mut stores);
    let alignment = AlignmentIdentity::new(47);
    let mut state = AlignmentDeliveryState::default();

    state.begin_alignment(alignment);
    state
        .begin_cell(alignment, templates.clone())
        .expect("alignment test precondition");
    assert_eq!(state.align_state, TEMPLATE_ALIGN_STATE);
    assert_eq!(
        state.active_cell_template(alignment),
        Ok(templates.u_template)
    );

    let u_level = InputLevelId(7);
    state
        .attach_u_template(alignment, u_level)
        .expect("alignment test precondition");
    assert!(state.finish_u_template(u_level));
    assert_eq!(state.align_state, CELL_ALIGN_STATE);

    let mut begin_group = resolve(
        &mut stores,
        Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        },
    );
    assert_eq!(
        state.classify_delivery(&mut begin_group),
        AlignmentDeliveryAdjustment::BeginGroup
    );
    assert_eq!(state.align_state, 1);

    let alias = stores.intern("brace-alias").symbol();
    stores.set_meaning(
        alias,
        Meaning::CharToken {
            ch: '{',
            cat: Catcode::BeginGroup,
        },
    );
    let mut alias = resolve(&mut stores, Token::Cs(alias));
    assert_eq!(
        state.classify_delivery(&mut alias),
        AlignmentDeliveryAdjustment::None,
        "a control sequence with brace meaning is not a physical brace"
    );
    assert_eq!(state.align_state, 1);

    let mut end_group = resolve(
        &mut stores,
        Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        },
    );
    assert_eq!(
        state.classify_delivery(&mut end_group),
        AlignmentDeliveryAdjustment::EndGroup
    );
    assert_eq!(state.align_state, CELL_ALIGN_STATE);

    let mut aliased_tab_state = state.clone();
    let tab_alias = stores.intern("tab-alias").symbol();
    stores.set_meaning(
        tab_alias,
        Meaning::CharToken {
            ch: '&',
            cat: Catcode::AlignmentTab,
        },
    );
    let mut tab_alias = resolve(&mut stores, Token::Cs(tab_alias));
    assert_eq!(
        aliased_tab_state.classify_delivery(&mut tab_alias),
        AlignmentDeliveryAdjustment::Delimiter(AlignmentDelimiter::Tab),
        "TeX82 §§24 and 342 classify a delimiter by resolved cur_cmd"
    );
    assert_eq!(
        tab_alias.meaning(),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate)
    );

    let mut tab = resolve(
        &mut stores,
        Token::Char {
            ch: '&',
            cat: Catcode::AlignmentTab,
        },
    );
    assert_eq!(
        state.classify_delivery(&mut tab),
        AlignmentDeliveryAdjustment::Delimiter(AlignmentDelimiter::Tab)
    );
    assert_eq!(
        tab.meaning(),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate)
    );
    assert_eq!(state.align_state, TEMPLATE_ALIGN_STATE);

    let v_level = InputLevelId(11);
    state
        .begin_v_template(alignment, v_level, AlignmentCellDelimiter::Tab)
        .expect("alignment test precondition");
    assert_eq!(
        state.v_template(
            alignment,
            stores.token_list_ref(tex_state::ids::TokenListId::EMPTY),
        ),
        Ok(templates.v_template),
    );
    let finished = state
        .finish_cell(alignment, v_level)
        .expect("alignment test precondition");
    assert_eq!(finished.templates, templates);
    assert_eq!(finished.delimiter, AlignmentCellDelimiter::Tab);
    assert_eq!(
        state.pending_fin_col_delimiter,
        Some((alignment, AlignmentCellDelimiter::Tab))
    );

    for (raw, name, primitive, adjustment, delimiter) in [
        (
            13,
            "span",
            UnexpandablePrimitive::Span,
            AlignmentDelimiter::Span,
            AlignmentCellDelimiter::Span,
        ),
        (
            17,
            "cr",
            UnexpandablePrimitive::Cr,
            AlignmentDelimiter::Cr,
            AlignmentCellDelimiter::Row,
        ),
        (
            19,
            "crcr",
            UnexpandablePrimitive::CrCr,
            AlignmentDelimiter::CrCr,
            AlignmentCellDelimiter::Row,
        ),
    ] {
        state
            .begin_cell(alignment, templates.clone())
            .expect("alignment test precondition");
        let u_level = InputLevelId(raw);
        state
            .attach_u_template(alignment, u_level)
            .expect("alignment test precondition");
        assert!(state.finish_u_template(u_level));
        let symbol = stores.intern(name).symbol();
        stores.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
        let mut command = resolve(&mut stores, Token::Cs(symbol));
        assert_eq!(
            state.classify_delivery(&mut command),
            AlignmentDeliveryAdjustment::Delimiter(adjustment)
        );
        let v_level = InputLevelId(raw + 1);
        state
            .begin_v_template(alignment, v_level, delimiter)
            .expect("alignment test precondition");
        assert_eq!(
            state
                .finish_cell(alignment, v_level)
                .expect("alignment test precondition")
                .delimiter,
            delimiter
        );
    }

    let omitted = AlignmentIdentity::new(53);
    let mut command = CommandState::default();
    command.begin_alignment(omitted);
    command
        .begin_alignment_cell(omitted, templates.clone())
        .expect("alignment test precondition");
    command
        .prepare_alignment_cell_lookahead()
        .expect("alignment test precondition");
    command
        .install_alignment_omit_cell_template(omitted)
        .expect("alignment test precondition");

    assert_eq!(command.alignment.align_state, CELL_ALIGN_STATE);
    assert_eq!(
        command.alignment.v_template(
            omitted,
            stores.token_list_ref(tex_state::ids::TokenListId::EMPTY),
        ),
        Ok(TracedTokenList::synthetic(
            stores.token_list_ref(tex_state::ids::TokenListId::EMPTY)
        ))
    );
    assert_eq!(
        command
            .alignment_omit_cell_observation(omitted)
            .expect("omit state transition")
            .previous_align_state,
        Some(TEMPLATE_ALIGN_STATE)
    );
    let omitted_cell = command
        .alignment
        .active_cell
        .as_ref()
        .expect("alignment test precondition");
    assert!(omitted_cell.omit);
    assert!(omitted_cell.u_template_installed);
    assert!(omitted_cell.u_level.is_none());
}

#[test]
fn alignment_misplaced_command_recovery_matrix() {
    for (initial, expected) in [
        (
            -1,
            Some(Token::Char {
                ch: '{',
                cat: Catcode::BeginGroup,
            }),
        ),
        (
            1,
            Some(Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            }),
        ),
        (-3, None),
        (3, None),
        (TOP_LEVEL_ALIGN_STATE, None),
    ] {
        let mut state = AlignmentDeliveryState {
            align_state: initial,
            ..AlignmentDeliveryState::default()
        };
        let recovery = state.correct_unbalanced_delimiter();
        assert_eq!(recovery, expected, "initial align_state {initial}");
        if let Some(brace) = recovery {
            assert_eq!(state.align_state, 0);
            state.correct_inserted_brace_backup(brace);
            assert_eq!(state.align_state, initial);
            let expected_adjustment = if initial < 0 {
                AlignmentDeliveryAdjustment::BeginGroup
            } else {
                AlignmentDeliveryAdjustment::EndGroup
            };
            assert_eq!(
                AlignmentDeliveryState::back_input_adjustment(brace),
                expected_adjustment
            );
            let mut stores = Universe::new();
            let mut replayed = resolve(&mut stores, brace);
            assert_eq!(state.classify_delivery(&mut replayed), expected_adjustment);
            assert_eq!(state.align_state, 0);
        } else {
            assert_eq!(state.align_state, initial);
        }
    }
}

#[test]
fn alignment_close_inserts_frozen_cr_before_brace_replay() {
    let mut stores = Universe::new_with_plain_catcodes();
    let alignment = AlignmentIdentity::new(61);
    let templates = AlignmentCellTemplates {
        u_template: None,
        v_template: TracedTokenList::synthetic(
            stores.token_list_ref(tex_state::ids::TokenListId::EMPTY),
        ),
    };
    let mut state = AlignmentDeliveryState::default();
    state.begin_alignment(alignment);
    state
        .begin_cell(alignment, templates)
        .expect("empty-template cell begins at brace depth zero");
    state
        .mark_u_template_installed(alignment)
        .expect("cell template installs");
    let mut closing = resolve(
        &mut stores,
        Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        },
    );

    let adjustment = state.classify_delivery(&mut closing);
    assert_eq!(adjustment, AlignmentDeliveryAdjustment::EndGroup);
    assert_eq!(state.align_state, -1);
    assert!(state.needs_closing_brace_recovery(&closing));

    state.undo_delivery(adjustment);
    assert_eq!(
        state.align_state, 0,
        "backing up the brace restores cell depth"
    );
    let cr = stores.intern("cr").symbol();
    stores.set_meaning(
        cr,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Cr),
    );
    let mut frozen_cr = resolve(&mut stores, Token::Cs(cr));
    assert_eq!(
        state.classify_delivery(&mut frozen_cr),
        AlignmentDeliveryAdjustment::Delimiter(AlignmentDelimiter::Cr),
        "the inserted row terminator reaches template delivery before brace replay"
    );
    state.undo_delivery(AlignmentDeliveryAdjustment::Delimiter(
        AlignmentDelimiter::Cr,
    ));
    assert_eq!(state.align_state, 0);
    assert_eq!(
        state.classify_delivery(&mut closing),
        AlignmentDeliveryAdjustment::EndGroup,
        "the original brace remains replayable after the inserted frozen cr"
    );
}
