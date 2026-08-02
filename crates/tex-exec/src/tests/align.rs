use super::*;
use tex_command::{
    AlignmentIdentity, CommandHostCapabilities, CommandHostContext, CommandObservation,
    CommandObserver, CommandProcessor, CommandRuntime, CommandSemanticDiagnostic, CommandState,
    FontResource, InputTransition, RecoveryKind, RegisteredSourceKind, ScannedPackingSpec,
    SourceRegistration,
};
use tex_state::env::banks::GlueParam;
use tex_state::glue::Order;
use tex_state::ids::GlueId;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::node::{BoxNode, GlueKind, Node, Sign, UnsetKind, UnsetNode, UnsetNodeFields};
use tex_state::scaled::Scaled;
use tex_state::{ExpansionState, InputOpenState};

fn scan_halign_preamble(source: &str) -> (Universe, AlignState) {
    let (stores, state, _) = scan_alignment_preamble(UnexpandablePrimitive::HAlign, source);
    (stores, state)
}

fn scan_valign_preamble(source: &str) -> (Universe, AlignState) {
    let (stores, state, _) = scan_alignment_preamble(UnexpandablePrimitive::VAlign, source);
    (stores, state)
}

fn scan_alignment_preamble(
    primitive: UnexpandablePrimitive,
    source: &str,
) -> (Universe, AlignState, Vec<CommandSemanticDiagnostic>) {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    install_unexpandable_primitives(&mut stores);
    let mut command = CommandState::default();
    let alignment = AlignmentIdentity::new(1);
    command.begin_alignment(alignment);
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            source.as_bytes().to_vec(),
        ))
        .expect("alignment preamble source should register");
    command
        .open_registered_source(source)
        .expect("alignment preamble source should open");
    let mut runtime = CommandRuntime::default();
    let mut capabilities = CommandHostCapabilities::default();
    let packing = {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            stores.command_context(),
            CommandHostContext::new(&mut capabilities),
        );
        let packing = processor
            .scan_alignment_preamble_opening()
            .expect("alignment packing specification should scan");
        processor
            .begin_alignment_preamble_scan(None)
            .expect("alignment preamble should scan");
        packing
    };
    let preamble = command
        .take_completed_alignment_preamble(alignment)
        .expect("alignment preamble should be frozen");
    let diagnostics = command.take_semantic_diagnostics();
    let kind = match primitive {
        UnexpandablePrimitive::HAlign => AlignmentKind::HAlign,
        UnexpandablePrimitive::VAlign => AlignmentKind::VAlign,
        _ => unreachable!("alignment helper requires an alignment primitive"),
    };
    let pack_spec = match packing {
        ScannedPackingSpec::Natural => AlignmentPackSpec::Natural,
        ScannedPackingSpec::Exactly(size) => AlignmentPackSpec::Exactly(size),
        ScannedPackingSpec::Spread(size) => AlignmentPackSpec::Spread(size),
    };
    let mut columns = Vec::with_capacity(preamble.columns.len());
    for templates in preamble.columns {
        let mut v_template = stores.tokens(templates.v_template.token_list()).to_vec();
        v_template.push(stores.frozen_end_template_token());
        columns.push(AlignColumn {
            u_template: templates
                .u_template
                .expect("canonical preamble columns retain u templates")
                .token_list(),
            v_template: stores.intern_token_list(&v_template),
        });
    }
    let tabskips = preamble
        .tabskips
        .into_iter()
        .map(|spec| stores.intern_glue(spec))
        .collect();
    let default_tabskip = stores.intern_glue(preamble.default_tabskip);
    let state = AlignState::new(
        kind,
        pack_spec,
        columns,
        tabskips,
        default_tabskip,
        preamble.repeat_start,
    );
    (stores, state, diagnostics)
}

fn char_token(ch: char, cat: Catcode) -> Token {
    Token::Char { ch, cat }
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
    let mut control = alignment_control(stores, source);
    loop {
        match control.step(stores).expect("alignment source executes") {
            MainControlStep::End | MainControlStep::EndOfInput => return,
            MainControlStep::Continue => {}
        }
    }
}

fn run_alignment_source_err(source: &str) -> String {
    let mut stores = support::stores_with_fonts();
    // The one helper here whose subject *is* the interactive path: extract
    // §1129's primary `\omit` diagnostic from canonical error reporting,
    // which the sibling nonstop-mode test contrasts with continued state.
    stores.set_interaction_mode(tex_state::InteractionMode::ErrorStop);
    let mut control = alignment_control(&mut stores, source);
    loop {
        match control.step(&mut stores) {
            Err(error) => return error.to_string(),
            Ok(MainControlStep::End | MainControlStep::EndOfInput) => {
                let output = support::terminal_effect_text(&stores);
                return output
                    .lines()
                    .find_map(|line| line.strip_prefix("! "))
                    .expect("alignment source should report an error")
                    .to_owned();
            }
            Ok(MainControlStep::Continue) => {}
        }
    }
}

fn run_boxed_alignment_source(source: &str) -> Universe {
    run_alignment_source(&format!("\\setbox0=\\vbox{{{source}}}"))
}

fn alignment_control(stores: &mut Universe, source: &str) -> CanonicalMainControl {
    let mut control = CanonicalMainControl::tex82_initex(stores);
    for name in [
        "cmr10.tfm",
        "cmmi10.tfm",
        "cmtt10.tfm",
        "cmsy10.tfm",
        "cmex10.tfm",
    ] {
        let metrics = tex_state::InputReadState::read_input_file(
            &mut stores.input_open_context(),
            std::path::Path::new(name),
        )
        .expect("seeded alignment font fixture reads through the world");
        control.capabilities_mut().register_font(
            name,
            FontResource::Tfm {
                metrics,
                opentype: None,
            },
        );
    }
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            format!("\\font\\f=cmr10 \\relax \\f {source}").into_bytes(),
        ))
        .expect("register alignment source");
    control
}

#[test]
fn stored_verbatim_delimiter_closes_inside_alignment_cell() {
    let stores = run_boxed_alignment_source(
        r"\let\bgroup={\let\egroup=}
          \def\sverb#1{\def\tempa##1#1{\leavevmode\null##1\egroup}\tempa}
          \def\verb{\relax\ifmmode\hbox\fi\bgroup\sverb}
          \def\author#1{\gdef\stored{#1}}
          \author{E-mail: \verb|{jennie,xianmo,yuliang}@example|}
          \halign{#\cr\stored\crcr}",
    );

    let output = support::terminal_effect_text(&stores);
    assert!(!output.contains("Misplaced \\crcr"), "{output}");
    assert!(!output.contains("Extra }, or forgotten $"), "{output}");
}

#[test]
fn lowercase_raw_text_closer_restores_alignment_brace_depth() {
    let stores = run_boxed_alignment_source(r"\halign{#\cr \lowercase{ABC}\cr}");

    let output = support::terminal_effect_text(&stores);
    assert!(!output.contains("Misplaced \\cr"), "{output}");
}

#[test]
fn recovered_token_assignment_brace_preserves_alignment_cell_boundaries() {
    let stores = run_boxed_alignment_source(r"\halign{#&#\cr \toks0=x}&y\cr}");
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));
    let cells = row_cells(&stores, rows[0]);

    assert_eq!(cells.len(), 2);
    assert_eq!(cell_text(&stores, cells[1]), "y");
    assert!(support::terminal_effect_text(&stores).contains("Missing { inserted"));
}

/// TeX82 §342 runs §789's ⟨v_j⟩ insertion at the tail of §341's `get_next`,
/// so it is transparent to every reader that pulls a raw command -- including
/// §392's macro parameter matcher. A macro argument may therefore open in a
/// ⟨u_j⟩ template and close on the right brace the ⟨v_j⟩ template supplies:
/// plain.tex's `\eqalignno` third column, `\llap{$\@lign##$}`, is exactly
/// that shape, and `$$\eqalignno{a &= b & (1)\cr}$$` is the smallest input
/// that reaches it.
///
/// When the matcher instead received the cell delimiter as ordinary argument
/// material, the delimiter's `\endv` was never delivered, the alignment never
/// advanced, and every remaining token of the job was absorbed into the
/// argument -- so the assignment after the alignment below never ran.
#[test]
fn macro_argument_opened_in_a_u_template_closes_on_the_v_template() {
    let stores = super::core::run_canonical_tex82(
        r"\def\lap#1{\hbox{#1}}
          \setbox0=\vbox{\halign{#&\lap{#}\cr a&b\cr}}
          \count7=42 \end",
    );

    assert_eq!(
        stores.count(7),
        42,
        "material after the alignment must still be executed"
    );
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));
    assert_eq!(rows.len(), 1, "the one `\\cr` row must be appended");
    let cells = row_cells(&stores, rows[0]);
    assert_eq!(cells.len(), 2, "both cells must be set");
    assert_eq!(
        row_cells(&stores, cells[1]).len(),
        1,
        "the second cell holds the \\lap hbox the v-template's brace closed"
    );
}

struct NestedShipoutObservation {
    checkpoints: Vec<EngineCheckpoint>,
    artifact_hashes: Vec<tex_state::ContentHash>,
}

fn nested_shipout_checkpoints(source: &str) -> NestedShipoutObservation {
    let mut stores = support::stores_with_fonts();
    let mut control = alignment_control(&mut stores, source);
    let mut checkpoints = vec![
        control
            .capture_checkpoint(
                EngineBoundary::JobStart,
                &mut stores,
                ExecutionBudgetCounters::default(),
            )
            .expect("nested shipout job-start checkpoint"),
    ];
    let mut pending_boundaries = Vec::new();
    loop {
        let step = control
            .step(&mut stores)
            .expect("nested shipout source executes");
        pending_boundaries.extend(control.take_completed_boundaries());
        while let Some(&boundary) = pending_boundaries.first() {
            let Ok(checkpoint) = control.capture_checkpoint(
                boundary,
                &mut stores,
                ExecutionBudgetCounters::default(),
            ) else {
                break;
            };
            checkpoints.push(checkpoint);
            pending_boundaries.remove(0);
        }
        if matches!(step, MainControlStep::End | MainControlStep::EndOfInput) {
            break;
        }
    }
    assert!(
        pending_boundaries.is_empty(),
        "every completed boundary becomes publishable after outer work unwinds: {pending_boundaries:?}"
    );
    let artifact_hashes = stores.world().artifact_commits().to_vec();
    assert_eq!(
        artifact_hashes.len(),
        1,
        "every committed nested shipout is surfaced to the output driver"
    );
    assert_eq!(
        stores.world().committed_artifacts()[0].hash(),
        artifact_hashes[0],
        "the committed artifact receipt identifies the published artifact"
    );
    NestedShipoutObservation {
        checkpoints,
        artifact_hashes,
    }
}

fn assert_nested_shipout_publishes_deterministic_outer_boundary(source: &str) {
    let first = nested_shipout_checkpoints(source);
    let second = nested_shipout_checkpoints(source);
    let boundaries = first
        .checkpoints
        .iter()
        .map(EngineCheckpoint::boundary)
        .collect::<Vec<_>>();
    assert_eq!(boundaries.first(), Some(&EngineBoundary::JobStart));
    assert_eq!(boundaries.last(), Some(&EngineBoundary::ShipoutComplete));
    assert_eq!(
        first
            .checkpoints
            .iter()
            .map(EngineCheckpoint::state_hash)
            .collect::<Vec<_>>(),
        second
            .checkpoints
            .iter()
            .map(EngineCheckpoint::state_hash)
            .collect::<Vec<_>>()
    );
    assert_eq!(first.artifact_hashes, second.artifact_hashes);
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
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let (control, observations, _) =
        observed_alignment_control(&mut stores, "x\\halign{#\\cr\\cr}\\end");
    let commands = observed_expanded_commands(&observations);
    let first = commands
        .iter()
        .position(|command| command.command == "halign")
        .unwrap();
    let par = commands[first + 1..]
        .iter()
        .find(|command| command.command == "par_end")
        .unwrap();
    let replayed = commands[first + 1..]
        .iter()
        .find(|command| command.command == "halign")
        .unwrap();

    assert_eq!(par.command, "par_end");
    assert_eq!(
        replayed.provenance.origin,
        commands[first].provenance.origin
    );
    assert_eq!(control.active_alignment(), None);
}

#[test]
fn hrule_head_for_vmode_defers_rule_until_after_paragraph_dispatch() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let (_, observations, _) = observed_alignment_control(&mut stores, "x\\hrule\\end");
    let commands = observed_expanded_commands(&observations);
    let first = commands
        .iter()
        .position(|command| command.command == "hrule")
        .unwrap();
    let par = commands[first + 1..]
        .iter()
        .position(|command| command.command == "par_end")
        .unwrap();
    let replayed = commands[first + 1..]
        .iter()
        .position(|command| command.command == "hrule")
        .unwrap();

    assert!(
        par < replayed,
        "paragraph must dispatch before the replayed rule"
    );
    assert_eq!(
        commands[first].provenance.origin,
        commands[first + 1 + replayed].provenance.origin
    );
}

#[test]
fn halign_in_restricted_horizontal_mode_with_open_group_retains_off_save_recovery() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let (_, observations, _) =
        observed_alignment_control(&mut stores, "\\setbox0=\\hbox{\\halign{#\\cr\\cr}\\end");
    let commands = observed_expanded_commands(&observations);
    let first = commands
        .iter()
        .position(|command| command.command == "halign")
        .unwrap();
    let closer = commands[first + 1..]
        .iter()
        .position(|command| command.command == "right_brace")
        .unwrap();
    let replayed = commands[first + 1..]
        .iter()
        .position(|command| command.command == "halign")
        .unwrap();

    assert!(
        closer < replayed,
        "off_save closer must precede command replay"
    );
    assert_eq!(
        commands[first].provenance.origin,
        commands[first + 1 + replayed].provenance.origin
    );
    assert!(observations.iter().any(|observation| matches!(observation,
        CommandObservation::Recovery(recovery) if recovery.kind == RecoveryKind::InsertedToken)));
    assert!(support::terminal_effect_text(&stores).contains("Missing } inserted"));
}

#[test]
fn bottom_level_halign_recovery_drops_command_without_growing_input_frames() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control.testing_push_mode(Mode::RestrictedHorizontal);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            b"\\halign".to_vec(),
        ))
        .expect("register recovery source");
    let mut observations = AlignmentObservationRecorder::default();
    let mut maximum_depth = control.input_level_count();
    for _ in 0..4 {
        let step = control
            .step_with_observer(&mut stores, &mut observations)
            .expect("recovery step");
        maximum_depth = maximum_depth.max(control.input_level_count());
        if matches!(step, MainControlStep::End | MainControlStep::EndOfInput) {
            break;
        }
    }

    assert_eq!(
        observed_expanded_commands(&observations.0)
            .iter()
            .filter(|command| command.command == "halign")
            .count(),
        1
    );
    assert!(
        maximum_depth <= 1,
        "recovery must not retain inserted frames"
    );
    assert!(
        !observations
            .0
            .iter()
            .any(|observation| matches!(observation,
        CommandObservation::Input(input) if input.transition == InputTransition::Recovery))
    );
    assert!(support::terminal_effect_text(&stores).contains("Extra \\halign"));
}

#[derive(Default)]
struct AlignmentObservationRecorder(Vec<CommandObservation>);

impl CommandObserver for AlignmentObservationRecorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

fn observed_alignment_control(
    stores: &mut Universe,
    source: &str,
) -> (CanonicalMainControl, Vec<CommandObservation>, usize) {
    let mut control = CanonicalMainControl::tex82_initex(stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            source.as_bytes().to_vec(),
        ))
        .expect("register observed alignment source");
    let mut observations = AlignmentObservationRecorder::default();
    let mut maximum_depth = control.input_level_count();
    loop {
        let step = control
            .step_with_observer(stores, &mut observations)
            .expect("alignment source executes");
        maximum_depth = maximum_depth.max(control.input_level_count());
        if matches!(step, MainControlStep::End | MainControlStep::EndOfInput) {
            break;
        }
    }
    (control, observations.0, maximum_depth)
}

fn observed_expanded_commands(
    observations: &[CommandObservation],
) -> Vec<&tex_command::CommandDeliveryRecord> {
    observations
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Command(command)
                if command.boundary == tex_command::CommandDeliveryBoundary::Expanded =>
            {
                Some(command)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn math_group_scanned_inside_cell_does_not_hide_row_terminator() {
    let stores = run_boxed_alignment_source("\\halign{#\\cr ${}^1$\\cr}");
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));

    assert_eq!(rows.len(), 1);
    assert_eq!(row_cells(&stores, rows[0]).len(), 1);
}

#[test]
fn end_template_closes_unterminated_math_before_packaging_cell() {
    let stores = run_boxed_alignment_source("\\halign{#\\cr $x\\cr}");
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));

    assert_eq!(rows.len(), 1);
    assert_eq!(row_cells(&stores, rows[0]).len(), 1);
    assert!(support::terminal_effect_text(&stores).contains("Missing $ inserted"));
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
fn macro_trailing_space_precedes_an_alignment_row_terminator() {
    let stores = run_boxed_alignment_source(
        "\\def\\address#1{\\def\\entry{#1}}\\address{\n A\n}\\halign{#\\cr\\ignorespaces\\entry\\cr}",
    );
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));
    let cell = row_cells(&stores, rows[0])[0];
    let finite_spaces = stores
        .nodes(cell.children)
        .testing_decoded()
        .iter()
        .filter(|node| {
            matches!(
                node,
                Node::Glue { spec, kind: GlueKind::Normal, .. }
                    if stores.glue(*spec).stretch_order == Order::Normal
            )
        })
        .count();

    assert_eq!(cell_text(&stores, cell), "A");
    assert_eq!(finite_spaces, 1);
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
            stores.frozen_end_template_token()
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
    let stores = run_boxed_alignment_source(
        "\\halign{#&#\\cr \\futurelet\\x\\missing&a\\cr}",
    );
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));
    let cells = row_cells(&stores, rows[0]);

    assert_eq!(rows.len(), 1);
    assert_eq!(cells.len(), 2);
    assert_eq!(cell_text(&stores, cells[0]), "");
    assert_eq!(cell_text(&stores, cells[1]), "a");

    // TeX82 §370 puts the offending name in §82's context display, not in
    // the message text.
    let text = support::terminal_effect_text(&stores);
    let diagnostic = text
        .find("! Undefined control sequence.")
        .expect("futurelet lookahead should report the undefined command");
    let context = text
        .find("\\missing")
        .expect("the undefined command should appear in the context display");
    assert!(diagnostic < context, "{text}");
    assert!(!text.contains("Misplaced alignment tab character &"), "{text}");
}

#[test]
fn expanded_definition_keeps_alignment_tabs_inside_its_braces() {
    let stores = run_boxed_alignment_source(
        r"\def\expandedtab{&}\halign{#\cr \xdef\saved{a\expandedtab b}\cr}",
    );
    let saved = stores.symbol("saved").expect("xdef installs saved");
    let Meaning::Macro { definition, .. } = stores.meaning(saved) else {
        panic!("saved should be a macro");
    };
    let replacement = stores.macro_definition(definition).replacement_text();

    assert_eq!(
        stores.tokens(replacement),
        &[
            char_token('a', Catcode::Letter),
            char_token('&', Catcode::AlignmentTab),
            char_token('b', Catcode::Letter),
        ]
    );
}

#[test]
fn futurelet_brace_lookahead_restores_alignment_depth_before_replay() {
    let stores = run_boxed_alignment_source(
        r"\def\consume#1{\global\count0=7}
          \halign{#\cr \futurelet\next\consume{X}\cr}",
    );

    assert_eq!(stores.count(0), 7);
    let output = support::terminal_effect_text(&stores);
    assert!(!output.contains("Misplaced \\cr"), "{output}");
}

#[test]
fn extra_alignment_tab_is_changed_to_row_terminator() {
    let stores = run_boxed_alignment_source("\\halign{#\\cr a&b\\cr}");
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));
    let output = support::terminal_effect_text(&stores);

    assert!(
        output.contains("Extra alignment tab has been changed to \\cr"),
        "{output}"
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[0])[0]), "a");
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[1])[0]), "b");
}

#[test]
fn extra_span_is_changed_to_row_terminator() {
    let stores = run_boxed_alignment_source("\\halign{#\\cr a\\span\\cr}");
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));
    let output = support::terminal_effect_text(&stores);

    assert!(
        output.contains("Extra alignment tab has been changed to \\cr"),
        "{output}"
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[0])[0]), "a");
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[1])[0]), "");
}

#[test]
fn trip_missing_cr_recovery_does_not_start_a_third_row() {
    let stores = run_boxed_alignment_source(
        "\\long\\def\\l#1{}\\let\\PAR=\\par\\def\\par{\\relax\\PAR}\
         \\halign{#&#&\\l{#}\\cr a&b&c&&&.}\n\\par\\cr}",
    );
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));

    assert_eq!(rows.len(), 2);
    let first = row_cells(&stores, rows[0]);
    let second = row_cells(&stores, rows[1]);
    assert_eq!(first.len(), 3);
    assert_eq!(cell_text(&stores, first[0]), "a");
    assert_eq!(cell_text(&stores, first[1]), "b");
    assert_eq!(cell_text(&stores, first[2]), "");
    assert_eq!(second.len(), 3);
    assert_eq!(cell_text(&stores, second[0]), "");
    assert_eq!(cell_text(&stores, second[1]), "");
    assert_eq!(cell_text(&stores, second[2]), "");
}

#[test]
fn continuing_column_restores_sentinel_before_u_template_final_brace() {
    let stores = run_boxed_alignment_source("\\def\\l#1{#1}\\halign{#&\\l{#}\\cr a&x\\cr}");
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));

    assert_eq!(rows.len(), 1);
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[0])[0]), "a");
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[0])[1]), "x");
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
fn hash_brace_macro_delimiter_preserves_alignment_cell_boundary() {
    let stores = run_boxed_alignment_source(
        "\\def\\dispatch{\\def\\next@##1##{\\finish{##1}}\\next@}\
         \\def\\finish#1#2#3!@{\\global\\count7=123\\hbox{#2}}\
         \\halign{\\dispatch#!@\\cr M{X}tail&\\cr N{Y}tail&\\cr}",
    );
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));
    let first_cells = row_cells(&stores, rows[0]);
    let second_cells = row_cells(&stores, rows[1]);
    let [Node::HList(first)] = stores.nodes(first_cells[0].children).testing_decoded() else {
        panic!("cell should contain the box built after the delimiter match");
    };
    let [Node::HList(second)] = stores.nodes(second_cells[0].children).testing_decoded() else {
        panic!("second cell should contain the box built after group restoration");
    };

    assert_eq!(rows.len(), 2);
    assert_eq!(cell_text(&stores, first), "X");
    assert_eq!(cell_text(&stores, second), "Y");
    assert_eq!(stores.count(7), 123, "global cell assignment survives");
    assert_eq!(
        stores.meaning(stores.symbol("let").expect("installed primitive")),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Let),
        "per-cell align-group restoration must not alter unrelated meanings"
    );
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
            stores.frozen_end_template_token()
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
fn preamble_recognizes_parameter_character_alias_by_meaning() {
    let stores = run_boxed_alignment_source("\\let\\sharp=#\\halign{u\\sharp v\\cr x\\cr}");
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));
    let cells = row_cells(&stores, rows[0]);

    assert_eq!(cells.len(), 1);
    assert_eq!(cell_text(&stores, cells[0]), "uxv");
    assert!(!support::terminal_effect_text(&stores).contains("Missing # inserted"));
}

#[test]
fn preamble_recognizes_alignment_tab_alias_by_meaning() {
    let stores = run_boxed_alignment_source("\\let\\tab=&\\halign{#\\tab#\\cr a&b\\cr}");
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));
    let cells = row_cells(&stores, rows[0]);

    assert_eq!(cells.len(), 2);
    assert_eq!(cell_text(&stores, cells[0]), "a");
    assert_eq!(cell_text(&stores, cells[1]), "b");
}

#[test]
fn alignment_brace_depth_ignores_control_sequence_aliases() {
    let stores = run_boxed_alignment_source(
        "\\let\\bgroup={\\let\\egroup=}\\halign{$\\displaystyle{#}$\\cr \\mathop\\bgroup\\let\\close\\egroup x\\close\\cr y\\cr}",
    );
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));

    assert_eq!(rows.len(), 2);
}

#[test]
fn alignment_brace_depth_survives_grouped_char_material() {
    let stores = run_boxed_alignment_source(
        "\\let\\bgroup={\\let\\egroup=}\\def\\symbol{\\bgroup\\char36\\egroup}\\halign{#\\cr \\symbol\\cr y\\cr}",
    );
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));

    assert_eq!(rows.len(), 2);
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[1])[0]), "y");
}

#[test]
fn expanded_text_scanning_preserves_alignment_brace_depth() {
    let stores = run_boxed_alignment_source("\\halign{#\\cr x\\expanded{}\\cr}");
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));

    assert_eq!(rows.len(), 1);
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[0])[0]), "x");
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
            stores.frozen_end_template_token()
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
        &[stores.frozen_end_template_token()]
    );
}

#[test]
fn alignment_preamble_errors_match_reference_wording() {
    let (_, _, diagnostics) = scan_alignment_preamble(UnexpandablePrimitive::HAlign, "{abc\\cr}");
    assert!(diagnostics.iter().any(|diagnostic| matches!(
        diagnostic,
        CommandSemanticDiagnostic::Recoverable { message, .. }
            if message.contains("Missing # inserted in alignment preamble")
    )));

    let (stores, state, diagnostics) =
        scan_alignment_preamble(UnexpandablePrimitive::HAlign, "{#a#b\\cr}");
    assert!(diagnostics.iter().any(|diagnostic| matches!(
        diagnostic,
        CommandSemanticDiagnostic::Recoverable { message, .. }
            if message.contains("Only one # is allowed per tab")
    )));
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
            stores.frozen_end_template_token(),
        ]
    );
}

#[test]
fn mid_alignment_snapshot_rollback_restores_summary_and_unset_rows() {
    let (mut stores, state) = scan_halign_preamble("{#&#\\cr}");
    let mut command = CommandState::default();
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            b"b&c\\cr}".to_vec(),
        ))
        .expect("alignment continuation source should register");
    command
        .open_registered_source(source)
        .expect("alignment continuation source should open");
    let command_summary = command
        .publish_summary()
        .expect("alignment continuation should be quiescent");
    let command_snapshot = command.snapshot();
    let mut nest = ModeNest::new();
    nest.push(Mode::InternalVertical).expect("test mode push");
    nest.current_list_mutation().set_align_state(state);

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
        let mut list = nest.current_list_mutation();
        list.push(row);
        list.with_align_state_mut(|state| {
            state.start_row();
            state.start_cell(1, 2);
            state.set_suppress_redundant_cr(true);
        })
        .expect("alignment state");
    }
    let snapshot = stores.snapshot();
    let summary = nest.summary();

    let _temporary = stores.freeze_node_list(&[Node::Penalty(99)]);
    command
        .load_next_source_line(-1)
        .expect("temporary command state should read the continuation");
    assert!(command.next_source_character().is_some());
    {
        let mut list = nest.current_list_mutation();
        list.push(Node::Penalty(123));
        list.with_align_state_mut(|state| state.start_cell(0, 1))
            .expect("alignment state");
    }

    stores.rollback(&snapshot);
    command
        .rollback(command_snapshot)
        .expect("canonical command snapshot should restore");
    let restored = ModeNest::from_summary(summary.clone()).expect("restored alignment summary");

    assert_eq!(
        command
            .publish_summary()
            .expect("restored command state should remain quiescent"),
        command_summary
    );
    assert_eq!(restored.summary(), summary);
    let restored_state = restored
        .current_list()
        .align_state()
        .expect("restored alignment state");
    assert_eq!(restored_state.current_col(), 1);
    assert_eq!(restored_state.current_span(), 2);
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
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let unset = unset_for_test(&mut stores, UnsetKind::HBox, &[], 1);
    let state_before = stores.testing_state_hash();
    let nodes_before = stores.testing_epoch_node_count();
    let effects_before = stores.world().effect_records().to_vec();
    let err = crate::canonical_main_control::test_shipout_replay_box(unset, &mut stores)
        .expect_err("unset alignment node must not lower to shipout artifact");

    assert!(matches!(
        err,
        crate::ExecError::UnsupportedShipoutNode {
            node: "unset alignment"
        }
    ));
    assert_eq!(stores.testing_state_hash(), state_before);
    assert_eq!(stores.testing_epoch_node_count(), nodes_before);
    assert_eq!(stores.world().effect_records(), effects_before);
    assert!(stores.world().artifact_commits().is_empty());
}

#[test]
fn box_group_shipout_publishes_only_after_outer_unwind() {
    assert_nested_shipout_publishes_deterministic_outer_boundary(
        "\\setbox0=\\hbox{\\shipout\\hbox{A}B}\\end",
    );
}

#[test]
fn alignment_shipout_publishes_only_after_outer_unwind() {
    assert_nested_shipout_publishes_deterministic_outer_boundary(
        "\\setbox0=\\vbox{\\halign{#\\cr \\shipout\\hbox{A}x\\cr}}\\end",
    );
}

#[test]
fn effectful_box_group_shipout_publishes_restartable_boundary() {
    assert_nested_shipout_publishes_deterministic_outer_boundary(
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
fn let_aliased_frozen_endv_finishes_cell_through_do_endv() {
    let stores = run_boxed_alignment_source(
        "\\def\\capture{\\afterassignment\\execute\\let\\endt=}\\def\\execute{\\endt}\\halign{#\\cr x\\capture\\cr}\\global\\count0=37",
    );
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);

    assert_eq!(rows.len(), 1);
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[0])[0]), "x");
    assert_eq!(stores.count(0), 37, "execution continued after the alignment");
}

#[test]
fn futurelet_aliased_frozen_endv_recovers_intervening_group_before_do_endv() {
    let stores = run_boxed_alignment_source(
        "\\def\\capture{\\futurelet\\endt\\consume}\\def\\consume{\\begingroup\\afterassignment\\execute\\let\\scratch=}\\def\\execute{\\endt}\\halign{#\\cr x\\capture\\cr}",
    );
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);

    assert_eq!(rows.len(), 1);
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[0])[0]), "x");
    assert!(support::terminal_effect_text(&stores).contains("Missing \\endgroup inserted"));
}

#[test]
fn aliased_endv_recovery_continues_through_shifted_void_box() {
    let stores = run_boxed_alignment_source(
        "\\def\\capture{\\futurelet\\endt\\consume}\\def\\consume{\\begingroup\\afterassignment\\execute\\let\\scratch=}\\def\\execute{\\endt}\\halign{#\\cr x\\capture\\cr}\\lower1pt\\box1\\global\\count0=7",
    );
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);

    assert_eq!(rows.len(), 1);
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[0])[0]), "x");
    assert_eq!(stores.count(0), 7, "void box must not stop continuation");
    assert!(support::terminal_effect_text(&stores).contains("Missing \\endgroup inserted"));
}

#[test]
fn aliased_frozen_endv_alignment_replays_identically_after_rollback() {
    let mut stores = support::stores_with_fonts();
    let checkpoint = stores.snapshot();
    let source = "\\def\\capture{\\futurelet\\endt\\consume}\\def\\consume{\\afterassignment\\execute\\let\\scratch=}\\def\\execute{\\endt}\\setbox0=\\vbox{\\halign{#\\cr x\\capture\\cr}}";

    run_alignment_source_in(&mut stores, source);
    let first_hash = stores.snapshot().state_hash();

    stores.rollback(&checkpoint);
    run_alignment_source_in(&mut stores, source);

    assert_eq!(stores.snapshot().state_hash(), first_hash);
}

#[test]
fn frozen_endv_recovers_open_box_groups_before_finishing_cell() {
    let stores = run_boxed_alignment_source(
        "\\let\\bgroup={\\let\\egroup=}\\def\\open{\\hbox\\bgroup\\begingroup\\bgroup}\\halign{\\open#\\cr x\\cr}",
    );
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);
    let cells = row_cells(&stores, rows[0]);

    assert_eq!(rows.len(), 1);
    assert_eq!(cells.len(), 1);
    let output = support::terminal_effect_text(&stores);
    assert!(output.contains("Missing \\endgroup inserted"), "{output}");
    assert!(output.contains("Missing } inserted"), "{output}");
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
fn misplaced_omit_in_nonstop_alignment_reports_and_continues() {
    let mut stores = support::stores_with_fonts();
    stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);

    run_alignment_source_in(
        &mut stores,
        "\\setbox0=\\vbox{\\halign{#\\cr a \\omit b\\cr}}",
    );

    assert!(stores.box_reg(0).is_some());
    assert!(support::terminal_effect_text(&stores).contains("! Misplaced \\omit."));
}

#[test]
fn misplaced_noalign_in_cell_reports_and_continues_in_error_stop_mode() {
    let stores = run_boxed_alignment_source("\\halign{#\\cr a \\noalign{ignored}b\\cr}");
    let rows = vlist_rows(&stores, box_zero_vlist(&stores));

    assert_eq!(rows.len(), 1);
    assert!(support::terminal_effect_text(&stores).contains("! Misplaced \\noalign."));
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
    tex_command::install_tex82_expandable_primitives(&mut stores);
    run_alignment_source_in(
        &mut stores,
        "\\setbox0=\\vbox{\\count1=2 \\def\\xx{\\global\\gdef\\A{\\global\\count\\count1=-17\\cr\\omit\\cr\\tabskip}}\\halign{#&\\A#\\cr \\expandafter\\xx\\span A&x\\cr}}",
    );

    assert_eq!(stores.count(2), -17);
}

#[test]
fn expandafter_intercepts_span_before_replaying_saved_macro() {
    let mut stores = support::stores_with_fonts();
    tex_command::install_tex82_expandable_primitives(&mut stores);
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
fn noalign_backtick_brace_keeps_local_meaning_until_balancing_idiom() {
    let mut stores = support::stores_with_fonts();
    tex_command::install_tex82_expandable_primitives(&mut stores);
    run_alignment_source_in(
        &mut stores,
        r"\let\normal\relax
          \def\rule{\ifx\longtable\undefined
              \let\switch\normal
            \else\ifx\hline\LThline
              \let\switch\normal
            \else
              \let\switch\normal
            \fi\fi
            \switch}
          \setbox0=\vbox{\halign{#\cr a\cr
          \noalign{\ifnum0=`}\fi
            \rule
          \ifnum0=`{\fi}
          b\cr}}",
    );

    let output = support::terminal_effect_text(&stores);
    assert!(!output.contains("Undefined control sequence"), "{output}");
    assert_eq!(vlist_rows(&stores, box_zero_vlist(&stores)).len(), 2);
    assert_eq!(
        stores.meaning(stores.symbol("switch").expect("switch symbol")),
        Meaning::Undefined,
        "the local switch definition must restore after noalign exits"
    );
}

#[test]
fn booktabs_rules_stay_structural_after_rows_with_unclosed_brace_alias_groups() {
    let stores = run_boxed_alignment_source(
        r"\let\bgroup={
          \def\complexrow#1{\bgroup\hbox{#1}\cr}
          \def\toprule{\noalign{\hrule height1pt}}
          \def\midrule{\noalign{\hrule height2pt}}
          \def\bottomrule{\noalign{\hrule height3pt}}
          \halign{#\cr
            \complexrow{header}
            \toprule
            \complexrow{body}
            \midrule
            \complexrow{footer}
            \bottomrule}",
    );

    let output = support::terminal_effect_text(&stores);
    assert!(!output.contains("Misplaced \\noalign"), "{output}");
    let nodes = stores
        .nodes(box_zero_vlist(&stores).children)
        .testing_decoded();
    assert_eq!(
        nodes
            .iter()
            .filter(|node| matches!(node, Node::HList(_)))
            .count(),
        3
    );
    assert_eq!(
        nodes
            .iter()
            .filter(|node| matches!(node, Node::Rule { .. }))
            .count(),
        3
    );
}

#[test]
fn noalign_ignored_prevdepth_suppresses_next_row_baseline_glue() {
    let stores = run_boxed_alignment_source(
        "\\baselineskip=20pt \\halign{#\\cr a\\cr\\noalign{\\prevdepth-1000pt}b\\cr}",
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
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut nest = ModeNest::new();
    nest.push(Mode::InternalVertical).expect("test mode push");
    nest.current_list_mutation().set_prev_depth(sp(1));

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
            aux_space_factor: None,
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
fn display_halign_splices_rows_instead_of_math_packing_them() {
    let stores = super::core::run_canonical_tex82(
        r"\setbox0=\vbox{\hsize=50pt
          \baselineskip=15pt \lineskip=4pt \lineskiplimit=3pt
          \noindent\vrule height0pt depth2pt width0pt\par
          $$\halign{#\cr
            \vrule height7.5pt depth2.5pt width0pt\cr
            \vrule height6pt depth2pt width0pt\cr
            \vrule height6pt depth2pt width0pt\cr}$$}\end",
    );
    let vbox = box_zero_vlist(&stores);
    let nodes = stores.nodes(vbox.children).testing_decoded();
    let display_rows = nodes
        .iter()
        .filter(
            |node| matches!(node, Node::HList(row) if row.box_lr == tex_state::node::BoxLr::DList),
        )
        .count();

    assert_eq!(
        display_rows, 3,
        "TeX82 §812 splices each finished alignment row into the display: {nodes:?}"
    );
    assert!(
        nodes
            .iter()
            .filter(|node| {
                matches!(
                    node,
                    Node::Glue {
                        kind: GlueKind::BaselineSkip | GlueKind::LineSkip,
                        ..
                    }
                )
            })
            .count()
            >= 3,
        "§799 row spacing must survive §812 display insertion: {nodes:?}"
    );
}

#[test]
fn display_halign_carries_last_row_depth_into_following_baseline_glue() {
    let stores = run_alignment_source(
        "\\setbox0=\\vbox{\\hsize=50pt \\baselineskip=12pt \\lineskiplimit=-100pt \
         \\noindent before $$\\halign{#\\cr \\vrule height7.5pt depth2.5pt width0pt\\cr}$$after\\par}",
    );
    let vbox = box_zero_vlist(&stores);
    let nodes = stores.nodes(vbox.children).testing_decoded();
    let below = nodes
        .iter()
        .position(|node| {
            matches!(
                node,
                Node::Glue {
                    kind: GlueKind::BelowDisplaySkip,
                    ..
                }
            )
        })
        .expect("below-display glue");
    let display_depth = nodes[..below]
        .iter()
        .rev()
        .find_map(|node| match node {
            Node::HList(row) => Some(row.depth),
            _ => None,
        })
        .expect("display alignment row");
    let (baseline, following_height) = nodes[below + 1..]
        .windows(2)
        .find_map(|pair| match pair {
            [
                Node::Glue {
                    spec,
                    kind: GlueKind::BaselineSkip,
                    ..
                },
                Node::HList(line),
            ] => Some((stores.glue(*spec).width, line.height)),
            _ => None,
        })
        .expect("baseline glue before following paragraph line");

    assert_eq!(display_depth, sp(2) + Scaled::from_raw(Scaled::UNITY / 2));
    assert_eq!(baseline, sp(12) - display_depth - following_height);
}

#[test]
fn display_halign_exposes_enclosing_prevdepth_to_initial_everycr() {
    let stores = super::core::run_canonical_tex82(
        "\\dimen0=1pt \\setbox0=\\vbox{\\hsize=50pt \\noindent\\vrule depth2pt \
         $$\\everycr{\\noalign{\\global\\dimen0=\\prevdepth \
         \\global\\everycr={}}}\\halign{#\\cr\\cr}$$}\\end",
    );

    assert_eq!(stores.dimen(0), sp(2));
}

#[test]
fn vertical_halign_keeps_current_prevdepth_for_initial_everycr() {
    let stores = super::core::run_canonical_tex82(
        "\\dimen0=1pt \\setbox0=\\vbox{\\prevdepth=3pt \
         \\everycr{\\noalign{\\global\\dimen0=\\prevdepth \
         \\global\\everycr={}}}\\halign{#\\cr\\cr}}\\end",
    );

    assert_eq!(stores.dimen(0), sp(3));
}

#[test]
fn display_halign_closes_semisimple_group_and_discards_prior_formula() {
    let stores = run_alignment_source(
        "\\count0=1 \\setbox0=\\vbox{\\hsize=50pt \\noindent$$x\\begingroup\\count0=2 \\halign{#\\cr a\\cr}$$\\par}",
    );

    assert_eq!(stores.count(0), 1);
    let output = support::terminal_effect_text(&stores);
    assert!(output.contains("! Missing \\endgroup inserted."));
    assert!(output.contains("! Improper \\halign inside $$'s."));
    assert!(
        output.contains("Displays can use special alignments (like \\eqalignno)"),
        "TeX82 §774's display-material rejection keeps its canonical help text"
    );
}

#[test]
fn display_halign_runs_assignments_before_missing_closer_recovery() {
    let stores = super::core::run_canonical_tex82_with_fonts(
        "\\font\\f=cmr10 \\f \\hsize=50pt \\noindent$$\\halign{#\\cr a\\cr} \
         \\global\\count6=5 \\global\\postdisplaypenalty=-17 \
         \\global\\setbox= \\eqno \\end",
    );

    assert_eq!(stores.count(6), 5);
    assert_eq!(stores.int_param(IntParam::POST_DISPLAY_PENALTY), -17);
    assert_eq!(stores.world().artifact_commits().len(), 1);
    assert_eq!(stores.execution_group_depth(), 0);
}

#[test]
fn display_halign_missing_closer_backs_up_eqno_before_retry() {
    let stores =
        super::core::run_canonical_tex82("\\nonstopmode\\noindent$$\\halign{#\\cr\\cr}\\eqno\\end");
    let output = support::terminal_effect_text(&stores);
    let missing = output
        .find("! Missing $$ inserted.")
        .expect("§1207 missing-closer diagnostic");
    let replay = output[missing..]
        .find("<to be read again>")
        .map(|offset| missing + offset)
        .expect("rejected equation-number token is backed up before error context");
    let replayed_eqno = output[replay..]
        .find("\\eqno")
        .map(|offset| replay + offset)
        .expect("backup context names the rejected equation-number token");
    let retry = output[replayed_eqno..]
        .find("You can't use `\\eqno' in horizontal mode")
        .map(|offset| replayed_eqno + offset)
        .expect("ordinary main control retries eqno after finishing the display");

    assert!(
        missing < replay && replay < replayed_eqno && replayed_eqno < retry,
        "TeX82 §1207 requires back_error context before display finish and retry: {output}"
    );
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
fn expanded_definition_brace_delta_preserves_outer_box_closer() {
    // LaTeX3's alignment-safe groups use braces skipped by false
    // conditionals around expansion work. The first skipped brace occurs
    // inside an expanded definition; its matching skipped brace follows a
    // makecell-shaped nested vcenter/alignment.
    let mut stores = support::stores_with_fonts();
    tex_command::install_tex82_expandable_primitives(&mut stores);
    run_alignment_source_in(
        &mut stores,
        "\\setbox0=\\vbox{\
         \\halign{#&#\\cr\
         \\edef\\saved{\\iffalse{\\fi}\
         \\hbox{$\\vcenter{\\halign{##\\cr x\\cr}}$}\
         \\iffalse}\\fi&y\\cr}}\
         \\global\\count7=123",
    );
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);

    assert_eq!(rows.len(), 1);
    assert_eq!(row_cells(&stores, rows[0]).len(), 2);
    assert_eq!(
        stores.count(7),
        123,
        "the real vbox closer must return control to outer main control"
    );
    assert_no_unset(&stores, stores.nodes(vbox.children).testing_decoded());
}

#[test]
fn token_parameter_assignment_before_nested_alignment_preserves_outer_brace_depth() {
    let stores = run_boxed_alignment_source(
        "\\def\\ialign{\\everycr{}\\tabskip=0pt \\halign}\\def\\inner{{\\vtop{\\ialign{##\\cr x\\cr}}}}\\halign{#\\cr \\inner\\cr y\\cr}",
    );
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);

    assert_eq!(rows.len(), 2);
    assert_eq!(cell_text(&stores, row_cells(&stores, rows[1])[0]), "y");
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
    assert_eq!(template.last(), Some(&stores.frozen_end_template_token()));
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
        "\\def\\angle{{\\vbox{\\halign{##\\cr x\\cr\\noalign{\\prevdepth-1000pt}\\leaders\\hrule height.34pt\\hfill\\cr}}}}\\halign{#\\cr $\\angle$\\cr}",
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
        "\\font\\sy=cmsy10 \\font\\ex=cmex10 \\textfont2=\\sy \\scriptfont2=\\sy \\scriptscriptfont2=\\sy \\textfont3=\\ex \\scriptfont3=\\ex \\scriptscriptfont3=\\ex \\def\\angle{{\\vbox{\\halign{$\\scriptstyle##$\\crcr x\\crcr\\noalign{\\prevdepth-1000pt}\\mkern2.5mu\\leaders\\hrule height.34pt\\hfill\\mkern2.5mu\\crcr}}}}\\setbox0=\\vbox{\\halign{#\\cr $\\angle$\\cr}}",
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
    let source = "\\def\\angle{{\\vbox{\\halign{##\\cr x\\cr\\noalign{\\prevdepth-1000pt}\\leaders\\hrule height.34pt\\hfill\\cr}}}}\\setbox0=\\vbox{\\halign{#\\cr $\\angle$\\cr}}";

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
    let stores = run_boxed_alignment_source("\\halign{#\\cr x}\\global\\count0=17");
    let vbox = box_zero_vlist(&stores);
    let rows = vlist_rows(&stores, vbox);
    let cells = row_cells(&stores, rows[0]);

    assert_eq!(rows.len(), 1);
    assert_eq!(cells.len(), 1);
    assert_eq!(cell_text(&stores, cells[0]), "x");
    assert_eq!(
        stores.count(0),
        17,
        "brace replay must resume following input"
    );
    let output = support::terminal_effect_text(&stores);
    assert_eq!(
        output.matches("Missing \\cr inserted").count(),
        1,
        "the backed-up right brace must insert exactly one frozen \\cr: {output}"
    );
}

#[test]
fn noexpand_unexpandable_cr_terminates_alignment_row() {
    let mut stores = support::stores_with_fonts();
    tex_command::install_tex82_expandable_primitives(&mut stores);
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
    tex_command::install_tex82_expandable_primitives(&mut stores);
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
    tex_command::install_tex82_expandable_primitives(&mut stores);
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

fn run_pathological_alignment_source(
    stores: &mut Universe,
    source: &str,
    step_limit: usize,
    fuel_limit: Option<u64>,
) -> (usize, u64) {
    let mut control = alignment_control(stores, source);
    if let Some(fuel_limit) = fuel_limit {
        control
            .set_fuel_limit(fuel_limit)
            .expect("pathological alignment fuel limit should be valid");
    }
    let mut steps = 0;
    loop {
        assert!(
            steps < step_limit,
            "pathological alignment exceeded {step_limit} steps"
        );
        steps += 1;
        if matches!(
            control
                .step(stores)
                .expect("pathological alignment source executes"),
            MainControlStep::End | MainControlStep::EndOfInput
        ) {
            assert_eq!(control.input_level_count(), 0, "input levels fully retire");
            return (steps, control.fuel_burned());
        }
    }
}

#[test]
fn trip_pathological_alignment_closes_before_following_material() {
    let mut stores = support::stores_with_fonts();
    tex_command::install_tex82_expandable_primitives(&mut stores);
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
    let (steps, _) = run_pathological_alignment_source(&mut stores, source, 10_000, None);

    assert_eq!(stores.count(2), -1_118_806);
    assert_eq!(stores.count(7), 123, "execution must leave the alignment");
    assert!(steps < 10_000, "alignment made bounded progress");
    assert!(
        stores.input_summary().is_empty(),
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
    tex_command::install_tex82_expandable_primitives(&mut stores);
    let before = stores.snapshot();
    let source = r#"
        \font\f=cmr10 \f
        \long\def\l#1{}
        \halign to 1pt\expandafter{\csname#\endcsname#&#&\l{#}\cr
          \global\futurelet\endt\foo&\show\endt&$&&&.}
        \global\count7=321}
    "#;
    let (steps, _) = run_pathological_alignment_source(&mut stores, source, 10_000, None);

    assert_eq!(stores.count(7), 321, "execution must leave the alignment");
    assert_eq!(
        stores.meaning(stores.symbol("endt").expect("futurelet target")),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate)
    );
    assert!(steps < 10_000, "recovery made bounded progress");
    assert!(
        stores.input_summary().is_empty(),
        "input stack fully retires"
    );
    assert!(stores.env_journal_bytes_since(&before) < 1_000_000);
}

#[test]
fn malformed_template_row_closes_before_following_box() {
    let mut stores = support::stores_with_fonts();
    tex_command::install_tex82_expandable_primitives(&mut stores);
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
    let (steps, _) = run_pathological_alignment_source(&mut stores, source, 1_000, None);

    assert_eq!(stores.count(7), 1, "following material is in vertical mode");
    assert!(support::terminal_effect_text(&stores).contains("Missing } inserted"));
    assert_eq!(stores.execution_group_depth(), 0);
    assert!(stores.input_summary().is_empty());
    assert!(steps < 1_000);
    assert!(stores.env_journal_bytes_since(&before) < 100_000);
}

#[test]
fn paragraph_at_alignment_base_depth_is_not_recovery_input() {
    let mut stores = support::stores_with_fonts();
    tex_command::install_tex82_expandable_primitives(&mut stores);
    let source = r#"
        \halign{#\cr \par\cr}
        \global\count7=789
    "#;
    let (steps, _) = run_pathological_alignment_source(&mut stores, source, 100, None);

    assert_eq!(stores.count(7), 789);
    assert!(!support::terminal_effect_text(&stores).contains("Missing } inserted"));
    assert!(steps < 100);
    assert!(stores.input_summary().is_empty());
}

#[test]
fn outer_macro_in_skipped_span_expansion_recovers_runaway_preamble() {
    let mut stores = support::stores_with_fonts();
    tex_command::install_tex82_expandable_primitives(&mut stores);
    let before = stores.snapshot();
    let source = r#"
        \outer\def\lo{}
        \halign{{\span\ifcase3 \lo#\cr............89{}\cr}
        \global\count7=456
    "#;
    let (steps, _) = run_pathological_alignment_source(&mut stores, source, 1_000, None);

    assert_eq!(stores.count(7), 456, "tokens after recovery must execute");
    // TeX82 §338 names the runaway by the surviving `cur_cs` and §339 by the
    // `aligning` scanner status, so an `\outer` macro reports as a forbidden
    // control sequence rather than as an exhausted file.
    let output = support::terminal_effect_text_unbroken(&stores);
    assert!(
        output.contains("Forbidden control sequence found while scanning preamble of \\halign."),
        "{output}"
    );
    assert!(steps < 1_000, "recovery must make bounded progress");
    assert!(stores.input_summary().is_empty());
    assert!(stores.env_journal_bytes_since(&before) < 100_000);
}

#[test]
fn runaway_preamble_names_delivered_alignment_control_sequence() {
    for (source, expected) in [
        (
            r#"
                \outer\def\lo{}
                \let\grid=\halign
                \grid{{\span\ifcase3 \lo#\cr............89{}\cr}
            "#,
            "Forbidden control sequence found while scanning preamble of \\grid.",
        ),
        (
            r#"
                \outer\def\lo{}
                \hbox{\valign{{\span\ifcase3 \lo#\cr............89{}\cr}}
            "#,
            "Forbidden control sequence found while scanning preamble of \\valign.",
        ),
    ] {
        let mut stores = support::stores_with_fonts();
        tex_command::install_tex82_expandable_primitives(&mut stores);
        let _ = run_pathological_alignment_source(&mut stores, source, 1_000, None);
        let output = support::terminal_effect_text_unbroken(&stores);
        assert!(output.contains(expected), "{output}");
    }
}

#[test]
fn expandafter_may_expand_outer_sentinel_in_alignment_cell() {
    let mut stores = support::stores_with_fonts();
    tex_command::install_tex82_expandable_primitives(&mut stores);
    let source = r#"
        \outer\def\sentinel{\relax}
        \def\scan{\futurelet\next\scanone}
        \def\scanone{\expandafter\consume}
        \def\consume#1{\global\count7=123}
        \halign{#\cr \scan\sentinel\cr}
        \global\count8=456
    "#;
    let (steps, fuel) = run_pathological_alignment_source(&mut stores, source, 100, Some(10_000));

    assert_eq!(stores.count(7), 123);
    assert_eq!(stores.count(8), 456);
    assert!(
        !support::terminal_effect_text(&stores).contains("Missing } inserted"),
        "scanner_status is normal after the preamble"
    );
    assert!(steps < 100);
    assert!(
        fuel < 10_000,
        "expansion must stay within the legacy fuel bound"
    );
    assert!(stores.input_summary().is_empty());
}

#[test]
fn trip_conditional_preamble_recovery_stops_before_following_input() {
    let mut stores = support::stores_with_fonts();
    tex_command::install_tex82_expandable_primitives(&mut stores);
    let checkpoint = stores.snapshot();
    let source = r#"
        \setbox0=\hbox{}\copy0
        \everycr{\noalign{\penalty97}}
        \halign\relax{\span\iffalse}\fi\cr#&\ifnum0=`{\fi\cr\cr}
        \global\count7=777
    "#;
    let (steps, _) = run_pathological_alignment_source(&mut stores, source, 1_000, None);

    assert_eq!(stores.count(7), 777, "following input must execute");
    assert_eq!(
        stores
            .current_page_nodes()
            .iter()
            .filter(|node| matches!(node, Node::Penalty(97)))
            .count(),
        2,
        "official line-420 recovery runs everycr initially and after its sole row"
    );
    assert!(steps < 1_000);
    let first_hash = stores.snapshot().state_hash();

    stores.rollback(&checkpoint);
    run_pathological_alignment_source(&mut stores, source, 1_000, None);
    assert_eq!(stores.snapshot().state_hash(), first_hash);
}

/// TeX82 §799's `fin_row` ends every alignment row with
/// `pop_nest; append_to_vlist(p)`, so consecutive rows are separated by §679's
/// ordinary `\baselineskip`/`\lineskip` decision against the running
/// `prev_depth` -- exactly like consecutive boxes in any other vertical list.
/// §774's `init_align` reaches that running value because §216's `push_nest`
/// preserves `aux`; the alignment's own list therefore starts at the enclosing
/// list's `prev_depth`, not at `ignore_depth`.
#[test]
fn canonical_alignment_rows_carry_interline_glue() {
    let (_, nodes) = super::core::run_canonical_tex82_current_list(
        r"\baselineskip=12pt \lineskip=0pt \lineskiplimit=0pt
          \vbox{\halign{#\cr\hbox to 7pt{}\cr\hbox to 9pt{}\cr}",
    );

    let glue: Vec<_> = nodes
        .iter()
        .filter(|node| {
            matches!(
                node,
                Node::Glue {
                    kind: GlueKind::BaselineSkip,
                    ..
                }
            )
        })
        .collect();
    assert_eq!(
        glue.len(),
        1,
        "one §679 interline glue between the two rows: {nodes:?}"
    );
    assert_eq!(nodes.len(), 3, "row, interline glue, row: {nodes:?}");
}
