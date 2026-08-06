use std::collections::BTreeSet;
use std::fs;

use test_support::closed_case::FixtureCase;

const FAMILIES: &[(&str, usize, usize, usize)] = &[
    ("canonical-dvi", 2, 4, 706),
    ("hello", 1, 2, 249),
    ("lexer", 6, 12, 621),
    ("lexer_dynamic", 4, 8, 784),
    ("stabilization", 2, 2, 1_616),
];

#[test]
fn lexical_session_minifixtures_are_closed_tracked_directories() {
    let repository = test_support::repository_root();
    let corpus = repository.join("tests/corpus");
    let mut identities = BTreeSet::new();
    let mut total_cases = 0;
    let mut total_files = 0;
    let mut total_bytes = 0;

    for &(area, expected_cases, expected_files, expected_bytes) in FAMILIES {
        let area_path = corpus.join(area);
        let mut family_cases = 0;
        let mut family_files = 0;
        let mut family_bytes = 0;
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
                "source.tex",
                (*area).to_owned(),
            )
            .unwrap_or_else(|error| panic!("{area}/{case} is not closed: {error:#}"));
            closed
                .read("source.tex")
                .unwrap_or_else(|error| panic!("{area}/{case} lacks source.tex: {error:#}"));

            family_cases += 1;
            for file in fs::read_dir(entry.path()).expect("read closed case") {
                let file = file.expect("read case entry");
                family_files += 1;
                family_bytes += file.metadata().expect("read case metadata").len() as usize;
            }
        }
        assert_eq!(family_cases, expected_cases, "{area} case census changed");
        assert_eq!(family_files, expected_files, "{area} file census changed");
        assert_eq!(family_bytes, expected_bytes, "{area} byte census changed");
        total_cases += family_cases;
        total_files += family_files;
        total_bytes += family_bytes;
    }
    assert_eq!((total_cases, total_files, total_bytes), (15, 28, 3_976));
}
