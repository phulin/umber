use super::*;
use tex_state::env::banks::GlueParam;
use tex_state::glue::Order;
use tex_state::ids::GlueId;
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::node::{BoxNode, GlueKind, Node, Sign, UnsetKind, UnsetNode, UnsetNodeFields};
use tex_state::scaled::Scaled;
use tex_state::{CheckpointMetadata, CheckpointResumeKind, ResumeFallback};

fn scan_halign_preamble(source: &str) -> (Universe, AlignState) {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(source));
    let mut hooks = crate::executor::NoopExecHooks;
    let state = crate::align::scan_preamble(
        UnexpandablePrimitive::HAlign,
        alignment_context(),
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
        alignment_context(),
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

fn alignment_context() -> TracedTokenWord {
    TracedTokenWord::pack(char_token('&', Catcode::AlignmentTab), OriginId::UNKNOWN)
}

fn sp(points: i32) -> Scaled {
    Scaled::from_raw(points * Scaled::UNITY)
}

fn unset_for_test(
    stores: &mut Universe,
    kind: UnsetKind,
    children: &[Node],
    span_count: u16,
) -> Node {
    let children = stores.freeze_node_list(children);
    let metrics = tex_typeset::measure_unset(stores, children, kind);
    Node::Unset(UnsetNode::new(UnsetNodeFields {
        kind,
        width: metrics.width,
        height: metrics.height,
        depth: metrics.depth,
        span_count,
        stretch: metrics.stretch,
        stretch_order: metrics.stretch_order,
        shrink: metrics.shrink,
        shrink_order: metrics.shrink_order,
        children,
    }))
}

fn run_alignment_source(source: &str) -> Universe {
    let mut stores = support::stores_with_fonts();
    run_alignment_source_in(&mut stores, source);
    stores
}

fn run_alignment_source_in(stores: &mut Universe, source: &str) {
    let mut input = InputStack::new(MemoryInput::new(format!(
        "\\font\\f=cmr10 \\relax \\f {source}"
    )));
    Executor::new()
        .run(&mut input, stores)
        .expect("alignment source executes");
}

fn run_alignment_source_err(source: &str) -> ExecError {
    let mut stores = support::stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new(format!(
        "\\font\\f=cmr10 \\relax \\f {source}"
    )));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect_err("alignment source should fail")
}

fn run_boxed_alignment_source(source: &str) -> Universe {
    run_alignment_source(&format!("\\setbox0=\\vbox{{{source}}}"))
}

fn run_nested_shipout_source(stores: &mut Universe, source: &str) -> CheckpointMetadata {
    let mut input = InputStack::new(MemoryInput::new(format!(
        "\\font\\f=cmr10 \\relax \\f {source}"
    )));
    let stats = Executor::new()
        .run(&mut input, stores)
        .expect("nested shipout source executes");
    assert_eq!(
        stats.shipped_artifacts.len(),
        1,
        "every committed nested shipout is surfaced to the output driver"
    );
    stores
        .last_checkpoint()
        .expect("nested shipout should create a checkpoint")
}

fn assert_nested_shipout_replays_from_resume_boundary(source: &str) {
    let mut stores = support::stores_with_fonts();
    let resume = stores.snapshot();
    let resume_boundary = resume
        .resume_fallback()
        .expect("initial checkpoint should be resume-valid")
        .boundary();

    let first = run_nested_shipout_source(&mut stores, source);
    assert_eq!(first.resume_kind(), CheckpointResumeKind::HashOnly);
    assert_eq!(
        first.resume_fallback(),
        Some(ResumeFallback::DirectRollback(resume_boundary))
    );

    stores.rollback(&resume);

    let second = run_nested_shipout_source(&mut stores, source);
    assert_eq!(second.resume_kind(), CheckpointResumeKind::HashOnly);
    assert_eq!(
        second.resume_fallback(),
        Some(ResumeFallback::DirectRollback(resume_boundary))
    );
    assert_eq!(second.state_hash(), first.state_hash());
}

fn assert_effectful_nested_shipout_fallback_unavailable(source: &str) {
    let mut stores = support::stores_with_fonts();
    let resume_boundary = stores
        .snapshot()
        .resume_fallback()
        .expect("initial checkpoint should be resume-valid")
        .boundary();

    let checkpoint = run_nested_shipout_source(&mut stores, source);

    assert_eq!(checkpoint.resume_kind(), CheckpointResumeKind::HashOnly);
    assert_eq!(
        checkpoint.resume_fallback(),
        Some(ResumeFallback::Unavailable(resume_boundary))
    );
}

fn box_zero_vlist(stores: &Universe) -> BoxNode {
    let root = stores.box_reg(0).expect("box0 should be assigned");
    let Some(tex_state::node_arena::NodeRef::VList(vbox)) = stores.nodes(root).first() else {
        panic!(
            "expected box0 to contain one vbox, got {:?}",
            stores.nodes(root).testing_decoded()
        );
    };
    vbox
}

fn box_zero_hlist(stores: &Universe) -> BoxNode {
    let root = stores.box_reg(0).expect("box0 should be assigned");
    let Some(tex_state::node_arena::NodeRef::HList(hbox)) = stores.nodes(root).first() else {
        panic!(
            "expected box0 to contain one hbox, got {:?}",
            stores.nodes(root).testing_decoded()
        );
    };
    hbox
}

fn vlist_rows(stores: &Universe, vbox: impl std::borrow::Borrow<BoxNode>) -> Vec<BoxNode> {
    let vbox = vbox.borrow();
    stores
        .nodes(vbox.children)
        .into_iter()
        .filter_map(|node| match node {
            tex_state::node_arena::NodeRef::HList(row) => Some(row),
            _ => None,
        })
        .collect()
}

fn hlist_vboxes(stores: &Universe, hbox: impl std::borrow::Borrow<BoxNode>) -> Vec<BoxNode> {
    let hbox = hbox.borrow();
    stores
        .nodes(hbox.children)
        .into_iter()
        .filter_map(|node| match node {
            tex_state::node_arena::NodeRef::VList(vbox) => Some(vbox),
            _ => None,
        })
        .collect()
}

fn row_cells(stores: &Universe, row: impl std::borrow::Borrow<BoxNode>) -> Vec<BoxNode> {
    let row = row.borrow();
    stores
        .nodes(row.children)
        .into_iter()
        .filter_map(|node| match node {
            tex_state::node_arena::NodeRef::HList(cell) => Some(cell),
            _ => None,
        })
        .collect()
}

fn cell_text(stores: &Universe, cell: impl std::borrow::Borrow<BoxNode>) -> String {
    let cell = cell.borrow();
    stores
        .nodes(cell.children)
        .into_iter()
        .filter_map(|node| match node {
            tex_state::node_arena::NodeRef::Char { ch, .. } => Some(ch),
            tex_state::node_arena::NodeRef::Lig { ch, .. } => Some(ch),
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
        for node in stores.nodes(list).testing_decoded() {
            match node {
                Node::Unset(_) => panic!("unset node escaped alignment"),
                Node::HList(box_node) | Node::VList(box_node) => stack.push(box_node.children),
                _ => {}
            }
        }
    }
}

fn contains_rule_leader(stores: &Universe, nodes: &[Node], kind: GlueKind, height: Scaled) -> bool {
    nodes.iter().any(|node| match node {
        Node::Glue {
            kind: actual_kind,
            leader: Some(tex_state::node::LeaderPayload::Rule { height: actual, .. }),
            ..
        } => *actual_kind == kind && *actual == Some(height),
        Node::HList(box_node) | Node::VList(box_node) => contains_rule_leader(
            stores,
            stores.nodes(box_node.children).testing_decoded(),
            kind,
            height,
        ),
        _ => false,
    })
}

fn collect_infinite_glue(
    stores: &Universe,
    nodes: &[Node],
    out: &mut Vec<tex_state::glue::GlueSpec>,
) {
    for node in nodes {
        match node {
            Node::Glue {
                spec,
                kind: GlueKind::Normal,
                ..
            } => {
                let spec = stores.glue(*spec);
                if spec.stretch_order != Order::Normal || spec.shrink_order != Order::Normal {
                    out.push(spec);
                }
            }
            Node::HList(box_node) | Node::VList(box_node) => {
                collect_infinite_glue(
                    stores,
                    stores.nodes(box_node.children).testing_decoded(),
                    out,
                );
            }
            _ => {}
        }
    }
}

#[test]
fn halign_in_unrestricted_horizontal_mode_finishes_paragraph_first() {
    let stores = run_boxed_alignment_source("x\\halign{#\\cr y\\cr}");
    let boxes = vlist_rows(&stores, box_zero_vlist(&stores));

    assert_eq!(boxes.len(), 2, "paragraph line must precede alignment row");
    assert_eq!(cell_text(&stores, row_cells(&stores, boxes[1])[0]), "y");
}

#[test]
fn halign_head_for_vmode_replay_preserves_command_origin() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let halign = Token::Cs(stores.intern("halign").symbol());
    let command_origin = stores.synthetic_origin(tex_state::provenance::SyntheticOriginKind::Test);
    let command = TracedTokenWord::pack(halign, command_origin);
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut nest = ModeNest::new();
    nest.push(Mode::Horizontal);
    let mut hooks = crate::executor::NoopExecHooks;

    assert_eq!(
        dispatch_delivered_token(&mut nest, command, &mut input, &mut stores, &mut hooks)
            .expect("head_for_vmode dispatch"),
        DispatchAction::Continue
    );
    let inserted = tex_expand::get_x_token_with_recorder_and_hooks(
        &mut input,
        &mut stores,
        &mut NoopRecorder,
        &mut hooks,
    )
    .expect("inserted paragraph read")
    .expect("inserted paragraph token");
    let replayed = tex_expand::get_x_token_with_recorder_and_hooks(
        &mut input,
        &mut stores,
        &mut NoopRecorder,
        &mut hooks,
    )
    .expect("halign replay read")
    .expect("halign replay token");

    assert_eq!(
        tex_expand::semantic_token(inserted),
        Token::Cs(stores.intern("par").symbol())
    );
    let tex_state::provenance::OriginRecord::Inserted(inserted_origin) =
        stores.origin(inserted.origin())
    else {
        panic!("synthetic paragraph should carry inserted provenance");
    };
    assert_eq!(
        inserted_origin.kind(),
        tex_state::provenance::InsertedOriginKind::Paragraph
    );
    assert_eq!(inserted_origin.parent(), command_origin);
    assert_eq!(replayed, command);
}

#[test]
fn hrule_head_for_vmode_defers_rule_until_after_paragraph_dispatch() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let hrule = Token::Cs(stores.intern("hrule").symbol());
    let command_origin = stores.synthetic_origin(tex_state::provenance::SyntheticOriginKind::Test);
    let command = TracedTokenWord::pack(hrule, command_origin);
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut nest = ModeNest::new();
    nest.push(Mode::Horizontal);
    let mut hooks = crate::executor::NoopExecHooks;

    assert_eq!(
        dispatch_delivered_token(&mut nest, command, &mut input, &mut stores, &mut hooks)
            .expect("head_for_vmode dispatch"),
        DispatchAction::Continue
    );
    assert!(stores.page_contributions().is_empty());

    let inserted = tex_expand::get_x_token_with_recorder_and_hooks(
        &mut input,
        &mut stores,
        &mut NoopRecorder,
        &mut hooks,
    )
    .expect("inserted paragraph read")
    .expect("inserted paragraph token");
    assert_eq!(
        tex_expand::semantic_token(inserted),
        Token::Cs(stores.intern("par").symbol())
    );

    let replayed = tex_expand::get_x_token_with_recorder_and_hooks(
        &mut input,
        &mut stores,
        &mut NoopRecorder,
        &mut hooks,
    )
    .expect("replayed hrule read")
    .expect("replayed hrule token");
    assert_eq!(tex_expand::semantic_token(replayed), hrule);
    assert_eq!(replayed.origin(), command_origin);
}

#[test]
fn halign_in_restricted_horizontal_mode_retains_off_save_recovery() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let halign = Token::Cs(stores.intern("halign").symbol());
    let command_origin = stores.synthetic_origin(tex_state::provenance::SyntheticOriginKind::Test);
    let command = TracedTokenWord::pack(halign, command_origin);
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut nest = ModeNest::new();
    nest.push(Mode::RestrictedHorizontal);
    let mut hooks = crate::executor::NoopExecHooks;

    dispatch_delivered_token(&mut nest, command, &mut input, &mut stores, &mut hooks)
        .expect("off_save should insert a closing group");
    let inserted = tex_expand::get_x_token_with_recorder_and_hooks(
        &mut input,
        &mut stores,
        &mut NoopRecorder,
        &mut hooks,
    )
    .expect("inserted group read")
    .expect("inserted group token");
    let replayed = tex_expand::get_x_token_with_recorder_and_hooks(
        &mut input,
        &mut stores,
        &mut NoopRecorder,
        &mut hooks,
    )
    .expect("halign replay read")
    .expect("halign replay token");

    assert_eq!(
        tex_expand::semantic_token(inserted),
        Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        }
    );
    let tex_state::provenance::OriginRecord::Inserted(inserted_origin) =
        stores.origin(inserted.origin())
    else {
        panic!("off_save token should carry inserted provenance");
    };
    assert_eq!(
        inserted_origin.kind(),
        tex_state::provenance::InsertedOriginKind::ErrorRecovery
    );
    assert_eq!(inserted_origin.parent(), command_origin);
    assert_eq!(replayed, command);
    assert!(support::terminal_effect_text(&stores).contains("Missing } inserted"));
}

#[test]
fn math_group_scanned_inside_cell_does_not_hide_row_terminator() {
    let stores = run_boxed_alignment_source("\\halign{#\\cr ${}^1$\\cr}");
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));

    assert_eq!(rows.len(), 1);
    assert_eq!(row_cells(&stores, rows[0]).len(), 1);
}

#[test]
fn split_hbox_template_injects_v_part_before_inline_math_row_terminator() {
    let stores = run_boxed_alignment_source(
        "\\halign{\\hbox to 20pt{#}\\cr \\hfil{}$\\mathrel{a}$Size$\\mathrel{b}$\\cr}",
    );
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));

    assert_eq!(rows.len(), 1);
    assert_eq!(row_cells(&stores, rows[0]).len(), 1);
}

#[test]
fn split_hbox_math_cell_replays_identically_after_rollback() {
    let mut stores = support::stores_with_fonts();
    let checkpoint = stores.snapshot();
    let source = "\\setbox0=\\vbox{\\halign{\\hbox to 20pt{#}\\cr \\hfil{}$\\mathrel{a}$Size$\\mathrel{b}$\\cr}}";

    run_alignment_source_in(&mut stores, source);
    let first_hash = stores.snapshot().state_hash();

    stores.rollback(&checkpoint);
    run_alignment_source_in(&mut stores, source);

    assert_eq!(stores.snapshot().state_hash(), first_hash);
}

#[test]
fn control_space_cell_ignores_following_source_blanks() {
    let stores = run_boxed_alignment_source(
        "\\font\\t=cmtt10 \\def\\\\{\\char92{}}\\def\\sp{\\char32{}}\
         \\halign{\\hfil\\t#\\hfil\\cr XXXXXXXXXX\\cr \\\\\\sp\\   \\cr}",
    );
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));
    let cell = row_cells(&stores, rows[1])[0];
    let font = stores
        .nodes(cell.children)
        .testing_decoded()
        .iter()
        .find_map(|node| match node {
            Node::Char { font, .. } => Some(*font),
            _ => None,
        })
        .expect("cell should contain typewriter characters");
    let finite_spaces: Vec<_> = stores
        .nodes(cell.children)
        .testing_decoded()
        .iter()
        .filter_map(|node| match node {
            Node::Glue { spec, .. } if stores.glue(*spec).stretch_order == Order::Normal => {
                Some(stores.glue(*spec))
            }
            _ => None,
        })
        .collect();

    assert_eq!(cell_text(&stores, cell), "\\ ");
    assert_eq!(finite_spaces.len(), 1);
    assert_eq!(finite_spaces[0].width, stores.font_parameter(font, 2));
}

#[test]
fn control_space_preserves_sentence_factor_for_v_template_space() {
    let stores = run_boxed_alignment_source(
        "\\font\\t=cmtt10 \\def\\\\{\\char92{}}\\sfcode33=3000 \
         \\halign{\\hfil\\t# \\hfil\\cr XXXXXXXXXX\\cr \\ \\\\!\\   \\cr}",
    );
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));
    let cell = row_cells(&stores, rows[1])[0];
    let font = stores
        .nodes(cell.children)
        .testing_decoded()
        .iter()
        .find_map(|node| match node {
            Node::Char { font, .. } => Some(*font),
            _ => None,
        })
        .expect("cell should contain typewriter characters");
    let finite_spaces: Vec<_> = stores
        .nodes(cell.children)
        .testing_decoded()
        .iter()
        .filter_map(|node| match node {
            Node::Glue { spec, .. } if stores.glue(*spec).stretch_order == Order::Normal => {
                Some(stores.glue(*spec))
            }
            _ => None,
        })
        .collect();

    assert_eq!(cell_text(&stores, cell), "\\!");
    assert_eq!(finite_spaces.len(), 3);
    assert_eq!(finite_spaces[0].width, stores.font_parameter(font, 2));
    assert_eq!(finite_spaces[1].width, stores.font_parameter(font, 2));
    assert_eq!(
        finite_spaces[2].width,
        stores.font_parameter(font, 2) + stores.font_parameter(font, 7)
    );
}

#[test]
fn math_group_cell_alignment_replays_identically_after_rollback() {
    let mut stores = support::stores_with_fonts();
    let checkpoint = stores.snapshot();
    let source = "\\setbox0=\\vbox{\\halign{#\\cr ${}^1$\\cr}}";

    run_alignment_source_in(&mut stores, source);
    let first_hash = stores.snapshot().state_hash();

    stores.rollback(&checkpoint);
    run_alignment_source_in(&mut stores, source);

    assert_eq!(stores.snapshot().state_hash(), first_hash);
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
        &[
            char_token('v', Catcode::Letter),
            Token::frozen_end_template()
        ]
    );
    assert_eq!(state.tabskips(), &[GlueId::ZERO, GlueId::ZERO]);
    assert_eq!(state.default_tabskip(), GlueId::ZERO);
}

#[test]
fn v_template_macros_expand_when_the_cell_finishes() {
    let stores = run_boxed_alignment_source(
        "\\def\\vpart{\\hbox to10pt{}}\
         \\halign{#\\vpart\\cr \\def\\vpart{\\hbox to20pt{}}\\cr}",
    );
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));
    let cells = row_cells(&stores, rows[0]);

    assert_eq!(cells[0].width, sp(20));
}

#[test]
fn futurelet_undefined_recovery_stays_inside_alignment_cell_driver() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\halign{#&#\\cr \\futurelet\\x\\missing&a\\cr}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("undefined futurelet token recovers without unwinding alignment");

    assert!(
        support::terminal_effect_text(&stores).contains("Undefined control sequence \\missing")
    );
}

#[test]
fn extra_alignment_tab_is_changed_to_row_terminator() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\halign{#\\cr a&b\\cr}"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("extra alignment tab ends the row recoverably");

    assert!(
        support::terminal_effect_text(&stores)
            .contains("Extra alignment tab has been changed to \\cr")
    );
}

#[test]
fn u_template_macro_argument_interleaves_cell_body_and_v_template() {
    let stores =
        run_boxed_alignment_source("\\def\\wrap#1{\\hbox{#1}}\\halign{\\wrap{#}\\cr x\\cr}");
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));
    let cells = row_cells(&stores, rows[0]);
    let [Node::HList(wrapped)] = stores.nodes(cells[0].children).testing_decoded() else {
        panic!("cell should contain the box built by the split template macro");
    };

    assert_eq!(cell_text(&stores, wrapped), "x");
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
    let (stores, state) =
        scan_halign_preamble("{#a&\\tabskip=1pt#b&&\\tabskip=2pt#c&\\tabskip=3pt#d\\cr}");

    assert_eq!(state.columns().len(), 4);
    assert_eq!(state.loop_start(), Some(2));
    assert_eq!(state.column_for(0), Some(&state.columns()[0]));
    assert_eq!(state.column_for(3), Some(&state.columns()[3]));
    assert_eq!(state.column_for(4), Some(&state.columns()[2]));
    assert_eq!(state.column_for(5), Some(&state.columns()[3]));
    assert_eq!(
        stores.glue(state.tabskip_for_boundary(5)).width.raw(),
        2 * Scaled::UNITY,
        "the boundary after repeated column 2 repeats boundary 3",
    );
    assert_eq!(
        stores.glue(state.tabskip_for_boundary(6)).width.raw(),
        3 * Scaled::UNITY,
        "the boundary after repeated column 3 repeats boundary 4",
    );
    assert_eq!(
        stores.tokens(state.column_for(4).expect("repeat col").v_template),
        &[
            char_token('c', Catcode::Letter),
            Token::frozen_end_template()
        ]
    );
}

#[test]
fn plain_ialign_accepts_bgroup_and_leading_periodic_preamble() {
    let stores = run_boxed_alignment_source("\\let\\bgroup={\\halign\\bgroup&#x\\cr a&b\\cr}");
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));
    let cells = row_cells(&stores, rows[0]);

    assert_eq!(cells.len(), 2);
    assert_eq!(cell_text(&stores, cells[0]), "ax");
    assert_eq!(cell_text(&stores, cells[1]), "bx");
}

#[test]
fn plain_tab_row_closes_alignment_and_box_before_surrounding_begingroup() {
    let source = "\\let\\bgroup={\\let\\egroup=}\
         \\def\\tbbox{\\setbox0=\\hbox\\bgroup}\
         \\def\\tbbx{\\egroup}\
         \\count0=7\
         \\def\\tabalign{\\begingroup\\count0=9\
           \\setbox0=\\vbox\\bgroup\
           \\def\\cr{\\crcr\\egroup\\egroup\\unvbox0\\lastbox\
             \\endgroup\\count1=\\count0}\
           \\halign\\bgroup&\\tbbox##\\tbbx\\crcr}\
         \\tabalign a&b\\cr";
    let mut stores = support::stores_with_fonts();
    let checkpoint = stores.snapshot();

    run_alignment_source_in(&mut stores, source);

    assert_eq!(stores.count(0), 7);
    assert_eq!(stores.count(1), 7);
    let first_hash = stores.snapshot().state_hash();

    stores.rollback(&checkpoint);
    run_alignment_source_in(&mut stores, source);

    assert_eq!(stores.count(0), 7);
    assert_eq!(stores.count(1), 7);
    assert_eq!(stores.snapshot().state_hash(), first_hash);
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
        &[
            char_token('y', Catcode::Letter),
            Token::frozen_end_template()
        ]
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
        &[Token::frozen_end_template()]
    );
}

#[test]
fn alignment_preamble_errors_match_reference_wording() {
    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("{abc\\cr}"));
    let mut hooks = crate::executor::NoopExecHooks;
    crate::align::scan_preamble(
        UnexpandablePrimitive::HAlign,
        alignment_context(),
        &mut input,
        &mut stores,
        &mut hooks,
    )
    .expect("missing hash should be inserted recoverably");
    assert!(
        support::terminal_effect_text(&stores).contains("Missing # inserted in alignment preamble")
    );

    let mut stores = Universe::new();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("{#a#b\\cr}"));
    let mut hooks = crate::executor::NoopExecHooks;
    let state = crate::align::scan_preamble(
        UnexpandablePrimitive::HAlign,
        alignment_context(),
        &mut input,
        &mut stores,
        &mut hooks,
    )
    .expect("extra hash should be ignored recoverably");
    assert!(support::terminal_effect_text(&stores).contains("Only one # is allowed per tab"));
    assert_eq!(
        stores.tokens(state.columns()[0].v_template),
        &[
            Token::Char {
                ch: 'a',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: 'b',
                cat: Catcode::Letter,
            },
            Token::frozen_end_template(),
        ]
    );
}

#[test]
fn mid_alignment_snapshot_rollback_restores_summary_and_unset_rows() {
    let (mut stores, state) = scan_halign_preamble("{#&#\\cr}");
    let seed =
        tex_state::InputSummary::new_with_resume_state(Vec::new(), None, None, None, 1, true);
    let mut input =
        InputStack::<MemoryInput>::from_summary(&seed, |_, _, _| -> Result<MemoryInput, ()> {
            unreachable!("empty seed has no source to restore")
        })
        .expect("restore empty input stack");
    input.push_source(MemoryInput::new("b&c\\cr}"));
    let input_summary = input.publication_summary(&mut stores);
    let mut nest = ModeNest::new();
    nest.push(Mode::InternalVertical);
    nest.current_list_mut().set_align_state(state);

    let cell = unset_for_test(
        &mut stores,
        UnsetKind::HBox,
        &[Node::Rule {
            width: Some(sp(3)),
            height: Some(sp(1)),
            depth: Some(Scaled::from_raw(0)),
        }],
        1,
    );
    let row = unset_for_test(
        &mut stores,
        UnsetKind::HBox,
        &[
            Node::Glue {
                spec: GlueId::ZERO,
                kind: GlueKind::TabSkip,
                leader: None,
            },
            cell,
            Node::Glue {
                spec: GlueId::ZERO,
                kind: GlueKind::TabSkip,
                leader: None,
            },
        ],
        1,
    );

    {
        let list = nest.current_list_mut();
        list.push(row);
        let state = list.align_state_mut().expect("alignment state");
        state.start_row();
        state.start_cell(1, 2);
        state.increment_brace_depth();
        state.set_suppress_redundant_cr(true);
    }
    stores.set_input_summary(input_summary.clone());
    let snapshot = stores.snapshot();
    let summary = nest.summary();

    let _temporary = stores.freeze_node_list(&[Node::Penalty(99)]);
    stores.set_input_summary(tex_state::InputSummary::default());
    {
        let list = nest.current_list_mut();
        list.push(Node::Penalty(123));
        let state = list.align_state_mut().expect("alignment state");
        state.start_cell(0, 1);
        state.set_brace_depth(0);
    }

    stores.rollback(&snapshot);
    let restored = ModeNest::from_summary(summary.clone()).expect("restored alignment summary");

    assert_eq!(stores.input_summary(), &input_summary);
    assert_eq!(restored.summary(), summary);
    let restored_state = restored
        .current_list()
        .align_state()
        .expect("restored alignment state");
    assert_eq!(restored_state.current_col(), 1);
    assert_eq!(restored_state.current_span(), 2);
    assert_eq!(restored_state.brace_depth(), 1);
    assert!(restored_state.suppress_redundant_cr());
    let [Node::Unset(row)] = restored.current_list().nodes() else {
        panic!(
            "expected a partial unset alignment row, got {:?}",
            restored.current_list().nodes()
        );
    };
    assert_eq!(stores.nodes(row.children).testing_decoded().len(), 3);
}

#[test]
fn shipout_rejects_unset_alignment_nodes() {
    let mut stores = Universe::new();
    let unset = unset_for_test(&mut stores, UnsetKind::HBox, &[], 1);
    let err = crate::assignments::shipout_node(unset, &mut stores, &mut NoopRecorder)
        .expect_err("unset alignment node must not lower to shipout artifact");

    assert_eq!(
        err.to_string(),
        "shipout artifact lowering does not support unset alignment nodes yet"
    );
}

#[test]
fn box_group_shipout_checkpoint_is_hash_only_and_replays_from_boundary() {
    assert_nested_shipout_replays_from_resume_boundary(
        "\\setbox0=\\hbox{\\shipout\\hbox{A}B}\\end",
    );
}

#[test]
fn alignment_shipout_checkpoint_is_hash_only_and_replays_from_boundary() {
    assert_nested_shipout_replays_from_resume_boundary(
        "\\setbox0=\\vbox{\\halign{#\\cr \\shipout\\hbox{A}x\\cr}}\\end",
    );
}

#[test]
fn effectful_box_group_shipout_checkpoint_marks_fallback_unavailable() {
    assert_effectful_nested_shipout_fallback_unavailable(
        "\\setbox0=\\hbox{\\shipout\\hbox{\\write16{nested}}B}\\end",
    );
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
    assert_no_unset(&stores, stores.nodes(vbox.children).testing_decoded());
}

#[test]
fn restricted_horizontal_u_template_ending_in_macro_stops_before_cell_input() {
    let stores =
        run_boxed_alignment_source("\\def\\templateend{\\relax}\\halign{\\templateend#\\cr x\\cr}");
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);
    let cells = row_cells(&stores, rows[0]);

    assert_eq!(rows.len(), 1);
    assert_eq!(cells.len(), 1);
    assert_eq!(cell_text(&stores, cells[0]), "x");
}

#[test]
fn v_template_ending_in_macro_delivers_frozen_endv_after_frame_retirement() {
    let stores =
        run_boxed_alignment_source("\\def\\templateend{\\relax}\\halign{#\\templateend\\cr x\\cr}");
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);

    assert_eq!(rows.len(), 1);
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[0])[0]), "x");
}

#[test]
fn user_endtemplate_control_sequence_cannot_alias_frozen_sentinel() {
    let stores = run_boxed_alignment_source("\\def\\endtemplate{BAD}\\halign{#\\cr x\\cr}");
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);

    assert_eq!(rows.len(), 1);
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[0])[0]), "x");
}

#[test]
fn frozen_endv_alignment_replays_identically_after_rollback() {
    let mut stores = support::stores_with_fonts();
    let checkpoint = stores.snapshot();
    let source = "\\def\\templateend{\\relax}\\setbox0=\\vbox{\\halign{#\\templateend\\cr x\\cr}}";

    run_alignment_source_in(&mut stores, source);
    let first_hash = stores.snapshot().state_hash();

    stores.rollback(&checkpoint);
    run_alignment_source_in(&mut stores, source);

    assert_eq!(stores.snapshot().state_hash(), first_hash);
}

#[test]
fn grouped_plain_style_accent_survives_at_cell_start_and_mid_cell() {
    let stores = run_boxed_alignment_source(
        "\\def\\tilde#1{{\\accent\"7E #1}}\\halign{\\hfil#\\hfil\\cr \\tilde{}\\cr x\\tilde{}y\\cr}",
    );
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);

    assert_eq!(rows.len(), 2);
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[0])[0]), "~");
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[1])[0]), "x~y");
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
    assert!(stores.nodes(cells[1].children).testing_decoded().is_empty());
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
fn leading_u_template_spaces_do_not_contribute_to_column_widths() {
    let compact = run_boxed_alignment_source("\\halign{#&#\\cr a&b\\cr}");
    let indented = run_boxed_alignment_source("\\halign{   #&   #\\cr a&b\\cr}");

    let compact_vbox = box_zero_vlist(&compact);
    let indented_vbox = box_zero_vlist(&indented);
    let compact_rows = vlist_rows(&compact, compact_vbox);
    let indented_rows = vlist_rows(&indented, indented_vbox);
    let compact_cells = row_cells(&compact, compact_rows[0]);
    let indented_cells = row_cells(&indented, indented_rows[0]);

    assert_eq!(indented_rows[0].width, compact_rows[0].width);
    assert_eq!(indented_cells[0].width, compact_cells[0].width);
    assert_eq!(indented_cells[1].width, compact_cells[1].width);
    assert_eq!(
        indented
            .nodes(indented_cells[0].children)
            .first()
            .expect("first cell should contain its character"),
        compact
            .nodes(compact_cells[0].children)
            .first()
            .expect("first cell should contain its character"),
    );
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
fn misplaced_omit_in_cell_body_reports_reference_primary_text() {
    let err = run_alignment_source_err("\\setbox0=\\vbox{\\halign{#\\cr a \\omit b\\cr}}");

    assert_eq!(err.to_string(), "Misplaced \\omit.");
}

#[test]
fn misplaced_noalign_outside_row_boundary_reports_reference_primary_text() {
    let err =
        run_alignment_source_err("\\setbox0=\\vbox{\\halign{#\\cr a \\noalign{\\hrule}\\cr}}");

    assert_eq!(err.to_string(), "Misplaced \\noalign.");
}

#[test]
fn show_get_token_intercepts_cell_terminator_before_reading_v_template() {
    let stores =
        run_boxed_alignment_source("\\def\\A{seen}\\halign{#\\A\\cr \\show\\cr \\omit x\\cr}");

    assert!(support::terminal_effect_text(&stores).contains("> \\A=macro:"));
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
    assert!(stores.nodes(cells[1].children).testing_decoded().is_empty());
    assert!(stores.nodes(cells[2].children).testing_decoded().is_empty());
}

#[test]
fn span_template_side_effects_are_local_to_alignment_entry() {
    let stores = run_boxed_alignment_source(
        "\\count2=48 \\def\\m{\\char\\count2 \\advance\\count2 by1 }\
         \\halign{#&\\m#&\\m#\\cr A\\span B\\span C\\cr D&E&F\\cr}",
    );
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);
    let first = row_cells(&stores, rows[0]);
    let second = row_cells(&stores, rows[1]);

    assert_eq!(cell_text(&stores, first[0]), "A0B1C");
    assert_eq!(cell_text(&stores, second[1]), "0E");
    assert_eq!(cell_text(&stores, second[2]), "0F");
}

#[test]
fn macro_after_span_executes_remaining_assignment_tokens() {
    let mut stores = support::stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    run_alignment_source_in(
        &mut stores,
        "\\setbox0=\\vbox{\\count1=2 \\def\\xx{\\global\\gdef\\A{\\global\\count\\count1=-17\\cr\\omit\\cr\\tabskip}}\\halign{#&\\A#\\cr \\expandafter\\xx\\span A&x\\cr}}",
    );

    assert_eq!(stores.count(2), -17);
}

#[test]
fn expandafter_intercepts_span_before_replaying_saved_macro() {
    let mut stores = support::stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    run_alignment_source_in(
        &mut stores,
        "\\setbox0=\\vbox{\\def\\A{\\ifnum\\count4=0 \\global\\count2=1\\fi \\global\\advance\\count4 by1}\\def\\xx{\\global\\def\\A{\\ifnum\\count4=0 \\global\\count2=2\\fi \\global\\advance\\count4 by1}}\\halign{#\\A&#\\cr z\\expandafter\\xx\\span x&y\\cr}}",
    );

    assert_eq!(stores.count(2), 1);
}

#[test]
fn noalign_material_is_spliced_between_finished_rows() {
    let stores =
        run_boxed_alignment_source("\\halign{#\\cr a\\cr\\noalign{\\hrule height2pt}b\\cr}");
    let vbox = box_zero_vlist(&stores);
    let nodes = stores.nodes(vbox.children).testing_decoded();
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
fn noalign_nointerlineskip_suppresses_next_row_baseline_glue() {
    let stores = run_boxed_alignment_source(
        "\\baselineskip=20pt \\halign{#\\cr a\\cr\\noalign{\\nointerlineskip}b\\cr}",
    );
    let vbox = box_zero_vlist(&stores);
    let nodes = stores.nodes(vbox.children).testing_decoded();
    let row_indices: Vec<_> = nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| matches!(node, Node::HList(_)).then_some(index))
        .collect();

    assert_eq!(row_indices.len(), 2);
    assert_eq!(row_indices[1], row_indices[0] + 1);
}

#[test]
fn ordinary_halign_inherits_enclosing_prevdepth_for_first_row() {
    let stores = run_alignment_source(
        "\\baselineskip=20pt \\lineskiplimit=-100pt \
         \\setbox1=\\hbox{} \\ht1=4pt \\dp1=1pt \
         \\setbox0=\\vbox{\\copy1 \\halign{#\\cr \\copy1\\cr}}",
    );
    let vbox = box_zero_vlist(&stores);
    let nodes = stores.nodes(vbox.children).testing_decoded();

    let [
        Node::HList(_),
        Node::Glue { spec, kind, .. },
        Node::HList(_),
    ] = nodes
    else {
        panic!("expected enclosing box, baseline glue, and alignment row, got {nodes:?}");
    };
    assert_eq!(*kind, GlueKind::BaselineSkip);
    assert_eq!(stores.glue(*spec).width, sp(15));
}

#[test]
fn everycr_can_insert_noalign_material() {
    let stores = run_boxed_alignment_source(
        "\\everycr{\\noalign{\\hrule height1pt}}\\halign{#\\cr a\\cr b\\cr}",
    );
    let vbox = box_zero_vlist(&stores);
    let rule_count = stores
        .nodes(vbox.children)
        .testing_decoded()
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
fn bare_cr_builds_an_empty_alignment_row() {
    let stores = run_boxed_alignment_source("\\halign{#\\cr\\cr}");
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);

    assert_eq!(rows.len(), 1);
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[0])[0]), "");
}

#[test]
fn valign_column_extent_includes_cell_depth() {
    let stores = run_alignment_source(
        "\\setbox0=\\hbox{\\valign{#\\cr \\vbox{\\hrule height20pt depth2pt}\\cr}}",
    );
    let root = stores.box_reg(0).expect("box0");
    let Some(Node::HList(hbox)) = stores.nodes(root).testing_decoded().first().cloned() else {
        panic!("box0 should contain an hbox");
    };
    let Some(Node::VList(cell)) = stores
        .nodes(hbox.children)
        .testing_decoded()
        .first()
        .cloned()
    else {
        panic!("valign should contain a vertical cell");
    };

    assert_eq!(cell.height.raw(), 22 * 65_536);
}

#[test]
fn fin_align_restores_saved_aux_instead_of_recomputing_it_from_set_nodes() {
    let mut stores = Universe::new();
    let mut nest = ModeNest::new();
    nest.push(Mode::InternalVertical);
    nest.current_list_mut().set_prev_depth(sp(1));

    crate::align::append_finished_alignment(
        &mut nest,
        &mut stores,
        crate::align::FinishedAlignment {
            nodes: vec![Node::Rule {
                width: Some(sp(3)),
                height: Some(sp(2)),
                depth: Some(Scaled::from_raw(0)),
            }],
            aux_prev_depth: Some(sp(7)),
        },
    );

    assert_eq!(
        nest.current_list().prev_depth(),
        Some(sp(7)),
        "fin_align must restore the alignment level's saved aux verbatim"
    );
    assert!(matches!(nest.current_list().nodes(), [Node::Rule { .. }]));
}

#[test]
fn valign_in_vertical_mode_starts_a_paragraph() {
    let stores = run_alignment_source("\\setbox0=\\vbox{\\valign{#\\cr \\cr}\\par}");
    let vbox = box_zero_vlist(&stores);
    let children = stores.nodes(vbox.children).testing_decoded();

    assert!(matches!(children, [Node::HList(_)]));
}

#[test]
fn display_halign_appends_display_vertical_material() {
    let stores = run_alignment_source(
        "\\setbox0=\\vbox{\\hsize=50pt \\predisplaypenalty=11 \\postdisplaypenalty=22 \
         \\abovedisplayskip=3pt \\belowdisplayskip=4pt \
         \\noindent$$\\halign{#\\cr a\\cr}$$\\par}",
    );
    let vbox = box_zero_vlist(&stores);
    let nodes = stores.nodes(vbox.children).testing_decoded();

    assert!(nodes.iter().any(|node| matches!(node, Node::Penalty(11))));
    assert!(nodes.iter().any(|node| matches!(node, Node::Penalty(22))));
    assert!(nodes.iter().any(|node| matches!(node, Node::Glue { .. })));
    assert!(nodes.iter().any(|node| matches!(node, Node::HList(_))));
}

#[test]
fn display_halign_exposes_enclosing_prevdepth_to_initial_everycr() {
    let stores = run_alignment_source(
        "\\dimen0=1pt \\setbox0=\\vbox{\\hsize=50pt \\noindent before\\par \
         $$\\everycr{\\noalign{\\global\\dimen0=\\prevdepth \
         \\global\\everycr={}}}\\halign{#\\cr x\\cr}$$\\par}",
    );

    assert_eq!(stores.dimen(0).raw(), 0);
}

#[test]
fn display_halign_closes_semisimple_group_and_discards_prior_formula() {
    let stores = run_alignment_source(
        "\\count0=1 \\setbox0=\\vbox{\\hsize=50pt \\noindent$$x\\begingroup\\count0=2 \\halign{#\\cr a\\cr}$$\\par}",
    );

    assert_eq!(stores.count(0), 1);
    let output = support::terminal_effect_text(&stores);
    assert!(output.contains("Missing \\endgroup inserted"));
    assert!(output.contains("Improper \\halign inside $$'s"));
}

#[test]
fn display_halign_runs_assignments_before_missing_closer_recovery() {
    let mut stores = support::stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new(
        "\\font\\f=cmr10 \\f \\hsize=50pt \\noindent$$\\halign{#\\cr a\\cr} \
         \\global\\count6=5 \\global\\postdisplaypenalty=-17 \
         \\global\\setbox= \\eqno \\end",
    ));

    let result = Executor::new().run(&mut input, &mut stores);

    assert_eq!(stores.count(6), 5);
    assert_eq!(stores.int_param(IntParam::POST_DISPLAY_PENALTY), -17);
    let stats = result.expect("display recovery should reach final cleanup");
    assert_eq!(stats.shipped_artifacts.len(), 1);
    assert_eq!(stores.execution_group_depth(), 0);
}

#[test]
fn nested_alignment_executes_inside_cell() {
    let stores = run_boxed_alignment_source("\\halign{#\\cr \\vbox{\\halign{#\\cr x\\cr}}\\cr}");
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);
    let cells = row_cells(&stores, rows[0]);

    assert_eq!(rows.len(), 1);
    assert_eq!(cells.len(), 1);
    assert!(
        stores
            .nodes(cells[0].children)
            .testing_decoded()
            .iter()
            .any(|node| matches!(node, Node::VList(_)))
    );
    assert_no_unset(&stores, stores.nodes(vbox.children).testing_decoded());
}

#[test]
fn nested_alignment_in_template_does_not_end_outer_preamble() {
    let (stores, state) = scan_halign_preamble("{\\vbox{\\halign{#\\cr x\\cr}}#\\cr}");
    let template = stores.tokens(state.columns()[0].v_template);

    assert_eq!(state.columns().len(), 1);
    assert_eq!(
        template
            .iter()
            .filter(|token| matches!(token, Token::Cs(symbol) if matches!(stores.meaning(*symbol), Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Cr))))
            .count(),
        2
    );
    assert_eq!(template.last(), Some(&Token::frozen_end_template()));
}

#[test]
fn alignment_cells_accept_all_fixed_infinite_glues_in_math_mode() {
    let stores = run_alignment_source(
        r"\font\sy=cmsy10 \font\ex=cmex10
          \textfont2=\sy \scriptfont2=\sy \scriptscriptfont2=\sy
          \textfont3=\ex \scriptfont3=\ex \scriptscriptfont3=\ex
          \setbox0=\vbox{\halign{$#$\cr \hfil\hfill\hss\hfilneg\cr}}",
    );
    let vbox = box_zero_vlist(&stores);
    let mut glue = Vec::new();
    collect_infinite_glue(
        &stores,
        stores.nodes(vbox.children).testing_decoded(),
        &mut glue,
    );

    assert_eq!(glue.len(), 4);
    assert_eq!(glue[0].stretch_order, Order::Fil);
    assert_eq!(glue[0].stretch.raw(), Scaled::UNITY);
    assert_eq!(glue[1].stretch_order, Order::Fill);
    assert_eq!(glue[1].stretch.raw(), Scaled::UNITY);
    assert_eq!(glue[2].stretch_order, Order::Fil);
    assert_eq!(glue[2].stretch.raw(), Scaled::UNITY);
    assert_eq!(glue[2].shrink_order, Order::Fil);
    assert_eq!(glue[2].shrink.raw(), Scaled::UNITY);
    assert_eq!(glue[3].stretch_order, Order::Fil);
    assert_eq!(glue[3].stretch.raw(), -Scaled::UNITY);
    assert_no_unset(&stores, stores.nodes(vbox.children).testing_decoded());
}

#[test]
fn plain_angle_style_alignment_restores_outer_cell_after_nested_leader_row() {
    let stores = run_boxed_alignment_source(
        "\\def\\angle{{\\vbox{\\halign{##\\cr x\\cr\\noalign{\\nointerlineskip}\\leaders\\hrule height.34pt\\hfill\\cr}}}}\\halign{#\\cr $\\angle$\\cr}",
    );
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);

    assert_eq!(rows.len(), 1);
    assert_eq!(row_cells(&stores, rows[0]).len(), 1);
    assert_no_unset(&stores, stores.nodes(vbox.children).testing_decoded());
}

#[test]
fn plain_angle_style_nested_alignment_executes_math_wrapped_leader_row() {
    let stores = run_alignment_source(
        "\\font\\sy=cmsy10 \\font\\ex=cmex10 \\textfont2=\\sy \\scriptfont2=\\sy \\scriptscriptfont2=\\sy \\textfont3=\\ex \\scriptfont3=\\ex \\scriptscriptfont3=\\ex \\def\\angle{{\\vbox{\\halign{$\\scriptstyle##$\\crcr x\\crcr\\noalign{\\nointerlineskip}\\mkern2.5mu\\leaders\\hrule height.34pt\\hfill\\mkern2.5mu\\crcr}}}}\\setbox0=\\vbox{\\halign{#\\cr $\\angle$\\cr}}",
    );
    let vbox = box_zero_vlist(&stores);

    assert!(contains_rule_leader(
        &stores,
        stores.nodes(vbox.children).testing_decoded(),
        GlueKind::Leaders,
        Scaled::from_raw(22_282),
    ));
    assert_no_unset(&stores, stores.nodes(vbox.children).testing_decoded());
}

#[test]
fn plain_angle_style_nested_alignment_replays_identically_after_rollback() {
    let mut stores = support::stores_with_fonts();
    let checkpoint = stores.snapshot();
    let source = "\\def\\angle{{\\vbox{\\halign{##\\cr x\\cr\\noalign{\\nointerlineskip}\\leaders\\hrule height.34pt\\hfill\\cr}}}}\\setbox0=\\vbox{\\halign{#\\cr $\\angle$\\cr}}";

    run_alignment_source_in(&mut stores, source);
    let first_hash = stores.snapshot().state_hash();

    stores.rollback(&checkpoint);
    run_alignment_source_in(&mut stores, source);

    assert_eq!(stores.snapshot().state_hash(), first_hash);
}

#[test]
fn valign_finishes_paragraph_cells_before_packaging() {
    let stores = run_alignment_source("\\setbox0=\\hbox{\\valign{#\\cr a\\cr b\\cr}}");
    let hbox = box_zero_hlist(&stores);
    let columns = hlist_vboxes(&stores, hbox);

    assert_eq!(columns.len(), 2);
    assert_eq!(columns[0].height, columns[1].height);
    assert!(
        stores
            .nodes(columns[0].children)
            .testing_decoded()
            .iter()
            .any(|node| matches!(node, Node::VList(_)))
    );
    assert_no_unset(&stores, stores.nodes(hbox.children).testing_decoded());
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

#[test]
fn noexpand_unexpandable_cr_terminates_alignment_row() {
    let mut stores = support::stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    run_alignment_source_in(
        &mut stores,
        "\\setbox0=\\vbox{\\halign{#\\cr x\\noexpand\\cr y\\cr}}",
    );
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);
    assert_eq!(rows.len(), 2);
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[0])[0]), "x");
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[1])[0]), "y");
}

#[test]
fn noexpand_preserves_unexpandable_cr_alias_but_suppresses_macro_alias() {
    let mut stores = support::stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    run_alignment_source_in(
        &mut stores,
        "\\def\\m{M}\\let\\endrow=\\cr \\setbox0=\\vbox{\\halign{#\\cr x\\noexpand\\m y\\noexpand\\endrow z\\cr}}",
    );
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);
    assert_eq!(rows.len(), 2);
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[0])[0]), "xy");
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[1])[0]), "z");
}

#[test]
fn noexpand_alignment_delivery_replays_identically_after_rollback() {
    let mut stores = support::stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let checkpoint = stores.snapshot();
    let source = "\\setbox0=\\vbox{\\halign{#\\cr x\\noexpand\\cr y\\cr}}";

    run_alignment_source_in(&mut stores, source);
    let first_hash = stores.snapshot().state_hash();
    stores.rollback(&checkpoint);
    run_alignment_source_in(&mut stores, source);

    assert_eq!(stores.snapshot().state_hash(), first_hash);
}

#[test]
fn empty_accent_group_preserves_later_alignment_delimiters() {
    let stores = run_alignment_source("\\setbox0=\\vbox{\\halign{#\\cr {\\accent18}\\cr X\\cr}}");

    assert!(stores.box_reg(0).is_some());
    let vbox = box_zero_vlist(&stores);
    assert_eq!(vlist_rows(&stores, vbox).len(), 2);
}

#[test]
fn trip_pathological_alignment_closes_before_following_material() {
    let mut stores = support::stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let before = stores.snapshot();
    let source = r#"
        \font\f=cmr10 \f \let\smalltrip=\f
        \def\t12#101001#{-.#1pt}
        \def\d#1\d{#1#1}
        \setbox3=\vtop{\vskip-3mm}
        \tabskip 1009.9sp minus .25cc
        \let\A=\relax \count1=2
        \halign spread-12.truedd{&#\span\iftrue\A\span\else\span\fi\span&
          \vbox{\halign to 0pt{\t2\dp3\A\crcr}#A}
          &\hss\tabskip1ex plus7200bp minus 4\wd4\d#\d\cr
          \global\let\t=\tabskip \spaceskip=4pt minus 1sp
          \def\A{B}\def\xx{\global\gdef\A{\global\count\count1=####\cr
            \omit\cr\tabskip}}\expandafter\xx\span
          A&\omit\valign to -5pt{#&#\cr A\char`}\span\cr{ }\span\cr}\cr
          \global\def\A{B}
          \lccode`Q=`b \span\omit$$\span\A&\show\cr\omit\cr
          \noalign{\global\prevdepth20pt}
          \omit\mark{a}&\omit\mark{b}\cr}
        \global\count7=123
    "#;
    let mut input = InputStack::new(MemoryInput::new(source));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("TRIP alignment and following sentinel execute");

    assert_eq!(stores.count(2), -1_118_806);
    assert_eq!(stores.count(7), 123, "execution must leave the alignment");
    assert!(
        stats.delivered_tokens < 10_000,
        "alignment made bounded progress"
    );
    assert!(
        input.summary().frames().is_empty(),
        "input stack fully retires"
    );
    assert!(
        stores.env_journal_bytes_since(&before) < 1_000_000,
        "alignment must not grow the state journal without bound"
    );
}

#[test]
fn trip_show_of_aliased_tab_recovers_and_closes_alignment() {
    let mut stores = support::stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let before = stores.snapshot();
    let source = r#"
        \font\f=cmr10 \f
        \long\def\l#1{}
        \halign to 1pt\expandafter{\csname#\endcsname#&#&\l{#}\cr
          \global\futurelet\endt\foo&\show\endt&$&&&.}
        \global\count7=321}
    "#;
    let mut input = InputStack::new(MemoryInput::new(source));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("aliased-tab show recovery and following sentinel execute");

    assert_eq!(stores.count(7), 321, "execution must leave the alignment");
    assert_eq!(
        stores.meaning(stores.symbol("endt").expect("futurelet target")),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate)
    );
    assert!(
        stats.delivered_tokens < 10_000,
        "recovery made bounded progress"
    );
    assert!(
        input.summary().frames().is_empty(),
        "input stack fully retires"
    );
    assert!(stores.env_journal_bytes_since(&before) < 1_000_000);
}

#[test]
fn malformed_template_row_closes_before_following_box() {
    let mut stores = support::stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let before = stores.snapshot();
    let source = r#"
        \font\f=cmr10 \f
        \long\def\l#1{}
        \let\PAR=\par \gdef\par{\relax\PAR}
        \halign to 1pt\expandafter{\csname#\endcsname#&#&\l{#}\cr
          \global\futurelet\endt\foo&\show\endt&$&&&.}
        \par
        \global\count7=\ifvmode1\else2\fi
        \hbox{Z}
        \cr}
    "#;
    let mut input = InputStack::new(MemoryInput::new(source));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("malformed row recovery should finish the alignment");

    assert_eq!(stores.count(7), 1, "following material is in vertical mode");
    assert!(support::terminal_effect_text(&stores).contains("Missing } inserted"));
    assert_eq!(stores.execution_group_depth(), 0);
    assert!(input.summary().frames().is_empty());
    assert!(stats.delivered_tokens < 1_000);
    assert!(stores.env_journal_bytes_since(&before) < 100_000);
}

#[test]
fn outer_macro_in_skipped_span_expansion_recovers_runaway_preamble() {
    let mut stores = support::stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let before = stores.snapshot();
    let source = r#"
        \outer\def\lo{}
        \halign{{\span\ifcase3 \lo#\cr............89{}\cr}
        \global\count7=456
    "#;
    let mut input = InputStack::new(MemoryInput::new(source));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("runaway preamble recovery should resume ordinary execution");

    assert_eq!(stores.count(7), 456, "tokens after recovery must execute");
    assert!(support::terminal_effect_text(&stores).contains("while scanning alignment preamble"));
    assert!(
        stats.delivered_tokens < 1_000,
        "recovery must make bounded progress"
    );
    assert!(input.summary().frames().is_empty());
    assert!(stores.env_journal_bytes_since(&before) < 100_000);
}

#[test]
fn trip_conditional_preamble_recovery_stops_before_following_input() {
    let mut stores = support::stores_with_fonts();
    tex_expand::install_expandable_primitives(&mut stores);
    let checkpoint = stores.snapshot();
    let source = r#"
        \setbox0=\hbox{}\copy0
        \everycr{\noalign{\penalty97}}
        \halign\relax{\span\iffalse}\fi\cr#&\ifnum0=`{\fi\cr\cr}
        \global\count7=777
    "#;
    let mut input = InputStack::new(MemoryInput::new(source));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("malformed conditional preamble should recover");

    assert_eq!(stores.count(7), 777, "following input must execute");
    assert_eq!(
        stores
            .current_page_nodes()
            .iter()
            .filter(|node| matches!(node, Node::Penalty(97)))
            .count(),
        3,
        "this line-420 reduction runs everycr initially and after two rows"
    );
    assert!(stats.delivered_tokens < 1_000);
    let first_hash = stores.snapshot().state_hash();

    stores.rollback(&checkpoint);
    let mut input = InputStack::new(MemoryInput::new(source));
    Executor::new()
        .run(&mut input, &mut stores)
        .expect("malformed conditional preamble replay should recover");
    assert_eq!(stores.snapshot().state_hash(), first_hash);
}
