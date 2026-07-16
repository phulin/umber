use std::alloc::System;

use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};
use tex_state::Universe;
use tex_state::glue::GlueSpec;
use tex_state::math::{MathChoice, MathField, MathNoad, NoadClass, NoadKind};
use tex_state::node::Node;
use tex_state::scaled::Scaled;
use tex_typeset::alignment::{AlignmentWidthRequirement, plan_alignment_widths};
use tex_typeset::linebreak::{LineBreakParams, LineShape, try_line_break_without_hyphenation};
use tex_typeset::math::{MathParams, Style, mlist_to_hlist};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const ALIGNMENT_ALLOCATION_BUDGET: usize = 1_200;
const LINEBREAK_ALLOCATION_BUDGET: usize = 12;
const DEEP_MATH_ALLOCATION_BUDGET: usize = 28;
const FLAT_MATH_ALLOCATION_BUDGET: usize = 14;
const ALIGNMENT_BYTE_BUDGET: usize = 220_000;
const LINEBREAK_BYTE_BUDGET: usize = 19_000_000_000;
const DEEP_MATH_BYTE_BUDGET: usize = 14_000_000;
const FLAT_MATH_BYTE_BUDGET: usize = 560_000;
const DEEP_SUBMLIST_ALLOCATION_BUDGET: usize = 180_000;
const DEEP_SUBMLIST_BYTE_BUDGET: usize = 42_000_000;

fn main() {
    let alignment = alignment_allocations();
    let linebreak = linebreak_allocations();
    let deep_math = deep_math_allocations();
    let deep_submlist = deep_submlist_allocations();
    let flat_math = flat_math_allocations();
    print_stats("alignment_many_spans", alignment);
    print_stats("linebreak_long_paragraph", linebreak);
    print_stats("math_deep_choice_stack", deep_math);
    print_stats("math_deep_submlist_stack", deep_submlist);
    print_stats("math_repeated_flat_layout", flat_math);
    assert!(alignment.allocations <= ALIGNMENT_ALLOCATION_BUDGET);
    assert!(alignment.bytes_allocated <= ALIGNMENT_BYTE_BUDGET);
    assert!(linebreak.allocations <= LINEBREAK_ALLOCATION_BUDGET);
    assert!(linebreak.bytes_allocated <= LINEBREAK_BYTE_BUDGET);
    assert!(deep_math.allocations <= DEEP_MATH_ALLOCATION_BUDGET);
    assert!(deep_math.bytes_allocated <= DEEP_MATH_BYTE_BUDGET);
    assert!(deep_submlist.allocations <= DEEP_SUBMLIST_ALLOCATION_BUDGET);
    assert!(deep_submlist.bytes_allocated <= DEEP_SUBMLIST_BYTE_BUDGET);
    assert!(flat_math.allocations <= FLAT_MATH_ALLOCATION_BUDGET);
    assert!(flat_math.bytes_allocated <= FLAT_MATH_BYTE_BUDGET);
}

fn print_stats(name: &str, stats: Stats) {
    println!(
        "{name} allocations={} bytes_allocated={}",
        stats.allocations, stats.bytes_allocated
    );
}

fn linebreak_allocations() -> Stats {
    let mut state = Universe::new();
    let glue = state.intern_glue(GlueSpec {
        width: Scaled::from_raw(10),
        stretch: Scaled::from_raw(5),
        ..GlueSpec::ZERO
    });
    let nodes = (0..4_096)
        .map(|index| {
            if index % 2 == 0 {
                Node::Rule {
                    width: Some(Scaled::from_raw(20)),
                    height: Some(Scaled::from_raw(10)),
                    depth: Some(Scaled::from_raw(0)),
                }
            } else {
                Node::Glue {
                    spec: glue,
                    kind: tex_state::node::GlueKind::Normal,
                    leader: None,
                }
            }
        })
        .collect::<Vec<_>>();
    let params = LineBreakParams {
        pretolerance: 10_000,
        tolerance: 10_000,
        line_penalty: 10,
        hyphen_penalty: 50,
        ex_hyphen_penalty: 50,
        adj_demerits: 10_000,
        double_hyphen_demerits: 10_000,
        final_hyphen_demerits: 5_000,
        emergency_stretch: Scaled::from_raw(0),
        looseness: 0,
        last_line_fit: 0,
        pdf_adjust_spacing: 0,
        pdf_protrude_chars: 0,
        left_skip: GlueSpec::ZERO,
        right_skip: GlueSpec::ZERO,
        par_fill_skip: GlueSpec::ZERO,
        shape: LineShape::natural(Scaled::from_raw(1_000)),
    };
    let region = Region::new(GLOBAL);
    let result = try_line_break_without_hyphenation(&state, &nodes, &params);
    std::hint::black_box(result);
    region.change()
}

fn alignment_allocations() -> Stats {
    let tabskips = vec![Scaled::from_raw(17); 513];
    let requirements = (0..4_096)
        .map(|index| {
            let first_column = index % 512;
            AlignmentWidthRequirement {
                first_column,
                span: (1 + index % 32).min(512 - first_column),
                width: Scaled::from_raw(10_000 + index as i32),
            }
        })
        .collect::<Vec<_>>();
    let region = Region::new(GLOBAL);
    let plan = plan_alignment_widths(512, &tabskips, requirements.iter().copied())
        .expect("allocation-gate alignment plan");
    std::hint::black_box(plan);
    region.change()
}

fn deep_math_allocations() -> Stats {
    let mut state = Universe::new();
    let mut selected = state.freeze_node_list(&[]);
    for _ in 0..20_000 {
        selected = state.freeze_node_list(&[Node::MathChoice(MathChoice {
            display: selected,
            text: selected,
            script: selected,
            script_script: selected,
        })]);
    }
    let params = MathParams::read(&state);
    let region = Region::new(GLOBAL);
    let layout = mlist_to_hlist(&state, selected, Style::TEXT, false, &params);
    std::hint::black_box(layout);
    region.change()
}

fn flat_math_allocations() -> Stats {
    let mut state = Universe::new();
    let nodes = (0..1_024)
        .map(|_| {
            Node::MathNoad(MathNoad::new(
                NoadKind::Normal(NoadClass::Ord),
                MathField::Empty,
            ))
        })
        .collect::<Vec<_>>();
    let list = state.freeze_node_list(&nodes);
    let params = MathParams::read(&state);
    let region = Region::new(GLOBAL);
    let layout = mlist_to_hlist(&state, list, Style::TEXT, false, &params);
    std::hint::black_box(layout);
    region.change()
}

fn deep_submlist_allocations() -> Stats {
    let mut state = Universe::new();
    let mut nested = state.freeze_node_list(&[]);
    for _ in 0..20_000 {
        nested = state.freeze_node_list(&[Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::SubMlist(nested),
        ))]);
    }
    let params = MathParams::read(&state);
    let region = Region::new(GLOBAL);
    let layout = mlist_to_hlist(&state, nested, Style::TEXT, false, &params);
    std::hint::black_box(layout);
    region.change()
}
