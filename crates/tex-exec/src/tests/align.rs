use super::*;
use tex_state::env::banks::GlueParam;
use tex_state::glue::Order;
use tex_state::ids::GlueId;
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::node::{BoxNode, Node, Sign};
use tex_state::scaled::Scaled;

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

fn sp(points: i32) -> Scaled {
    Scaled::from_raw(points * Scaled::UNITY)
}

fn run_alignment_source(source: &str) -> Universe {
    let mut stores = support::stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new(format!("\\font\\f=cmr10 \\f {source}")));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("alignment source executes");
    stores
}

fn run_alignment_source_err(source: &str) -> ExecError {
    let mut stores = support::stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new(format!("\\font\\f=cmr10 \\f {source}")));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("alignment source should fail")
}

fn run_boxed_alignment_source(source: &str) -> Universe {
    run_alignment_source(&format!("\\setbox0=\\vbox{{{source}}}"))
}

fn box_zero_vlist(stores: &Universe) -> &BoxNode {
    let root = stores.box_reg(0).expect("box0 should be assigned");
    let [Node::VList(vbox)] = stores.nodes(root) else {
        panic!(
            "expected box0 to contain one vbox, got {:?}",
            stores.nodes(root)
        );
    };
    vbox
}

fn vlist_rows<'a>(stores: &'a Universe, vbox: &'a BoxNode) -> Vec<&'a BoxNode> {
    stores
        .nodes(vbox.children)
        .iter()
        .filter_map(|node| match node {
            Node::HList(row) => Some(row),
            _ => None,
        })
        .collect()
}

fn row_cells<'a>(stores: &'a Universe, row: &'a BoxNode) -> Vec<&'a BoxNode> {
    stores
        .nodes(row.children)
        .iter()
        .filter_map(|node| match node {
            Node::HList(cell) => Some(cell),
            _ => None,
        })
        .collect()
}

fn cell_text(stores: &Universe, cell: &BoxNode) -> String {
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

fn assert_no_unset(stores: &Universe, nodes: &[Node]) {
    let mut stack = Vec::new();
    for node in nodes {
        match node {
            Node::Unset(_) => panic!("unset node escaped alignment"),
            Node::HList(box_node) | Node::VList(box_node) => stack.push(box_node.children),
            _ => {}
        }
    }
    while let Some(list) = stack.pop() {
        for node in stores.nodes(list) {
            match node {
                Node::Unset(_) => panic!("unset node escaped alignment"),
                Node::HList(box_node) | Node::VList(box_node) => stack.push(box_node.children),
                _ => {}
            }
        }
    }
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
    assert_eq!(stores.glue(state.tabskips()[1]), GlueSpec::ZERO);
    assert_eq!(
        stores.glue(state.tabskips()[2]).width.raw(),
        3 * tex_state::scaled::Scaled::UNITY
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
fn executes_rows_and_replays_u_and_v_templates_into_set_cells() {
    let stores = run_boxed_alignment_source("\\halign{u#v\\cr x\\cr}");
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);
    let cells = row_cells(&stores, rows[0]);

    assert_eq!(rows.len(), 1);
    assert_eq!(cells.len(), 1);
    assert_eq!(cell_text(&stores, cells[0]), "uxv");
    assert_no_unset(&stores, stores.nodes(vbox.children));
}

#[test]
fn let_aliased_alignment_tab_terminates_cell_by_meaning() {
    let stores = run_boxed_alignment_source("\\let\\t=&\\halign{#&#\\cr a\\t b\\cr}");
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);
    let cells = row_cells(&stores, rows[0]);

    assert_eq!(rows.len(), 1);
    assert_eq!(cells.len(), 2);
    assert_eq!(cell_text(&stores, cells[0]), "a");
    assert_eq!(cell_text(&stores, cells[1]), "b");
}

#[test]
fn grouped_alignment_tab_does_not_terminate_outer_cell() {
    let stores = run_boxed_alignment_source("\\halign{#&#\\cr {a&b}&c\\cr}");
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);
    let cells = row_cells(&stores, rows[0]);

    assert_eq!(rows.len(), 1);
    assert_eq!(cells.len(), 2);
    assert_eq!(cell_text(&stores, cells[0]), "a&b");
    assert_eq!(cell_text(&stores, cells[1]), "c");
}

#[test]
fn span_replays_next_column_template_and_inserts_blank_set_column() {
    let stores = run_boxed_alignment_source("\\halign{<#>&[#]\\cr a\\span b\\cr}");
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);
    let cells = row_cells(&stores, rows[0]);

    assert_eq!(rows.len(), 1);
    assert_eq!(cells.len(), 2);
    assert_eq!(cell_text(&stores, cells[0]), "<a>[b]");
    assert!(stores.nodes(cells[1].children).is_empty());
}

#[test]
fn spanned_width_excess_is_added_to_last_spanned_column() {
    let stores = run_boxed_alignment_source("\\halign{#&#\\cr a\\span b\\cr c&d\\cr}");
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);
    let first = row_cells(&stores, rows[0]);
    let second = row_cells(&stores, rows[1]);

    assert_eq!(rows.len(), 2);
    assert_eq!(first.len(), 2);
    assert_eq!(second.len(), 2);
    assert_eq!(cell_text(&stores, first[0]), "ab");
    assert_eq!(cell_text(&stores, second[0]), "c");
    assert_eq!(cell_text(&stores, second[1]), "d");
    assert_eq!(first[0].width, second[0].width);
    assert_eq!(first[1].width, second[1].width);
    assert!(second[1].width.raw() > first[0].width.raw());
}

#[test]
fn outer_to_spec_sets_row_width_and_tabskip_glue() {
    let stores =
        run_boxed_alignment_source("\\tabskip=0pt plus 1fil\\halign to 30pt{#&#\\cr a&b\\cr}");
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].width, sp(30));
    assert_eq!(rows[0].glue_sign, Sign::Stretching);
    assert_eq!(rows[0].glue_order, Order::Fil);
}

#[test]
fn omit_skips_cell_templates() {
    let stores = run_boxed_alignment_source("\\halign{u#v\\cr \\omit x\\cr}");
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);
    let cells = row_cells(&stores, rows[0]);

    assert_eq!(rows.len(), 1);
    assert_eq!(cells.len(), 1);
    assert_eq!(cell_text(&stores, cells[0]), "x");
}

#[test]
fn misplaced_omit_in_cell_body_reports_pdftex_primary_text() {
    let err = run_alignment_source_err("\\setbox0=\\vbox{\\halign{#\\cr a \\omit b\\cr}}");

    assert_eq!(err.to_string(), "Misplaced \\omit.");
}

#[test]
fn misplaced_noalign_outside_row_boundary_reports_pdftex_primary_text() {
    let err =
        run_alignment_source_err("\\setbox0=\\vbox{\\halign{#\\cr a \\noalign{\\hrule}\\cr}}");

    assert_eq!(err.to_string(), "Misplaced \\noalign.");
}

#[test]
fn omit_span_chain_merges_template_free_cells() {
    let stores = run_boxed_alignment_source(
        "\\halign{<#>&[#]&( # )\\cr \\omit a\\span\\omit b\\span\\omit c\\cr}",
    );
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);
    let cells = row_cells(&stores, rows[0]);

    assert_eq!(rows.len(), 1);
    assert_eq!(cells.len(), 3);
    assert_eq!(cell_text(&stores, cells[0]), "abc");
    assert!(stores.nodes(cells[1].children).is_empty());
    assert!(stores.nodes(cells[2].children).is_empty());
}

#[test]
fn noalign_material_is_spliced_between_finished_rows() {
    let stores =
        run_boxed_alignment_source("\\halign{#\\cr a\\cr\\noalign{\\hrule height2pt}b\\cr}");
    let vbox = box_zero_vlist(&stores);
    let nodes = stores.nodes(vbox.children);
    let first_row = nodes
        .iter()
        .position(|node| matches!(node, Node::HList(_)))
        .expect("first row");
    let rule = nodes
        .iter()
        .position(|node| matches!(node, Node::Rule { .. }))
        .expect("noalign rule");
    let second_row = nodes
        .iter()
        .enumerate()
        .skip(rule + 1)
        .find_map(|(index, node)| matches!(node, Node::HList(_)).then_some(index))
        .expect("second row");

    assert!(first_row < rule);
    assert!(rule < second_row);
    assert_eq!(vlist_rows(&stores, vbox).len(), 2);
}

#[test]
fn everycr_can_insert_noalign_material() {
    let stores = run_boxed_alignment_source(
        "\\everycr{\\noalign{\\hrule height1pt}}\\halign{#\\cr a\\cr b\\cr}",
    );
    let vbox = box_zero_vlist(&stores);
    let rule_count = stores
        .nodes(vbox.children)
        .iter()
        .filter(|node| matches!(node, Node::Rule { .. }))
        .count();

    assert_eq!(vlist_rows(&stores, vbox).len(), 2);
    assert_eq!(rule_count, 3);
}

#[test]
fn everycr_replayed_crcr_is_ignored_around_rows_and_after_last_cr() {
    let stores = run_boxed_alignment_source("\\everycr{\\crcr}\\halign{#\\cr a\\cr b\\cr}");
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);

    assert_eq!(rows.len(), 2);
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[0])[0]), "a");
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[1])[0]), "b");
}

#[test]
fn display_halign_appends_display_vertical_material() {
    let stores = run_alignment_source(
        "\\setbox0=\\vbox{\\hsize=50pt \\predisplaypenalty=11 \\postdisplaypenalty=22 \
         \\abovedisplayskip=3pt \\belowdisplayskip=4pt \
         \\noindent$$\\halign{#\\cr a\\cr}$$\\par}",
    );
    let vbox = box_zero_vlist(&stores);
    let nodes = stores.nodes(vbox.children);

    assert!(nodes.iter().any(|node| matches!(node, Node::Penalty(11))));
    assert!(nodes.iter().any(|node| matches!(node, Node::Penalty(22))));
    assert!(nodes.iter().any(|node| matches!(node, Node::Glue { .. })));
    assert!(nodes.iter().any(|node| matches!(node, Node::HList(_))));
}

#[test]
fn nested_alignment_executes_inside_cell() {
    let stores = run_boxed_alignment_source("\\halign{#\\cr \\halign{#\\cr x\\cr}\\cr}");
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);
    let cells = row_cells(&stores, rows[0]);

    assert_eq!(rows.len(), 1);
    assert_eq!(cells.len(), 1);
    assert!(
        stores
            .nodes(cells[0].children)
            .iter()
            .any(|node| matches!(node, Node::HList(_)))
    );
    assert_no_unset(&stores, stores.nodes(vbox.children));
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
    let stores = run_boxed_alignment_source("\\halign{#\\cr x}");
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);
    let cells = row_cells(&stores, rows[0]);

    assert_eq!(rows.len(), 1);
    assert_eq!(cells.len(), 1);
    assert_eq!(cell_text(&stores, cells[0]), "x");
    assert!(support::terminal_effect_text(&stores).contains("Missing \\cr inserted"));
}
