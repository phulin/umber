use tex_arith::{saturating_add as add, saturating_sub as sub_scaled};
use tex_state::glue::GlueSpec;
use tex_state::ids::{GlueId, NodeListId};
use tex_state::node::{KernKind, Node};
use tex_state::scaled::Scaled;

use crate::{INF_BAD, TypesetState};

const EJECT_PENALTY: i32 = -10_000;
const INF_PENALTY: i32 = 10_000;
const AWFUL_BAD: i32 = 0o7777777777;

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
    pub empty_list: NodeListId,
    pub left_skip: GlueId,
    pub right_skip: GlueId,
    pub interline_penalty: i32,
    pub club_penalty: i32,
    pub widow_penalty: i32,
    pub broken_penalty: i32,
    pub prev_graf: i32,
    pub interline_penalties: Vec<i32>,
    pub club_penalties: Vec<i32>,
    pub widow_penalties: Vec<i32>,
    pub shape: LineShape,
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
        let one_based = line_no.max(1).saturating_add(self.line_offset);
        if let Some(parshape) = &self.parshape
            && !parshape.lines.is_empty()
        {
            let index = one_based.saturating_sub(1).min(parshape.lines.len() - 1);
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BreakDecision {
    pub position: usize,
    pub penalty: i32,
    pub hyphenated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LineBreakResult {
    pub breaks: Vec<BreakDecision>,
    pub demerits: i32,
    pub nodes: Vec<Node>,
    pub last_line_fill: Option<GlueSpec>,
}

/// The result of choosing line breaks, independent of paragraph ownership.
///
/// Keeping the plan separate lets execution move the selected original or
/// hyphenated node list into post-line-breaking instead of cloning it while
/// reconstructing the winning route.
#[derive(Clone, Debug, PartialEq)]
pub struct BreakPlan {
    pub breaks: Vec<BreakDecision>,
    pub demerits: i32,
    pub last_line_fill: Option<GlueSpec>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BrokenLine {
    pub nodes: Vec<Node>,
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
    if let Some(plan) = try_line_break_without_hyphenation(state, nodes, &params) {
        return plan.with_nodes(nodes.to_vec());
    }

    let hyphenated = hyphenation.hyphenate(nodes);
    line_break_hyphenated(state, &hyphenated, &params).with_nodes(hyphenated)
}

impl BreakPlan {
    pub fn with_nodes(self, nodes: Vec<Node>) -> LineBreakResult {
        LineBreakResult {
            breaks: self.breaks,
            demerits: self.demerits,
            nodes,
            last_line_fill: self.last_line_fill,
        }
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
    (params.pretolerance >= 0)
        .then(|| run_pass(state, nodes, params, params.pretolerance, false, false))
        .flatten()
}

/// Runs TeX82's tolerance and emergency passes on an already-hyphenated list.
pub fn line_break_hyphenated<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    params: &LineBreakParams,
) -> BreakPlan {
    let second = run_pass(
        state,
        nodes,
        params,
        params.tolerance,
        false,
        params.emergency_stretch.raw() <= 0,
    );
    if let Some(result) = second {
        return result;
    }

    run_pass(state, nodes, params, params.tolerance, true, true)
        .expect("final line-breaking pass always permits an artificial demerits path")
}

mod post;
mod widths;

pub use post::{LineMaterializer, post_line_break, post_line_break_owned};

use widths::{Widths, line_badness, line_widths_nodes, line_widths_view, node_width_at};

/// Validates pdfTeX's paragraph-wide expansion-step and limit invariants.
///
/// Callers need this only when `pdf_adjust_spacing > 1`; mode 1 performs
/// final-line expansion independently and permits unlike font settings.
pub fn validate_paragraph_expansion<S: TypesetState>(
    state: &S,
    nodes: &[Node],
) -> Result<(), crate::expansion::FontExpansionError> {
    let mut paragraph = crate::expansion::ParagraphExpansion::default();
    observe_expansion_fonts(state, nodes, &mut paragraph)
}

fn observe_expansion_fonts<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    paragraph: &mut crate::expansion::ParagraphExpansion,
) -> Result<(), crate::expansion::FontExpansionError> {
    for node in nodes {
        match node {
            Node::Char { font, .. } | Node::Lig { font, .. } => {
                if let Some(spec) = state.font_expansion_spec(*font) {
                    paragraph.observe(spec)?;
                }
            }
            Node::Disc {
                pre, post, replace, ..
            } => {
                for list in [*pre, *post, *replace] {
                    let owned: Vec<_> = state
                        .nodes(list)
                        .into_iter()
                        .map(|node| node.to_owned())
                        .collect();
                    observe_expansion_fonts(state, &owned, paragraph)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Plans pdfTeX's normalized signed expansion ratio for one finalized line.
///
/// This is pure: execution uses the result to intern and substitute discrete
/// generated fonts before performing ordinary final hpack.
#[must_use]
pub fn plan_line_expansion<S: TypesetState>(state: &S, nodes: &[Node], target: Scaled) -> i32 {
    let widths = line_widths_nodes(state, nodes);
    let shortfall = Scaled::from_raw(target.raw().saturating_sub(widths.natural.raw()));
    crate::expansion::line_expansion_ratio(
        shortfall,
        crate::expansion::ExpansionCapacity {
            stretch: widths.font_stretch,
            shrink: widths.font_shrink,
        },
        widths.has_infinite_adjustment(shortfall.raw()),
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
    width_position: usize,
    start_width: Widths,
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

#[derive(Clone, Copy, Debug)]
struct PassiveRoute {
    decision: BreakDecision,
    previous: Option<usize>,
}

#[derive(Clone, Copy, Debug)]
struct Breakpoint {
    position: usize,
    width_position: usize,
    penalty: i32,
    hyphenated: bool,
    add_width: Widths,
    line_width: Widths,
    next_position: usize,
    next_width: Widths,
}

fn run_pass<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    params: &LineBreakParams,
    tolerance: i32,
    emergency: bool,
    final_pass: bool,
) -> Option<BreakPlan> {
    let mut background = Widths::from_glue(params.left_skip);
    background.add_assign(Widths::from_glue(params.right_skip));
    let mut active = vec![Candidate {
        serial: 0,
        position: 0,
        width_position: 0,
        start_width: Widths::zero(),
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
    let mut passive = Vec::new();
    let mut next_serial = 1;
    let last_line_fit = LastLineFit::new(params, background);
    let easy_line = tex_easy_line(params);

    for bp in LegalBreakpoints::new(state, nodes, params) {
        let prior_active_len = active.len();
        let mut survivor_count = 0;
        let forced = bp.penalty <= EJECT_PENALTY;
        for active_index in 0..prior_active_len {
            let active_candidate = active[active_index];
            // Material discarded after the active break can extend beyond a
            // later syntactic breakpoint (for example, through consecutive
            // penalties). Such a breakpoint is no longer reachable from this
            // active node and must not create a backwards break chain.
            if active_candidate.width_position > bp.width_position {
                active[survivor_count] = active_candidate;
                survivor_count += 1;
                continue;
            }
            let mut widths = bp.line_width.sub(active_candidate.start_width);
            widths.add_assign(background);
            widths.add_assign(bp.add_width);
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
            // Character protrusion, when breakpoint-aware, adjusts `target`
            // through the same signed-shortfall seam before expansion. The
            // protrusion module owns edge discovery and will supply the
            // adjustment here without changing the expansion/glue ordering.
            let normal_b = line_badness(
                widths,
                target,
                Scaled::from_raw(0),
                (params.pdf_adjust_spacing > 1).then_some(expansion_steps(widths)),
            );
            let fitted = terminal
                .then(|| last_line_fit.badness(&active_candidate, widths, target))
                .flatten();
            let (b, fitness) = fitted
                .map(|(bad, fitness, _)| (bad, fitness))
                .unwrap_or_else(|| {
                    let badness = normal_b.min(INF_BAD);
                    (
                        normal_b,
                        fitness_class(badness, widths.natural.raw(), target.raw()),
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
                    serial: next_serial,
                    position: bp.position,
                    width_position: bp.next_position,
                    start_width: bp.next_width,
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
                        Scaled::from_raw(target.raw().saturating_sub(widths.natural.raw()))
                    },
                    line_glue: fitted.map_or_else(
                        || candidate_line_glue(widths, target, b),
                        |(_, _, adjustment)| adjustment,
                    ),
                };
                next_serial += 1;
                record_best_route(&mut active, prior_active_len, candidate);
            }
            if !deactivates {
                active[survivor_count] = active_candidate;
                survivor_count += 1;
            }
        }
        let winner_count = active.len() - prior_active_len;
        for candidate in &mut active[prior_active_len..] {
            let passive_id = passive.len();
            passive.push(PassiveRoute {
                decision: BreakDecision {
                    position: candidate.position.min(nodes.len()),
                    penalty: candidate.penalty,
                    hyphenated: candidate.hyphenated,
                },
                previous: candidate.previous,
            });
            candidate.passive = Some(passive_id);
        }
        active.copy_within(
            prior_active_len..prior_active_len + winner_count,
            survivor_count,
        );
        active.truncate(survivor_count + winner_count);
        sort_active_candidates(&mut active, params, easy_line);
    }

    let chosen = choose_final(&active, params.looseness)?;
    let best = active
        .iter()
        .min_by_key(|candidate| candidate.path_demerits)?;
    let actual_looseness = active[chosen].line as i32 - best.line as i32;
    if !final_pass && actual_looseness != params.looseness {
        return None;
    }
    Some(reconstruct(active[chosen], &passive, last_line_fit))
}

fn record_best_route(active: &mut Vec<Candidate>, winner_start: usize, candidate: Candidate) {
    if let Some(slot) = active[winner_start..]
        .iter()
        .position(|current| current.line == candidate.line && current.fitness == candidate.fitness)
    {
        let slot = winner_start + slot;
        if candidate.path_demerits <= active[slot].path_demerits {
            // TeX82 uses `d <= minimal_demerits[fit_class]`, so an equal
            // later route replaces the earlier route in its first-visit slot.
            active[slot] = candidate;
        }
    } else {
        active.push(candidate);
    }
}

fn tex_easy_line(params: &LineBreakParams) -> usize {
    if params.looseness != 0 {
        return usize::MAX;
    }
    if let Some(parshape) = &params.shape.parshape {
        return parshape.lines.len().saturating_sub(1);
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
    active.sort_unstable_by(|left, right| {
        left.line
            .cmp(&right.line)
            .then_with(|| {
                let effective_line = left
                    .line
                    .saturating_add(1)
                    .saturating_add(params.shape.line_offset);
                if effective_line > easy_line {
                    left.position.cmp(&right.position)
                } else {
                    right.position.cmp(&left.position)
                }
            })
            // Candidate serials encode insertion/visit order. This makes the
            // comparator total while preserving stable-sort behavior for
            // routes with the same TeX active-list key, without allocating a
            // temporary merge buffer at every breakpoint.
            .then_with(|| left.serial.cmp(&right.serial))
    });
}

#[derive(Clone, Copy)]
struct LastLineFit {
    amount: i32,
    par_fill: GlueSpec,
    fill_width: [Scaled; 3],
    enabled: bool,
}

impl LastLineFit {
    fn new(params: &LineBreakParams, background: Widths) -> Self {
        let mut fill_width = [Scaled::from_raw(0); 3];
        let par_fill = params.par_fill_skip;
        let enabled = params.last_line_fit > 0
            && par_fill.stretch.raw() > 0
            && par_fill.stretch_order != tex_state::glue::Order::Normal
            && background.infinite_stretch_is_zero();
        if enabled {
            fill_width[par_fill.stretch_order as usize - 1] = par_fill.stretch;
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
        let mut adjustment = rounded_fraction(
            available.raw(),
            previous.line_shortfall.raw(),
            previous.line_glue.raw(),
        );
        if self.amount < 1000 {
            adjustment = rounded_fraction(adjustment, self.amount, 1000);
        }
        if adjustment > 0 {
            adjustment = adjustment.min(target.raw().saturating_sub(widths.natural.raw()));
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
            width: Scaled::from_raw(
                self.par_fill
                    .width
                    .raw()
                    .saturating_add(chosen.line_shortfall.raw())
                    .saturating_sub(chosen.line_glue.raw()),
            ),
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
    let shortfall = target.raw().saturating_sub(widths.natural.raw());
    if badness > INF_BAD || widths.has_infinite_adjustment(shortfall) {
        Scaled::from_raw(0)
    } else if shortfall > 0 {
        widths.normal_stretch()
    } else if shortfall < 0 {
        widths.normal_shrink()
    } else {
        Scaled::from_raw(0)
    }
}

fn discretionary_post_is_nonempty<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    position: usize,
) -> bool {
    matches!(
        position.checked_sub(1).and_then(|index| nodes.get(index)),
        Some(Node::Disc { post, .. }) if !state.nodes(*post).is_empty()
    )
}

fn next_width_position(nodes: &[Node], position: usize) -> usize {
    let mut position = position.min(nodes.len());
    while position < nodes.len() && is_discardable(&nodes[position]) {
        position += 1;
    }
    position
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
    let line_bad = params.line_penalty.saturating_add(bad);
    let mut dem = if line_bad.abs() >= INF_BAD {
        100_000_000
    } else {
        line_bad.saturating_mul(line_bad)
    };
    if penalty > 0 {
        dem = dem.saturating_add(penalty.saturating_mul(penalty));
    } else if penalty > EJECT_PENALTY {
        dem = dem.saturating_sub(penalty.saturating_mul(penalty));
    }
    if active.hyphenated {
        if terminal {
            dem = dem.saturating_add(params.final_hyphen_demerits);
        } else if bp.hyphenated {
            dem = dem.saturating_add(params.double_hyphen_demerits);
        }
    }
    if incompatible(active.fitness, fitness) {
        dem = dem.saturating_add(params.adj_demerits);
    }
    dem.saturating_add(active.path_demerits)
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
    nodes: &'a [Node],
    params: &'a LineBreakParams,
    index: usize,
    prefix: Widths,
    auto_breaking: bool,
    last_position: Option<usize>,
    terminal_emitted: bool,
}

impl<'a, S: TypesetState> LegalBreakpoints<'a, S> {
    fn new(state: &'a S, nodes: &'a [Node], params: &'a LineBreakParams) -> Self {
        Self {
            state,
            nodes,
            params,
            index: 0,
            prefix: Widths::zero(),
            auto_breaking: true,
            last_position: None,
            terminal_emitted: false,
        }
    }

    fn breakpoint(
        &self,
        position: usize,
        width_position: usize,
        penalty: i32,
        hyphenated: bool,
        add_width: Widths,
        line_width: Widths,
    ) -> Breakpoint {
        let next_position =
            if hyphenated && discretionary_post_is_nonempty(self.state, self.nodes, position) {
                position
            } else {
                next_width_position(self.nodes, position)
            };
        let mut next_width = line_width;
        for index in width_position..next_position {
            next_width.add_assign(node_width_at(self.state, self.nodes, index));
        }
        Breakpoint {
            position,
            width_position,
            penalty,
            hyphenated,
            add_width,
            line_width,
            next_position,
            next_width,
        }
    }
}

fn expansion_steps(widths: Widths) -> (i32, i32) {
    let step = widths.expansion_step.unwrap_or(1).max(1);
    (
        widths.expansion_stretch_limit.unwrap_or(0) / step,
        widths.expansion_shrink_limit.unwrap_or(0) / step,
    )
}

impl<S: TypesetState> Iterator for LegalBreakpoints<'_, S> {
    type Item = Breakpoint;

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.nodes.len() {
            let i = self.index;
            let before = self.prefix;
            self.prefix
                .add_assign(node_width_at(self.state, self.nodes, i));
            self.index += 1;

            let definition = match &self.nodes[i] {
                Node::Glue { .. }
                    if self.auto_breaking && i > 0 && !is_discardable(&self.nodes[i - 1]) =>
                {
                    Some((i + 1, i, 0, false, Widths::zero(), before))
                }
                Node::Kern {
                    kind: KernKind::Explicit,
                    ..
                } if self.auto_breaking
                    && i + 1 < self.nodes.len()
                    && matches!(self.nodes[i + 1], Node::Glue { .. }) =>
                {
                    Some((i + 1, i + 1, 0, false, Widths::zero(), self.prefix))
                }
                Node::Penalty(penalty) if *penalty < INF_PENALTY => {
                    Some((i + 1, i, *penalty, false, Widths::zero(), before))
                }
                Node::Disc { pre, .. } => Some((
                    i + 1,
                    i,
                    discretionary_penalty(self.state.nodes(*pre).is_empty(), self.params),
                    true,
                    line_widths_view(
                        self.state,
                        self.state.nodes(*pre),
                        0,
                        self.state.nodes(*pre).len(),
                    ),
                    before,
                )),
                Node::MathOff(_) if matches!(self.nodes.get(i + 1), Some(Node::Glue { .. })) => {
                    self.auto_breaking = true;
                    Some((i + 1, i + 1, 0, false, Widths::zero(), self.prefix))
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
            if let Some((position, width_position, penalty, hyphenated, add_width, line_width)) =
                definition
            {
                self.last_position = Some(position);
                return Some(self.breakpoint(
                    position,
                    width_position,
                    penalty,
                    hyphenated,
                    add_width,
                    line_width,
                ));
            }
        }

        if !self.terminal_emitted
            && self
                .last_position
                .is_none_or(|position| position < self.nodes.len())
        {
            self.terminal_emitted = true;
            return Some(self.breakpoint(
                self.nodes.len(),
                self.nodes.len(),
                EJECT_PENALTY,
                false,
                Widths::zero(),
                self.prefix,
            ));
        }
        None
    }
}

#[cfg(test)]
fn legal_breakpoints<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    params: &LineBreakParams,
) -> Vec<Breakpoint> {
    LegalBreakpoints::new(state, nodes, params).collect()
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

fn fitness_class(bad: i32, natural: i32, target: i32) -> Fitness {
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
    }
}

#[cfg(test)]
mod tests;
