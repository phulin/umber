use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::GlueId;
use tex_state::node::{GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;

use crate::{TypesetState, badness};

const EJECT_PENALTY: i32 = -10_000;
const INF_PENALTY: i32 = 10_000;
const AWFUL_BAD: i32 = 0o7777777777;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineBreakParams {
    pub pretolerance: i32,
    pub tolerance: i32,
    pub line_penalty: i32,
    pub adj_demerits: i32,
    pub double_hyphen_demerits: i32,
    pub final_hyphen_demerits: i32,
    pub emergency_stretch: Scaled,
    pub looseness: i32,
    pub hsize: Scaled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PostLineBreakParams {
    pub left_skip: GlueId,
    pub right_skip: GlueId,
    pub interline_penalty: i32,
    pub club_penalty: i32,
    pub widow_penalty: i32,
    pub broken_penalty: i32,
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
        let first = run_pass(state, nodes, params, params.pretolerance, false, false);
        if let Some(result) = first {
            return result;
        }
    }

    let hyphenated = hyphenation.hyphenate(nodes);
    let second = run_pass(state, &hyphenated, params, params.tolerance, true, false);
    if let Some(result) = second {
        return result;
    }

    run_pass(state, &hyphenated, params, params.tolerance, true, true)
        .expect("final line-breaking pass always permits an artificial demerits path")
}

pub fn post_line_break<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    breaks: &[BreakDecision],
    params: PostLineBreakParams,
) -> Vec<BrokenLine> {
    let mut lines = Vec::new();
    let mut start = 0usize;
    let mut pending_post = Vec::new();
    for (line_no, decision) in breaks.iter().enumerate() {
        let mut line = Vec::new();
        line.push(Node::Glue {
            spec: params.left_skip,
            kind: GlueKind::Normal,
        });
        line.append(&mut pending_post);
        pending_post = push_line_segment(state, nodes, start, decision, &mut line);
        line.push(Node::Glue {
            spec: params.right_skip,
            kind: GlueKind::Normal,
        });

        let penalty_after = line_penalty_after(line_no, breaks, decision.hyphenated, params);
        lines.push(BrokenLine {
            nodes: line,
            penalty_after,
            hyphenated: decision.hyphenated,
        });
        start = next_start(nodes, decision.position);
    }
    lines
}

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
    line: usize,
    fitness: Fitness,
    demerits: i32,
    previous: Option<usize>,
    hyphenated: bool,
}

#[derive(Clone, Copy, Debug)]
struct Breakpoint {
    position: usize,
    penalty: i32,
    hyphenated: bool,
}

fn run_pass<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    params: LineBreakParams,
    tolerance: i32,
    allow_hyphenation: bool,
    emergency: bool,
) -> Option<LineBreakResult> {
    let breakpoints = legal_breakpoints(nodes, allow_hyphenation);
    if breakpoints.is_empty() {
        return Some(LineBreakResult {
            breaks: Vec::new(),
            demerits: 0,
            nodes: nodes.to_vec(),
        });
    }

    let mut candidates = vec![Candidate {
        position: 0,
        line: 0,
        fitness: Fitness::Decent,
        demerits: 0,
        previous: None,
        hyphenated: false,
    }];
    let mut active = vec![0usize];
    let mut finals = Vec::new();

    for bp in breakpoints {
        let mut next = Vec::new();
        for &active_id in &active {
            let active_candidate = &candidates[active_id];
            let widths = line_widths(state, nodes, active_candidate.position, bp.position);
            let target = params.hsize;
            let extra = if emergency {
                params.emergency_stretch
            } else {
                Scaled::from_raw(0)
            };
            let b = line_badness(widths, target, extra);
            let forced = bp.penalty <= EJECT_PENALTY;
            if !forced && (bp.penalty >= INF_PENALTY || b > tolerance) {
                continue;
            }
            let fitness = fitness_class(b, widths.natural.raw(), target.raw());
            let dem = compute_demerits(params, active_candidate, b, bp.penalty, fitness, bp);
            let id = candidates.len();
            candidates.push(Candidate {
                position: bp.position,
                line: active_candidate.line + 1,
                fitness,
                demerits: dem,
                previous: Some(active_id),
                hyphenated: bp.hyphenated,
            });
            next.push(id);
            if forced && bp.position >= nodes.len() {
                finals.push(id);
            }
        }
        active.extend(next);
    }

    let chosen = choose_final(&candidates, &finals, params.looseness)?;
    Some(reconstruct(nodes, &candidates, chosen))
}

fn compute_demerits(
    params: LineBreakParams,
    active: &Candidate,
    bad: i32,
    penalty: i32,
    fitness: Fitness,
    bp: Breakpoint,
) -> i32 {
    let line_bad = params.line_penalty.saturating_add(bad).abs();
    let mut dem = line_bad.saturating_mul(line_bad);
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
    dem.saturating_add(active.demerits)
}

fn legal_breakpoints(nodes: &[Node], allow_hyphenation: bool) -> Vec<Breakpoint> {
    let mut out = Vec::new();
    for i in 0..nodes.len() {
        match &nodes[i] {
            Node::Glue { .. } if i > 0 && !is_discardable(&nodes[i - 1]) => out.push(Breakpoint {
                position: i,
                penalty: 0,
                hyphenated: false,
            }),
            Node::Kern {
                kind: KernKind::Explicit,
                ..
            } if i + 1 < nodes.len() && matches!(nodes[i + 1], Node::Glue { .. }) => {
                out.push(Breakpoint {
                    position: i + 1,
                    penalty: 0,
                    hyphenated: false,
                });
            }
            Node::Penalty(penalty) if *penalty < INF_PENALTY => out.push(Breakpoint {
                position: i + 1,
                penalty: *penalty,
                hyphenated: false,
            }),
            Node::Disc { .. } if allow_hyphenation => out.push(Breakpoint {
                position: i + 1,
                penalty: 50,
                hyphenated: true,
            }),
            Node::MathOff => out.push(Breakpoint {
                position: i + 1,
                penalty: 0,
                hyphenated: false,
            }),
            _ => {}
        }
    }
    if !matches!(out.last(), Some(bp) if bp.position >= nodes.len()) {
        out.push(Breakpoint {
            position: nodes.len(),
            penalty: EJECT_PENALTY,
            hyphenated: false,
        });
    }
    out
}

#[derive(Clone, Copy, Debug)]
struct Widths {
    natural: Scaled,
    stretch: [Scaled; 4],
    shrink: [Scaled; 4],
}

fn line_widths<S: TypesetState>(state: &S, nodes: &[Node], start: usize, end: usize) -> Widths {
    let mut widths = Widths {
        natural: Scaled::from_raw(0),
        stretch: [Scaled::from_raw(0); 4],
        shrink: [Scaled::from_raw(0); 4],
    };
    for node in &nodes[start..end.min(nodes.len())] {
        match node {
            Node::Char { font, ch } | Node::Lig { font, ch, .. } => {
                if let Ok(code) = u8::try_from(*ch as u32)
                    && let Some(metrics) = state.font_char_metrics(*font, code)
                {
                    widths.natural = add(widths.natural, metrics.width);
                }
            }
            Node::Kern { amount, .. } => widths.natural = add(widths.natural, *amount),
            Node::Glue { spec, .. } => add_glue(&mut widths, state.glue(*spec)),
            Node::Rule { width, .. } => {
                if let Some(width) = width {
                    widths.natural = add(widths.natural, *width);
                }
            }
            Node::HList(box_node) | Node::VList(box_node) => {
                widths.natural = add(widths.natural, box_node.width);
            }
            Node::Disc { replace, .. } => {
                let replace =
                    line_widths(state, state.nodes(*replace), 0, state.nodes(*replace).len());
                widths.natural = add(widths.natural, replace.natural);
            }
            Node::Penalty(_)
            | Node::Unset
            | Node::Mark { .. }
            | Node::Ins { .. }
            | Node::Whatsit(_)
            | Node::MathOn
            | Node::MathOff
            | Node::Adjust(_) => {}
        }
    }
    widths
}

fn add_glue(widths: &mut Widths, spec: GlueSpec) {
    widths.natural = add(widths.natural, spec.width);
    widths.stretch[spec.stretch_order as usize] =
        add(widths.stretch[spec.stretch_order as usize], spec.stretch);
    widths.shrink[spec.shrink_order as usize] =
        add(widths.shrink[spec.shrink_order as usize], spec.shrink);
}

fn line_badness(widths: Widths, target: Scaled, emergency: Scaled) -> i32 {
    let diff = target.raw() - widths.natural.raw();
    if diff >= 0 {
        let stretch_order = highest_order(widths.stretch);
        if stretch_order != Order::Normal && widths.stretch[stretch_order as usize].raw() > 0 {
            0
        } else {
            badness(
                Scaled::from_raw(diff),
                add(widths.stretch[Order::Normal as usize], emergency),
            )
        }
    } else {
        let shrink_order = highest_order(widths.shrink);
        if shrink_order != Order::Normal && widths.shrink[shrink_order as usize].raw() > 0 {
            0
        } else {
            badness(
                Scaled::from_raw(diff.saturating_abs()),
                widths.shrink[Order::Normal as usize],
            )
        }
    }
}

fn highest_order(values: [Scaled; 4]) -> Order {
    for order in [Order::Filll, Order::Fill, Order::Fil, Order::Normal] {
        if values[order as usize].raw() != 0 {
            return order;
        }
    }
    Order::Normal
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
            penalty: breakpoint_penalty(nodes, candidates[id].position),
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

fn breakpoint_penalty(nodes: &[Node], position: usize) -> i32 {
    if position > 0
        && let Some(Node::Penalty(penalty)) = nodes.get(position - 1)
    {
        return *penalty;
    }
    if position >= nodes.len() {
        EJECT_PENALTY
    } else {
        0
    }
}

fn push_line_segment<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    start: usize,
    decision: &BreakDecision,
    out: &mut Vec<Node>,
) -> Vec<Node> {
    let end = decision.position.min(nodes.len());
    let mut post = Vec::new();
    for (offset, node) in nodes[start..end].iter().enumerate() {
        let absolute = start + offset;
        match node {
            Node::Disc {
                pre,
                post: post_list,
                ..
            } if decision.hyphenated && absolute + 1 == end => {
                out.extend_from_slice(state.nodes(*pre));
                post.extend_from_slice(state.nodes(*post_list));
            }
            Node::Penalty(_) if absolute + 1 == end => {}
            Node::Glue { .. } if absolute + 1 == end => {}
            _ => out.push(node.clone()),
        }
    }
    post
}

fn next_start(nodes: &[Node], position: usize) -> usize {
    let mut start = position.min(nodes.len());
    while start < nodes.len() && is_discardable(&nodes[start]) {
        start += 1;
    }
    start
}

fn line_penalty_after(
    line_no: usize,
    breaks: &[BreakDecision],
    hyphenated: bool,
    params: PostLineBreakParams,
) -> Option<i32> {
    if line_no + 1 >= breaks.len() {
        return None;
    }
    let mut penalty = params.interline_penalty;
    if line_no == 0 {
        penalty = penalty.saturating_add(params.club_penalty);
    }
    if line_no + 2 == breaks.len() {
        penalty = penalty.saturating_add(params.widow_penalty);
    }
    if hyphenated {
        penalty = penalty.saturating_add(params.broken_penalty);
    }
    (penalty != 0).then_some(penalty)
}

fn is_discardable(node: &Node) -> bool {
    matches!(
        node,
        Node::Glue { .. } | Node::Kern { .. } | Node::Penalty(_) | Node::MathOn | Node::MathOff
    )
}

fn add(left: Scaled, right: Scaled) -> Scaled {
    Scaled::from_raw(left.raw().saturating_add(right.raw()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_state::Universe;

    fn sp(raw: i32) -> Scaled {
        Scaled::from_raw(raw)
    }

    fn params(width: i32) -> LineBreakParams {
        LineBreakParams {
            pretolerance: 100,
            tolerance: 1000,
            line_penalty: 10,
            adj_demerits: 10_000,
            double_hyphen_demerits: 10_000,
            final_hyphen_demerits: 5_000,
            emergency_stretch: sp(0),
            looseness: 0,
            hsize: sp(width),
        }
    }

    #[test]
    fn breaks_at_legal_glue() {
        let mut universe = Universe::new();
        let glue = universe.intern_glue(GlueSpec {
            width: sp(10),
            stretch: sp(10),
            stretch_order: Order::Normal,
            shrink: sp(5),
            shrink_order: Order::Normal,
        });
        let nodes = vec![
            Node::Kern {
                amount: sp(20),
                kind: KernKind::Explicit,
            },
            Node::Glue {
                spec: glue,
                kind: GlueKind::Normal,
            },
            Node::Kern {
                amount: sp(20),
                kind: KernKind::Explicit,
            },
            Node::Glue {
                spec: glue,
                kind: GlueKind::Normal,
            },
            Node::Kern {
                amount: sp(20),
                kind: KernKind::Explicit,
            },
        ];
        let mut hook = NoHyphenation;
        let result = line_break(&universe, &nodes, params(30), &mut hook);
        assert_eq!(
            result.breaks.last().map(|br| br.position),
            Some(nodes.len())
        );
    }
}
