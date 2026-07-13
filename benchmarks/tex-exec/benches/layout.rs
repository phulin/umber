use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use tex_state::Universe;
use tex_state::glue::GlueSpec;
use tex_state::math::{MathChoice, MathField, MathNoad, NoadClass, NoadKind};
use tex_state::node::Node;
use tex_state::scaled::Scaled;
use tex_typeset::alignment::{AlignmentWidthRequirement, plan_alignment_widths};
use tex_typeset::linebreak::{LineBreakParams, LineShape, try_line_break_without_hyphenation};
use tex_typeset::math::{MathParams, Style, mlist_to_hlist};

const ALIGN_COLUMNS: usize = 512;
const ALIGN_CELLS: usize = 4_096;
const PARAGRAPH_NODES: usize = 1_024;
const DEEP_MATH_LEVELS: usize = 20_000;
const REPEATED_MATH_NOADS: usize = 1_024;

fn layout(c: &mut Criterion) {
    alignment(c);
    linebreak(c);
    math(c);
}

fn alignment(c: &mut Criterion) {
    let tabskips = vec![Scaled::from_raw(17); ALIGN_COLUMNS + 1];
    let requirements = (0..ALIGN_CELLS)
        .map(|index| {
            let first_column = index % ALIGN_COLUMNS;
            AlignmentWidthRequirement {
                first_column,
                span: (1 + index % 32).min(ALIGN_COLUMNS - first_column),
                width: Scaled::from_raw(10_000 + index as i32),
            }
        })
        .collect::<Vec<_>>();
    let mut group = c.benchmark_group("alignment_width_planning");
    group.throughput(Throughput::Elements(ALIGN_CELLS as u64));
    group.bench_function("many_spans", |b| {
        b.iter(|| {
            black_box(
                plan_alignment_widths(
                    ALIGN_COLUMNS,
                    black_box(&tabskips),
                    black_box(requirements.iter().copied()),
                )
                .expect("adversarial alignment plan"),
            )
        })
    });
    group.finish();
}

fn linebreak(c: &mut Criterion) {
    let mut state = Universe::new();
    let glue = state.intern_glue(GlueSpec {
        width: Scaled::from_raw(10),
        stretch: Scaled::from_raw(5),
        ..GlueSpec::ZERO
    });
    let nodes = (0..PARAGRAPH_NODES)
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
    let params = line_params();
    let mut group = c.benchmark_group("linebreak_frontier");
    group.throughput(Throughput::Elements(PARAGRAPH_NODES as u64));
    group.bench_function("long_paragraph", |b| {
        b.iter(|| {
            black_box(try_line_break_without_hyphenation(
                black_box(&state),
                black_box(&nodes),
                black_box(&params),
            ))
        })
    });
    group.finish();
}

fn math(c: &mut Criterion) {
    let (deep_state, deep) = deep_math_choices(DEEP_MATH_LEVELS);
    let deep_params = MathParams::read(&deep_state);
    let mut group = c.benchmark_group("math_conversion");
    group.throughput(Throughput::Elements(DEEP_MATH_LEVELS as u64));
    group.bench_function("deep_choice_stack", |b| {
        b.iter(|| {
            black_box(mlist_to_hlist(
                black_box(&deep_state),
                deep,
                Style::TEXT,
                false,
                black_box(&deep_params),
            ))
        })
    });

    let (nested_state, nested) = deep_math_submlists(DEEP_MATH_LEVELS);
    let nested_params = MathParams::read(&nested_state);
    group.bench_function("deep_submlist_stack", |b| {
        b.iter(|| {
            black_box(mlist_to_hlist(
                black_box(&nested_state),
                nested,
                Style::TEXT,
                false,
                black_box(&nested_params),
            ))
        })
    });

    let mut repeated_state = Universe::new();
    let noads = (0..REPEATED_MATH_NOADS)
        .map(|_| {
            Node::MathNoad(MathNoad::new(
                NoadKind::Normal(NoadClass::Ord),
                MathField::Empty,
            ))
        })
        .collect::<Vec<_>>();
    let repeated = repeated_state.freeze_node_list(&noads);
    let repeated_params = MathParams::read(&repeated_state);
    group.throughput(Throughput::Elements(REPEATED_MATH_NOADS as u64));
    group.bench_function("repeated_flat_layout", |b| {
        b.iter(|| {
            black_box(mlist_to_hlist(
                black_box(&repeated_state),
                repeated,
                Style::TEXT,
                false,
                black_box(&repeated_params),
            ))
        })
    });
    group.finish();
}

fn deep_math_choices(levels: usize) -> (Universe, tex_state::ids::NodeListId) {
    let mut state = Universe::new();
    let mut selected = state.freeze_node_list(&[]);
    for _ in 0..levels {
        selected = state.freeze_node_list(&[Node::MathChoice(MathChoice {
            display: selected,
            text: selected,
            script: selected,
            script_script: selected,
        })]);
    }
    (state, selected)
}

fn deep_math_submlists(levels: usize) -> (Universe, tex_state::ids::NodeListId) {
    let mut state = Universe::new();
    let mut nested = state.freeze_node_list(&[]);
    for _ in 0..levels {
        nested = state.freeze_node_list(&[Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::SubMlist(nested),
        ))]);
    }
    (state, nested)
}

fn line_params() -> LineBreakParams {
    LineBreakParams {
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
        left_skip: GlueSpec::ZERO,
        right_skip: GlueSpec::ZERO,
        par_fill_skip: GlueSpec::ZERO,
        shape: LineShape::natural(Scaled::from_raw(1_000)),
    }
}

criterion_group!(benches, layout);
criterion_main!(benches);
