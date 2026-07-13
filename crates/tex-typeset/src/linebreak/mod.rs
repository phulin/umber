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

#[derive(Clone, Debug, Eq, PartialEq)]
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
    if let Some(result) = try_line_break_without_hyphenation(state, nodes, &params) {
        return result;
    }

    let hyphenated = hyphenation.hyphenate(nodes);
    line_break_hyphenated(state, &hyphenated, &params)
}

/// Tries TeX82's pretolerance pass without requesting automatic hyphenation.
///
/// Returning `None` means the caller must materialize automatic
/// discretionary nodes before running the tolerance and emergency passes.
pub fn try_line_break_without_hyphenation<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    params: &LineBreakParams,
) -> Option<LineBreakResult> {
    (params.pretolerance >= 0)
        .then(|| run_pass(state, nodes, params, params.pretolerance, false, false))
        .flatten()
}

/// Runs TeX82's tolerance and emergency passes on an already-hyphenated list.
pub fn line_break_hyphenated<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    params: &LineBreakParams,
) -> LineBreakResult {
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

pub use post::post_line_break;

use widths::{PrefixWidths, Widths, line_badness, line_widths_view};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Fitness {
    Tight = 0,
    Decent = 1,
    Loose = 2,
    VeryLoose = 3,
}

impl Fitness {
    const COUNT: usize = 4;

    const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Debug)]
struct Candidate {
    position: usize,
    width_position: usize,
    penalty: i32,
    line: usize,
    fitness: Fitness,
    demerits: i32,
    path_demerits: i32,
    previous: Option<usize>,
    hyphenated: bool,
    line_shortfall: Scaled,
    line_glue: Scaled,
}

#[derive(Clone, Copy, Debug)]
struct Breakpoint {
    position: usize,
    width_position: usize,
    penalty: i32,
    hyphenated: bool,
    add_width: Widths,
}

#[derive(Default)]
struct BestRoutes {
    slots: Vec<Option<usize>>,
    touched: Vec<usize>,
}

impl BestRoutes {
    fn clear(&mut self) {
        for &slot in &self.touched {
            self.slots[slot] = None;
        }
        self.touched.clear();
    }

    fn is_empty(&self) -> bool {
        self.touched.is_empty()
    }

    fn record(&mut self, candidates: &[Candidate], candidate_id: usize) {
        let candidate = &candidates[candidate_id];
        let slot = candidate.line * Fitness::COUNT + candidate.fitness.index();
        if self.slots.len() <= slot {
            self.slots.resize(slot + 1, None);
        }
        match self.slots[slot] {
            Some(current) if candidate.path_demerits <= candidates[current].path_demerits => {
                // TeX82 uses `d <= minimal_demerits[fit_class]`, so an equal
                // later route replaces the earlier route without changing
                // this class's active-list visit order.
                self.slots[slot] = Some(candidate_id);
            }
            None => {
                self.slots[slot] = Some(candidate_id);
                self.touched.push(slot);
            }
            Some(_) => {}
        }
    }

    fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.touched
            .iter()
            .map(|&slot| self.slots[slot].expect("touched route slot is populated"))
    }
}

fn run_pass<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    params: &LineBreakParams,
    tolerance: i32,
    emergency: bool,
    final_pass: bool,
) -> Option<LineBreakResult> {
    let prefix = PrefixWidths::new(state, nodes);
    let mut background = Widths::from_glue(params.left_skip);
    background.add_assign(Widths::from_glue(params.right_skip));
    let breakpoints = legal_breakpoints(state, nodes, params);
    if breakpoints.is_empty() {
        return Some(LineBreakResult {
            breaks: Vec::new(),
            demerits: 0,
            nodes: nodes.to_vec(),
            last_line_fill: None,
        });
    }

    let mut candidates = vec![Candidate {
        position: 0,
        width_position: 0,
        penalty: 0,
        line: 0,
        fitness: Fitness::Decent,
        demerits: 0,
        path_demerits: 0,
        previous: None,
        hyphenated: false,
        line_shortfall: Scaled::from_raw(0),
        line_glue: Scaled::from_raw(0),
    }];
    let mut active = vec![0usize];
    let mut next = Vec::new();
    let mut best_new = BestRoutes::default();
    let mut finals = Vec::new();
    let last_line_fit = LastLineFit::new(params, background);
    let easy_line = tex_easy_line(params);

    for bp in breakpoints {
        next.clear();
        best_new.clear();
        let forced = bp.penalty <= EJECT_PENALTY;
        for (active_index, &active_id) in active.iter().enumerate() {
            let active_candidate = &candidates[active_id];
            // Material discarded after the active break can extend beyond a
            // later syntactic breakpoint (for example, through consecutive
            // penalties). Such a breakpoint is no longer reachable from this
            // active node and must not create a backwards break chain.
            if active_candidate.width_position > bp.width_position {
                next.push(active_id);
                continue;
            }
            let mut widths = prefix.between(active_candidate.width_position, bp.width_position);
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
            let normal_b = line_badness(widths, target, Scaled::from_raw(0));
            let fitted = terminal
                .then(|| last_line_fit.badness(active_candidate, widths, target))
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
                && next.is_empty()
                && best_new.is_empty()
                && active_index + 1 == active.len()
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
                        active_candidate,
                        badness,
                        bp.penalty,
                        fitness,
                        bp,
                        terminal,
                    )
                };
                let id = candidates.len();
                candidates.push(Candidate {
                    position: bp.position,
                    width_position: if bp.hyphenated
                        && discretionary_post_is_nonempty(state, nodes, bp.position)
                    {
                        bp.position
                    } else {
                        next_width_position(nodes, bp.position)
                    },
                    penalty: bp.penalty,
                    line: active_candidate.line + 1,
                    fitness,
                    demerits: dem,
                    path_demerits: dem,
                    previous: Some(active_id),
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
                });
                best_new.record(&candidates, id);
            }
            if !deactivates {
                next.push(active_id);
            }
        }
        if forced && bp.position >= nodes.len() {
            finals.extend(best_new.iter());
        }
        // BestRoutes preserves the first visit position for each class while
        // replacing equal-demerit routes in place. Re-establish TeX's active
        // list order after extending the reusable frontier buffer.
        next.extend(best_new.iter());
        sort_active_candidates(&mut next, &candidates, params, easy_line);
        std::mem::swap(&mut active, &mut next);
    }

    let chosen = choose_final(&candidates, &finals, params.looseness)?;
    let best = *finals.iter().min_by_key(|&&id| candidates[id].demerits)?;
    let actual_looseness = candidates[chosen].line as i32 - candidates[best].line as i32;
    if !final_pass && actual_looseness != params.looseness {
        return None;
    }
    Some(reconstruct(nodes, &candidates, chosen, last_line_fit))
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

fn sort_active_candidates(
    active: &mut [usize],
    candidates: &[Candidate],
    params: &LineBreakParams,
    easy_line: usize,
) {
    // TeX normally keeps active nodes ordered by line number and inserts a
    // new break before existing nodes in the same class. Beyond `easy_line`,
    // all equal-width lines form one deferred class and new breaks instead
    // accumulate in source order. The visit order is observable because an
    // equal demerit replaces the route recorded earlier in `try_break`.
    active.sort_by(|&left, &right| {
        let left = &candidates[left];
        let right = &candidates[right];
        left.line.cmp(&right.line).then_with(|| {
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

fn legal_breakpoints<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    params: &LineBreakParams,
) -> Vec<Breakpoint> {
    let mut out = Vec::new();
    let mut auto_breaking = true;
    for i in 0..nodes.len() {
        match &nodes[i] {
            Node::Glue { .. } if auto_breaking && i > 0 && !is_discardable(&nodes[i - 1]) => {
                out.push(Breakpoint {
                    position: i + 1,
                    width_position: i,
                    penalty: 0,
                    hyphenated: false,
                    add_width: Widths::zero(),
                });
            }
            Node::Kern {
                kind: KernKind::Explicit,
                ..
            } if auto_breaking
                && i + 1 < nodes.len()
                && matches!(nodes[i + 1], Node::Glue { .. }) =>
            {
                out.push(Breakpoint {
                    position: i + 1,
                    width_position: i + 1,
                    penalty: 0,
                    hyphenated: false,
                    add_width: Widths::zero(),
                });
            }
            Node::Penalty(penalty) if *penalty < INF_PENALTY => out.push(Breakpoint {
                position: i + 1,
                width_position: i,
                penalty: *penalty,
                hyphenated: false,
                add_width: Widths::zero(),
            }),
            Node::Disc { pre, .. } => {
                out.push(Breakpoint {
                    position: i + 1,
                    width_position: i,
                    penalty: discretionary_penalty(state.nodes(*pre).is_empty(), params),
                    hyphenated: true,
                    add_width: line_widths_view(
                        state,
                        state.nodes(*pre),
                        0,
                        state.nodes(*pre).len(),
                    ),
                });
            }
            Node::MathOff(_) if matches!(nodes.get(i + 1), Some(Node::Glue { .. })) => {
                out.push(Breakpoint {
                    position: i + 1,
                    width_position: i + 1,
                    penalty: 0,
                    hyphenated: false,
                    add_width: Widths::zero(),
                });
                auto_breaking = true;
            }
            Node::MathOn(_) => auto_breaking = false,
            Node::MathOff(_) => auto_breaking = true,
            _ => {}
        }
    }
    if !matches!(out.last(), Some(bp) if bp.position >= nodes.len()) {
        out.push(Breakpoint {
            position: nodes.len(),
            width_position: nodes.len(),
            penalty: EJECT_PENALTY,
            hyphenated: false,
            add_width: Widths::zero(),
        });
    }
    out
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

fn choose_final(candidates: &[Candidate], finals: &[usize], looseness: i32) -> Option<usize> {
    let first = *finals.iter().min_by_key(|&&id| candidates[id].demerits)?;
    let target = candidates[first].line as i32 + looseness;
    finals
        .iter()
        .copied()
        .min_by_key(|&id| {
            let diff = (candidates[id].line as i32 - target).abs();
            (diff, candidates[id].demerits)
        })
        .or(Some(first))
}

fn reconstruct(
    nodes: &[Node],
    candidates: &[Candidate],
    mut id: usize,
    last_line_fit: LastLineFit,
) -> LineBreakResult {
    let mut breaks = Vec::new();
    let demerits = candidates[id].demerits.min(AWFUL_BAD);
    let last_line_fill = last_line_fit.adjusted_fill(&candidates[id]);
    while let Some(prev) = candidates[id].previous {
        breaks.push(BreakDecision {
            position: candidates[id].position.min(nodes.len()),
            penalty: candidates[id].penalty,
            hyphenated: candidates[id].hyphenated,
        });
        id = prev;
    }
    breaks.reverse();
    LineBreakResult {
        breaks,
        demerits,
        nodes: nodes.to_vec(),
        last_line_fill,
    }
}

#[cfg(test)]
mod tests;
