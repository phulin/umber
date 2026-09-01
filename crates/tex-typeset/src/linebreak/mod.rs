use tex_arith::WideScaled;
use tex_state::glue::GlueSpec;
use tex_state::node::{KernKind, Node};
use tex_state::node_arena::{NodeCursor, PageListId, PageNodeSequenceId};
use tex_state::node_sequence::DirectHighCellLineages;
use tex_state::node_sequence::NodeSequence;
use tex_state::scaled::Scaled;

use crate::{INF_BAD, TypesetState};

const EJECT_PENALTY: i32 = -10_000;
const INF_PENALTY: i32 = 10_000;
const AWFUL_BAD: i32 = 0o7777777777;

fn add(left: Scaled, right: Scaled) -> Scaled {
    left.checked_add(right)
        .expect("line-local scaled addition overflow")
}

fn sub_scaled(left: Scaled, right: Scaled) -> Scaled {
    left.checked_sub(right)
        .expect("line-local scaled subtraction overflow")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineBreakParams {
    pub pretolerance: i32,
    pub tolerance: i32,
    pub line_penalty: i32,
    pub hyphen_penalty: i32,
    pub ex_hyphen_penalty: i32,
    pub adj_demerits: i32,
    pub double_hyphen_demerits: i32,
    pub final_hyphen_demerits: i32,
    pub emergency_stretch: Scaled,
    pub looseness: i32,
    pub last_line_fit: i32,
    /// pdfTeX's `\pdfadjustspacing`: positive values expand finalized lines;
    /// values greater than one also affect breakpoint feasibility.
    pub pdf_adjust_spacing: i32,
    /// Paragraph-wide validated font-expansion step counts. Ignored unless
    /// `pdf_adjust_spacing > 1`.
    pub expansion_steps: Option<(i32, i32)>,
    /// pdfTeX's `\pdfprotrudechars`: positive values materialize margin
    /// kerns; values greater than one also affect breakpoint feasibility.
    pub pdf_protrude_chars: i32,
    pub left_skip: GlueSpec,
    pub right_skip: GlueSpec,
    pub par_fill_skip: GlueSpec,
    pub shape: LineShape,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostLineBreakParams {
    pub empty_list: PageListId,
    pub left_skip: GlueSpec,
    pub right_skip: GlueSpec,
    pub interline_penalty: i32,
    pub club_penalty: i32,
    pub widow_penalties: WidowPenalties,
    pub broken_penalty: i32,
    pub prev_graf: i32,
    pub interline_penalties: Vec<i32>,
    pub club_penalties: Vec<i32>,
    pub shape: LineShape,
}

/// e-TeX's paragraph-ending context retained until `post_line_break` chooses
/// the widow-penalty family.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WidowPenalties {
    pub selector: WidowPenaltySelector,
    pub ordinary: PenaltySequence,
    pub display: PenaltySequence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WidowPenaltySelector {
    Ordinary,
    DisplayInterrupted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PenaltySequence {
    pub fallback: i32,
    pub values: Vec<i32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParagraphShape {
    pub lines: Vec<LineShapeEntry>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineShapeEntry {
    pub indent: Scaled,
    pub width: Scaled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineShape {
    pub hsize: Scaled,
    pub parshape: Option<ParagraphShape>,
    pub hang_indent: Scaled,
    pub hang_after: i32,
    pub line_offset: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineDimensions {
    pub indent: Scaled,
    pub width: Scaled,
}

impl LineShape {
    #[must_use]
    pub fn natural(hsize: Scaled) -> Self {
        Self {
            hsize,
            parshape: None,
            hang_indent: Scaled::from_raw(0),
            hang_after: 1,
            line_offset: 0,
        }
    }

    #[must_use]
    pub fn dimensions(&self, line_no: usize) -> LineDimensions {
        let one_based = line_no
            .max(1)
            .checked_add(self.line_offset)
            .expect("line number exceeds usize");
        if let Some(parshape) = &self.parshape
            && !parshape.lines.is_empty()
        {
            let index = (one_based - 1).min(parshape.lines.len() - 1);
            let entry = parshape.lines[index];
            return LineDimensions {
                indent: entry.indent,
                width: entry.width,
            };
        }

        if self.hang_indent.raw() == 0 || !hanging_applies(one_based, self.hang_after) {
            return LineDimensions {
                indent: Scaled::from_raw(0),
                width: self.hsize,
            };
        }

        let amount = self.hang_indent.raw();
        if amount >= 0 {
            LineDimensions {
                indent: self.hang_indent,
                width: sub_scaled(self.hsize, Scaled::from_raw(amount)),
            }
        } else {
            LineDimensions {
                indent: Scaled::from_raw(0),
                width: add(self.hsize, Scaled::from_raw(amount)),
            }
        }
    }
}

fn hanging_applies(line_no: usize, hang_after: i32) -> bool {
    if hang_after < 0 {
        line_no <= hang_after.saturating_abs() as usize
    } else {
        line_no > hang_after as usize
    }
}

pub use tex_state::{
    PureBreakDecision as BreakDecision, PureBreakMemoryEvent as BreakMemoryEvent,
    PureBreakMemoryOwner as BreakMemoryOwner, PureBreakMemoryPlan as BreakMemoryPlan,
    PureBreakPlan as BreakPlan,
};

#[derive(Clone, Debug, PartialEq)]
pub struct LineBreakResult {
    pub breaks: Vec<BreakDecision>,
    pub demerits: i32,
    pub tape: ParagraphTape<'static>,
    pub last_line_fill: Option<GlueSpec>,
    pub memory: BreakMemoryPlan,
}

/// One paragraph's immutable topology and line-breaking analysis.
///
/// The tape is deliberately replay-independent: it contains only native node
/// channels and values derived from them.  Break searches, diagnostics, and
/// post-line-break materialization all consume this same analysis.
#[derive(Clone, Debug, PartialEq)]
pub struct ParagraphTape<'a> {
    source: ParagraphSource<'a>,
    break_sites: Vec<BreakSite>,
    materialization: Vec<MaterializationAction>,
    par_fill_override: Option<GlueSpec>,
}

#[derive(Clone, Debug, PartialEq)]
enum ParagraphSource<'a> {
    Owned(NodeSequence),
    BorrowedMirrored(&'a [Node]),
    BorrowedArena(NodeCursor<'a>),
    ArenaId {
        semantic: PageNodeSequenceId,
        physical: PageNodeSequenceId,
        physical_boundaries: Option<Vec<usize>>,
        semantic_high_cell_lineages: Vec<DirectHighCellLineages>,
        physical_high_cell_lineages: Vec<DirectHighCellLineages>,
    },
}

/// Move-only production description of an arena-backed paragraph.
///
/// Node payload remains in the page-material arena. The vectors contain only
/// breakpoint and allocator scalar evidence assembled by the line breaker.
pub struct ArenaParagraphMaterialization {
    pub semantic: PageNodeSequenceId,
    pub diagnostic: Option<PageNodeSequenceId>,
    pub diagnostic_boundaries: Option<Vec<usize>>,
    pub semantic_high_cell_lineages: Vec<DirectHighCellLineages>,
    pub diagnostic_high_cell_lineages: Option<Vec<DirectHighCellLineages>>,
    pub actions: Vec<MaterializationAction>,
    pub par_fill_override: Option<GlueSpec>,
}

#[derive(Clone, Debug, PartialEq)]
struct BreakSite {
    breakpoint: Breakpoint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TraceSpan {
    display_end: usize,
    next_start: usize,
    display_suffix: Option<PageListId>,
    breakpoint: TraceBreakpoint,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaterializationAction {
    Copy,
    Discretionary,
    BreakDiscardable,
    BreakMath,
}

impl ParagraphTape<'static> {
    #[must_use]
    pub fn analyze<S: TypesetState>(
        state: &S,
        sequence: NodeSequence,
        params: &LineBreakParams,
    ) -> Self {
        let nodes = sequence.semantic();
        let nodes = NodeCursor::owned(nodes);
        let mut analyzer = LegalBreakpoints::new(state, nodes, params);
        let break_sites = analyzer.collect_direct();
        let materialization = analyzer.materialization;
        Self {
            source: ParagraphSource::Owned(sequence),
            break_sites,
            materialization,
            par_fill_override: None,
        }
    }

    /// Analyzes an arena coordinate through short-lived borrowed views. The
    /// resulting tape stores only the compact coordinate and scalar/index
    /// scratch, so it does not keep the execution context borrowed.
    #[must_use]
    pub fn analyze_arena_id<S: TypesetState>(
        state: &S,
        sequence: PageNodeSequenceId,
        params: &LineBreakParams,
    ) -> Self {
        Self::analyze_arena_projection_ids(state, sequence, sequence, None, params)
    }

    /// Analyzes distinct semantic and TeX-physical arena projections while
    /// retaining only coordinates plus scalar/index lineage scratch.
    ///
    /// `physical_boundaries` maps every semantic boundary, including the
    /// terminal boundary, to its physical-channel position. A mirrored input
    /// passes `None` and shares the same coordinate for both channels.
    #[must_use]
    pub fn analyze_arena_projection_ids<S: TypesetState>(
        state: &S,
        semantic: PageNodeSequenceId,
        physical: PageNodeSequenceId,
        physical_boundaries: Option<Vec<usize>>,
        params: &LineBreakParams,
    ) -> Self {
        if let Some(boundaries) = &physical_boundaries {
            assert_eq!(boundaries.len(), semantic.len() + 1);
            assert_eq!(boundaries.first(), Some(&0));
            assert_eq!(boundaries.last(), Some(&physical.len()));
            assert!(boundaries.windows(2).all(|pair| pair[0] <= pair[1]));
        } else {
            assert_eq!(semantic.len(), physical.len());
        }
        let view = state
            .page_node_sequence(semantic)
            .expect("paragraph sequence belongs to the typesetting page arena");
        let semantic_high_cell_lineages =
            tex_state::node_sequence::borrowed_mirrored_high_cell_lineages_cursor(view);
        let physical_high_cell_lineages = if semantic == physical {
            semantic_high_cell_lineages.clone()
        } else {
            tex_state::node_sequence::borrowed_mirrored_high_cell_lineages_cursor(
                state
                    .page_node_sequence(physical)
                    .expect("physical paragraph sequence belongs to the page arena"),
            )
        };
        let nodes = view;
        let mut analyzer = LegalBreakpoints::new(state, nodes, params);
        let break_sites = analyzer.collect_direct();
        let materialization = analyzer.materialization;
        Self {
            source: ParagraphSource::ArenaId {
                semantic,
                physical,
                physical_boundaries,
                semantic_high_cell_lineages,
                physical_high_cell_lineages,
            },
            break_sites,
            materialization,
            par_fill_override: None,
        }
    }
}

impl<'a> ParagraphTape<'a> {
    /// Analyzes an immutable paragraph directly through its owner's borrow.
    ///
    /// The source payload remains outside the tape and is never cloned merely
    /// to run breakpoint search or post-line materialization.
    #[must_use]
    pub fn analyze_borrowed<S: TypesetState>(
        state: &S,
        nodes: &'a [Node],
        params: &LineBreakParams,
    ) -> Self {
        let source = nodes;
        let nodes = NodeCursor::owned(nodes);
        let mut analyzer = LegalBreakpoints::new(state, nodes, params);
        let break_sites = analyzer.collect_direct();
        let materialization = analyzer.materialization;
        Self {
            source: ParagraphSource::BorrowedMirrored(source),
            break_sites,
            materialization,
            par_fill_override: None,
        }
    }

    /// Analyzes a direct or flat-composite page-arena sequence without
    /// materializing its immutable node payload into a contiguous buffer.
    #[must_use]
    pub fn analyze_arena<S: TypesetState>(
        state: &S,
        sequence: NodeCursor<'a>,
        params: &LineBreakParams,
    ) -> Self {
        let nodes = sequence;
        let mut analyzer = LegalBreakpoints::new(state, nodes, params);
        let break_sites = analyzer.collect_direct();
        let materialization = analyzer.materialization;
        Self {
            source: ParagraphSource::BorrowedArena(sequence),
            break_sites,
            materialization,
            par_fill_override: None,
        }
    }

    #[must_use]
    pub fn nodes<'state, S: TypesetState>(&'state self, state: &'state S) -> NodeCursor<'state> {
        match &self.source {
            ParagraphSource::Owned(sequence) => NodeCursor::owned(sequence.semantic()),
            ParagraphSource::BorrowedMirrored(nodes) => NodeCursor::owned(nodes),
            ParagraphSource::BorrowedArena(sequence) => *sequence,
            ParagraphSource::ArenaId { semantic, .. } => state
                .page_node_sequence(*semantic)
                .expect("paragraph sequence remains live while its tape is consumed"),
        }
    }

    pub fn replace_last_par_fill(&mut self, spec: GlueSpec) {
        self.par_fill_override = Some(spec);
    }

    /// Consumes the production arena-backed tape without materializing either
    /// node channel. Pure slice and owned adapters deliberately remain separate.
    #[must_use]
    pub fn into_arena_materialization(self) -> Option<ArenaParagraphMaterialization> {
        let Self {
            source,
            materialization,
            par_fill_override,
            ..
        } = self;
        let ParagraphSource::ArenaId {
            semantic,
            physical,
            physical_boundaries,
            semantic_high_cell_lineages,
            physical_high_cell_lineages,
        } = source
        else {
            return None;
        };
        let has_diagnostic_projection = semantic != physical || physical_boundaries.is_some();
        Some(ArenaParagraphMaterialization {
            semantic,
            diagnostic: has_diagnostic_projection.then_some(physical),
            diagnostic_boundaries: has_diagnostic_projection
                .then_some(physical_boundaries)
                .flatten(),
            semantic_high_cell_lineages,
            diagnostic_high_cell_lineages: has_diagnostic_projection
                .then_some(physical_high_cell_lineages),
            actions: materialization,
            par_fill_override,
        })
    }

    #[must_use]
    pub fn into_semantic_nodes<S: TypesetState>(self, state: &S) -> Vec<Node> {
        match self.source {
            ParagraphSource::Owned(sequence) => sequence.into_semantic(),
            ParagraphSource::BorrowedMirrored(nodes) => nodes.to_vec(),
            ParagraphSource::BorrowedArena(sequence) => {
                let mut nodes = Vec::with_capacity(sequence.len());
                sequence.for_each(|node| nodes.push(node.clone()));
                nodes
            }
            ParagraphSource::ArenaId { semantic, .. } => state
                .page_node_sequence(semantic)
                .expect("paragraph sequence remains live while its tape is consumed")
                .iter()
                .cloned()
                .collect(),
        }
    }
}

/// Detached TeX82 `\tracingparagraphs` evidence produced by the pure breaker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LineBreakTrace {
    Pass(LineBreakPass),
    Feasible {
        display: core::ops::Range<usize>,
        display_suffix: Option<PageListId>,
        breakpoint: TraceBreakpoint,
        via: usize,
        badness: Option<i32>,
        penalty: i32,
        demerits: Option<i32>,
    },
    Active {
        serial: usize,
        line: usize,
        fitness: i32,
        hyphenated: bool,
        total_demerits: i32,
        /// e-TeX's additional active-node evidence when its last-line-fit
        /// algorithm is enabled (e-TeX change-file section 38.846).
        last_line_fit: Option<LastLineFitTrace>,
        previous: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LastLineFitTrace {
    pub shortfall: Scaled,
    pub glue: Scaled,
    pub terminal: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineBreakPass {
    First,
    Second,
    Emergency,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceBreakpoint {
    Glue,
    Penalty,
    Discretionary,
    Kern,
    Math,
    Paragraph,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BrokenLine {
    pub nodes: Vec<Node>,
    /// TeX-physical topology retained for diagnostics only.
    pub physical_nodes: Vec<Node>,
    /// Allocator-only identities for direct high-memory cells in `nodes`.
    pub high_cell_lineages: Vec<tex_state::node_sequence::DirectHighCellLineage>,
    /// Allocator-only identities for direct high-memory cells in the retained
    /// TeX-physical diagnostic predecessor.
    pub physical_high_cell_lineages: Vec<tex_state::node_sequence::DirectHighCellLineage>,
    pub penalty_after: Option<i32>,
    pub hyphenated: bool,
    pub dimensions: LineDimensions,
}

pub trait HyphenationHook<S: TypesetState> {
    fn hyphenate(&mut self, nodes: &[Node]) -> Vec<Node>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoHyphenation;

impl<S: TypesetState> HyphenationHook<S> for NoHyphenation {
    fn hyphenate(&mut self, nodes: &[Node]) -> Vec<Node> {
        nodes.to_vec()
    }
}

pub fn line_break<S, H>(
    state: &S,
    nodes: &[Node],
    params: LineBreakParams,
    hyphenation: &mut H,
) -> LineBreakResult
where
    S: TypesetState,
    H: HyphenationHook<S>,
{
    let tape = ParagraphTape::analyze(state, NodeSequence::mirrored(nodes.to_vec()), &params);
    if let Some(plan) = try_tape_without_hyphenation(state, &tape, &params) {
        return plan_with_tape(plan, tape);
    }

    let hyphenated = hyphenation.hyphenate(nodes);
    let tape = ParagraphTape::analyze(state, NodeSequence::mirrored(hyphenated), &params);
    let plan = break_hyphenated_tape(state, &tape, &params);
    plan_with_tape(plan, tape)
}

pub fn plan_with_tape(plan: BreakPlan, tape: ParagraphTape<'static>) -> LineBreakResult {
    LineBreakResult {
        breaks: plan.breaks,
        demerits: plan.demerits,
        tape,
        last_line_fill: plan.last_line_fill,
        memory: plan.memory,
    }
}

/// Tries TeX82's pretolerance pass without requesting automatic hyphenation.
///
/// Returning `None` means the caller must materialize automatic
/// discretionary nodes before running the tolerance and emergency passes.
pub fn try_line_break_without_hyphenation<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    params: &LineBreakParams,
) -> Option<BreakPlan> {
    let tape = ParagraphTape::analyze(state, NodeSequence::mirrored(nodes.to_vec()), params);
    try_tape_without_hyphenation(state, &tape, params)
}

pub fn try_tape_without_hyphenation<S: TypesetState>(
    state: &S,
    tape: &ParagraphTape<'_>,
    params: &LineBreakParams,
) -> Option<BreakPlan> {
    (params.pretolerance >= 0)
        .then(|| run_pass(state, tape, params, params.pretolerance, false, false, None))
        .flatten()
}

pub fn try_line_break_without_hyphenation_traced<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    params: &LineBreakParams,
) -> (Option<BreakPlan>, Vec<LineBreakTrace>) {
    let tape = ParagraphTape::analyze(state, NodeSequence::mirrored(nodes.to_vec()), params);
    try_tape_without_hyphenation_traced(state, &tape, params)
}

pub fn try_tape_without_hyphenation_traced<S: TypesetState>(
    state: &S,
    tape: &ParagraphTape<'_>,
    params: &LineBreakParams,
) -> (Option<BreakPlan>, Vec<LineBreakTrace>) {
    let mut trace = Vec::new();
    // Produce §851--§854's canonical diagnostic evidence in a detached pass.
    // The returned plan remains the ordinary planner's result, so enabling
    // `\tracingparagraphs` cannot alter paragraph geometry.
    let plan = (params.pretolerance >= 0)
        .then(|| {
            trace.push(LineBreakTrace::Pass(LineBreakPass::First));
            let _ = run_pass(
                state,
                tape,
                params,
                params.pretolerance,
                false,
                false,
                Some(&mut trace),
            );
            run_pass(state, tape, params, params.pretolerance, false, false, None)
        })
        .flatten();
    (plan, trace)
}

/// Runs TeX82's tolerance and emergency passes on an already-hyphenated list.
pub fn line_break_hyphenated<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    params: &LineBreakParams,
) -> BreakPlan {
    let tape = ParagraphTape::analyze(state, NodeSequence::mirrored(nodes.to_vec()), params);
    break_hyphenated_tape(state, &tape, params)
}

pub fn break_hyphenated_tape<S: TypesetState>(
    state: &S,
    tape: &ParagraphTape<'_>,
    params: &LineBreakParams,
) -> BreakPlan {
    let second = run_pass(
        state,
        tape,
        params,
        params.tolerance,
        false,
        params.emergency_stretch.raw() <= 0,
        None,
    );
    if let Some(result) = second {
        return result;
    }

    run_pass(state, tape, params, params.tolerance, true, true, None)
        .expect("final line-breaking pass always permits an artificial demerits path")
}

pub fn line_break_hyphenated_traced<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    params: &LineBreakParams,
    trace: Vec<LineBreakTrace>,
) -> (BreakPlan, Vec<LineBreakTrace>) {
    let tape = ParagraphTape::analyze(state, NodeSequence::mirrored(nodes.to_vec()), params);
    break_hyphenated_tape_traced(state, &tape, params, trace)
}

pub fn break_hyphenated_tape_traced<S: TypesetState>(
    state: &S,
    tape: &ParagraphTape<'_>,
    params: &LineBreakParams,
    mut trace: Vec<LineBreakTrace>,
) -> (BreakPlan, Vec<LineBreakTrace>) {
    // As above, diagnostic admission is replayed independently so the
    // observational switch remains semantically inert.
    // TeX82 §816 prints `@secondpass` only when a failed pretolerance pass
    // transitions into the hyphenating pass. With `pretolerance<0`, TeX opens
    // the diagnostic in the second pass directly and prints no pass label.
    if params.pretolerance >= 0 {
        trace.push(LineBreakTrace::Pass(LineBreakPass::Second));
    }
    let _ = run_pass(
        state,
        tape,
        params,
        params.tolerance,
        false,
        params.emergency_stretch.raw() <= 0,
        Some(&mut trace),
    );
    let second = run_pass(
        state,
        tape,
        params,
        params.tolerance,
        false,
        params.emergency_stretch.raw() <= 0,
        None,
    );
    if let Some(result) = second {
        return (result, trace);
    }
    trace.push(LineBreakTrace::Pass(LineBreakPass::Emergency));
    let _ = run_pass(
        state,
        tape,
        params,
        params.tolerance,
        true,
        true,
        Some(&mut trace),
    );
    let result = run_pass(state, tape, params, params.tolerance, true, true, None)
        .expect("final line-breaking pass always permits an artificial demerits path");
    (result, trace)
}

mod post;
mod widths;

pub use post::{LineMaterializer, line_penalty_after, post_line_break, post_line_break_owned};

#[cfg(test)]
use widths::line_widths_nodes;
use widths::{
    DiscretionaryWidths, Widths, add_node_width_value, line_badness, line_widths_cursor,
    line_widths_view,
};

/// Validates pdfTeX's paragraph-wide expansion-step and limit invariants.
///
/// Callers need this only when `pdf_adjust_spacing > 1`; mode 1 performs
/// final-line expansion independently and permits unlike font settings.
pub fn validate_paragraph_expansion<S: TypesetState>(
    state: &S,
    nodes: NodeCursor<'_>,
) -> Result<Option<(i32, i32)>, crate::expansion::FontExpansionError> {
    let mut paragraph = crate::expansion::ParagraphExpansion::default();
    observe_expansion_fonts(state, nodes, &mut paragraph)?;
    Ok(paragraph.steps())
}

fn observe_expansion_fonts<S: TypesetState>(
    state: &S,
    nodes: NodeCursor<'_>,
    paragraph: &mut crate::expansion::ParagraphExpansion,
) -> Result<(), crate::expansion::FontExpansionError> {
    let mut error = None;
    nodes.for_each(|node| {
        if error.is_some() {
            return;
        }
        match node {
            Node::Char { font, .. } | Node::Lig { font, .. } => {
                if let Some(spec) = state.font_expansion_spec(*font)
                    && let Err(found) = paragraph.observe(spec)
                {
                    error = Some(found);
                }
            }
            Node::Disc {
                pre, post, replace, ..
            } => {
                for list in [pre, post, replace] {
                    let owned = state.page_nodes(*list).iter().cloned().collect::<Vec<_>>();
                    if let Err(found) =
                        observe_expansion_fonts(state, NodeCursor::owned(&owned), paragraph)
                    {
                        error = Some(found);
                        break;
                    }
                }
            }
            _ => {}
        }
    });
    error.map_or(Ok(()), Err)
}

/// Plans pdfTeX's normalized signed expansion ratio for one finalized line.
///
/// This is pure: execution uses the result to intern and substitute discrete
/// generated fonts before performing ordinary final hpack.
#[must_use]
pub fn plan_line_expansion<S: TypesetState>(state: &S, nodes: &[Node], target: Scaled) -> i32 {
    plan_line_expansion_cursor(state, NodeCursor::owned(nodes), target)
}

/// Arena-backed form of [`plan_line_expansion`].
#[must_use]
pub fn plan_line_expansion_cursor<S: TypesetState>(
    state: &S,
    nodes: NodeCursor<'_>,
    target: Scaled,
) -> i32 {
    // pdftex.web §823: final `hpack(..., cal_expand_ratio)` ignores
    // discretionary nodes. Post-line-break material already places the chosen
    // replacement nodes in the physical line, so traversing `replace` again
    // would double-count them in the final expansion ratio.
    let widths = line_widths_cursor(
        state,
        nodes,
        0,
        nodes.len(),
        true,
        DiscretionaryWidths::Ignore,
    );
    let shortfall = WideScaled::from_scaled(target)
        .checked_sub(widths.natural)
        .expect("line shortfall fits the wide scaled domain")
        .to_scaled()
        .expect("a finalized line width fits the stored scaled domain");
    crate::expansion::line_expansion_ratio(
        shortfall,
        crate::expansion::ExpansionCapacity {
            stretch: widths
                .font_stretch
                .to_scaled()
                .expect("finalized line expansion capacity fits Scaled"),
            shrink: widths
                .font_shrink
                .to_scaled()
                .expect("finalized line expansion capacity fits Scaled"),
        },
        widths.has_infinite_adjustment(i64::from(shortfall.raw())),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Fitness {
    Tight = 0,
    Decent = 1,
    Loose = 2,
    VeryLoose = 3,
}

#[derive(Clone, Copy, Debug)]
struct Candidate {
    serial: usize,
    position: usize,
    break_site: u32,
    penalty: i32,
    line: usize,
    fitness: Fitness,
    path_demerits: i32,
    passive: Option<usize>,
    previous: Option<usize>,
    hyphenated: bool,
    line_shortfall: Scaled,
    line_glue: Scaled,
}

const INITIAL_BREAK_SITE: u32 = u32::MAX;

impl Candidate {
    fn breakpoint<'a>(&self, break_sites: &'a [BreakSite]) -> Option<&'a Breakpoint> {
        (self.break_site != INITIAL_BREAK_SITE).then(|| {
            &break_sites[usize::try_from(self.break_site).expect("break-site index fits usize")]
                .breakpoint
        })
    }

    fn width_position(&self, break_sites: &[BreakSite]) -> usize {
        self.breakpoint(break_sites)
            .map_or(0, |bp| bp.next_position)
    }

    fn start_width(&self, break_sites: &[BreakSite]) -> Widths {
        self.breakpoint(break_sites)
            .map_or_else(Widths::zero, |bp| bp.next_width)
    }
}

#[derive(Clone, Copy, Debug)]
struct PassiveRoute {
    decision: BreakDecision,
    previous: Option<usize>,
    serial: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Breakpoint {
    position: usize,
    /// Exclusive source boundary of material visible at this line's right
    /// edge. pdfTeX §1025 starts protrusion discovery immediately before the
    /// current breakpoint, except that a discretionary exposes its pre-break
    /// list as the line tail.
    protrusion_end: usize,
    penalty: i32,
    hyphenated: bool,
    add_width: Widths,
    line_width: Widths,
    next_position: usize,
    next_width: Widths,
}

fn run_pass<S: TypesetState>(
    state: &S,
    tape: &ParagraphTape<'_>,
    params: &LineBreakParams,
    tolerance: i32,
    emergency: bool,
    final_pass: bool,
    mut trace: Option<&mut Vec<LineBreakTrace>>,
) -> Option<BreakPlan> {
    let mut memory = BreakMemoryPlan::default();
    memory.search.push(BreakMemoryEvent::Allocate {
        owner: BreakMemoryOwner::Active(0),
        words: 3,
    });
    let nodes = tape.nodes(state);
    let canonical_trace_admission = trace.is_some();
    let mut background = Widths::from_glue(params.left_skip);
    background.add_assign(Widths::from_glue(params.right_skip));
    let mut active = vec![Candidate {
        serial: 0,
        position: 0,
        break_site: INITIAL_BREAK_SITE,
        penalty: 0,
        line: 0,
        fitness: Fitness::Decent,
        path_demerits: 0,
        passive: None,
        previous: None,
        hyphenated: false,
        line_shortfall: Scaled::from_raw(0),
        line_glue: Scaled::from_raw(0),
    }];
    let mut next_active = Vec::new();
    let mut passive: Vec<PassiveRoute> = Vec::new();
    let mut next_serial = 1;
    let last_line_fit = LastLineFit::new(params, background);
    let easy_line = tex_easy_line(params);
    let expansion_steps = (params.pdf_adjust_spacing > 1)
        .then_some(params.expansion_steps)
        .flatten();
    let mut displayed_through = 0;

    for (break_site, site) in tape.break_sites.iter().enumerate() {
        let break_site = u32::try_from(break_site).expect("paragraph break-site count exceeds u32");
        let bp = site.breakpoint;
        let mut trace_span = None;
        // Background and discretionary material depend only on this
        // breakpoint. Combine them once instead of once per active route.
        let mut breakpoint_width = bp.line_width;
        breakpoint_width.add_assign(background);
        breakpoint_width.add_assign(bp.add_width);
        let prior_active_len = active.len();
        let mut survivor_count = 0;
        let forced = bp.penalty <= EJECT_PENALTY;
        let mut feasible_traces = Vec::new();
        let mut traced_feasible = false;
        for active_index in 0..prior_active_len {
            let active_candidate = active[active_index];
            // TeX82 §822's break width can advance past discardable nodes,
            // but that adjusted width cursor does not remove those nodes from
            // §§851--854's later active-list traversal. In particular, a
            // glue break may be followed immediately by a forced penalty;
            // the route through the glue still has to be considered there.
            // Only an already chosen breakpoint at or beyond this syntactic
            // position would make the chain non-forward.
            if active_candidate.position >= bp.position {
                active[survivor_count] = active_candidate;
                survivor_count += 1;
                continue;
            }
            let mut widths = breakpoint_width.sub(active_candidate.start_width(&tape.break_sites));
            let target = params.shape.dimensions(active_candidate.line + 1).width;
            let extra = if emergency {
                params.emergency_stretch
            } else {
                Scaled::from_raw(0)
            };
            // TeX adds emergency stretch to the finite-stretch component of
            // the line-breaking background. This also makes it participate in
            // e-TeX's last-line adjustment ratio.
            widths.add_normal_stretch(extra);
            let terminal = forced && bp.position >= nodes.len();
            let scoring_target = if params.pdf_protrude_chars > 1 {
                let start = active_candidate
                    .width_position(&tape.break_sites)
                    .min(nodes.len());
                let end = bp.protrusion_end.min(nodes.len()).max(start);
                let protrusion =
                    crate::protrusion::line_protrusion_cursor(state, nodes, start, end);
                if expansion_steps.is_some() {
                    let (stretch, shrink) = protrusion.margin_variation();
                    widths.add_font_variation(stretch, shrink);
                }
                target
                    .checked_add(protrusion.total())
                    .expect("pdfTeX protruded line target fits Scaled")
            } else {
                target
            };
            let normal_b =
                line_badness(widths, scoring_target, Scaled::from_raw(0), expansion_steps);
            let fitted = terminal
                .then(|| last_line_fit.badness(&active_candidate, widths, scoring_target))
                .flatten();
            let (b, fitness) = fitted
                .map(|(bad, fitness, _)| (bad, fitness))
                .unwrap_or_else(|| {
                    let badness = normal_b.min(INF_BAD);
                    (
                        normal_b,
                        fitness_class(
                            badness,
                            widths.natural.raw(),
                            i64::from(scoring_target.raw()),
                        ),
                    )
                });
            let artificial = final_pass
                && survivor_count == 0
                && active.len() == prior_active_len
                && active_index + 1 == prior_active_len
                && (b > INF_BAD || forced);
            let deactivates = b > INF_BAD || forced;
            let feasible = bp.penalty < INF_PENALTY && (artificial || b <= tolerance);
            if feasible {
                let badness = b.min(INF_BAD);
                let dem = if artificial {
                    active_candidate.path_demerits
                } else {
                    compute_demerits(
                        params,
                        &active_candidate,
                        badness,
                        bp.penalty,
                        fitness,
                        bp,
                        terminal,
                    )
                };
                let candidate = Candidate {
                    serial: if canonical_trace_admission {
                        0
                    } else {
                        next_serial
                    },
                    position: bp.position,
                    break_site,
                    penalty: bp.penalty,
                    line: active_candidate.line + 1,
                    fitness,
                    path_demerits: dem,
                    passive: None,
                    previous: active_candidate.passive,
                    hyphenated: bp.hyphenated,
                    line_shortfall: if terminal && fitted.is_none() {
                        Scaled::from_raw(0)
                    } else {
                        line_shortfall_for_route(scoring_target, widths.natural)
                    },
                    line_glue: fitted.map_or_else(
                        || candidate_line_glue(widths, scoring_target, b),
                        |(_, _, adjustment)| adjustment,
                    ),
                };
                if trace.is_some() {
                    let trace_span = trace_span.get_or_insert_with(|| {
                        let display_end = trace_display_end(state, nodes, bp);
                        TraceSpan {
                            display_end,
                            next_start: trace_display_next_start(state, nodes, bp, display_end),
                            display_suffix: trace_display_suffix(nodes, bp),
                            breakpoint: trace_breakpoint(nodes, bp),
                        }
                    });
                    traced_feasible = true;
                    feasible_traces.push((
                        line_number_class(candidate.line, easy_line),
                        LineBreakTrace::Feasible {
                            // TeX82 §851 temporarily terminates the list at
                            // `cur_p` and calls `short_display`, so the displayed
                            // range includes the breakpoint node itself. This is
                            // visible for glue (a trailing space) and
                            // discretionaries (their pre/post lists), even though
                            // width accounting stops before those nodes.
                            display: displayed_through..trace_span.display_end,
                            display_suffix: trace_span.display_suffix,
                            breakpoint: trace_span.breakpoint,
                            via: active_candidate.passive.map_or(0, |id| passive[id].serial),
                            badness: (b <= INF_BAD).then_some(b),
                            penalty: bp.penalty,
                            demerits: (!artificial).then(|| {
                                compute_route_demerits(
                                    params,
                                    &active_candidate,
                                    badness,
                                    bp.penalty,
                                    fitness,
                                    bp,
                                    terminal,
                                )
                            }),
                        },
                    ));
                    // TeX82 §855 advances `printed_node` across a
                    // discretionary's replacement nodes. Umber retains those
                    // nodes in the flattened paragraph as well as the disc's
                    // side list, so keep them out of the next trace fragment.
                    // Consecutive discretionaries are still distinct
                    // breakpoints; their shared replacement run begins only
                    // after the cluster.
                    // Every later feasible route to this same `cur_p` sees
                    // `printed_node=cur_p` and therefore prints no fragment.
                    // The detached cursor for the next breakpoint is restored
                    // after this active-list traversal finishes.
                    displayed_through = trace_span.display_end;
                }
                if !canonical_trace_admission {
                    next_serial += 1;
                }
                record_best_route(&mut active, prior_active_len, candidate, Some(easy_line));
            }
            if !deactivates {
                active[survivor_count] = active_candidate;
                survivor_count += 1;
            } else {
                memory
                    .search
                    .push(BreakMemoryEvent::Free(BreakMemoryOwner::Active(
                        u32::try_from(active_candidate.serial)
                            .expect("active-node serial exceeds u32"),
                    )));
            }
        }
        let winner_count = retain_competitive_routes(
            &mut active,
            prior_active_len,
            params.adj_demerits,
            easy_line,
        );
        let mut active_traces = Vec::new();
        for candidate in &mut active[prior_active_len..] {
            if canonical_trace_admission {
                candidate.serial = next_serial;
                next_serial += 1;
            }
            let passive_id = passive.len();
            memory.search.push(BreakMemoryEvent::Allocate {
                owner: BreakMemoryOwner::Passive(
                    u32::try_from(passive_id).expect("passive-node id exceeds u32"),
                ),
                words: 2,
            });
            memory.search.push(BreakMemoryEvent::Allocate {
                owner: BreakMemoryOwner::Active(
                    u32::try_from(candidate.serial).expect("active-node serial exceeds u32"),
                ),
                words: 3,
            });
            passive.push(PassiveRoute {
                decision: BreakDecision {
                    position: candidate.position.min(nodes.len()),
                    penalty: candidate.penalty,
                    hyphenated: candidate.hyphenated,
                },
                previous: candidate.previous,
                serial: candidate.serial,
            });
            candidate.passive = Some(passive_id);
            if trace.is_some() {
                active_traces.push((
                    line_number_class(candidate.line, easy_line),
                    LineBreakTrace::Active {
                        serial: candidate.serial,
                        // TeX82 §§816/854 stores absolute `line_number` on
                        // active nodes, initialized from `prev_graf+1`.
                        // The pure breaker keeps relative candidates and
                        // carries `prev_graf` as the shape's line offset, so
                        // restore that offset in detached trace evidence.
                        line: candidate
                            .line
                            .checked_add(params.shape.line_offset)
                            .expect("line number exceeds usize"),
                        fitness: trace_fitness(candidate.fitness),
                        hyphenated: candidate.hyphenated
                            || (candidate.penalty <= EJECT_PENALTY
                                && candidate.position >= nodes.len()),
                        total_demerits: candidate.path_demerits,
                        last_line_fit: last_line_fit.enabled.then_some(LastLineFitTrace {
                            shortfall: candidate.line_shortfall,
                            glue: candidate.line_glue,
                            terminal: candidate.position >= nodes.len(),
                        }),
                        previous: candidate.previous.map_or(0, |id| passive[id].serial),
                    },
                ));
            }
        }
        if let Some(events) = trace.as_deref_mut() {
            // TeX82 §§851--854 creates and reports the champions for one
            // line-number class before it examines feasible routes in the
            // next class. The pure breaker computes all champions in one
            // batch, so restore that observable class boundary here.
            let mut active_traces = active_traces.into_iter().peekable();
            for (class, event) in feasible_traces {
                while active_traces
                    .peek()
                    .is_some_and(|(active_class, _)| *active_class < class)
                {
                    if let Some((_, active_event)) = active_traces.next() {
                        events.push(active_event);
                    }
                }
                events.push(event);
            }
            events.extend(active_traces.map(|(_, event)| event));
        }
        if traced_feasible {
            displayed_through = trace_span
                .expect("a feasible traced route computes its display span")
                .next_start;
        }
        merge_active_candidates(
            &mut active,
            survivor_count,
            prior_active_len,
            winner_count,
            &mut next_active,
            params,
            easy_line,
        );
    }

    let chosen = choose_final(&active, params.looseness)?;
    let best = active
        .iter()
        .min_by_key(|candidate| candidate.path_demerits)?;
    let actual_looseness = active[chosen].line as i32 - best.line as i32;
    if !final_pass && actual_looseness != params.looseness {
        return None;
    }
    memory.cleanup.extend(active.iter().map(|candidate| {
        BreakMemoryEvent::Free(BreakMemoryOwner::Active(
            u32::try_from(candidate.serial).expect("active-node serial exceeds u32"),
        ))
    }));
    memory.cleanup.extend((0..passive.len()).rev().map(|id| {
        BreakMemoryEvent::Free(BreakMemoryOwner::Passive(
            u32::try_from(id).expect("passive-node id exceeds u32"),
        ))
    }));
    Some(reconstruct(active[chosen], &passive, last_line_fit, memory))
}

fn trace_display_suffix(nodes: NodeCursor<'_>, bp: Breakpoint) -> Option<PageListId> {
    // §903's boundary-kern reconstitution keeps the displaced ligature in
    // the automatic discretionary's side list. TeX82's linked list exposes
    // it to §851; Umber carries it as this detached trace suffix instead.
    if !matches!(
        bp.position
            .checked_sub(2)
            .and_then(|index| nodes.owned_node(index)),
        Some(Node::Kern {
            kind: KernKind::Font,
            ..
        })
    ) {
        return None;
    }
    let Node::Disc {
        kind: tex_state::node::DiscKind::AutomaticHyphen,
        replace,
        ..
    } = nodes.owned_node(bp.position.checked_sub(1)?)?
    else {
        return None;
    };
    Some(*replace)
}

fn trace_display_end(state: &impl TypesetState, nodes: NodeCursor<'_>, bp: Breakpoint) -> usize {
    let Some(Node::Disc { replace, .. }) = bp
        .position
        .checked_sub(1)
        .and_then(|index| nodes.owned_node(index))
    else {
        return bp.position;
    };
    if matches!(
        bp.position
            .checked_sub(2)
            .and_then(|index| nodes.owned_node(index)),
        Some(Node::Kern {
            kind: KernKind::Font,
            ..
        })
    ) {
        // The flattened paragraph retains both the boundary kern and the
        // displaced replacement after the discretionary. The trace slice
        // consumes the kern; `trace_display_next_start` advances over the
        // replacement after its detached suffix has been rendered.
        return bp
            .position
            .saturating_add(state.page_nodes(*replace).len())
            .min(nodes.len());
    }
    if !matches!(
        bp.position
            .checked_sub(2)
            .and_then(|index| nodes.owned_node(index)),
        Some(Node::Disc { .. })
    ) || matches!(nodes.owned_node(bp.position), Some(Node::Disc { .. }))
    {
        return bp.position;
    }
    let mut replacement_count = state.page_nodes(*replace).len();
    let mut index = bp.position - 1;
    while let Some(previous) = index.checked_sub(1) {
        let Some(Node::Disc { replace, .. }) = nodes.owned_node(previous) else {
            break;
        };
        replacement_count = replacement_count.saturating_add(state.page_nodes(*replace).len());
        index = previous;
    }
    bp.position
        .saturating_add(replacement_count)
        .min(nodes.len())
}

fn trace_display_next_start(
    state: &impl TypesetState,
    nodes: NodeCursor<'_>,
    bp: Breakpoint,
    display_end: usize,
) -> usize {
    if trace_display_suffix(nodes, bp).is_some() {
        display_end.saturating_add(1).min(nodes.len())
    } else if let Some(Node::Disc { replace, .. }) = bp
        .position
        .checked_sub(1)
        .and_then(|index| nodes.owned_node(index))
        && display_end.saturating_sub(bp.position) == state.page_nodes(*replace).len()
    {
        // §851's temporary link surgery may make the current structural slice
        // include nodes used to model `replace_count`; §855 skips them only
        // while displaying this discretionary. If every extended node was
        // hidden this way, none was consumed by `short_display`, so the next
        // detached fragment begins there. A longer extension contains real
        // rendered successors and keeps `display_end` instead.
        bp.position
    } else {
        display_end
    }
}

fn trace_breakpoint(nodes: NodeCursor<'_>, bp: Breakpoint) -> TraceBreakpoint {
    if bp.penalty <= EJECT_PENALTY && bp.position >= nodes.len() {
        return TraceBreakpoint::Paragraph;
    }
    match nodes
        .owned_node(bp.position - 1)
        .expect("breakpoint belongs to paragraph")
    {
        Node::Glue { .. } => TraceBreakpoint::Glue,
        Node::Penalty(_) => TraceBreakpoint::Penalty,
        Node::Disc { .. } => TraceBreakpoint::Discretionary,
        Node::Kern { .. } => TraceBreakpoint::Kern,
        Node::MathOn(_) | Node::MathOff(_) => TraceBreakpoint::Math,
        _ => TraceBreakpoint::Glue,
    }
}

fn trace_fitness(fitness: Fitness) -> i32 {
    match fitness {
        Fitness::VeryLoose => 0,
        Fitness::Loose => 1,
        Fitness::Decent => 2,
        Fitness::Tight => 3,
    }
}

fn line_number_class(line: usize, easy_line: usize) -> usize {
    // TeX82 §848 keeps `easy_line` itself distinct; only strictly later
    // equal-width lines share the deferred class.
    line.min(easy_line.saturating_add(1))
}

fn record_best_route(
    active: &mut Vec<Candidate>,
    winner_start: usize,
    candidate: Candidate,
    easy_line: Option<usize>,
) {
    let class = easy_line.map_or(candidate.line, |easy| {
        line_number_class(candidate.line, easy)
    });
    // The active list is visited in line order, so winners are appended in
    // nondecreasing line-number class order. Only the final class can match,
    // and it contains at most one champion for each of the four fitness
    // classes. Do not rescan the growing history of earlier line classes.
    let slot = active[winner_start..]
        .iter()
        .enumerate()
        .rev()
        .take_while(|(_, current)| {
            easy_line.map_or(current.line, |easy| line_number_class(current.line, easy)) == class
        })
        .find_map(|(offset, current)| {
            (current.fitness == candidate.fitness).then_some(winner_start + offset)
        });
    if let Some(slot) = slot {
        if candidate.path_demerits <= active[slot].path_demerits {
            // TeX82 uses `d <= minimal_demerits[fit_class]`, so an equal
            // later route replaces the earlier route in its first-visit slot.
            active[slot] = candidate;
        }
    } else {
        active.push(candidate);
    }
}

fn retain_competitive_routes(
    active: &mut Vec<Candidate>,
    winner_start: usize,
    adj_demerits: i32,
    easy_line: usize,
) -> usize {
    let winner_end = active.len();
    let margin = i64::from(adj_demerits).abs();
    let mut retained = 0;
    for read in winner_start..winner_end {
        let candidate = active[read];
        let class = line_number_class(candidate.line, easy_line);
        let minimum = active[winner_start..winner_end]
            .iter()
            .filter(|other| line_number_class(other.line, easy_line) == class)
            .map(|other| other.path_demerits)
            .min()
            .expect("a winner's line-number class is nonempty");
        // TeX82 §853 admits only fitness-class champions within
        // `minimum_demerits + abs(adj_demerits)` for this line-number class.
        // The saturation matches its `awful_bad - 1` overflow guard.
        let threshold = (i64::from(minimum) + margin).min(i64::from(AWFUL_BAD - 1));
        if i64::from(candidate.path_demerits) <= threshold {
            active[winner_start + retained] = candidate;
            retained += 1;
        }
    }
    active.truncate(winner_start + retained);
    active[winner_start..].sort_unstable_by_key(|candidate| {
        (
            line_number_class(candidate.line, easy_line),
            trace_fitness(candidate.fitness),
        )
    });
    retained
}

fn tex_easy_line(params: &LineBreakParams) -> usize {
    if params.looseness != 0 {
        return usize::MAX;
    }
    if let Some(parshape) = &params.shape.parshape {
        return parshape.lines.len() - 1;
    }
    if params.shape.hang_indent.raw() == 0 {
        0
    } else {
        params.shape.hang_after.saturating_abs() as usize
    }
}

fn sort_active_candidates(active: &mut [Candidate], params: &LineBreakParams, easy_line: usize) {
    // TeX normally keeps active nodes ordered by line number and inserts a
    // new break before existing nodes in the same class. Beyond `easy_line`,
    // all equal-width lines form one deferred class and new breaks instead
    // accumulate in source order. The visit order is observable because an
    // equal demerit replaces the route recorded earlier in `try_break`.
    active.sort_unstable_by(|left, right| active_candidate_order(left, right, params, easy_line));
}

fn active_candidate_order(
    left: &Candidate,
    right: &Candidate,
    params: &LineBreakParams,
    easy_line: usize,
) -> core::cmp::Ordering {
    let effective_line = |candidate: &Candidate| {
        candidate
            .line
            .checked_add(1)
            .and_then(|line| line.checked_add(params.shape.line_offset))
            .expect("line number exceeds usize")
    };
    let left_line = effective_line(left);
    let right_line = effective_line(right);
    line_number_class(left_line, easy_line)
        .cmp(&line_number_class(right_line, easy_line))
        .then_with(|| {
            if left_line > easy_line {
                left.position.cmp(&right.position)
            } else {
                right.position.cmp(&left.position)
            }
        })
        // TeX82 §853 inserts the four fitness-class champions in ascending
        // class order. This is decisive when §858's final-pass fallback can
        // admit only the last active route at an overfull forced break.
        .then_with(|| trace_fitness(left.fitness).cmp(&trace_fitness(right.fitness)))
        // Candidate serials encode insertion/visit order. This makes the
        // comparator total while preserving stable-sort behavior for routes
        // with the same TeX active-list key.
        .then_with(|| left.serial.cmp(&right.serial))
}

fn merge_active_candidates(
    active: &mut Vec<Candidate>,
    survivor_count: usize,
    winner_start: usize,
    winner_count: usize,
    scratch: &mut Vec<Candidate>,
    params: &LineBreakParams,
    easy_line: usize,
) {
    if winner_count == 0 {
        active.truncate(survivor_count);
        return;
    }
    let winner_end = winner_start + winner_count;
    sort_active_candidates(&mut active[winner_start..winner_end], params, easy_line);
    if survivor_count == 0 {
        active.copy_within(winner_start..winner_end, 0);
        active.truncate(winner_count);
        return;
    }

    // Buffer only the small winner tail. Merging backward leaves every
    // unread survivor in place, so the much larger survivor run need not be
    // copied into scratch and back at every legal breakpoint.
    scratch.clear();
    scratch.extend_from_slice(&active[winner_start..winner_end]);
    let (mut survivor, mut winner) = (survivor_count, winner_count);
    let mut output = survivor_count + winner_count;
    while survivor > 0 && winner > 0 {
        output -= 1;
        if active_candidate_order(
            &active[survivor - 1],
            &scratch[winner - 1],
            params,
            easy_line,
        )
        .is_gt()
        {
            survivor -= 1;
            active[output] = active[survivor];
        } else {
            winner -= 1;
            active[output] = scratch[winner];
        }
    }
    if winner > 0 {
        active[..winner].copy_from_slice(&scratch[..winner]);
    }
    active.truncate(survivor_count + winner_count);
}

#[derive(Clone, Copy)]
struct LastLineFit {
    amount: i32,
    par_fill: GlueSpec,
    fill_width: [WideScaled; 3],
    enabled: bool,
}

impl LastLineFit {
    fn new(params: &LineBreakParams, background: Widths) -> Self {
        let mut fill_width = [WideScaled::ZERO; 3];
        let par_fill = params.par_fill_skip;
        let enabled = params.last_line_fit > 0
            && par_fill.stretch.raw() > 0
            && par_fill.stretch_order != tex_state::glue::Order::Normal
            && background.infinite_stretch_is_zero();
        if enabled {
            fill_width[par_fill.stretch_order as usize - 1] =
                WideScaled::from_scaled(par_fill.stretch);
        }
        Self {
            amount: params.last_line_fit,
            par_fill,
            fill_width,
            enabled,
        }
    }

    fn badness(
        self,
        previous: &Candidate,
        widths: Widths,
        target: Scaled,
    ) -> Option<(i32, Fitness, Scaled)> {
        if !self.enabled
            || previous.line_shortfall.raw() == 0
            || previous.line_glue.raw() <= 0
            // e-TeX change-file section 38.852 reaches last-line fitting
            // only from TeX's `shortfall > 0` infinite-stretch branch.
            || widths.natural.raw() >= i64::from(target.raw())
            || widths.infinite_stretch() != self.fill_width
        {
            return None;
        }
        let available = if previous.line_shortfall.raw() > 0 {
            widths.normal_stretch()
        } else {
            widths.normal_shrink()
        };
        if available.raw() <= 0 {
            return None;
        }
        let available = available
            .to_scaled()
            .expect("a feasible line's finite glue fits Scaled");
        let mut adjustment = rounded_fraction(
            available.raw(),
            previous.line_shortfall.raw(),
            previous.line_glue.raw(),
        );
        if self.amount < 1000 {
            adjustment = rounded_fraction(adjustment, self.amount, 1000);
        }
        if adjustment > 0 {
            let remaining = i64::from(target.raw()) - widths.natural.raw();
            adjustment = adjustment
                .min(i32::try_from(remaining).expect("last-line-fit adjustment fits Scaled"));
            let bad = crate::badness(Scaled::from_raw(adjustment), available);
            let fitness = if bad > 99 {
                Fitness::VeryLoose
            } else if bad > 12 {
                Fitness::Loose
            } else {
                Fitness::Decent
            };
            Some((bad, fitness, Scaled::from_raw(adjustment)))
        } else if adjustment < 0 {
            adjustment = adjustment.max(-available.raw());
            let bad = crate::badness(Scaled::from_raw(-adjustment), available);
            Some((
                bad,
                if bad > 12 {
                    Fitness::Tight
                } else {
                    Fitness::Decent
                },
                Scaled::from_raw(adjustment),
            ))
        } else {
            None
        }
    }

    fn adjusted_fill(self, chosen: &Candidate) -> Option<GlueSpec> {
        (self.enabled && chosen.line_shortfall.raw() != 0).then(|| GlueSpec {
            width: self
                .par_fill
                .width
                .checked_add(chosen.line_shortfall)
                .and_then(|width| width.checked_sub(chosen.line_glue))
                .expect("last-line-fit parfill width fits Scaled"),
            stretch: Scaled::from_raw(0),
            ..self.par_fill
        })
    }
}

fn rounded_fraction(x: i32, n: i32, d: i32) -> i32 {
    if d == 0 {
        return if (i64::from(x) * i64::from(n)).is_negative() {
            -Scaled::MAX_DIMEN.raw()
        } else {
            Scaled::MAX_DIMEN.raw()
        };
    }
    let numerator = i128::from(x) * i128::from(n);
    let denominator = i128::from(d);
    let negative = numerator.is_negative() != denominator.is_negative();
    let rounded = (numerator.abs() + denominator.abs() / 2) / denominator.abs();
    let signed = if negative { -rounded } else { rounded };
    signed.clamp(
        -i128::from(Scaled::MAX_DIMEN.raw()),
        i128::from(Scaled::MAX_DIMEN.raw()),
    ) as i32
}

fn candidate_line_glue(widths: Widths, target: Scaled, badness: i32) -> Scaled {
    let shortfall = i64::from(target.raw()) - widths.natural.raw();
    if badness > INF_BAD || widths.has_infinite_adjustment(shortfall) {
        Scaled::from_raw(0)
    } else if shortfall > 0 {
        widths
            .normal_stretch()
            .to_scaled()
            .expect("feasible line stretch fits Scaled")
    } else if shortfall < 0 {
        widths
            .normal_shrink()
            .to_scaled()
            .expect("feasible line shrink fits Scaled")
    } else {
        Scaled::from_raw(0)
    }
}

fn line_shortfall_for_route(target: Scaled, natural: WideScaled) -> Scaled {
    WideScaled::from_scaled(target)
        .checked_sub(natural)
        .expect("line shortfall fits the wide scaled domain")
        .to_scaled()
        // Only TeX's artificial final-pass route can retain an infeasible
        // line this wide. Zero disables last-line-fit reuse of that value.
        .unwrap_or(Scaled::from_raw(0))
}

fn compute_demerits(
    params: &LineBreakParams,
    active: &Candidate,
    bad: i32,
    penalty: i32,
    fitness: Fitness,
    bp: Breakpoint,
    terminal: bool,
) -> i32 {
    let route = compute_route_demerits(params, active, bad, penalty, fitness, bp, terminal);
    let dem = i64::from(route) + i64::from(active.path_demerits);
    // TeX.web's line breaker caps accumulated demerits at `awful_bad`.
    i32::try_from(dem.clamp(i64::from(i32::MIN), i64::from(AWFUL_BAD)))
        .expect("clamped demerits fit i32")
}

fn compute_route_demerits(
    params: &LineBreakParams,
    active: &Candidate,
    bad: i32,
    penalty: i32,
    fitness: Fitness,
    bp: Breakpoint,
    terminal: bool,
) -> i32 {
    let line_bad = i64::from(params.line_penalty) + i64::from(bad);
    let mut dem = if line_bad.abs() >= i64::from(INF_BAD) {
        100_000_000_i64
    } else {
        line_bad * line_bad
    };
    if penalty > 0 {
        dem += i64::from(penalty) * i64::from(penalty);
    } else if penalty > EJECT_PENALTY {
        dem -= i64::from(penalty) * i64::from(penalty);
    }
    if active.hyphenated {
        if terminal {
            dem += i64::from(params.final_hyphen_demerits);
        } else if bp.hyphenated {
            dem += i64::from(params.double_hyphen_demerits);
        }
    }
    if incompatible(active.fitness, fitness) {
        dem += i64::from(params.adj_demerits);
    }
    i32::try_from(dem).expect("one line's demerits fit i32")
}

fn discretionary_penalty(pre_is_empty: bool, params: &LineBreakParams) -> i32 {
    if pre_is_empty {
        params.ex_hyphen_penalty
    } else {
        params.hyphen_penalty
    }
}

struct LegalBreakpoints<'a, S> {
    state: &'a S,
    nodes: NodeCursor<'a>,
    params: &'a LineBreakParams,
    prefix: Widths,
    auto_breaking: bool,
    include_font_expansion: bool,
    materialization: Vec<MaterializationAction>,
}

impl<'a, S: TypesetState> LegalBreakpoints<'a, S> {
    fn new(state: &'a S, nodes: NodeCursor<'a>, params: &'a LineBreakParams) -> Self {
        Self {
            state,
            nodes,
            params,
            prefix: Widths::zero(),
            auto_breaking: true,
            include_font_expansion: params.pdf_adjust_spacing > 1,
            materialization: Vec::with_capacity(nodes.len()),
        }
    }

    fn collect_direct(&mut self) -> Vec<BreakSite> {
        let nodes = self.nodes;
        let mut output = Vec::new();
        let mut pending_start = None;
        let mut previous = None;
        let mut pending = None;
        let mut index = 0_usize;
        nodes.for_each(|node| {
            if let Some((pending_index, pending_node, pending_previous)) = pending.take() {
                self.observe_direct_node(
                    pending_index,
                    pending_node,
                    pending_previous,
                    Some(node),
                    &mut output,
                    &mut pending_start,
                );
            }
            pending = Some((index, node, previous));
            previous = Some(node);
            index += 1;
        });
        if let Some((pending_index, pending_node, pending_previous)) = pending {
            self.observe_direct_node(
                pending_index,
                pending_node,
                pending_previous,
                None,
                &mut output,
                &mut pending_start,
            );
        }
        Self::finalize_pending(&mut output, &mut pending_start, nodes.len(), self.prefix);
        if output
            .last()
            .is_none_or(|site| site.breakpoint.position < nodes.len())
        {
            output.push(BreakSite {
                breakpoint: Breakpoint {
                    position: nodes.len(),
                    protrusion_end: nodes.len(),
                    penalty: EJECT_PENALTY,
                    hyphenated: false,
                    add_width: Widths::zero(),
                    line_width: self.prefix,
                    next_position: nodes.len(),
                    next_width: self.prefix,
                },
            });
        }
        output
    }

    fn observe_direct_node(
        &mut self,
        index: usize,
        node: &Node,
        previous: Option<&Node>,
        next: Option<&Node>,
        output: &mut Vec<BreakSite>,
        pending_start: &mut Option<usize>,
    ) {
        if !is_discardable(node) {
            Self::finalize_pending(output, pending_start, index, self.prefix);
        }
        let Some((site, awaits_discardable_end)) = self.observe_node(index, node, previous, next)
        else {
            return;
        };
        if awaits_discardable_end {
            pending_start.get_or_insert(output.len());
        } else {
            debug_assert!(pending_start.is_none());
        }
        output.push(site);
    }

    fn finalize_pending(
        output: &mut [BreakSite],
        pending_start: &mut Option<usize>,
        next_position: usize,
        next_width: Widths,
    ) {
        let Some(start) = pending_start.take() else {
            return;
        };
        for site in &mut output[start..] {
            site.breakpoint.next_position = next_position;
            site.breakpoint.next_width = next_width;
        }
    }

    fn observe_node(
        &mut self,
        index: usize,
        node: &Node,
        previous: Option<&Node>,
        next: Option<&Node>,
    ) -> Option<(BreakSite, bool)> {
        let before = self.prefix;
        add_node_width_value(
            &mut self.prefix,
            self.state,
            node,
            previous,
            next,
            self.include_font_expansion,
            DiscretionaryWidths::Replacement,
        );
        let definition = match node {
            Node::Glue { .. }
                if self.auto_breaking
                    && index > 0
                    && previous.is_some_and(|node| !is_discardable(node)) =>
            {
                Some((index + 1, index, 0, false, Widths::zero(), before, None))
            }
            Node::Kern {
                kind: KernKind::Explicit,
                ..
            } if self.auto_breaking && matches!(next, Some(Node::Glue { .. })) => {
                Some((index + 1, index, 0, false, Widths::zero(), before, None))
            }
            Node::Penalty(penalty) if *penalty < INF_PENALTY => Some((
                index + 1,
                index,
                (*penalty).max(EJECT_PENALTY),
                false,
                Widths::zero(),
                before,
                None,
            )),
            Node::Disc { pre, post, .. } => Some((
                index + 1,
                index + 1,
                discretionary_penalty(pre.is_empty(), self.params),
                true,
                line_widths_view(
                    self.state,
                    pre,
                    0,
                    self.state.page_nodes(*pre).len(),
                    self.include_font_expansion,
                ),
                before,
                Some(*post),
            )),
            Node::MathOff(_) if matches!(next, Some(Node::Glue { .. })) => {
                self.auto_breaking = true;
                Some((index + 1, index, 0, false, Widths::zero(), before, None))
            }
            Node::MathOn(_) => {
                self.auto_breaking = false;
                None
            }
            Node::MathOff(_) => {
                self.auto_breaking = true;
                None
            }
            _ => None,
        };
        self.materialization.push(match node {
            Node::Disc { .. } => MaterializationAction::Discretionary,
            Node::Glue { .. }
            | Node::Kern {
                kind: KernKind::Explicit,
                ..
            } if definition.is_some() => MaterializationAction::BreakDiscardable,
            Node::MathOff(_) if definition.is_some() => MaterializationAction::BreakMath,
            _ => MaterializationAction::Copy,
        });
        definition.map(
            |(position, protrusion_end, penalty, hyphenated, add_width, line_width, post)| {
                let (next_position, next_width, awaits_discardable_end) =
                    if let Some(post) = post.filter(|post| !post.is_empty()) {
                        (
                            position,
                            self.prefix.sub(line_widths_view(
                                self.state,
                                &post,
                                0,
                                self.state.page_nodes(post).len(),
                                self.include_font_expansion,
                            )),
                            false,
                        )
                    } else {
                        (position, self.prefix, true)
                    };
                (
                    BreakSite {
                        breakpoint: Breakpoint {
                            position,
                            protrusion_end,
                            penalty,
                            hyphenated,
                            add_width,
                            line_width,
                            next_position,
                            next_width,
                        },
                    },
                    awaits_discardable_end,
                )
            },
        )
    }
}

#[cfg(test)]
fn legal_breakpoints<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    params: &LineBreakParams,
) -> Vec<Breakpoint> {
    LegalBreakpoints::new(state, NodeCursor::owned(nodes), params)
        .collect_direct()
        .into_iter()
        .map(|site| site.breakpoint)
        .collect()
}

fn is_discardable(node: &Node) -> bool {
    matches!(
        node,
        Node::Glue { .. }
            | Node::Kern {
                kind: KernKind::Explicit | KernKind::Mu,
                ..
            }
            | Node::Penalty(_)
            | Node::MathOn(_)
            | Node::MathOff(_)
    )
}

fn fitness_class(bad: i32, natural: i64, target: i64) -> Fitness {
    if bad > 12 {
        if natural > target {
            Fitness::Tight
        } else if bad > 99 {
            Fitness::VeryLoose
        } else {
            Fitness::Loose
        }
    } else {
        Fitness::Decent
    }
}

fn incompatible(left: Fitness, right: Fitness) -> bool {
    (left as i32 - right as i32).abs() > 1
}

fn choose_final(finals: &[Candidate], looseness: i32) -> Option<usize> {
    let first = finals
        .iter()
        .enumerate()
        .min_by_key(|(_, candidate)| candidate.path_demerits)?
        .0;
    let target = finals[first].line as i32 + looseness;
    finals
        .iter()
        .enumerate()
        .min_by_key(|(_, candidate)| {
            let diff = (candidate.line as i32 - target).abs();
            (diff, candidate.path_demerits)
        })
        .map(|(id, _)| id)
        .or(Some(first))
}

fn reconstruct(
    chosen: Candidate,
    passive: &[PassiveRoute],
    last_line_fit: LastLineFit,
    memory: BreakMemoryPlan,
) -> BreakPlan {
    let mut breaks = Vec::new();
    let demerits = chosen.path_demerits.min(AWFUL_BAD);
    let last_line_fill = last_line_fit.adjusted_fill(&chosen);
    let mut id = chosen.passive;
    while let Some(passive_id) = id {
        let route = passive[passive_id];
        breaks.push(route.decision);
        id = route.previous;
    }
    breaks.reverse();
    BreakPlan {
        breaks,
        demerits,
        last_line_fill,
        memory,
    }
}

#[cfg(test)]
mod tests;
