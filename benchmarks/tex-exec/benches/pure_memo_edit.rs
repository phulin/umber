use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use tex_incr::{Edit, RevisionId, Session};
use tex_state::{ContentHash, PureMemoConfig, Universe};

const RULES: usize = 128;

fn pure_memo_edit(c: &mut Criterion) {
    let mut group = c.benchmark_group("pure_memo_accepted_edit");
    group.sample_size(20);
    group.bench_function("disabled", |b| {
        b.iter_batched(
            || prepared_session(false),
            |(mut session, edit)| black_box(session.advance(RevisionId::new(2), edit)),
            BatchSize::SmallInput,
        )
    });
    group.bench_function("enabled", |b| {
        b.iter_batched(
            || prepared_session(true),
            |(mut session, edit)| black_box(session.advance(RevisionId::new(2), edit)),
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

fn prepared_session(enabled: bool) -> (Session, Edit) {
    let mut template = Universe::new();
    tex_expand::install_expandable_primitives(&mut template);
    tex_exec::install_unexpandable_primitives(&mut template);
    if enabled {
        template.enable_pure_memo(PureMemoConfig::default());
    }
    let source = source();
    let mut session = Session::start(
        template,
        "pure-memo-edit",
        RevisionId::new(1),
        source.clone(),
        usize::MAX,
    )
    .expect("benchmark session starts");
    session.cold().expect("benchmark cold run");
    let first_width = source.find("width1pt").expect("first rule width");
    let digit = first_width + "width".len();
    let edit = Edit {
        base_revision: RevisionId::new(1),
        expected_hash: ContentHash::from_bytes(source.as_bytes()),
        range: digit..digit + 1,
        replacement: "2".to_owned(),
    };
    (session, edit)
}

fn source() -> String {
    let paragraph = (0..RULES)
        .map(|_| "\\vrule width1pt height1pt depth0pt")
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "\\hsize=20pt\\pretolerance=10000 {paragraph}\\par\n\
         \\prevgraf=0 {paragraph}\\par\n\
         \\vfill\\eject\\end"
    )
}

criterion_group!(benches, pure_memo_edit);
criterion_main!(benches);
