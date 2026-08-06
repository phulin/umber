use std::collections::BTreeSet;
use std::fs;

use test_support::closed_case::FixtureCase;

const AREAS: &[&str] = &[
    "align",
    "etex_exec",
    "exec",
    "expand",
    "math",
    "tex_exec",
    "tex_exec_io",
    "typeset",
];

#[test]
fn execution_minifixtures_are_closed_tracked_directories() {
    let repository = test_support::repository_root();
    let corpus = repository.join("tests/corpus");
    let mut identities = BTreeSet::new();

    for area in AREAS {
        let area_path = corpus.join(area);
        for entry in fs::read_dir(&area_path).expect("read fixture area") {
            let entry = entry.expect("read fixture-area entry");
            let file_type = entry.file_type().expect("read fixture-area entry type");
            assert!(
                file_type.is_dir() && !file_type.is_symlink(),
                "{} must contain only regular case directories",
                area_path.display()
            );
            let case = entry.file_name().into_string().expect("case name is UTF-8");
            assert!(
                identities.insert(format!("{area}/{case}")),
                "duplicate case {area}/{case}"
            );
            let closed = FixtureCase::discover_tracked_at(
                &repository,
                format!("tests/corpus/{area}/{case}"),
                format!("{case}.tex"),
                (*area).to_owned(),
            )
            .unwrap_or_else(|error| panic!("{area}/{case} is not closed: {error:#}"));
            closed
                .read(&format!("{case}.tex"))
                .unwrap_or_else(|error| panic!("{area}/{case} lacks its source: {error:#}"));
        }
    }
    assert_eq!(identities.len(), 123, "execution case census changed");
}
