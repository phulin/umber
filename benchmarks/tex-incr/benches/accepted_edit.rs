use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use tex_incr::{Edit, RevisionId, Session};
use tex_state::ContentHash;

const RULES: usize = 128;

fn accepted_edit(c: &mut Criterion) {
    let mut group = c.benchmark_group("two_generation_edit");
    group.sample_size(20);
    group.bench_function("accept_current_and_drop_prior", |b| {
        b.iter_batched(
            prepared_session,
            |(mut session, edit)| {
                let accepted = session
                    .advance(RevisionId::new(2), edit)
                    .expect("benchmark edit accepts");
                assert_eq!(session.retained_generation_count(), 1);
                assert_eq!(session.retired_generation_count(), 1);
                black_box(accepted);
            },
            BatchSize::SmallInput,
        )
    });
    group.bench_function("reject_current_and_preserve_prior", |b| {
        b.iter_batched(
            prepared_session,
            |(session, edit)| {
                let candidate = session
                    .start_advance_candidate(RevisionId::new(2), edit)
                    .expect("benchmark candidate starts");
                black_box(candidate);
                assert_eq!(session.retained_generation_count(), 1);
                assert_eq!(session.retired_generation_count(), 0);
            },
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

fn prepared_session() -> (Session, Edit) {
    let source = source();
    let mut session = Session::start(
        (),
        "two-generation-edit",
        RevisionId::new(1),
        source.clone(),
        usize::MAX,
    )
    .expect("benchmark session starts");
    session.cold().expect("benchmark cold run");
    assert_eq!(session.retained_generation_count(), 1);
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

criterion_group!(benches, accepted_edit);
criterion_main!(benches);
