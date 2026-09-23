use std::path::Path;
use std::sync::Arc;

use tex_command::{
    CommandDeliveryBoundary, CommandObservation, CommandObserver, InputReason, InputTransition,
    ObservedToken, RecoveryKind, RegisteredSourceKind, SourceRegistration,
};
use tex_state::hyphenation::PatternSpec;
use tex_state::page::PageMark;
use tex_state::token::{Catcode, Token};

use crate::{
    ResourceFulfillment, ResourceHost, ResourceHostProvider, ResourceOutcome, ResourceWorld,
    canonical_font_resource_path,
};

use super::*;
mod etex_diagnostic_tracing;
#[path = "tests/material_property_matrices.rs"]
mod material_property_matrices;
#[path = "tests/tex82_whatsit_evidence.rs"]
mod tex82_whatsit_evidence;
#[path = "tests/tracked_region_coverage.rs"]
mod tracked_region_coverage;

/// Evaluates one short test projection inside an admitted command episode.
///
/// Keeping admission visible at each call site prevents branded state values
/// from being mistaken for detached test results.
macro_rules! admitted {
    ($stores:expr, |$context:ident| $projection:expr) => {
        crate::test_harness::with_admitted($stores, |$context| $projection)
    };
}

fn assign_static_meaning<G>(
    stores: &mut Universe<G>,
    symbol: tex_state::interner::SymbolId,
    meaning: Meaning,
) {
    admitted!(stores, |context| context
        .assign_resolved_meaning(
            symbol.symbol(),
            tex_state::ResolvedMeaning::Static(meaning),
            tex_state::AssignmentScope::Global,
        )
        .expect("static test meaning assignment"));
}

fn allocate_tokens<G>(stores: &mut Universe<G>, tokens: &[Token]) -> tex_state::TokenListId<G> {
    let words = tokens
        .iter()
        .copied()
        .map(tex_state::token::TokenWord::pack)
        .collect::<Vec<_>>();
    stores
        .allocate_token_list(&words)
        .expect("test token-list allocation")
}

fn font_by_name<G>(stores: &mut Universe<G>, name: &str) -> FontId {
    admitted!(stores, |context| {
        let symbol = context.intern_control_sequence(name);
        match context.meaning(symbol) {
            ResolvedMeaning::Static(Meaning::Font(font)) => font,
            meaning => panic!("{name} has {meaning:?}"),
        }
    })
}

fn register_source<G>(control: &mut MainControl<G>, bytes: &[u8]) {
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(bytes),
        ))
        .expect("root source registers and opens");
}

fn page_vec<G>(stores: &Universe<G>, root: tex_state::node_arena::PageListId) -> Vec<Node> {
    stores
        .page_node_list(root)
        .expect("test list belongs to the page arena")
        .nodes()
        .iter()
        .cloned()
        .collect()
}

fn mode_vec<G>(control: &MainControl<G>, stores: &mut Universe<G>) -> Vec<Node> {
    admitted!(stores, |context| control
        .modes
        .current_list()
        .nodes(context)
        .iter()
        .cloned()
        .collect())
}

fn current_list_owner_vec<G>(control: &MainControl<G>, stores: &mut Universe<G>) -> Vec<Node> {
    if crate::vertical::is_outer_vertical(&control.modes) {
        admitted!(stores, |context| context.page_contributions().to_vec())
    } else {
        mode_vec(control, stores)
    }
}

struct ImmediateInputResourceHost {
    calls: usize,
}

impl ResourceHost for ImmediateInputResourceHost {
    fn fulfill(&mut self, world: &mut ResourceWorld<'_>, need: &ResourceNeed) -> ResourceOutcome {
        self.calls += 1;
        let ResourceNeed::Input { name, .. } = need else {
            return ResourceOutcome::Unavailable;
        };
        match world.read_file(name) {
            Ok(content) => {
                ResourceOutcome::Fulfilled(ResourceFulfillment::world_input(name, content))
            }
            Err(error) if error.io_error_kind() == Some(std::io::ErrorKind::NotFound) => {
                ResourceOutcome::Unavailable
            }
            Err(error) => ResourceOutcome::Failed(error.into()),
        }
    }
}

fn run_immediate_input_provider_route(observed: bool) -> (String, usize, u64) {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        stores
            .world_mut()
            .set_memory_file("child.tex", br"B\par".to_vec())
            .expect("child input installs");
        register_source(&mut control, br"A\input child \message{done}\end");
        let mut host = ImmediateInputResourceHost { calls: 0 };
        let mut provider = ResourceHostProvider::new(&mut host);
        let mut observer = ObservationRecorder::default();
        let mut finished = false;
        for _ in 0..TEST_STEP_LIMIT {
            let step = if observed {
                control
                    .advance_with_observer_and_resource_provider(
                        stores,
                        &mut observer,
                        &mut provider,
                    )
                    .expect("observed provider route executes")
            } else {
                control
                    .advance_with_resource_provider(stores, &mut provider)
                    .expect("ordinary provider route executes")
            };
            match step {
                StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => {
                    finished = true;
                    break;
                }
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(need) => {
                    panic!("ready provider unexpectedly suspended: {need:?}")
                }
            }
        }
        assert!(
            finished,
            "ready input provider route exceeded the step bound"
        );
        (
            terminal_text(stores),
            host.calls,
            control.advance_telemetry().resource_replayed_dispatches,
        )
    })
}

struct UnavailableInputResourceHost {
    calls: usize,
}

impl ResourceHost for UnavailableInputResourceHost {
    fn fulfill(&mut self, _world: &mut ResourceWorld<'_>, need: &ResourceNeed) -> ResourceOutcome {
        self.calls += 1;
        assert!(matches!(need, ResourceNeed::Input { .. }));
        ResourceOutcome::Unavailable
    }
}

struct ImmediateFontResourceHost {
    calls: usize,
}

impl ResourceHost for ImmediateFontResourceHost {
    fn fulfill(&mut self, world: &mut ResourceWorld<'_>, need: &ResourceNeed) -> ResourceOutcome {
        self.calls += 1;
        let ResourceNeed::Font { request } = need else {
            return ResourceOutcome::Unavailable;
        };
        let path = canonical_font_resource_path(&request.name);
        match world.read_file(&path) {
            Ok(metrics) => ResourceOutcome::Fulfilled(ResourceFulfillment::Font {
                request: request.clone(),
                resource: Box::new(FontResource::Tfm {
                    metrics,
                    opentype: None,
                }),
            }),
            Err(error) if error.io_error_kind() == Some(std::io::ErrorKind::NotFound) => {
                ResourceOutcome::Unavailable
            }
            Err(error) => ResourceOutcome::Failed(error.into()),
        }
    }
}

struct ImmediatePdfImageResourceHost {
    calls: usize,
}

impl ResourceHost for ImmediatePdfImageResourceHost {
    fn fulfill(&mut self, _world: &mut ResourceWorld<'_>, need: &ResourceNeed) -> ResourceOutcome {
        self.calls += 1;
        let ResourceNeed::PdfImage { request } = need else {
            return ResourceOutcome::Unavailable;
        };
        ResourceOutcome::Fulfilled(ResourceFulfillment::PdfImage {
            request: request.clone(),
            resource: Box::new(PdfImageResource::Available(test_pdf_image_source())),
        })
    }
}

struct ImmediateInputProbeResourceHost {
    calls: usize,
}

impl ResourceHost for ImmediateInputProbeResourceHost {
    fn fulfill(&mut self, world: &mut ResourceWorld<'_>, need: &ResourceNeed) -> ResourceOutcome {
        self.calls += 1;
        let ResourceNeed::InputProbe { request } = need else {
            return ResourceOutcome::Unavailable;
        };
        match world.read_file(&request.name) {
            Ok(content) => ResourceOutcome::Fulfilled(ResourceFulfillment::world_input_probe(
                request.clone(),
                content,
            )),
            Err(error) if error.io_error_kind() == Some(std::io::ErrorKind::NotFound) => {
                ResourceOutcome::Unavailable
            }
            Err(error) => ResourceOutcome::Failed(error.into()),
        }
    }
}

fn register_cmr10_as<G>(control: &mut MainControl<G>, stores: &mut Universe<G>, name: &str) {
    const CMR10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    stores
        .world_mut()
        .set_memory_file(name, CMR10.to_vec())
        .expect("font fixture installs");
    let metrics = InputReadState::read_input_file(
        &mut stores.input_open_context(),
        std::path::Path::new(name),
    )
    .expect("font fixture reads");
    control.capabilities_mut().register_font(
        name,
        FontResource::Tfm {
            metrics,
            opentype: None,
        },
    );
}

const TEST_STEP_LIMIT: usize = 16_384;

fn run_to_end<G>(control: &mut MainControl<G>, stores: &mut Universe<G>) {
    let mut finished = false;
    for _ in 0..TEST_STEP_LIMIT {
        match control.step(stores).unwrap_or_else(|error| {
            panic!(
                "program executes: {error:?}; terminal={}",
                terminal_text(stores)
            )
        }) {
            MainControlStep::End | MainControlStep::EndOfInput => {
                finished = true;
                break;
            }
            MainControlStep::Continue => {}
        }
    }
    assert!(
        finished,
        "test job exceeded the bounded {TEST_STEP_LIMIT}-step semantic driver"
    );
}

fn box_child_nodes<G>(stores: &mut Universe<G>, register: u16) -> Vec<Node> {
    let list = stores
        .copy_box_to_page(register)
        .unwrap_or_else(|| panic!("box register {register} is nonvoid"));
    let boxed = page_vec(stores, list)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("box register {register} has a root node"));
    let children = match boxed {
        Node::HList(boxed) | Node::VList(boxed) => boxed.children,
        other => panic!("box register {register} has a box root: {other:?}"),
    };
    page_vec(stores, children)
}

fn first_published_node<G>(
    stores: &Universe<G>,
    list: tex_state::node_arena::PageListId,
) -> Option<Node> {
    page_vec(stores, list).into_iter().next()
}

fn tabskip_widths<G>(stores: &Universe<G>, nodes: &[Node], widths: &mut Vec<i32>) {
    for node in nodes {
        match node {
            Node::Glue {
                spec,
                kind: GlueKind::TabSkip,
                ..
            } => widths.push(spec.width.raw()),
            Node::HList(boxed) | Node::VList(boxed) => {
                tabskip_widths(stores, &page_vec(stores, boxed.children), widths);
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AlignmentRuntimeSnapshot {
    alignment: AlignmentIdentity,
    column: usize,
    cell_span: u16,
    rows: usize,
    captured_cells: usize,
    row_mode: Mode,
    row_space_factor: i32,
    row_prev_depth: Option<i32>,
    cell_mode: Mode,
    cell_space_factor: i32,
    cell_prev_depth: Option<i32>,
}

fn active_alignment_runtime_snapshot<G>(
    control: &MainControl<G>,
) -> Option<AlignmentRuntimeSnapshot> {
    let active = control.active_alignment.as_ref()?;
    if !active.row_open || !active.cell_open {
        return None;
    }
    let summary = control.modes.summary();
    let levels = summary.levels();
    let [.., row, cell] = levels else {
        return None;
    };
    Some(AlignmentRuntimeSnapshot {
        alignment: active.identity,
        column: active.column,
        cell_span: active.cell_span,
        rows: active.captured_row_count,
        captured_cells: active.captured_cell_count,
        row_mode: row.mode(),
        row_space_factor: row.list().raw_space_factor(),
        row_prev_depth: row.list().prev_depth().map(Scaled::raw),
        cell_mode: cell.mode(),
        cell_space_factor: cell.list().raw_space_factor(),
        cell_prev_depth: cell.list().prev_depth().map(Scaled::raw),
    })
}

fn step_until_alignment_snapshot<G>(
    control: &mut MainControl<G>,
    stores: &mut Universe<G>,
    observations: &mut dyn CommandObserver,
    accept: impl Fn(AlignmentRuntimeSnapshot) -> bool,
) -> AlignmentRuntimeSnapshot {
    for _ in 0..TEST_STEP_LIMIT {
        match control
            .step_with_observer(stores, observations)
            .expect("program executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => {
                panic!("input ended before the requested alignment state")
            }
            MainControlStep::Continue => {}
        }
        if let Some(snapshot) = active_alignment_runtime_snapshot(control)
            && accept(snapshot)
        {
            return snapshot;
        }
    }
    panic!("alignment semantic driver exceeded the bounded {TEST_STEP_LIMIT}-step limit");
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AlignmentNodeProjection {
    TabSkip(i32),
    Cell { span_count: u16 },
    Box { shift: i32, kerns: Vec<i32> },
    Penalty(i32),
    AboveDisplay(i32),
    BelowDisplay(i32),
    Baseline(i32),
    Kern(i32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PackagedRowItem {
    TabSkip(i32),
    HorizontalCell(Vec<i32>),
    VerticalCell(Vec<i32>),
}

fn packaged_row_projection<G>(stores: &Universe<G>, row: &Node) -> Vec<PackagedRowItem> {
    fn material_widths<G>(
        stores: &Universe<G>,
        nodes: &tex_state::node_arena::PageListId,
    ) -> Vec<i32> {
        let mut widths = Vec::new();
        for node in page_vec(stores, *nodes) {
            match node {
                Node::Kern { amount, .. } => widths.push(amount.raw()),
                Node::Glue {
                    spec,
                    kind: GlueKind::Normal,
                    ..
                } => widths.push(spec.width.raw()),
                Node::HList(boxed) | Node::VList(boxed) => {
                    widths.extend(material_widths(stores, &boxed.children));
                }
                _ => {}
            }
        }
        widths
    }

    let children = match row {
        Node::HList(boxed) | Node::VList(boxed) => page_vec(stores, boxed.children),
        other => panic!("alignment outcome is a packaged row: {other:?}"),
    };
    children
        .iter()
        .filter_map(|node| match node {
            Node::Glue {
                spec,
                kind: GlueKind::TabSkip,
                ..
            } => Some(PackagedRowItem::TabSkip(spec.width.raw())),
            Node::HList(boxed) => Some(PackagedRowItem::HorizontalCell(material_widths(
                stores,
                &boxed.children,
            ))),
            Node::VList(boxed) => Some(PackagedRowItem::VerticalCell(material_widths(
                stores,
                &boxed.children,
            ))),
            _ => None,
        })
        .collect()
}

fn alignment_node_projection<G>(
    stores: &Universe<G>,
    nodes: &[Node],
) -> Vec<AlignmentNodeProjection> {
    fn kerns<G>(stores: &Universe<G>, nodes: tex_state::node_arena::PageListId) -> Vec<i32> {
        let mut out = Vec::new();
        for node in page_vec(stores, nodes) {
            match node {
                Node::Kern { amount, .. } => out.push(amount.raw()),
                Node::HList(boxed) | Node::VList(boxed) => {
                    out.extend(kerns(stores, boxed.children));
                }
                _ => {}
            }
        }
        out
    }

    nodes
        .iter()
        .filter_map(|node| match node {
            Node::Glue {
                spec,
                kind: GlueKind::TabSkip,
                ..
            } => Some(AlignmentNodeProjection::TabSkip(spec.width.raw())),
            Node::Glue {
                spec,
                kind: GlueKind::AboveDisplaySkip,
                ..
            } => Some(AlignmentNodeProjection::AboveDisplay(spec.width.raw())),
            Node::Glue {
                spec,
                kind: GlueKind::BelowDisplaySkip,
                ..
            } => Some(AlignmentNodeProjection::BelowDisplay(spec.width.raw())),
            Node::Glue {
                spec,
                kind: GlueKind::BaselineSkip,
                ..
            } => Some(AlignmentNodeProjection::Baseline(spec.width.raw())),
            Node::Unset(unset) => Some(AlignmentNodeProjection::Cell {
                span_count: unset.span_count,
            }),
            Node::HList(boxed) | Node::VList(boxed) => Some(AlignmentNodeProjection::Box {
                shift: boxed.shift.raw(),
                kerns: kerns(stores, boxed.children),
            }),
            Node::Penalty(value) => Some(AlignmentNodeProjection::Penalty(*value)),
            Node::Kern { amount, .. } => Some(AlignmentNodeProjection::Kern(amount.raw())),
            _ => None,
        })
        .collect()
}

fn restoration_trace_lines(text: &str) -> String {
    text.lines()
        .filter(|line| line.starts_with("{restoring ") || line.starts_with("{retaining "))
        .map(|line| format!("{line}\n"))
        .collect()
}

fn etex_initex<G>(stores: &mut Universe<G>) -> MainControl<G> {
    tex_command::install_tex82_expandable_primitives(stores);
    tex_command::install_etex_expandable_primitives(stores);
    crate::install_unexpandable_primitives(stores);
    crate::install_etex_unexpandable_primitives(stores);
    MainControl::prepared_initex(CommandProfile::ETEX26)
}

fn pdftex_initex<G>(stores: &mut Universe<G>) -> MainControl<G> {
    tex_command::install_tex82_expandable_primitives(stores);
    tex_command::install_etex_expandable_primitives(stores);
    tex_command::install_pdftex_expandable_primitives(stores);
    crate::install_unexpandable_primitives(stores);
    crate::install_etex_unexpandable_primitives(stores);
    tex_command::install_pdftex_unexpandable_primitives(stores);
    MainControl::prepared_initex(CommandProfile::PDFTEX14029)
}

fn run_to_end_observed<G>(
    control: &mut MainControl<G>,
    stores: &mut Universe<G>,
    observations: &mut dyn CommandObserver,
) {
    let mut finished = false;
    for _ in 0..TEST_STEP_LIMIT {
        match control
            .step_with_observer(stores, observations)
            .expect("program executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => {
                finished = true;
                break;
            }
            MainControlStep::Continue => {}
        }
    }
    assert!(
        finished,
        "observed test job exceeded the bounded {TEST_STEP_LIMIT}-step semantic driver"
    );
}

fn terminal_text<G>(stores: &Universe<G>) -> String {
    let committed = stores
        .world()
        .memory_terminal_output()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default();
    let pending: String = stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            tex_state::EffectRecord::StreamWrite {
                sink:
                    tex_state::PrintSink::Terminal
                    | tex_state::PrintSink::TerminalAndLog
                    | tex_state::PrintSink::Log,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    committed + &pending
}

fn pending_sink_text<G>(stores: &Universe<G>, terminal: bool) -> String {
    stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            tex_state::EffectRecord::StreamWrite { sink, text }
                if if terminal {
                    matches!(
                        sink,
                        tex_state::PrintSink::Terminal | tex_state::PrintSink::TerminalAndLog
                    )
                } else {
                    matches!(
                        sink,
                        tex_state::PrintSink::Log | tex_state::PrintSink::TerminalAndLog
                    )
                } =>
            {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect()
}

fn macro_words<G>(stores: &mut Universe<G>, name: &str) -> Vec<tex_state::token::TokenWord> {
    let symbol = stores
        .intern(name)
        .expect("macro control sequence")
        .symbol();
    admitted!(stores, |context| match context.meaning(symbol) {
        tex_state::ResolvedMeaning::Macro { definition, .. } => {
            context.definition(definition).replacement_text().to_vec()
        }
        _ => panic!("{name} is a macro"),
    })
}

fn macro_character_text<G>(stores: &mut Universe<G>, name: &str) -> String {
    macro_words(stores, name)
        .into_iter()
        .filter_map(|word| match word.semantic_token() {
            Token::Char { ch, .. } => Some(ch),
            Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => None,
        })
        .collect()
}

fn macro_semantic_tokens<G>(stores: &mut Universe<G>, name: &str) -> Vec<Token> {
    macro_words(stores, name)
        .into_iter()
        .map(tex_state::token::TokenWord::semantic_token)
        .collect()
}

/// Runs a direct pdfTeX probe job only with its fixture resources admitted
/// before execution. A direct `MainControl` has no checkpoint owner, so a
/// resource miss cannot be answered by calling the runner again on the same
/// control. Full miss/replay equivalence belongs to the `tex-incr` session
/// tests; this helper keeps local scanner/collector semantics independent of
/// that host lifecycle.
fn run_pdftex_file_probe_job(source: &[u8], preloaded: &[&str]) -> String {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = pdftex_initex(stores);
        let resource = |name: &str| {
            let bytes: &[u8] = match name {
                "first" => b"ABCD",
                "second" => b"AB",
                "third" => b"CD",
                "4" => b"EF",
                other => panic!("unexpected file enquiry {other:?}"),
            };
            tex_command::FileEnquiryResource::new(
                SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(bytes)),
                None,
            )
        };
        for name in preloaded {
            control
                .capabilities_mut()
                .register_input_probe(*name, resource(name));
        }
        register_source(&mut control, source);

        let mut ledger = crate::OutputLedger::new();
        let mut checkpoints = Vec::new();
        let cancellation = crate::Cancellation::new();
        let mut terminal = None;
        for _ in 0..TEST_STEP_LIMIT {
            match crate::CanonicalStepRunner::new(&mut control, stores, &mut ledger)
                .step(&mut checkpoints, &cancellation)
            {
                crate::CanonicalStepResult::ResourceNeed(need) => panic!(
                    "direct semantic helper crossed a resource boundary for {need:?}; use the tex-incr checkpoint owner for replay"
                ),
                crate::CanonicalStepResult::Completed(step @ ReplayStep::End) => {
                    terminal = Some(step);
                    break;
                }
                crate::CanonicalStepResult::Progress(_)
                | crate::CanonicalStepResult::Committed(_) => {}
                other => panic!("unexpected file-enquiry step: {other:?}"),
            }
        }
        let terminal = terminal.expect("bounded file-enquiry semantic driver reached terminal");
        assert_eq!(control.pending_resource_site(), None);
        assert!(
            control.command.named_boundary_is_quiescent(),
            "terminal command continuation remained live after preloading {preloaded:?}: {}",
            terminal_text(stores)
        );
        ledger
            .terminal_receipt(&control, stores, terminal)
            .expect("fulfilled file enquiries leave terminal completion quiescent");
        terminal_text(stores)
    })
}

#[cfg(feature = "profiling")]
fn ordinary_command_episode_evidence(
    repetitions: usize,
) -> (
    tex_state::measurement::HotCoreAllocationMeasurement,
    usize,
    usize,
    usize,
) {
    let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
    let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    let mut scalar_transitions = 0;
    let mut whole_frame_copies = 0;
    let mut overlapping_frame_moves = 0;
    {
        let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
        let mut frame = CommandEpisode::<()>::default();
        let stationary_address = std::ptr::addr_of!(frame);
        for _ in 0..repetitions {
            frame.admit_immediate_pdf(UnexpandablePrimitive::PdfObject);
            assert!(matches!(
                frame.phase,
                Some(PreflightCommandPhase::ImmediatePdfRetry(
                    UnexpandablePrimitive::PdfObject
                ))
            ));
            scalar_transitions += 1;
            whole_frame_copies += usize::from(std::ptr::addr_of!(frame) != stationary_address);
            frame.clear_preflight();
            frame.assert_empty();
            scalar_transitions += 1;
            whole_frame_copies += usize::from(std::ptr::addr_of!(frame) != stationary_address);
            let mut hot = std::hint::black_box(hot_apply::HotOperation::<()>::end_ordinary_group());
            scalar_transitions += 1;
            whole_frame_copies += usize::from(std::ptr::addr_of!(frame) != stationary_address);
            let _ = std::hint::black_box(&mut hot);
            frame.assert_empty();
            scalar_transitions += 1;
            whole_frame_copies += usize::from(std::ptr::addr_of!(frame) != stationary_address);
        }
        // Every ordinary transition above mutates the same nonoverlapping
        // destination in place. Only typed suspension code moves a complete
        // frame, so the ordinary-path memmove-shaped count is exactly zero.
        overlapping_frame_moves += 0;
    }
    let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    (
        tex_state::measurement::HotCoreAllocationMeasurement {
            calls: after.calls - before.calls,
            requested_bytes: after.requested_bytes - before.requested_bytes,
        },
        scalar_transitions,
        whole_frame_copies,
        overlapping_frame_moves,
    )
}

#[cfg(feature = "profiling")]
fn resident_cold_scan_evidence(
    repetitions: usize,
) -> (
    tex_state::measurement::HotCoreAllocationMeasurement,
    usize,
    usize,
    usize,
    u64,
) {
    let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
    let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    let mut scalar_transitions = 0;
    let mut address_changes = 0;
    let mut overlapping_leaf_moves = 0;
    let mut checksum = 0_u64;
    {
        let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
        let mut frame = CommandEpisode::<()>::default();
        let mut cold = ColdOperationSlot::<()>::default();
        let stationary_frame = std::ptr::addr_of!(frame);
        let stationary_leaf = std::ptr::addr_of!(cold);
        for index in 0..repetitions {
            let index = std::hint::black_box(index + 1);
            write_cold_scan!(
                cold,
                ColdOperation::Count {
                    index: (index & 0xff) as u16,
                    value: index as i32,
                    global: index & 1 != 0,
                }
            );
            frame.mark_resident_cold(&cold);
            scalar_transitions += 1;
            address_changes += usize::from(std::ptr::addr_of!(cold) != stationary_leaf);
            address_changes += usize::from(std::ptr::addr_of!(frame) != stationary_frame);
            let ColdOperation::Count {
                index,
                value,
                global,
            } = frame.unavailable(&cold)
            else {
                unreachable!("resident cold scan installs its exact typed leaf")
            };
            checksum = checksum.wrapping_add(
                (*index as u64).rotate_left(7)
                    ^ (*value as u64).rotate_left(19)
                    ^ u64::from(*global),
            );
            frame.clear_cold(&mut cold);
            scalar_transitions += 1;
            address_changes += usize::from(std::ptr::addr_of!(cold) != stationary_leaf);
            address_changes += usize::from(std::ptr::addr_of!(frame) != stationary_frame);
            overlapping_leaf_moves += 0;
        }
        frame.assert_empty();
    }
    let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    (
        tex_state::measurement::HotCoreAllocationMeasurement {
            calls: after.calls - before.calls,
            requested_bytes: after.requested_bytes - before.requested_bytes,
        },
        scalar_transitions,
        address_changes,
        overlapping_leaf_moves,
        std::hint::black_box(checksum),
    )
}

const GENERATED_MAIN_AUX: &[u8] = br"\newlabel{sec:intro}{{1}{1}}
";
const GENERATED_MAIN_TOC: &[u8] = br"\contentsline{section}{Introduction}{1}
";

#[derive(Default)]
struct GeneratedReferenceInputHost {
    calls: usize,
}

impl ResourceHost for GeneratedReferenceInputHost {
    fn fulfill(&mut self, _world: &mut ResourceWorld<'_>, need: &ResourceNeed) -> ResourceOutcome {
        let ResourceNeed::Input { name, .. } = need else {
            return ResourceOutcome::Declined;
        };
        let bytes = match name.as_str() {
            "main.aux" => GENERATED_MAIN_AUX,
            "main.toc" => GENERATED_MAIN_TOC,
            _ => return ResourceOutcome::Declined,
        };
        self.calls += 1;
        ResourceOutcome::Fulfilled(ResourceFulfillment::input(
            name.clone(),
            RegisteredSourceKind::Generated,
            Arc::from(bytes),
        ))
    }
}

const PREFIXED_DEFINITION_RESOURCE_SOURCE: &[u8] = br"
\protected\long\def\gsetNpx{\protected\outer\long\global\edef}
\long\def\expArgsNc#1#2{\expandafter#1\csname#2\endcsname}
\protected\def\gsetCpx{\expArgsNc\gsetNpx}
\gsetCpx{result\pdffilesize{first}}{\pdffilesize{second}}
\end
";

fn register_file_size_probe<G>(
    control: &mut MainControl<G>,
    need: &ResourceNeed,
    bytes: &'static [u8],
) {
    let ResourceNeed::InputProbe { request } = need else {
        panic!("expected file-size probe, got {need:?}");
    };
    control.capabilities_mut().register_input_probe(
        request.name.clone(),
        tex_command::FileEnquiryResource::new(
            SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(bytes)),
            None,
        ),
    );
}

fn register_named_file_size_probe<G>(
    control: &mut MainControl<G>,
    name: &str,
    bytes: &'static [u8],
) {
    control.capabilities_mut().register_input_probe(
        name,
        tex_command::FileEnquiryResource::new(
            SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(bytes)),
            None,
        ),
    );
}

fn next_input_probe<G>(control: &mut MainControl<G>, stores: &mut Universe<G>) -> ResourceNeed {
    for _ in 0..TEST_STEP_LIMIT {
        match control
            .advance_episode(stores)
            .expect("prefixed definition probe suspends")
        {
            StepResult::Suspended(need @ ResourceNeed::InputProbe { .. }) => return need,
            StepResult::Progress(_) => {}
            other => panic!("unexpected prefixed-definition step: {other:?}"),
        }
    }
    panic!("prefixed-definition probe exceeded the bounded semantic driver")
}

fn recursive_test_box<G>(stores: &mut Universe<G>) -> tex_state::node_arena::PageListId {
    use tex_state::font::NULL_FONT;
    use tex_state::glue::Order;
    use tex_state::node::{
        AdjustNode, BoxLr, BoxNode, BoxNodeFields, DiscKind, GlueKind, LeaderPayload, MathBoundary,
        Sign, UnsetKind, UnsetNode, UnsetNodeFields,
    };
    use tex_state::scaled::GlueSetRatio;

    let leaf = crate::test_harness::publish_page_nodes(
        stores,
        [
            Node::Penalty(19),
            Node::Rule {
                width: Some(Scaled::from_raw(101)),
                height: Some(Scaled::from_raw(102)),
                depth: Some(Scaled::from_raw(103)),
            },
        ],
    );
    let box_node = |children| {
        BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(201),
            height: Scaled::from_raw(202),
            depth: Scaled::from_raw(203),
            shift: Scaled::from_raw(204),
            box_lr: BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Stretching,
            glue_order: Order::Fill,
            children,
        })
    };
    let glue = GlueSpec {
        width: Scaled::from_raw(301),
        stretch: Scaled::from_raw(302),
        stretch_order: Order::Fil,
        shrink: Scaled::from_raw(303),
        shrink_order: Order::Filll,
    };
    let tokens = allocate_tokens(
        stores,
        &[
            Token::Char {
                ch: 'm',
                cat: Catcode::Letter,
            },
            Token::Char {
                ch: '!',
                cat: Catcode::Other,
            },
        ],
    );
    let tokens = admitted!(stores, |context| context.node_token_list(&tokens));
    let pre = crate::test_harness::publish_page_nodes(
        stores,
        [Node::Char {
            font: NULL_FONT,
            ch: 'p',
            origin: tex_state::token::OriginId::UNKNOWN,
        }],
    );
    let post = crate::test_harness::publish_page_nodes(
        stores,
        [Node::Kern {
            amount: Scaled::from_raw(401),
            kind: tex_state::node::KernKind::Explicit,
        }],
    );
    let replace = crate::test_harness::publish_page_nodes(
        stores,
        [Node::Lig {
            font: NULL_FONT,
            ch: 'L',
            orig: vec!['f', 'i'],
            origins: vec![tex_state::token::OriginId::UNKNOWN; 2],
            left_hit: false,
            right_hit: false,
        }],
    );

    let children = crate::test_harness::publish_page_nodes(
        stores,
        [
            Node::Rule {
                width: Some(Scaled::from_raw(1)),
                height: None,
                depth: Some(Scaled::from_raw(3)),
            },
            Node::Glue {
                spec: glue,
                kind: GlueKind::Leaders,
                leader: Some(LeaderPayload::HList(box_node(leaf))),
            },
            Node::Ins {
                class: 7,
                size: Scaled::from_raw(501),
                split_top_skip: glue,
                split_max_depth: Scaled::from_raw(502),
                floating_penalty: 503,
                content: leaf,
            },
            Node::Mark { class: 9, tokens },
            Node::Adjust(AdjustNode {
                content: post,
                pre: true,
            }),
            Node::MathOn(Scaled::from_raw(601)),
            Node::MathOff(Scaled::from_raw(602)),
            Node::Direction(MathBoundary::BeginR),
            Node::Lig {
                font: NULL_FONT,
                ch: 'L',
                orig: vec!['f', 'i'],
                origins: vec![tex_state::token::OriginId::UNKNOWN; 2],
                left_hit: false,
                right_hit: false,
            },
            Node::Disc {
                kind: DiscKind::Discretionary,
                pre,
                post,
                replace,
                physical_replace_count: 1,
            },
            Node::HList(box_node(pre)),
            Node::VList(box_node(post)),
            Node::Unset(UnsetNode::new(UnsetNodeFields {
                kind: UnsetKind::HBox,
                width: Scaled::from_raw(701),
                height: Scaled::from_raw(702),
                depth: Scaled::from_raw(703),
                span_count: 4,
                stretch: Scaled::from_raw(704),
                stretch_order: Order::Fill,
                shrink: Scaled::from_raw(705),
                shrink_order: Order::Fil,
                children: replace,
            })),
        ],
    );
    crate::test_harness::publish_page_nodes(stores, [Node::HList(box_node(children))])
}

fn recursive_node_signature<G>(
    stores: &Universe<G>,
    list: &tex_state::node_arena::PageListId,
) -> String {
    recursive_owned_node_signature(stores, list)
}

fn recursive_owned_node_signature<G>(
    stores: &Universe<G>,
    list: &tex_state::node_arena::PageListId,
) -> String {
    use tex_state::node::{LeaderPayload, Node};

    page_vec(stores, *list)
        .iter()
        .map(|node| match node {
            Node::HList(box_node) | Node::VList(box_node) => format!(
                "box={}/{:?}/{:?}/{:?}/{:?}/{:?}/{:?}/{:?}/{:?}/children={}",
                if matches!(node, Node::HList(_)) {
                    "h"
                } else {
                    "v"
                },
                box_node.width,
                box_node.height,
                box_node.depth,
                box_node.shift,
                box_node.box_lr,
                box_node.glue_set,
                box_node.glue_sign,
                box_node.glue_order,
                recursive_owned_node_signature(stores, &box_node.children)
            ),
            Node::Unset(unset) => format!(
                "unset={:?}/{:?}/{:?}/{:?}/{}/{:?}/{:?}/{:?}/{:?}/children={}",
                unset.kind,
                unset.width,
                unset.height,
                unset.depth,
                unset.span_count,
                unset.stretch,
                unset.stretch_order,
                unset.shrink,
                unset.shrink_order,
                recursive_owned_node_signature(stores, &unset.children)
            ),
            Node::Glue { spec, leader, .. } => {
                let leader = leader.as_ref().map(|leader| match leader {
                    LeaderPayload::HList(box_node) | LeaderPayload::VList(box_node) => format!(
                        "box={}/{:?}/{:?}/{:?}/{:?}/{:?}/{:?}/{:?}/{:?}/children={}",
                        if matches!(leader, LeaderPayload::HList(_)) {
                            "h"
                        } else {
                            "v"
                        },
                        box_node.width,
                        box_node.height,
                        box_node.depth,
                        box_node.shift,
                        box_node.box_lr,
                        box_node.glue_set,
                        box_node.glue_sign,
                        box_node.glue_order,
                        recursive_owned_node_signature(stores, &box_node.children)
                    ),
                    LeaderPayload::Rule { .. } => format!("{leader:?}"),
                });
                format!("glue={spec:?}/leader={leader:?}")
            }
            Node::Disc {
                pre,
                post,
                replace,
                kind,
                ..
            } => format!(
                "disc={kind:?}/pre={}/post={}/replace={}",
                recursive_owned_node_signature(stores, pre),
                recursive_owned_node_signature(stores, post),
                recursive_owned_node_signature(stores, replace)
            ),
            Node::Mark { class, tokens } => {
                format!("mark={class}/tokens={tokens:?}")
            }
            Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => format!(
                "ins={class}/{size:?}/{:?}/{split_max_depth:?}/{floating_penalty}/content={}",
                split_top_skip,
                recursive_owned_node_signature(stores, content)
            ),
            Node::Adjust(adjust) => format!(
                "adjust={}/content={}",
                adjust.pre,
                recursive_owned_node_signature(stores, &adjust.content)
            ),
            _ => format!("{node:?}"),
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn pdftex_random_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    let set_seed = stores.intern("pdfsetrandomseed").expect("symbol interning");
    assign_static_meaning(
        stores,
        set_seed,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfSetRandomSeed),
    );
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn pdftex_timer_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    let reset_timer = stores.intern("pdfresettimer").expect("symbol interning");
    assign_static_meaning(
        stores,
        reset_timer,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfResetTimer),
    );
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn pdftex_interword_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    for (name, primitive) in [
        (
            "pdfinterwordspaceon",
            UnexpandablePrimitive::PdfInterwordSpaceOn,
        ),
        (
            "pdfinterwordspaceoff",
            UnexpandablePrimitive::PdfInterwordSpaceOff,
        ),
        ("pdffakespace", UnexpandablePrimitive::PdfFakeSpace),
        ("pdfrunninglinkon", UnexpandablePrimitive::PdfRunningLinkOn),
        (
            "pdfrunninglinkoff",
            UnexpandablePrimitive::PdfRunningLinkOff,
        ),
        ("pdfspacefont", UnexpandablePrimitive::PdfSpaceFont),
    ] {
        let symbol = stores.intern(name).expect("symbol interning");
        assign_static_meaning(stores, symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn pdftex_font_action_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    let nullfont = stores.intern("nullfont").expect("symbol interning");
    assign_static_meaning(stores, nullfont, Meaning::Font(tex_state::font::NULL_FONT));
    for (name, primitive) in [
        ("pdffontexpand", UnexpandablePrimitive::PdfFontExpand),
        ("pdffontattr", UnexpandablePrimitive::PdfFontAttr),
        ("pdfincludechars", UnexpandablePrimitive::PdfIncludeChars),
        ("pdfmapfile", UnexpandablePrimitive::PdfMapFile),
        ("pdfmapline", UnexpandablePrimitive::PdfMapLine),
        (
            "pdfglyphtounicode",
            UnexpandablePrimitive::PdfGlyphToUnicode,
        ),
        (
            "pdfnobuiltintounicode",
            UnexpandablePrimitive::PdfNoBuiltinToUnicode,
        ),
    ] {
        let symbol = stores.intern(name).expect("symbol interning");
        assign_static_meaning(stores, symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn pdftex_snapping_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    for (name, primitive) in [
        ("pdfsnaprefpoint", UnexpandablePrimitive::PdfSnapRefPoint),
        ("pdfsnapy", UnexpandablePrimitive::PdfSnapY),
        ("pdfsnapycomp", UnexpandablePrimitive::PdfSnapYComp),
    ] {
        let symbol = stores.intern(name).expect("symbol interning");
        assign_static_meaning(stores, symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn pdftex_graphics_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    for (name, primitive) in [
        ("pdfliteral", UnexpandablePrimitive::PdfLiteral),
        ("pdfsetmatrix", UnexpandablePrimitive::PdfSetMatrix),
        ("pdfsave", UnexpandablePrimitive::PdfSave),
        ("pdfrestore", UnexpandablePrimitive::PdfRestore),
        ("pdfcolorstack", UnexpandablePrimitive::PdfColorStack),
        ("pdfsavepos", UnexpandablePrimitive::PdfSavePos),
    ] {
        let symbol = stores.intern(name).expect("symbol interning");
        assign_static_meaning(stores, symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn pdftex_outline_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    let outline = stores.intern("pdfoutline").expect("symbol interning");
    assign_static_meaning(
        stores,
        outline,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfOutline),
    );
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn pdftex_thread_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    for (name, primitive) in [
        ("pdfthread", UnexpandablePrimitive::PdfThread),
        ("pdfstartthread", UnexpandablePrimitive::PdfStartThread),
        ("pdfendthread", UnexpandablePrimitive::PdfEndThread),
    ] {
        let symbol = stores.intern(name).expect("symbol interning");
        assign_static_meaning(stores, symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn pdftex_object_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    for (name, primitive) in [
        ("pdfobj", UnexpandablePrimitive::PdfObject),
        ("pdfrefobj", UnexpandablePrimitive::PdfReferenceObject),
        ("immediate", UnexpandablePrimitive::Immediate),
    ] {
        let symbol = stores.intern(name).expect("symbol interning");
        assign_static_meaning(stores, symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn pdftex_form_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    for (name, primitive) in [
        ("pdfxform", UnexpandablePrimitive::PdfXForm),
        ("pdfrefxform", UnexpandablePrimitive::PdfRefXForm),
        ("immediate", UnexpandablePrimitive::Immediate),
    ] {
        let symbol = stores.intern(name).expect("symbol interning");
        assign_static_meaning(stores, symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn pdftex_image_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    for (name, primitive) in [
        ("pdfximage", UnexpandablePrimitive::PdfXImage),
        ("pdfrefximage", UnexpandablePrimitive::PdfRefXImage),
        ("immediate", UnexpandablePrimitive::Immediate),
    ] {
        let symbol = stores.intern(name).expect("symbol interning");
        assign_static_meaning(stores, symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn test_pdf_image_source() -> tex_state::PdfExternalImageSource {
    tex_state::PdfExternalImageSource {
        identity: tex_state::ContentHash::from_bytes(b"canonical image preflight"),
        metadata: tex_state::PdfExternalImageMetadata::Raster(tex_state::PdfRasterImageMetadata {
            format: tex_state::PdfRasterFormat::Png,
            width: 1,
            height: 1,
            bits_per_component: 8,
            color_space: tex_state::PdfRasterColorSpace::Gray,
            alpha: false,
            png_color_type: Some(0),
        }),
        natural_width: Scaled::from_raw(Scaled::UNITY),
        natural_height: Scaled::from_raw(Scaled::UNITY),
        bytes: b"image bytes".to_vec().into(),
    }
}

fn install_test_hbox<G>(stores: &mut Universe<G>, register: u16, width: Scaled) {
    let children = tex_state::node_arena::PageListId::empty();
    let list = crate::test_harness::publish_page_nodes(
        stores,
        [Node::HList(tex_state::node::BoxNode::new(
            tex_state::node::BoxNodeFields {
                width,
                height: Scaled::from_raw(2),
                depth: Scaled::from_raw(3),
                shift: Scaled::from_raw(0),
                box_lr: tex_state::node::BoxLr::Normal,
                glue_set: tex_state::scaled::GlueSetRatio::ZERO,
                glue_sign: tex_state::node::Sign::Normal,
                glue_order: Order::Normal,
                children,
            },
        ))],
    );
    stores.assign_page_box_local(register, list);
}

fn install_test_form<G>(stores: &mut Universe<G>) {
    install_test_hbox(stores, 0, Scaled::from_raw(11));
    let list = stores.take_box_to_page(0).expect("test form box");
    admitted!(stores, |context| {
        let identity = context.reserve_pdf_form().expect("reserve test form");
        context
            .initialize_pdf_form(
                identity,
                list,
                (
                    Scaled::from_raw(11),
                    Scaled::from_raw(2),
                    Scaled::from_raw(3),
                ),
                None,
                None,
                false,
            )
            .expect("initialize test form");
    });
}

fn token_character_text<G>(stores: &mut Universe<G>, tokens: tex_state::TokenListId<G>) -> String {
    admitted!(stores, |context| context
        .token_list(tokens)
        .iter()
        .collect::<Vec<_>>())
    .into_iter()
    .filter_map(|word| match word.semantic_token() {
        Token::Char { ch, .. } => Some(ch),
        Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => None,
    })
    .collect()
}

fn pdftex_annotation_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    for (name, primitive) in [
        ("pdfannot", UnexpandablePrimitive::PdfAnnot),
        ("pdfstartlink", UnexpandablePrimitive::PdfStartLink),
        ("pdfendlink", UnexpandablePrimitive::PdfEndLink),
    ] {
        let symbol = stores.intern(name).expect("symbol interning");
        assign_static_meaning(stores, symbol, Meaning::UnexpandablePrimitive(primitive));
    }
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn pdftex_destination_control<G>(stores: &mut Universe<G>) -> MainControl<G> {
    let destination = stores.intern("pdfdest").expect("symbol interning");
    assign_static_meaning(
        stores,
        destination,
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfDest),
    );
    MainControl::with_profile(tex_command::CommandProfile::PDFTEX14029)
}

fn step_until_pdf_seed<G>(control: &mut MainControl<G>, stores: &mut Universe<G>, expected: i32) {
    for _ in 0..4 {
        control.step(stores).expect("random command");
        if stores.world().pdf_random_seed() == expected {
            return;
        }
    }
    panic!("pdfTeX random seed did not become {expected}");
}

#[derive(Default)]
struct ObservationRecorder(Vec<CommandObservation>);

impl CommandObserver for ObservationRecorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

#[derive(Default)]
struct GeometryObservationRecorder(Vec<CommandObservation>);

impl CommandObserver for GeometryObservationRecorder {
    fn observes_geometry(&self) -> bool {
        true
    }

    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

fn observation_name(value: &ObservationValue) -> Option<&str> {
    match value {
        ObservationValue::Name(name) => Some(name),
        _ => None,
    }
}

fn observation_tokens(value: &ObservationValue) -> Option<&[tex_command::ObservedToken]> {
    match value {
        ObservationValue::Tokens(tokens) => Some(tokens),
        _ => None,
    }
}

/// Collects every `\setlanguage` whatsit inside box register zero.
fn language_whatsits<G>(stores: &mut Universe<G>) -> Vec<(u8, u8, u8)> {
    let outer = stores
        .copy_box_to_page(0)
        .expect("box 0 holds the constructed hbox");
    let Some(Node::HList(boxed)) = first_published_node(stores, outer) else {
        panic!("box 0 holds an hlist");
    };
    page_vec(stores, boxed.children)
        .iter()
        .filter_map(|node| match node {
            Node::Whatsit(tex_state::node::Whatsit::Language {
                language,
                left_hyphen_min,
                right_hyphen_min,
            }) => Some((*language, *left_hyphen_min, *right_hyphen_min)),
            _ => None,
        })
        .collect()
}

fn spanning_alignment_source(spans: &str) -> Vec<u8> {
    format!(
        concat!(
            r"\catcode`{{=1 \catcode`}}=2 \catcode`\#=6 \catcode`\&=4",
            "\n",
            r"\def\a{{\span}}\def\b{{\a\a}}\def\c{{\b\b}}\def\d{{\c\c}}",
            "\n",
            r"\def\e{{\d\d}}\def\f{{\e\e}}\def\g{{\f\f}}\def\h{{\g\g}}\def\i{{\h\h}}",
            "\n",
            r"\setbox0=\vbox{{\halign{{#&&#\cr\relax{spans}\relax\cr}}}}",
            "\n",
            r"\global\count0=1\end",
            "\n",
        ),
        spans = spans
    )
    .into_bytes()
}

fn with_etex<R>(
    source: &[u8],
    test: impl for<'id> FnOnce(&mut Universe<tex_state::GenerationBrand<'id>>) -> R,
) -> R {
    crate::test_harness::with_plain_universe(|stores| {
        tex_command::install_tex82_expandable_primitives(stores);
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_unexpandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
        let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
        register_source(&mut control, source);
        run_to_end(&mut control, stores);
        test(stores)
    })
}

#[path = "tests/alignment.rs"]
mod alignment;
#[path = "tests/assignments.rs"]
mod assignments;
#[path = "tests/diagnostics_recovery.rs"]
mod diagnostics_recovery;
#[path = "tests/episode_observation.rs"]
mod episode_observation;
#[path = "tests/execution_lifecycle.rs"]
mod execution_lifecycle;
#[path = "tests/initialization_formats.rs"]
mod initialization_formats;
#[path = "tests/list_material.rs"]
mod list_material;
#[path = "tests/paragraph_math.rs"]
mod paragraph_math;
#[path = "tests/pdf_commands.rs"]
mod pdf_commands;
#[path = "tests/resource_replay.rs"]
mod resource_replay;
#[path = "tests/shipout.rs"]
mod shipout;
#[path = "tests/tracing.rs"]
mod tracing;
