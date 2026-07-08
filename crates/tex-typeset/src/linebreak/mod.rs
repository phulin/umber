use tex_state::ids::GlueId;
use tex_state::node::{DiscKind, KernKind, Node};
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
    pub shape: LineShape,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostLineBreakParams {
    pub left_skip: GlueId,
    pub right_skip: GlueId,
    pub interline_penalty: i32,
    pub club_penalty: i32,
    pub widow_penalty: i32,
    pub broken_penalty: i32,
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
                width: Scaled::from_raw(self.hsize.raw().saturating_sub(amount)),
            }
        } else {
            LineDimensions {
                indent: Scaled::from_raw(0),
                width: Scaled::from_raw(self.hsize.raw().saturating_add(amount)),
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
    if params.pretolerance >= 0 {
        let first = run_pass(
            state,
            nodes,
            &params,
            params.pretolerance,
            false,
            false,
            false,
        );
        if let Some(result) = first {
            return result;
        }
    }

    let hyphenated = hyphenation.hyphenate(nodes);
    let second = run_pass(
        state,
        &hyphenated,
        &params,
        params.tolerance,
        true,
        false,
        params.emergency_stretch.raw() <= 0,
    );
    if let Some(result) = second {
        return result;
    }

    run_pass(
        state,
        &hyphenated,
        &params,
        params.tolerance,
        true,
        true,
        true,
    )
    .expect("final line-breaking pass always permits an artificial demerits path")
}

mod post;
mod widths;

pub use post::post_line_break;

use widths::{PrefixWidths, Widths, line_badness, line_widths};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Fitness {
    Tight = 0,
    Decent = 1,
    Loose = 2,
    VeryLoose = 3,
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
}

#[derive(Clone, Copy, Debug)]
struct Breakpoint {
    position: usize,
    width_position: usize,
    penalty: i32,
    hyphenated: bool,
    add_width: Widths,
}

fn run_pass<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    params: &LineBreakParams,
    tolerance: i32,
    allow_hyphenation: bool,
    emergency: bool,
    final_pass: bool,
) -> Option<LineBreakResult> {
    let prefix = PrefixWidths::new(state, nodes);
    let breakpoints = legal_breakpoints(state, nodes, params, allow_hyphenation);
    if breakpoints.is_empty() {
        return Some(LineBreakResult {
            breaks: Vec::new(),
            demerits: 0,
            nodes: nodes.to_vec(),
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
    }];
    let mut active = vec![0usize];
    let mut finals = Vec::new();

    for bp in breakpoints {
        let mut next = Vec::new();
        for &active_id in &active {
            let active_candidate = &candidates[active_id];
            let mut widths = prefix.between(active_candidate.width_position, bp.width_position);
            widths.add_assign(bp.add_width);
            let target = params.shape.dimensions(active_candidate.line + 1).width;
            let extra = if emergency {
                params.emergency_stretch
            } else {
                Scaled::from_raw(0)
            };
            let b = line_badness(widths, target, extra);
            let forced = bp.penalty <= EJECT_PENALTY;
            let artificial =
                final_pass && next.is_empty() && active.len() == 1 && (b > INF_BAD || forced);
            let deactivates = b > INF_BAD || forced;
            let feasible = bp.penalty < INF_PENALTY && (artificial || b <= tolerance);
            if feasible {
                let badness = b.min(INF_BAD);
                let fitness = fitness_class(badness, widths.natural.raw(), target.raw());
                let dem = if artificial {
                    active_candidate.path_demerits
                } else {
                    compute_demerits(params, active_candidate, badness, bp.penalty, fitness, bp)
                };
                let id = candidates.len();
                candidates.push(Candidate {
                    position: bp.position,
                    width_position: bp.position,
                    penalty: bp.penalty,
                    line: active_candidate.line + 1,
                    fitness,
                    demerits: dem,
                    path_demerits: dem,
                    previous: Some(active_id),
                    hyphenated: bp.hyphenated,
                });
                next.push(id);
                if forced && bp.position >= nodes.len() {
                    finals.push(id);
                }
            }
            if !deactivates {
                next.push(active_id);
            }
        }
        active = next;
    }

    apply_final_hyphen_demerits(&mut candidates, &finals, params.final_hyphen_demerits);
    let chosen = choose_final(&candidates, &finals, params.looseness)?;
    Some(reconstruct(nodes, &candidates, chosen))
}

fn compute_demerits(
    params: &LineBreakParams,
    active: &Candidate,
    bad: i32,
    penalty: i32,
    fitness: Fitness,
    bp: Breakpoint,
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
    if incompatible(active.fitness, fitness) {
        dem = dem.saturating_add(params.adj_demerits);
    }
    if active.hyphenated && bp.hyphenated {
        dem = dem.saturating_add(params.double_hyphen_demerits);
    }
    dem.saturating_add(active.path_demerits)
}

fn apply_final_hyphen_demerits(candidates: &mut [Candidate], finals: &[usize], demerits: i32) {
    for &id in finals {
        let Some(prev) = candidates[id].previous else {
            continue;
        };
        if candidates[prev].hyphenated {
            candidates[id].demerits = candidates[id].path_demerits.saturating_add(demerits);
        }
    }
}

fn discretionary_penalty(kind: DiscKind, params: &LineBreakParams) -> i32 {
    match kind {
        DiscKind::AutomaticHyphen => params.hyphen_penalty,
        DiscKind::ExplicitHyphen => params.ex_hyphen_penalty,
        DiscKind::Discretionary => 0,
    }
}

fn legal_breakpoints<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    params: &LineBreakParams,
    allow_hyphenation: bool,
) -> Vec<Breakpoint> {
    let mut out = Vec::new();
    for i in 0..nodes.len() {
        match &nodes[i] {
            Node::Glue { .. } if i > 0 && !is_discardable(&nodes[i - 1]) => out.push(Breakpoint {
                position: i + 1,
                width_position: i,
                penalty: 0,
                hyphenated: false,
                add_width: Widths::zero(),
            }),
            Node::Kern {
                kind: KernKind::Explicit,
                ..
            } if i + 1 < nodes.len() && matches!(nodes[i + 1], Node::Glue { .. }) => {
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
            Node::Disc { kind, pre, .. } if allow_hyphenation => out.push(Breakpoint {
                position: i + 1,
                width_position: i,
                penalty: discretionary_penalty(*kind, params),
                hyphenated: true,
                add_width: line_widths(state, state.nodes(*pre), 0, state.nodes(*pre).len()),
            }),
            Node::Disc { replace, .. } => out.push(Breakpoint {
                position: i + 1,
                width_position: i,
                penalty: INF_PENALTY,
                hyphenated: false,
                add_width: line_widths(
                    state,
                    state.nodes(*replace),
                    0,
                    state.nodes(*replace).len(),
                ),
            }),
            Node::MathOff => out.push(Breakpoint {
                position: i + 1,
                width_position: i + 1,
                penalty: 0,
                hyphenated: false,
                add_width: Widths::zero(),
            }),
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
        Node::Glue { .. } | Node::Kern { .. } | Node::Penalty(_) | Node::MathOn | Node::MathOff
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

fn reconstruct(nodes: &[Node], candidates: &[Candidate], mut id: usize) -> LineBreakResult {
    let mut breaks = Vec::new();
    let demerits = candidates[id].demerits.min(AWFUL_BAD);
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
    }
}

fn add(left: Scaled, right: Scaled) -> Scaled {
    Scaled::from_raw(left.raw().saturating_add(right.raw()))
}

fn sub_scaled(left: Scaled, right: Scaled) -> Scaled {
    Scaled::from_raw(left.raw().saturating_sub(right.raw()))
}

#[cfg(test)]
mod tests;
