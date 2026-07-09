use super::*;
use tex_state::env::banks::GlueParam;
use tex_state::ids::GlueId;
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::node::{Node, UnsetKind};

fn scan_halign_preamble(source: &str) -> (Universe, AlignState) {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(source));
    let mut hooks = crate::executor::NoopExecHooks;
    let state = crate::align::scan_preamble(
        UnexpandablePrimitive::HAlign,
        &mut input,
        &mut stores,
        &mut hooks,
    )
    .expect("alignment preamble should scan");
    (stores, state)
}

fn scan_valign_preamble(source: &str) -> (Universe, AlignState) {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(source));
    let mut hooks = crate::executor::NoopExecHooks;
    let state = crate::align::scan_preamble(
        UnexpandablePrimitive::VAlign,
        &mut input,
        &mut stores,
        &mut hooks,
    )
    .expect("alignment preamble should scan");
    (stores, state)
}

fn char_token(ch: char, cat: Catcode) -> Token {
    Token::Char { ch, cat }
}

fn run_alignment_source(source: &str) -> Universe {
    let mut stores = support::stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new(format!("\\font\\f=cmr10 \\f {source}")));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("alignment source executes");
    stores
}

fn page_alignment(stores: &Universe) -> &tex_state::node::UnsetNode {
    let [Node::Unset(alignment)] = stores.page_contributions() else {
        panic!(
            "expected one unset alignment, got {:?}",
            stores.page_contributions()
        );
    };
    assert_eq!(alignment.kind, UnsetKind::VBox);
    alignment
}

fn unset_children<'a>(stores: &'a Universe, unset: &tex_state::node::UnsetNode) -> &'a [Node] {
    stores.nodes(unset.children)
}

fn row_cells<'a>(
    stores: &'a Universe,
    row: &'a tex_state::node::UnsetNode,
) -> Vec<&'a tex_state::node::UnsetNode> {
    stores
        .nodes(row.children)
        .iter()
        .filter_map(|node| match node {
            Node::Unset(cell) => Some(cell),
            _ => None,
        })
        .collect()
}

fn cell_text(stores: &Universe, cell: &tex_state::node::UnsetNode) -> String {
    stores
        .nodes(cell.children)
        .iter()
        .filter_map(|node| match node {
            Node::Char { ch, .. } => Some(*ch),
            Node::Lig { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect()
}

#[test]
fn scans_empty_u_template_and_end_template_sentinel() {
    let (stores, state) = scan_halign_preamble("{#v\\cr}");

    assert_eq!(state.kind(), AlignmentKind::HAlign);
    assert_eq!(state.pack_spec(), AlignmentPackSpec::Natural);
    assert_eq!(state.columns().len(), 1);
    assert!(stores.tokens(state.columns()[0].u_template).is_empty());
    assert_eq!(
        stores.tokens(state.columns()[0].v_template),
        &[char_token('v', Catcode::Letter), state.end_template()]
    );
    assert_eq!(state.tabskips(), &[GlueId::ZERO, GlueId::ZERO]);
    assert_eq!(state.default_tabskip(), GlueId::ZERO);
}

#[test]
fn captures_mid_preamble_tabskip_boundaries() {
    let (stores, state) = scan_halign_preamble("{#a&\\tabskip=3pt#b&\\tabskip=5pt#c\\cr}");

    assert_eq!(state.columns().len(), 3);
    assert_eq!(state.tabskips().len(), 4);
    assert_eq!(stores.glue(state.tabskips()[0]), GlueSpec::ZERO);
    assert_eq!(
        stores.glue(state.tabskips()[1]).width.raw(),
        3 * tex_state::scaled::Scaled::UNITY
    );
    assert_eq!(
        stores.glue(state.tabskips()[2]).width.raw(),
        5 * tex_state::scaled::Scaled::UNITY
    );
    assert_eq!(
        stores.glue(state.tabskips()[3]).width.raw(),
        5 * tex_state::scaled::Scaled::UNITY
    );
    assert_eq!(state.default_tabskip(), state.tabskips()[3]);
    assert_eq!(
        stores
            .glue(stores.glue_param(GlueParam::TAB_SKIP))
            .width
            .raw(),
        5 * tex_state::scaled::Scaled::UNITY
    );
}

#[test]
fn records_repeat_point_and_resolves_extra_columns() {
    let (stores, state) = scan_halign_preamble("{#a&#b&&#c&#d\\cr}");

    assert_eq!(state.columns().len(), 4);
    assert_eq!(state.loop_start(), Some(2));
    assert_eq!(state.column_for(0), Some(&state.columns()[0]));
    assert_eq!(state.column_for(3), Some(&state.columns()[3]));
    assert_eq!(state.column_for(4), Some(&state.columns()[2]));
    assert_eq!(state.column_for(5), Some(&state.columns()[3]));
    assert_eq!(
        stores.tokens(state.column_for(4).expect("repeat col").v_template),
        &[char_token('c', Catcode::Letter), state.end_template()]
    );
}

#[test]
fn alignment_pack_spec_matches_box_keywords() {
    let (_stores, state) = scan_halign_preamble("{#\\cr}");
    assert_eq!(state.pack_spec(), AlignmentPackSpec::Natural);

    let (_stores, state) = scan_halign_preamble("to 12pt{#\\cr}");
    assert_eq!(
        state.pack_spec(),
        AlignmentPackSpec::Exactly(tex_state::scaled::Scaled::from_raw(
            12 * tex_state::scaled::Scaled::UNITY
        ))
    );

    let (_stores, state) = scan_halign_preamble("spread 2pt{#\\cr}");
    assert_eq!(
        state.pack_spec(),
        AlignmentPackSpec::Spread(tex_state::scaled::Scaled::from_raw(
            2 * tex_state::scaled::Scaled::UNITY
        ))
    );
}

#[test]
fn span_expands_next_preamble_token_without_becoming_template_material() {
    let (stores, state) = scan_halign_preamble("{\\span x#y\\cr}");

    assert_eq!(
        stores.tokens(state.columns()[0].u_template),
        &[char_token('x', Catcode::Letter)]
    );
    assert_eq!(
        stores.tokens(state.columns()[0].v_template),
        &[char_token('y', Catcode::Letter), state.end_template()]
    );
}

#[test]
fn valign_and_crcr_use_alignment_preamble_scanner() {
    let (stores, state) = scan_valign_preamble("{u#\\crcr}");

    assert_eq!(state.kind(), AlignmentKind::VAlign);
    assert_eq!(
        stores.tokens(state.columns()[0].u_template),
        &[char_token('u', Catcode::Letter)]
    );
    assert_eq!(
        stores.tokens(state.columns()[0].v_template),
        &[state.end_template()]
    );
}

#[test]
fn alignment_preamble_errors_match_pdftex_wording() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("{abc\\cr}"));
    let mut hooks = crate::executor::NoopExecHooks;
    let err = crate::align::scan_preamble(
        UnexpandablePrimitive::HAlign,
        &mut input,
        &mut stores,
        &mut hooks,
    )
    .expect_err("missing hash should be rejected");
    assert_eq!(err.to_string(), "Missing # inserted in alignment preamble.");

    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("{#a#b\\cr}"));
    let mut hooks = crate::executor::NoopExecHooks;
    let err = crate::align::scan_preamble(
        UnexpandablePrimitive::HAlign,
        &mut input,
        &mut stores,
        &mut hooks,
    )
    .expect_err("extra hash should be rejected");
    assert_eq!(err.to_string(), "Only one # is allowed per tab.");
}

#[test]
fn executes_rows_and_replays_u_and_v_templates_into_unset_cells() {
    let stores = run_alignment_source("\\halign{u#v\\cr x\\cr}");
    let alignment = page_alignment(&stores);
    let [Node::Unset(row)] = unset_children(&stores, alignment) else {
        panic!("expected one unset row");
    };
    let cells = row_cells(&stores, row);

    assert_eq!(cells.len(), 1);
    assert_eq!(cell_text(&stores, cells[0]), "uxv");
    assert_eq!(cells[0].span_count, 1);
}

#[test]
fn let_aliased_alignment_tab_terminates_cell_by_meaning() {
    let stores = run_alignment_source("\\let\\t=&\\halign{#&#\\cr a\\t b\\cr}");
    let alignment = page_alignment(&stores);
    let [Node::Unset(row)] = unset_children(&stores, alignment) else {
        panic!("expected one unset row");
    };
    let cells = row_cells(&stores, row);

    assert_eq!(cells.len(), 2);
    assert_eq!(cell_text(&stores, cells[0]), "a");
    assert_eq!(cell_text(&stores, cells[1]), "b");
}

#[test]
fn grouped_alignment_tab_does_not_terminate_outer_cell() {
    let stores = run_alignment_source("\\halign{#&#\\cr {a&b}&c\\cr}");
    let alignment = page_alignment(&stores);
    let [Node::Unset(row)] = unset_children(&stores, alignment) else {
        panic!("expected one unset row");
    };
    let cells = row_cells(&stores, row);

    assert_eq!(cells.len(), 2);
    assert_eq!(cell_text(&stores, cells[0]), "a&b");
    assert_eq!(cell_text(&stores, cells[1]), "c");
}

#[test]
fn span_replays_next_column_template_at_span_time_and_packages_one_cell() {
    let stores = run_alignment_source("\\halign{<#>&[#]\\cr a\\span b\\cr}");
    let alignment = page_alignment(&stores);
    let [Node::Unset(row)] = unset_children(&stores, alignment) else {
        panic!("expected one unset row");
    };
    let cells = row_cells(&stores, row);

    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0].span_count, 2);
    assert_eq!(cell_text(&stores, cells[0]), "<a>[b]");
}

#[test]
fn omit_skips_cell_templates() {
    let stores = run_alignment_source("\\halign{u#v\\cr \\omit x\\cr}");
    let alignment = page_alignment(&stores);
    let [Node::Unset(row)] = unset_children(&stores, alignment) else {
        panic!("expected one unset row");
    };
    let cells = row_cells(&stores, row);

    assert_eq!(cells.len(), 1);
    assert_eq!(cell_text(&stores, cells[0]), "x");
}

#[test]
fn nested_alignment_executes_inside_cell() {
    let stores = run_alignment_source("\\halign{#\\cr \\halign{#\\cr x\\cr}\\cr}");
    let alignment = page_alignment(&stores);
    let [Node::Unset(row)] = unset_children(&stores, alignment) else {
        panic!("expected one outer unset row");
    };
    let cells = row_cells(&stores, row);

    assert_eq!(cells.len(), 1);
    assert!(
        stores
            .nodes(cells[0].children)
            .iter()
            .any(|node| matches!(node, Node::Unset(unset) if unset.kind == UnsetKind::VBox))
    );
}

#[test]
fn showlists_inside_cell_reports_alignment_submode_nest() {
    let stores = run_alignment_source(
        "\\showboxbreadth=100 \\showboxdepth=100 \\halign{#\\cr x\\showlists\\cr}",
    );
    let log = support::terminal_effect_text(&stores);

    assert!(log.contains("### restricted horizontal mode entered at line 0"));
    assert!(log.contains("### internal vertical mode entered at line 0"));
}

#[test]
fn right_brace_before_cr_uses_missing_cr_recovery() {
    let stores = run_alignment_source("\\halign{#\\cr x}");
    let alignment = page_alignment(&stores);
    let [Node::Unset(row)] = unset_children(&stores, alignment) else {
        panic!("expected recovered unset row");
    };
    let cells = row_cells(&stores, row);

    assert_eq!(cells.len(), 1);
    assert_eq!(cell_text(&stores, cells[0]), "x");
    assert!(support::terminal_effect_text(&stores).contains("Missing \\cr inserted"));
}
